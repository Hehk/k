//! Central application state and transitions, independent of IO.
//! The host executes effects and returns timer/write completions as events.
//! Visibility and transaction time are inputs; effects are never journaled.
//! Git/query loading remains in the host and returns typed navigation completions.

use crate::{
    state::{AppState, Navigation, StateError, TabId, WindowId},
    viewing_history::{Foreground, Record, ViewChange, ViewingHistory, ViewingId},
};

mod closed;
use closed::Closed;

#[cfg(test)]
mod tests;

const CLOSED_HISTORY_LIMIT: usize = 100;

#[derive(Clone, Copy, Debug)]
pub struct Context {
    pub foreground: Foreground,
    pub timestamp_ms: i64,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Persistence {
    pub workspace: bool,
    pub history: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SaveId(u64);

#[derive(Debug)]
pub enum Event {
    Navigate { tab: TabId, action: Navigation },
    OpenView { id: ViewingId, target: OpenViewIn },
    FocusWindow(WindowId),
    Replace(AppState),
    Reopen(AppState),
    SaveRequested,
    SaveDue(SaveId),
    Saved(SaveResult),
    Quit,
}

#[derive(Debug)]
pub enum Effect {
    Changed,
    /// Restart the debounce timer; a superseded timer cannot start a write.
    ScheduleSave(SaveId),
    Save(Box<SaveBatch>),
    /// Finish synchronously, after earlier writes, and fence the writers before teardown.
    Flush(Box<SaveBatch>),
}

#[derive(Debug)]
pub struct SaveBatch {
    pub id: SaveId,
    pub revision: u64,
    pub workspace: Option<AppState>,
    pub history: Option<Vec<(u64, Record)>>,
}

#[derive(Debug)]
pub struct SaveResult {
    pub id: SaveId,
    pub workspace: Option<Result<(), String>>,
    pub history: Option<Result<(), String>>,
}

#[derive(Clone, Copy, Debug)]
pub enum OpenViewIn {
    Tab(TabId),
    NewTab(WindowId),
}

struct PendingSave {
    id: SaveId,
    revision: u64,
    generations: Vec<(ViewingId, u64)>,
}

enum Saving {
    Idle,
    Scheduled(SaveId),
    Writing(PendingSave),
    Stopped(Option<PendingSave>),
}

pub struct Model {
    state: AppState,
    history: ViewingHistory,
    closed: Vec<Closed>,
    persistence: Persistence,
    revision: u64,
    next_save: u64,
    saving: Saving,
    save_error: Option<String>,
}

impl Model {
    pub fn new(state: AppState, history: ViewingHistory, persistence: Persistence) -> Self {
        Self {
            state,
            history,
            closed: Vec::new(),
            persistence,
            revision: 0,
            next_save: 0,
            saving: Saving::Idle,
            save_error: None,
        }
    }

    pub fn state(&self) -> &AppState {
        &self.state
    }

    pub fn history(&self) -> &ViewingHistory {
        &self.history
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn save_error(&self) -> Option<&str> {
        self.save_error.as_deref()
    }

    pub fn stopped(&self) -> bool {
        matches!(self.saving, Saving::Stopped(_))
    }

    pub fn closed_count(&self) -> usize {
        self.closed.len()
    }

    pub fn can_reopen(&self) -> bool {
        !self.stopped() && !self.closed.is_empty()
    }

    pub fn transition(
        &mut self,
        event: Event,
        context: Context,
    ) -> Result<Vec<Effect>, StateError> {
        match event {
            Event::Navigate { tab, action } => self
                .edit(context, |state| state.navigate(tab, action))
                .map(|(_, effects)| effects),
            Event::OpenView { id, target } => {
                self.ensure_running()?;
                let viewed = self
                    .history
                    .get(id)
                    .ok_or(StateError("unknown viewing record"))?
                    .viewed
                    .clone();
                self.edit(context, |state| {
                    let snapshot = viewed.snapshot_for(state)?;
                    let tab = match target {
                        OpenViewIn::Tab(tab) => tab,
                        OpenViewIn::NewTab(window) => state.new_tab(window)?,
                    };
                    state.navigate(tab, Navigation::OpenSnapshot(snapshot))
                })
                .map(|(_, effects)| effects)
            }
            Event::FocusWindow(id) => self
                .edit(context, |state| state.focus_window(id))
                .map(|(_, effects)| effects),
            Event::Replace(state) => {
                if self.stopped() {
                    return Ok(Vec::new());
                }
                Ok(self.commit(state, Vec::new(), context))
            }
            Event::Reopen(mut state) => {
                self.ensure_running()?;
                let (_, changes) = state.collect_view_changes(context.foreground, |state| {
                    let tab = state
                        .active_window()
                        .and_then(|id| state.window(id).ok())
                        .and_then(|window| window.active_tab)
                        .ok_or(StateError("reopened container has no active tab"))?;
                    state.navigate(tab, Navigation::ReopenTab)
                })?;
                self.closed.pop();
                Ok(self.commit(state, changes, context))
            }
            Event::SaveRequested => Ok(self.schedule_save()),
            Event::SaveDue(id) => {
                if !matches!(self.saving, Saving::Scheduled(current) if current == id) {
                    return Ok(Vec::new());
                }
                let (batch, pending) = self.capture(id);
                self.saving = Saving::Writing(pending);
                Ok(vec![Effect::Save(Box::new(batch))])
            }
            Event::Saved(result) => Ok(self.finish_save(result)),
            Event::Quit => {
                if self.stopped() {
                    return Ok(Vec::new());
                }
                let id = self.save_id();
                // Dirty values include unacknowledged writes. The host orders this final
                // batch after a running write; duplicate same-ID journal values are harmless.
                let (batch, pending) = self.capture(id);
                self.saving = Saving::Stopped(Some(pending));
                Ok(vec![Effect::Flush(Box::new(batch))])
            }
        }
    }

    /// Transactional adapter for structural workspace operations (new/move/close tabs).
    /// Navigation inside the transaction still emits the same typed viewing decisions.
    pub fn edit<R>(
        &mut self,
        context: Context,
        change: impl FnOnce(&mut AppState) -> Result<R, StateError>,
    ) -> Result<(R, Vec<Effect>), StateError> {
        self.ensure_running()?;
        let mut state = self.state.clone();
        let (value, changes) = state.collect_view_changes(context.foreground, change)?;
        Ok((value, self.commit(state, changes, context)))
    }

    /// Prepare native window creation before committing the restored container.
    pub fn reopen_state(
        &self,
        target: Option<WindowId>,
    ) -> Result<Option<(AppState, WindowId)>, StateError> {
        self.ensure_running()?;
        let Some(closed) = self.closed.last() else {
            return Ok(None);
        };
        let mut state = self.state.clone();
        let id = closed.restore(&mut state, target)?;
        Ok(Some((state, id)))
    }

    fn ensure_running(&self) -> Result<(), StateError> {
        if self.stopped() {
            Err(StateError("application is quitting"))
        } else {
            Ok(())
        }
    }

    fn commit(
        &mut self,
        state: AppState,
        changes: Vec<ViewChange>,
        context: Context,
    ) -> Vec<Effect> {
        let viewed = self.history.apply(changes, || context.timestamp_ms);
        self.history.retain_live_bindings(&state);
        if self.state == state && !viewed {
            return Vec::new();
        }
        self.closed.extend(Closed::removed(&self.state, &state));
        let excess = self.closed.len().saturating_sub(CLOSED_HISTORY_LIMIT);
        self.closed.drain(..excess);
        self.state = state;
        self.revision += 1;
        let mut effects = self.schedule_save();
        effects.push(Effect::Changed);
        effects
    }

    fn save_id(&mut self) -> SaveId {
        let id = SaveId(self.next_save);
        self.next_save += 1;
        id
    }

    fn schedule_save(&mut self) -> Vec<Effect> {
        if matches!(self.saving, Saving::Writing(_) | Saving::Stopped(_))
            || !(self.persistence.workspace || self.persistence.history)
        {
            return Vec::new();
        }
        let id = self.save_id();
        self.saving = Saving::Scheduled(id);
        vec![Effect::ScheduleSave(id)]
    }

    fn capture(&self, id: SaveId) -> (SaveBatch, PendingSave) {
        // Capture after debounce, not on every edit. Only dirty history records are cloned.
        let history = self.persistence.history.then(|| self.history.pending());
        let generations = history
            .iter()
            .flatten()
            .map(|(generation, record)| (record.id, *generation))
            .collect();
        (
            SaveBatch {
                id,
                revision: self.revision,
                workspace: self.persistence.workspace.then(|| self.state.clone()),
                history,
            },
            PendingSave {
                id,
                revision: self.revision,
                generations,
            },
        )
    }

    fn finish_save(&mut self, result: SaveResult) -> Vec<Effect> {
        let pending = match &self.saving {
            Saving::Writing(pending) | Saving::Stopped(Some(pending)) => pending,
            Saving::Idle | Saving::Scheduled(_) | Saving::Stopped(None) => return Vec::new(),
        };
        if pending.id != result.id {
            return Vec::new();
        }
        if matches!(result.history, Some(Ok(()))) {
            self.history
                .acknowledge_generations(pending.generations.iter().copied());
        }
        let revision = pending.revision;
        let stopped = self.stopped();
        self.saving = if stopped {
            Saving::Stopped(None)
        } else {
            Saving::Idle
        };
        let errors = [
            ("Workspace", result.workspace),
            ("Viewing history", result.history),
        ]
        .into_iter()
        .filter_map(|(name, result)| match result {
            Some(Err(error)) => Some(format!("{name}: {error}")),
            Some(Ok(())) | None => None,
        })
        .collect::<Vec<_>>();
        self.save_error = (!errors.is_empty()).then(|| errors.join("\n"));
        // A failed batch stays dirty, but must not busy-loop. Retry only for an edit
        // made during IO, a subsequent edit, an explicit save, or the final quit flush.
        let mut effects = if !stopped && self.revision != revision {
            self.schedule_save()
        } else {
            Vec::new()
        };
        effects.push(Effect::Changed);
        effects
    }
}
