//! Which of a buffer's attached servers a request goes to, decided when it
//! is sent: the one place a [`RouteSpec`] meets the live attachments, their
//! feature filters and each server's capabilities.

use hume_engine::pipeline::BufferId;
use hume_lsp::backend::ServerId;
use hume_lsp::client::ServerState;
use hume_rope::position_encoding::PositionEncoding;
use hume_scripting::{LspFeature, RouteSpec, ServerName, ServerRef};

use super::features::{advertises, capability_at, requirement};
use crate::editor::EditorState;

/// One server a request is routed to, with what serializing for it needs.
#[derive(Debug, Clone)]
pub(in crate::editor) struct Routed {
    pub(in crate::editor) server: ServerRef,
    pub(in crate::editor) encoding: PositionEncoding,
}

/// Why a route reached no server. Its `Display` is the error a request's
/// callback receives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::editor) enum Unavailable {
    Closed,
    NoServer,
    NotAttached(ServerName),
    /// The name is attached, as a different instance than the server value
    /// the request named.
    Restarted(ServerName),
    /// A `#:to` server whose list entry excludes the feature.
    Excluded {
        feature: LspFeature,
        servers: Vec<ServerName>,
    },
    Starting(Vec<ServerName>),
    Crashed(Vec<ServerName>),
    Unsupported {
        what: String,
        servers: Vec<ServerName>,
    },
}

impl std::fmt::Display for Unavailable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let names = |servers: &[ServerName]| {
            servers
                .iter()
                .map(ServerName::as_str)
                .collect::<Vec<_>>()
                .join(", ")
        };
        match self {
            Unavailable::Closed => write!(f, "the buffer is closed"),
            Unavailable::NoServer => write!(f, "no LSP server attached to this buffer"),
            Unavailable::NotAttached(name) => write!(f, "{name} is not attached to this buffer"),
            Unavailable::Restarted(name) => write!(
                f,
                "{name} was restarted; the server value taken before is stale"
            ),
            Unavailable::Excluded { feature, servers } => write!(
                f,
                "{} is excluded for {} by the language's server list",
                feature.name(),
                names(servers)
            ),
            Unavailable::Starting(servers) => write!(f, "{} still starting", names(servers)),
            Unavailable::Crashed(servers) => {
                let texts: Vec<String> = servers
                    .iter()
                    .map(|name| super::crashed_text(name.as_str(), None))
                    .collect();
                write!(f, "{}", texts.join("; "))
            }
            Unavailable::Unsupported { what, servers } => {
                write!(f, "{what} is not supported by {}", names(servers))
            }
        }
    }
}

/// The servers `spec` admits among `bid`'s attachments, in attachment
/// order, every one `Running`. A starting server is never routed to: its
/// capabilities are not known yet. A crashed one is skipped, and named in
/// the error when no other server can take the request. A server whose
/// list entry excludes the feature is skipped without being named in the
/// error, since the user chose that, unless `#:to` named it.
pub(in crate::editor) fn route(
    state: &EditorState,
    bid: BufferId,
    method: Option<&str>,
    spec: &RouteSpec,
) -> Result<Vec<Routed>, Unavailable> {
    if state.buffers.try_get(bid).is_none() {
        return Err(Unavailable::Closed);
    }
    let attachments = state.buffer_positions.lsp.attachments(bid);
    if let Some(to) = &spec.to
        && !attachments.iter().any(|a| a.server == to.id)
    {
        let replaced = attachments.iter().any(|a| {
            state
                .lsp
                .instances
                .get(a.server)
                .is_some_and(|instance| instance.name == to.name)
        });
        return Err(if replaced {
            Unavailable::Restarted(to.name.clone())
        } else {
            Unavailable::NotAttached(to.name.clone())
        });
    }
    let required = method.and_then(requirement);
    let feature = spec.feature.or(required.as_ref().map(|r| r.feature));
    let capability = required.as_ref().and_then(|r| r.capability);
    let mut routed = Vec::new();
    let mut starting = Vec::new();
    let mut crashed = Vec::new();
    let mut excluded = Vec::new();
    let mut unsupported = Vec::new();
    for att in attachments
        .iter()
        .filter(|a| spec.to.as_ref().is_none_or(|to| a.server == to.id))
    {
        let Some(instance) = state.lsp.instances.get(att.server) else {
            continue;
        };
        match instance.client.state() {
            ServerState::Starting => {
                starting.push(instance.name.clone());
                continue;
            }
            ServerState::Running => {}
            ServerState::Crashed => {
                crashed.push(instance.name.clone());
                continue;
            }
            ServerState::Dead => continue,
        }
        if feature.is_some_and(|f| !att.filter.admits(f)) {
            if spec.to.is_some() {
                excluded.push(instance.name.clone());
            }
            continue;
        }
        let capabilities = instance
            .client
            .capabilities_json()
            .map(|caps| caps.as_ref())
            .unwrap_or(&serde_json::Value::Null);
        let supported = feature.is_none_or(|f| advertises(f, capabilities))
            && capability.is_none_or(|path| capability_at(capabilities, path).is_some());
        if !supported {
            unsupported.push(instance.name.clone());
            continue;
        }
        routed.push(Routed {
            server: instance.server_ref(att.server),
            encoding: instance.client.encoding(),
        });
    }
    if !routed.is_empty() {
        Ok(routed)
    } else if !starting.is_empty() {
        Err(Unavailable::Starting(starting))
    } else if !crashed.is_empty() {
        Err(Unavailable::Crashed(crashed))
    } else if let (Some(feature), false) = (feature, excluded.is_empty()) {
        Err(Unavailable::Excluded {
            feature,
            servers: excluded,
        })
    } else if !unsupported.is_empty() {
        Err(Unavailable::Unsupported {
            what: match (feature, capability) {
                (Some(feature), _) => feature.name().to_string(),
                (None, Some(path)) => path.join("."),
                (None, None) => "this request".to_string(),
            },
            servers: unsupported,
        })
    } else {
        Err(Unavailable::NoServer)
    }
}

impl EditorState {
    /// Whether `sid` is attached to `bid`, running, its list entry admits
    /// `feature` and it advertises it: the condition under which [`route`]
    /// would send a `feature` request to it.
    pub(in crate::editor) fn lsp_handles(
        &self,
        bid: BufferId,
        sid: ServerId,
        feature: LspFeature,
    ) -> bool {
        self.buffer_positions.lsp.admits(bid, sid, feature)
            && self.lsp.instances.is_running(sid)
            && self
                .lsp
                .instances
                .get(sid)
                .and_then(|instance| instance.client.capabilities_json())
                .is_some_and(|caps| advertises(feature, caps))
    }
}
