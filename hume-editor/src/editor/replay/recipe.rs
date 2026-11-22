//! The selection recipe: the steps that built the extent a repeatable
//! command acted on, recorded so `.` can rebuild it first.

use std::rc::Rc;

use hume_core::CommandArg;

use crate::editor::EditorState;
use crate::editor::commands::FindChar;
use crate::editor::dispatch::CmdCtx;
use crate::editor::registry::SelectionEffect;

/// One selection-building step in a dot-repeat recipe.
///
/// Recorded by `update_recipe` as Motion/Selection commands (or
/// EditorCmds opting in as a `SelectionRole::recipe_step`) run,
/// so that `replay_dot` can replay them before the edit, rebuilding the
/// extent the edit acted on.
#[derive(Debug, Clone)]
pub(crate) struct SelectionStep {
    /// Command name (e.g. `"select-line"`, `"surround-paren"`).
    pub command: Rc<str>,
    /// The count and mode the step first ran with.
    pub(in crate::editor) ctx: CmdCtx,
    /// The operand the step first ran with (`f`'s character, `s`'s
    /// pattern), which replay hands it instead of asking again.
    pub(in crate::editor) operand: Option<CommandArg>,
    /// The find `=` and `-` repeated when the step ran. Replay sets it back
    /// before the step, so a replayed `=` repeats the find it first did.
    pub(in crate::editor) last_find: Option<FindChar>,
}

/// The recipe state an outer Steel command's body started from, held across
/// the body by `dispatch`.
pub(in crate::editor) struct SteelRecipe {
    recipe: Vec<SelectionStep>,
    /// `selection_recipe_touched` as the enclosing command left it.
    outer_touched: bool,
}

impl SteelRecipe {
    /// The recipe the change this command begins records.
    pub(in crate::editor) fn recipe(&self) -> &[SelectionStep] {
        &self.recipe
    }
}

impl EditorState {
    /// Snapshot selection recipe before body for dot-repeat recording.
    ///
    /// The snapshot captures the selection extent the user built before the edit,
    /// so `.` can re-establish it.  Inner dispatches (Steel `call!`) may overwrite
    /// `selection_recipe` during the body; the snapshot is taken before they run.
    pub(in crate::editor) fn take_recipe(
        &mut self,
        repeatable: bool,
    ) -> Option<Vec<SelectionStep>> {
        repeatable.then(|| std::mem::take(&mut self.selection_recipe))
    }

    /// Update the selection recipe buffer after a command dispatch.
    ///
    /// Accumulation rule, checked in this order:
    ///   no step (`step` is `None`)          → clear
    ///   Extends + move                      → clear (no replayable extent)
    ///   any other step + no change          → leave the recipe as-is
    ///   Establishes + move                  → reset + push establish
    ///   Establishes + extend                → append step
    ///   Extends + extend                    → append step
    ///   Composes                            → append step
    ///
    /// `Extends` covers every `Motion`, word motions included: a Move-mode word
    /// motion looks replayable but would advance past the word on replay. Extend
    /// steps grow a selection by a relative amount and replay safely.
    ///
    /// `selection_changed` gates every step but a Move-mode `Extends`, so a
    /// command that found nothing (`select-all-matches`, `ms(`, an `f` whose
    /// character isn't there) leaves the staged recipe untouched.
    /// It is `None` for a command that records no selection history, which
    /// pushes no step.
    ///
    /// # Panics
    /// Panics if a command that could push a step has no `selection_changed`.
    pub(in crate::editor) fn update_recipe(
        &mut self,
        step: Option<(SelectionEffect, SelectionStep)>,
        selection_changed: Option<bool>,
    ) {
        self.selection_recipe_touched = true;
        let Some((effect, step)) = step else {
            self.selection_recipe.clear();
            return;
        };
        // A Move-mode motion has no replayable extent to restart the recipe with.
        if effect == SelectionEffect::Extends && !step.ctx.extend {
            self.selection_recipe.clear();
            return;
        }
        let changed = selection_changed
            .expect("a command that can push a recipe step records its selections");
        if !changed {
            return;
        }
        // A Move-mode establish restarts the recipe.
        if effect == SelectionEffect::Establishes && !step.ctx.extend {
            self.selection_recipe.clear();
        }
        self.selection_recipe.push(step);
    }

    /// The recipe an outer Steel command's body starts from, cloned rather
    /// than taken: an inner `call!` dispatch (e.g. vim-keybind's `C` wrapper
    /// calling `copy-selection-on-next-line`) must see and compose onto
    /// whatever the user already staged. Resets `selection_recipe_touched`, so
    /// [`Self::close_steel_recipe`] sees only what this body dispatched
    /// natively.
    pub(in crate::editor) fn open_steel_recipe(&mut self) -> SteelRecipe {
        SteelRecipe {
            recipe: self.selection_recipe.clone(),
            outer_touched: std::mem::replace(&mut self.selection_recipe_touched, false),
        }
    }

    /// Ends the recipe scope [`Self::open_steel_recipe`] opened, putting back
    /// the outer `selection_recipe_touched`. A body that failed has whatever
    /// a partial inner dispatch wrote undone. Otherwise the live recipe is
    /// cleared when this outer command owns the decision: a repeatable
    /// command's recipe was just consumed, and a body that dispatched nothing
    /// natively is a non-selection command that clears it like any command
    /// that is no recipe step. A body that did dispatch natively already got the
    /// right decision from that inner call, which this must not override.
    pub(in crate::editor) fn close_steel_recipe(
        &mut self,
        opened: SteelRecipe,
        ran: bool,
        repeatable: bool,
    ) {
        let touched = std::mem::replace(&mut self.selection_recipe_touched, opened.outer_touched);
        if !ran {
            self.selection_recipe = opened.recipe;
        } else if repeatable || !touched {
            self.selection_recipe.clear();
        }
    }
}
