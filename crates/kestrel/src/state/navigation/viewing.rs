use super::super::{
    AppState, BaseSelection, Destination, StateError, TabId, ViewCollection, VisitId, WindowId,
};
use crate::viewing_history::{Exposure, Foreground, Labels, Source, ViewChange, Viewed};

pub(super) enum ViewPolicy {
    Navigate,
    Select,
    Update,
    Reopen,
}

impl AppState {
    /// Capture only explicit policy decisions in a transaction. Failed edits discard them.
    /// Ordinary model construction, loading, and replacement do not replay viewing events.
    pub fn collect_view_changes<R>(
        &mut self,
        foreground: Foreground,
        change: impl FnOnce(&mut Self) -> Result<R, StateError>,
    ) -> Result<(R, Vec<ViewChange>), StateError> {
        if self.view_changes.is_some() {
            return Err(StateError("view changes are already being collected"));
        }
        self.view_changes = Some(ViewCollection {
            foreground,
            changes: Vec::new(),
        });
        let result = change(self);
        let changes = self.view_changes.take().ok_or(StateError(
            "workspace was replaced during a navigation edit",
        ))?;
        result.map(|value| (value, changes.changes))
    }

    fn foreground_window(&self) -> Option<WindowId> {
        match self.view_changes.as_ref()?.foreground {
            Foreground::SelectedWindow => self.active_window,
            Foreground::Window(id) => Some(id),
            Foreground::Hidden => None,
        }
    }

    pub(super) fn selected(&self, id: TabId) -> bool {
        self.windows
            .iter()
            .any(|window| window.active_tab == Some(id))
    }

    pub(super) fn apply_view_policy(
        &mut self,
        id: TabId,
        before: VisitId,
        selected: bool,
        focused: Option<WindowId>,
        policy: ViewPolicy,
    ) -> Result<(), StateError> {
        if self.view_changes.is_none() {
            return Ok(());
        }
        let (source, viewed) = self.view_context(id)?;
        let exposure = if self.windows.iter().any(|window| {
            Some(window.id) == self.foreground_window() && window.active_tab == Some(id)
        }) {
            Exposure::Foreground
        } else {
            Exposure::Background
        };
        let change = match policy {
            ViewPolicy::Navigate if source.visit != before => ViewChange::Enter {
                source,
                viewed,
                exposure,
            },
            ViewPolicy::Select if !selected => ViewChange::Enter {
                source,
                viewed,
                exposure,
            },
            ViewPolicy::Reopen => ViewChange::Enter {
                source,
                viewed,
                exposure,
            },
            ViewPolicy::Navigate | ViewPolicy::Update => ViewChange::Update { source, viewed },
            ViewPolicy::Select
                if focused != self.active_window && exposure == Exposure::Foreground =>
            {
                ViewChange::ShowPending { source, viewed }
            }
            ViewPolicy::Select => return Ok(()),
        };
        self.view_changes
            .as_mut()
            .expect("collection is active")
            .changes
            .push(change);
        Ok(())
    }

    pub(in crate::state) fn show_pending_view(&mut self) {
        if self.view_changes.is_none() || self.foreground_window() != self.active_window {
            return;
        }
        let Some(tab) = self
            .windows
            .iter()
            .find(|window| Some(window.id) == self.active_window)
            .and_then(|window| window.active_tab)
        else {
            return;
        };
        let (source, viewed) = self
            .view_context(tab)
            .expect("selected tab has a valid visit");
        // Window focus never logs an old page. It can finish a deferred first
        // viewing of a destination explicitly opened in that background window.
        self.view_changes
            .as_mut()
            .expect("collection is active")
            .changes
            .push(ViewChange::ShowPending { source, viewed });
    }

    fn view_context(&self, id: TabId) -> Result<(Source, Option<Viewed>), StateError> {
        let tab = self.tab(id)?;
        let visit = tab.snapshot()?;
        let source = Source {
            tab: id,
            visit: visit.id,
            activation: tab.activation(),
        };
        let snapshot = visit.snapshot;
        let Some(project) = snapshot.project() else {
            return Ok((source, None));
        };
        let project = self.project(project)?;
        let (branch, base) = match &snapshot.destination {
            Destination::Comparison {
                branch,
                base,
                comparison,
                ..
            } => (
                Some(branch.label()),
                comparison
                    .as_ref()
                    .map(|comparison| comparison.base.label())
                    .or_else(|| match base {
                        BaseSelection::Default => None,
                        BaseSelection::Explicit(base) => Some(base.label()),
                    }),
            ),
            Destination::Home | Destination::Branches(_) => (None, None),
        };
        let labels = Labels {
            repository: project
                .directory
                .file_name()
                .unwrap_or(project.directory.as_os_str())
                .to_string_lossy()
                .into_owned(),
            directory: project.directory.to_string_lossy().into_owned(),
            branch,
            base,
            file: snapshot
                .view
                .browsing
                .file
                .as_ref()
                .map(|file| file.label()),
        };
        Ok((
            source,
            Some(Viewed {
                repository: project.into(),
                snapshot,
                labels,
            }),
        ))
    }
}
