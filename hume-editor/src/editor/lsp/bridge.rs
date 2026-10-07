//! Steel's requests and notifications: routed among the buffer's servers
//! when sent, serialized for each one, and answered through a [`Delivery`]
//! whose callback runs once every server it went to has answered, failed or
//! timed out. `lsp-request!` and `lsp-request-all!` are the two ways a
//! delivery calls back ([`RequestMode`]); everything else is shared.

use std::sync::Arc;
use std::time::{Duration, Instant};

use rustc_hash::FxHashMap;

use hume_editing::text::{BufferText, TextVersion};
use hume_engine::pipeline::{BufferId, EngineView};
use hume_lsp::backend::ServerId;
use hume_lsp::client::{Outcome, RequestMeta};
use hume_lsp::codec::RequestId;
use hume_rope::offset::ExclusiveRange;
use hume_rope::position_encoding::{PositionEncoding, char_range_to_wire_range, char_to_wire};
use hume_scripting::json::{WireOrigin, steel_to_json_with, to_steel_handle};
use hume_scripting::{
    DocPos, DocRange, Params, PendingLspNotify, PendingLspRequest, RequestMode, RequestParams,
    RouteSpec, ServerRef, WhenUnavailable, symbol_hash,
};
use steel::rvals::SteelVal;

use super::ResponseAnchor;
use super::route::{Routed, route};
use crate::editor::EditorState;
use crate::editor::message_log::Severity;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(in crate::editor) struct DeliveryId(u64);

/// Why a request has no result, from one server or from all of them.
enum SlotError {
    /// The server answered with a JSON-RPC error.
    Response { code: i64, message: String },
    /// No answer came before the deadline.
    TimedOut,
    /// The server stopped or crashed before it answered.
    Stopped,
    /// No server could take the request, or the named one is not attached
    /// and running.
    Unavailable(String),
    /// The params could not be serialized for the server.
    Unsent(String),
}

impl SlotError {
    /// `(kind, message, code)`: the `'kind` a caller tells a request no
    /// server could take from one that failed, its message, and a server's
    /// own error code.
    fn parts(self) -> (&'static str, String, Option<i64>) {
        match self {
            SlotError::Response { code, message } => ("server", message, Some(code)),
            SlotError::TimedOut => ("timeout", "timed out".to_string(), None),
            SlotError::Stopped => (
                "stopped",
                "server stopped before answering".to_string(),
                None,
            ),
            SlotError::Unavailable(message) => ("unavailable", message, None),
            SlotError::Unsent(message) => ("unsent", message, None),
        }
    }

    /// The message a Rust responder reports.
    fn describe(self) -> String {
        match self.parts() {
            (_, message, Some(code)) => format!("{message} ({code})"),
            (_, message, None) => message,
        }
    }

    /// The `err` a callback receives: `(hash 'kind k 'message m)`, with
    /// `'code` for a server's own error.
    fn into_steel(self) -> SteelVal {
        let (kind, message, code) = self.parts();
        symbol_hash(
            [
                ("kind", SteelVal::SymbolV(kind.into())),
                ("message", SteelVal::StringV(message.into())),
            ]
            .into_iter()
            .chain(code.map(|code| ("code", SteelVal::IntV(code as isize)))),
        )
    }
}

/// One server's part of a delivery.
struct Slot {
    server: ServerRef,
    state: SlotState,
}

/// Where one server's part of a delivery stands. A slot is sent with the
/// encoding its server negotiated, which its answer's positions are in.
enum SlotState {
    Waiting {
        request: RequestId,
        encoding: PositionEncoding,
    },
    Answered {
        value: serde_json::Value,
        encoding: PositionEncoding,
    },
    Failed(SlotError),
}

impl Slot {
    fn failed(server: ServerRef, error: SlotError) -> Self {
        Self {
            server,
            state: SlotState::Failed(error),
        }
    }

    fn waiting(&self) -> bool {
        matches!(self.state, SlotState::Waiting { .. })
    }
}

/// What a delivery calls back once its last slot is filled.
enum Responder {
    /// A Steel callback, given `(err result)` or `(err results)` by mode.
    Steel(SteelVal),
    /// A Rust closure, given the one server's answer or why it has none.
    Rust(RustResponder),
}

/// The Rust side of a [`Responder`]: runs once, with the answer's JSON as
/// the server sent it.
pub(in crate::editor) type RustResponder =
    Box<dyn FnOnce(&mut EditorState, &EngineView, Result<serde_json::Value, String>)>;

/// What a delivery needs beyond its members, whoever asked for it.
struct DeliverySpec {
    bid: BufferId,
    method: String,
    mode: RequestMode,
    supersede: Option<String>,
    allow_stale: bool,
    require_focus: Option<hume_engine::pipeline::PaneId>,
    tracked: Option<hume_scripting::host::HostToken>,
    responder: Responder,
}

/// A request in flight to one or more servers, and the responder waiting
/// for all of them.
struct Delivery {
    anchor: ResponseAnchor,
    responder: Responder,
    mode: RequestMode,
    supersede: Option<String>,
    slots: Vec<Slot>,
}

/// Every delivery in flight. A server's answer finds its slot by scanning:
/// a handful of deliveries are in flight at once.
#[derive(Default)]
pub(in crate::editor) struct Deliveries {
    map: FxHashMap<DeliveryId, Delivery>,
    next: u64,
}

/// What a server's answer did to the delivery waiting for it.
enum Fill {
    /// No delivery waits for the request; the outcome is handed back.
    Unknown(Outcome),
    /// The slot is filled and others still wait.
    Waiting,
    /// The slot was the delivery's last; the delivery is ready to finish.
    Complete(DeliveryId),
}

impl Deliveries {
    fn next_id(&mut self) -> DeliveryId {
        let did = DeliveryId(self.next);
        self.next += 1;
        did
    }

    /// Records `server`'s answer to `request` in the slot waiting for it.
    fn fill(&mut self, server: ServerId, request: RequestId, outcome: Outcome) -> Fill {
        let found = self.map.iter_mut().find_map(|(&did, delivery)| {
            let hit =
                delivery
                    .slots
                    .iter()
                    .enumerate()
                    .find_map(|(index, slot)| match &slot.state {
                        SlotState::Waiting {
                            request: waiting,
                            encoding,
                        } if slot.server.id == server && *waiting == request => {
                            Some((index, *encoding))
                        }
                        _ => None,
                    });
            hit.map(|(index, encoding)| (did, delivery, index, encoding))
        });
        let Some((did, delivery, index, encoding)) = found else {
            return Fill::Unknown(outcome);
        };
        delivery.slots[index].state = match outcome {
            Outcome::Ok(value) => SlotState::Answered { value, encoding },
            Outcome::Err(e) => SlotState::Failed(SlotError::Response {
                code: e.code,
                message: e.message,
            }),
            Outcome::TimedOut => SlotState::Failed(SlotError::TimedOut),
            Outcome::Stopped => SlotState::Failed(SlotError::Stopped),
        };
        if delivery.slots.iter().any(Slot::waiting) {
            Fill::Waiting
        } else {
            Fill::Complete(did)
        }
    }

    pub(in crate::editor) fn clear(&mut self) {
        self.map.clear();
    }

    /// The delivery still in flight under `key`. A new request under the
    /// same key cancels it first, so at most one delivery holds a key.
    fn in_flight_under(&self, key: &str) -> Option<DeliveryId> {
        self.map
            .iter()
            .find(|(_, delivery)| delivery.supersede.as_deref() == Some(key))
            .map(|(&did, _)| did)
    }

    #[cfg(test)]
    pub(in crate::editor) fn len(&self) -> usize {
        self.map.len()
    }

    #[cfg(test)]
    pub(in crate::editor) fn waiting_requests(&self) -> usize {
        self.map
            .values()
            .flat_map(|delivery| &delivery.slots)
            .filter(|slot| slot.waiting())
            .count()
    }
}

impl EditorState {
    /// Sends one queued `lsp-request!`/`lsp-request-all!`. A request no
    /// server can take calls back at once with the reason; otherwise every
    /// member server gets its own serialization of the params, and one
    /// that cannot be serialized for fills its slot with the error.
    pub(in crate::editor) fn lsp_send_request(
        &mut self,
        view: &EngineView,
        req: PendingLspRequest,
    ) {
        self.lsp_flush_pending();
        if let Some(key) = &req.supersede
            && let Some(old) = self.lsp.deliveries.in_flight_under(key)
        {
            self.cancel_delivery(old);
        }
        let members = match self.request_members(&req) {
            Ok(members) => members,
            Err(reason) => {
                self.fail_request(&req, reason);
                self.release_request_position(req.tracked);
                return;
            }
        };
        let spec = DeliverySpec {
            bid: req.bid,
            method: req.method,
            mode: req.mode,
            supersede: req.supersede,
            allow_stale: req.allow_stale,
            require_focus: req.require_focus,
            tracked: req.tracked,
            responder: Responder::Steel(req.callback),
        };
        if let Some(did) = self.begin_delivery(spec, members) {
            self.finish_delivery(view, did);
        }
    }

    /// Sends `method` with wire-form `params` to `server` alone, and calls
    /// `responder` with its answer. A server that is not attached to `bid`
    /// and running sends nothing and never calls back.
    pub(in crate::editor) fn lsp_request_json(
        &mut self,
        bid: BufferId,
        server: hume_lsp::backend::ServerId,
        method: &str,
        params: serde_json::Value,
        allow_stale: bool,
        responder: RustResponder,
    ) {
        let Some(server) = self.lsp.instances.server_ref(server) else {
            return;
        };
        let spec = RouteSpec {
            to: Some(server),
            ..RouteSpec::default()
        };
        let Ok(mut routed) = route(self, bid, Some(method), &spec) else {
            return;
        };
        self.lsp_flush_pending();
        let spec = DeliverySpec {
            bid,
            method: method.to_string(),
            mode: RequestMode::Single,
            supersede: None,
            allow_stale,
            require_focus: None,
            tracked: None,
            responder: Responder::Rust(responder),
        };
        let members = vec![Member::Send(routed.remove(0), params)];
        if let Some(did) = self.begin_delivery(spec, members) {
            // Nothing is in flight, so the responder has no answer to give.
            self.cancel_delivery(did);
        }
    }

    /// Sends to every member and files the delivery. `Some` when no member
    /// is waiting, so the caller completes it now.
    fn begin_delivery(&mut self, spec: DeliverySpec, members: Vec<Member>) -> Option<DeliveryId> {
        let anchor = ResponseAnchor {
            bid: spec.bid,
            version: self.buffers.get(spec.bid).text().version(),
            allow_stale: spec.allow_stale,
            require_focus: spec.require_focus,
            tracked: spec.tracked,
        };

        let did = self.lsp.deliveries.next_id();
        let deadline =
            Instant::now() + Duration::from_millis(self.settings.lsp_request_timeout_ms as u64);
        let mut slots = Vec::with_capacity(members.len());
        for member in members {
            slots.push(match member {
                Member::Send(routed, json) => self.send_slot(&spec.method, routed, json, deadline),
                Member::Failed(server, error) => Slot::failed(server, error),
            });
        }
        let complete = !slots.iter().any(Slot::waiting);
        self.lsp.deliveries.map.insert(
            did,
            Delivery {
                anchor,
                responder: spec.responder,
                mode: spec.mode,
                supersede: spec.supersede,
                slots,
            },
        );
        complete.then_some(did)
    }

    /// The servers `req` goes to, each with its params, or why none can
    /// take it.
    fn request_members(&self, req: &PendingLspRequest) -> Result<Vec<Member>, String> {
        if self.buffers.try_get(req.bid).is_none() {
            return Err(super::route::Unavailable::Closed.to_string());
        }
        let verb = req.mode.verb();
        let method = Some(req.method.as_str());
        match &req.params {
            RequestParams::Shared(params) => {
                let mut routed =
                    route(self, req.bid, method, &req.route).map_err(|u| u.to_string())?;
                if req.mode == RequestMode::Single {
                    routed.truncate(1);
                }
                let jsons = self.serialize_for(req.bid, params, &routed);
                Ok(routed
                    .into_iter()
                    .zip(jsons)
                    .map(|(r, json)| Member::new(r, json, verb))
                    .collect())
            }
            RequestParams::PerServer(pairs) => Ok(pairs
                .iter()
                .map(|(server, params)| {
                    let spec = RouteSpec {
                        to: Some(server.clone()),
                        ..req.route.clone()
                    };
                    match route(self, req.bid, method, &spec) {
                        Ok(mut routed) => {
                            let r = routed.remove(0);
                            let json = self
                                .serialize_for(req.bid, params, std::slice::from_ref(&r))
                                .remove(0);
                            Member::new(r, json, verb)
                        }
                        Err(u) => {
                            Member::Failed(server.clone(), SlotError::Unavailable(u.to_string()))
                        }
                    }
                })
                .collect()),
        }
    }

    /// `params` as each of `routed` receives it, in order: reused whole
    /// when it holds no position, otherwise converted once per position
    /// encoding.
    fn serialize_for(
        &self,
        bid: BufferId,
        params: &Params,
        routed: &[Routed],
    ) -> Vec<Result<serde_json::Value, String>> {
        let text = self.buffers.get(bid).text();
        let mut by_encoding: Vec<(PositionEncoding, Result<serde_json::Value, String>)> =
            Vec::new();
        routed
            .iter()
            .map(|r| {
                if let Some(json) = &params.json {
                    return Ok(json.clone());
                }
                if let Some((_, json)) = by_encoding.iter().find(|(e, _)| *e == r.encoding) {
                    return json.clone();
                }
                let json = serialize_params(text, bid, r.encoding, &params.value);
                by_encoding.push((r.encoding, json.clone()));
                json
            })
            .collect()
    }

    /// Sends `json` to `routed`; the slot it returns waits for the answer.
    fn send_slot(
        &mut self,
        method: &str,
        routed: Routed,
        json: serde_json::Value,
        deadline: Instant,
    ) -> Slot {
        let server = routed.server;
        let meta = RequestMeta {
            method: method.to_string(),
            deadline,
        };
        let Some(id) = self.lsp.send_request(server.id, method, json, meta) else {
            let reason = format!("{} is not running", server.name);
            return Slot::failed(server, SlotError::Unavailable(reason));
        };
        Slot {
            server,
            state: SlotState::Waiting {
                request: id,
                encoding: routed.encoding,
            },
        }
    }

    /// Calls `req`'s callback for a request that reached no server: with an
    /// `'unavailable` error saying `reason` and no result, or with no error
    /// and an empty answer when it asked for `#:unavailable 'empty`.
    fn fail_request(&mut self, req: &PendingLspRequest, reason: String) {
        let args = match (req.when_unavailable, req.mode) {
            (WhenUnavailable::Error, RequestMode::Single) => {
                vec![
                    SlotError::Unavailable(reason).into_steel(),
                    SteelVal::BoolV(false),
                ]
            }
            (WhenUnavailable::Error, RequestMode::All) => vec![
                SlotError::Unavailable(reason).into_steel(),
                SteelVal::ListV(Vec::<SteelVal>::new().into()),
            ],
            (WhenUnavailable::Empty, RequestMode::Single) => {
                vec![SteelVal::BoolV(false), SteelVal::Void]
            }
            (WhenUnavailable::Empty, RequestMode::All) => vec![
                SteelVal::BoolV(false),
                SteelVal::ListV(Vec::<SteelVal>::new().into()),
            ],
        };
        self.queue_steel_call(req.callback.clone(), args);
    }

    /// Hands `server`'s answer to `request` to the delivery waiting for it,
    /// and calls the delivery back when that was its last slot. Gives the
    /// outcome back when no delivery waits for the request.
    pub(in crate::editor::lsp) fn lsp_fill_slot(
        &mut self,
        view: &EngineView,
        server: ServerId,
        request: RequestId,
        meta: &RequestMeta,
        outcome: Outcome,
    ) -> Option<Outcome> {
        let timed_out = matches!(outcome, Outcome::TimedOut);
        let complete = match self.lsp.deliveries.fill(server, request, outcome) {
            Fill::Unknown(outcome) => return Some(outcome),
            Fill::Waiting => None,
            Fill::Complete(did) => Some(did),
        };
        if timed_out {
            self.report(Severity::Trace, format!("lsp: {} timed out", meta.method));
        }
        if let Some(did) = complete {
            self.finish_delivery(view, did);
        }
        None
    }

    /// Removes a complete delivery and queues its callback, unless its
    /// anchor no longer admits it.
    fn finish_delivery(&mut self, view: &EngineView, did: DeliveryId) {
        let Some(delivery) = self.lsp.deliveries.map.remove(&did) else {
            return;
        };
        if !self.anchor_admits(view, &delivery.anchor) {
            self.release_request_position(delivery.anchor.tracked);
            return;
        }
        let callback = match delivery.responder {
            Responder::Steel(callback) => callback,
            Responder::Rust(respond) => {
                let slot = delivery
                    .slots
                    .into_iter()
                    .next()
                    .expect("a Rust responder's delivery has one slot");
                let result = match slot.state {
                    SlotState::Answered { value, .. } => Ok(value),
                    SlotState::Failed(error) => Err(error.describe()),
                    SlotState::Waiting { .. } => {
                        unreachable!("a delivery finishes only once no slot is waiting")
                    }
                };
                respond(self, view, result);
                return;
            }
        };
        let args = match delivery.mode {
            RequestMode::Single => {
                let slot = delivery
                    .slots
                    .into_iter()
                    .next()
                    .expect("a single-mode delivery has one slot");
                let (err, result) = slot_values(slot);
                vec![err, result]
            }
            RequestMode::All => {
                let results = delivery.slots.into_iter().map(|slot| {
                    let server = slot.server.clone().into_steel_val();
                    let (err, result) = slot_values(slot);
                    symbol_hash([("server", server), ("err", err), ("result", result)])
                });
                vec![
                    SteelVal::BoolV(false),
                    SteelVal::ListV(results.collect::<Vec<_>>().into()),
                ]
            }
        };
        self.queue_steel_call_anchored(callback, args, delivery.anchor);
    }

    /// Drops `did` without calling it back: every request it still has in
    /// flight is cancelled, and the position it holds released.
    fn cancel_delivery(&mut self, did: DeliveryId) {
        let Some(delivery) = self.lsp.deliveries.map.remove(&did) else {
            return;
        };
        for slot in delivery.slots {
            let SlotState::Waiting { request, .. } = slot.state else {
                continue;
            };
            if let Some((client, backend)) = self.lsp.client_and_backend(slot.server.id) {
                client.cancel(backend, request);
            }
        }
        self.release_request_position(delivery.anchor.tracked);
    }

    /// Sends one queued `lsp-notify!` to every server its route admits.
    pub(in crate::editor) fn lsp_send_notify(&mut self, notif: PendingLspNotify) {
        self.lsp_flush_pending();
        let routed = match route(self, notif.bid, Some(&notif.method), &notif.route) {
            Ok(routed) => routed,
            Err(u) => {
                self.report(Severity::Info, format!("lsp-notify!: {u}"));
                return;
            }
        };
        let jsons = self.serialize_for(notif.bid, &notif.params, &routed);
        for (r, json) in routed.into_iter().zip(jsons) {
            match json {
                Ok(params) => self.lsp_send_doc_notification(r.server.id, &notif.method, params),
                Err(e) => self.report(Severity::Error, format!("lsp-notify! params: {e}")),
            }
        }
    }
}

/// One server a request goes to with its params, or a server whose slot
/// fails before anything is sent.
enum Member {
    Send(Routed, serde_json::Value),
    Failed(ServerRef, SlotError),
}

impl Member {
    /// `routed` with its serialized params, or the failure to serialize
    /// them.
    fn new(routed: Routed, json: Result<serde_json::Value, String>, verb: &str) -> Self {
        match json {
            Ok(json) => Member::Send(routed, json),
            Err(e) => Member::Failed(
                routed.server,
                SlotError::Unsent(format!("{verb} params: {e}")),
            ),
        }
    }
}

/// `params` as the JSON `encoding`'s server receives: every `DocPos` and
/// `DocRange` in it becomes a wire position or range in `encoding`. A
/// position must belong to `bid` and have been taken at its current text
/// version.
fn serialize_params(
    text: &BufferText,
    bid: BufferId,
    encoding: PositionEncoding,
    params: &SteelVal,
) -> Result<serde_json::Value, String> {
    let check = |buffer: BufferId, version: TextVersion| {
        if buffer != bid {
            Err("position belongs to another buffer".to_string())
        } else if version != text.version() {
            Err("position is from an earlier version of the buffer".to_string())
        } else {
            Ok(())
        }
    };
    let mut positions = |v: &SteelVal| {
        if let Some(pos) = DocPos::from_steel_val(v) {
            return Some(check(pos.buffer, pos.version).map(|()| {
                hume_lsp::position::to_json_position(char_to_wire(
                    text.rope(),
                    pos.offset,
                    encoding,
                ))
            }));
        }
        let range = DocRange::from_steel_val(v)?;
        Some(check(range.buffer, range.version).map(|()| {
            hume_lsp::position::to_json_range(char_range_to_wire_range(
                text.rope(),
                ExclusiveRange::new(range.start, range.end),
                encoding,
            ))
        }))
    };
    steel_to_json_with(params, &mut positions)
}

/// A slot as the `(err result)` pair a callback receives: one of the two
/// is `#f`. A result crosses as a handle tagged with the server that wrote
/// it, so the positions inside decode in its encoding; `null` is `Void`.
fn slot_values(slot: Slot) -> (SteelVal, SteelVal) {
    let error = match slot.state {
        SlotState::Answered { value, encoding } => {
            return (
                SteelVal::BoolV(false),
                to_steel_handle(
                    Arc::new(value),
                    WireOrigin::Server {
                        id: slot.server.id,
                        encoding,
                    },
                ),
            );
        }
        SlotState::Failed(error) => error,
        SlotState::Waiting { .. } => {
            unreachable!("a delivery finishes only once no slot is waiting")
        }
    };
    (error.into_steel(), SteelVal::BoolV(false))
}
