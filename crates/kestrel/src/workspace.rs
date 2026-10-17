use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use gpui::{
    AnyWindowHandle, App, AppContext, Context, Entity, Global, Subscription, Task, WeakEntity,
};
use kestrel::{
    state::{AppState, Navigation, Project, Route, StateError, TabId, WindowId},
    storage::{self, Store},
    viewing_history::{
        Foreground, Record, ViewChange, ViewingHistory, ViewingId, storage as history_storage,
    },
};

use crate::{repository_model::RepositorySnapshot, top_bar::model::Tab};

mod history;
use history::Closed;

const CLOSED_HISTORY_LIMIT: usize = 100;

pub struct Session(pub Entity<Workspace>);
impl Global for Session {}

pub struct LoadedWorkspace {
    state: AppState,
    viewing_history: ViewingHistory,
    writer: Option<Writer>,
    history_writer: Option<HistoryWriter>,
    notice: Option<String>,
}

impl LoadedWorkspace {
    pub fn load(store: Result<Store, storage::Error>) -> Self {
        let history_path = store
            .as_ref()
            .ok()
            .map(|store| store.path().with_extension("history.jsonl"));
        let result = store.and_then(|mut store| {
            let loaded = store.load()?;
            Ok(Self {
                state: loaded.state,
                viewing_history: ViewingHistory::default(),
                history_writer: None,
                writer: Some(Writer {
                    store,
                    revision: 0,
                    closed: false,
                }),
                notice: loaded.recovery.map(|recovery| {
                    format!(
                        "Saved state could not be restored: {}. Original preserved at {}.",
                        recovery.reason,
                        recovery.backup.display()
                    )
                }),
            })
        });
        let mut loaded = result.unwrap_or_else(|error| Self {
            state: AppState::default(),
            viewing_history: ViewingHistory::default(),
            writer: None,
            history_writer: None,
            notice: Some(format!(
                "Saved state could not be loaded: {error}. Workspace saving is disabled; restart to retry."
            )),
        });
        // Workspace first, then reconcile the independently saved log by repository identity.
        // Either file can be newer or unavailable; neither load requires a cross-file transaction.
        if let Some(path) = history_path {
            let legacy = path.with_extension("bin");
            if !path.exists() && legacy.exists() {
                loaded.add_notice(format!("Experimental binary history at {} was left untouched. This version starts a new JSON history journal.", legacy.display()));
            }
            let mut store = history_storage::Store::new(path);
            match store.load(&mut loaded.state) {
                Ok(history) => {
                    loaded.viewing_history = history.history;
                    loaded.history_writer = Some(HistoryWriter { store, closed: false });
                    if let Some(recovery) = history.recovery {
                        loaded.add_notice(format!("Viewing history was recovered through its last valid entry: {}. Original preserved at {}.", recovery.reason, recovery.backup.display()));
                    }
                }
                Err(error) => loaded.add_notice(format!("Viewing history could not be loaded: {error}. History saving is disabled; restart to retry.")),
            }
        }
        loaded
    }

    fn add_notice(&mut self, message: String) {
        if let Some(notice) = &mut self.notice {
            notice.push('\n');
            notice.push_str(&message);
        } else {
            self.notice = Some(message);
        }
    }
}

struct Writer {
    store: Store,
    revision: u64,
    closed: bool,
}

impl Writer {
    fn save(&mut self, revision: u64, state: &AppState) -> Result<(), storage::Error> {
        if self.closed || revision < self.revision {
            return Ok(());
        }
        self.revision = revision;
        self.store.save(state)
    }
}

struct HistoryWriter {
    store: history_storage::Store,
    closed: bool,
}

impl HistoryWriter {
    fn save(&mut self, records: &[(u64, Record)]) -> Result<(), history_storage::Error> {
        if self.closed {
            return Ok(());
        }
        self.store.append(records.iter().map(|(_, record)| record))
    }
}

pub enum OpenViewIn {
    Tab(TabId),
    NewTab(WindowId),
}

pub struct Workspace {
    state: AppState,
    viewing_history: ViewingHistory,
    writer: Option<Arc<Mutex<Writer>>>,
    history_writer: Option<Arc<Mutex<HistoryWriter>>>,
    revision: u64,
    save_task: Option<Task<()>>,
    saving: bool,
    save_error: Option<String>,
    notice: Option<String>,
    quitting: bool,
    closed: Vec<Closed>,
    native_windows: HashMap<WindowId, AnyWindowHandle>,
    repository_snapshots: HashMap<PathBuf, WeakEntity<RepositorySnapshot>>,
    watching_repositories: bool,
    // Native focus can be absent while the persisted active window still names the last window.
    foreground: Option<WindowId>,
    _subscriptions: Vec<Subscription>,
}

impl Workspace {
    pub fn new(loaded: LoadedWorkspace, cx: &mut Context<Self>) -> Self {
        let quit = cx.on_app_quit(|this, _| {
            this.flush_on_quit();
            std::future::ready(())
        });
        let this = cx.weak_entity();
        let closed = cx.on_window_closed(move |cx| {
            let _ = this.update(cx, |this, cx| {
                let windows = cx.windows();
                let closed = this
                    .native_windows
                    .iter()
                    .filter(|(_, handle)| !windows.contains(handle))
                    .map(|(id, _)| *id)
                    .collect::<Vec<_>>();
                for id in closed {
                    this.close_window(id, cx);
                }
            });
        });
        Self {
            state: loaded.state,
            viewing_history: loaded.viewing_history,
            writer: loaded.writer.map(|writer| Arc::new(Mutex::new(writer))),
            history_writer: loaded
                .history_writer
                .map(|writer| Arc::new(Mutex::new(writer))),
            revision: 0,
            save_task: None,
            saving: false,
            save_error: None,
            notice: loaded.notice,
            quitting: false,
            closed: Vec::new(),
            native_windows: HashMap::new(),
            repository_snapshots: HashMap::new(),
            watching_repositories: false,
            foreground: None,
            _subscriptions: vec![quit, closed],
        }
    }

    pub fn start_watching_repositories(&mut self, cx: &mut Context<Self>) {
        if self.watching_repositories {
            return;
        }
        self.watching_repositories = true;
        for snapshot in self
            .repository_snapshots
            .values()
            .filter_map(WeakEntity::upgrade)
        {
            snapshot.update(cx, |snapshot, cx| snapshot.start_watching(cx));
        }
    }

    pub fn repository_snapshot(
        &mut self,
        project: &Project,
        cx: &mut Context<Self>,
    ) -> Entity<RepositorySnapshot> {
        // Views own the lifetime; remembered projects must not keep refresh workers alive.
        self.repository_snapshots
            .retain(|_, snapshot| snapshot.upgrade().is_some());
        if let Some(snapshot) = self
            .repository_snapshots
            .get(&project.common_dir)
            .and_then(WeakEntity::upgrade)
        {
            return snapshot;
        }
        let snapshot = cx.new(|cx| RepositorySnapshot::new(project.clone(), cx));
        if self.watching_repositories {
            snapshot.update(cx, |snapshot, cx| snapshot.start_watching(cx));
        }
        self.repository_snapshots
            .insert(project.common_dir.clone(), snapshot.downgrade());
        snapshot
    }

    pub fn state(&self) -> &AppState {
        &self.state
    }

    pub fn viewing_history(&self) -> &ViewingHistory {
        &self.viewing_history
    }

    pub fn open_view(
        &mut self,
        id: ViewingId,
        target: OpenViewIn,
        cx: &mut Context<Self>,
    ) -> Result<TabId, StateError> {
        let viewed = self
            .viewing_history
            .get(id)
            .ok_or(StateError("unknown viewing record"))?
            .viewed
            .clone();
        self.edit(
            |state| {
                let snapshot = viewed.snapshot_for(state)?;
                let tab = match target {
                    OpenViewIn::Tab(tab) => tab,
                    OpenViewIn::NewTab(window) => state.new_tab(window)?,
                };
                state.navigate(tab, Navigation::OpenSnapshot(snapshot))?;
                Ok(tab)
            },
            cx,
        )
    }

    pub fn notice(&self) -> Option<&str> {
        self.notice.as_deref()
    }

    pub fn save_error(&self) -> Option<&str> {
        self.save_error.as_deref()
    }

    pub fn navigate(
        &mut self,
        tab: TabId,
        action: Navigation,
        cx: &mut Context<Self>,
    ) -> Result<(), StateError> {
        self.edit(|state| state.navigate(tab, action), cx)
    }

    pub fn edit<R>(
        &mut self,
        change: impl FnOnce(&mut AppState) -> Result<R, StateError>,
        cx: &mut Context<Self>,
    ) -> Result<R, StateError> {
        if self.quitting {
            return Err(StateError("application is quitting"));
        }
        let mut state = self.state.clone();
        let (result, changes) = state.collect_view_changes(self.viewing_foreground(), change)?;
        self.commit(state, changes, cx);
        Ok(result)
    }

    pub fn replace(&mut self, state: AppState, cx: &mut Context<Self>) {
        self.commit(state, Vec::new(), cx);
    }

    fn commit(&mut self, state: AppState, changes: Vec<ViewChange>, cx: &mut Context<Self>) {
        if self.quitting {
            return;
        }
        let viewed = self.viewing_history.apply(changes, || {
            match SystemTime::now().duration_since(UNIX_EPOCH) {
                Ok(duration) => duration.as_millis() as i64,
                Err(error) => -(error.duration().as_millis() as i64),
            }
        });
        self.viewing_history.retain_live_bindings(&state);
        if self.state == state && !viewed {
            return;
        }
        self.closed.extend(Closed::removed(&self.state, &state));
        let excess = self.closed.len().saturating_sub(CLOSED_HISTORY_LIMIT);
        self.closed.drain(..excess);
        self.state = state;
        self.revision += 1;
        self.save(cx);
        cx.notify();
    }

    pub fn close_window(&mut self, id: WindowId, cx: &mut Context<Self>) {
        self.native_windows.remove(&id);
        if self.foreground == Some(id) {
            self.foreground = None;
        }
        if !self.quitting {
            let _ = self.edit(
                |state| {
                    state.close_window(id);
                    Ok(())
                },
                cx,
            );
        }
    }

    pub fn can_reopen(&self) -> bool {
        !self.quitting && !self.closed.is_empty()
    }

    pub fn reopen_state(
        &self,
        target: Option<WindowId>,
    ) -> Result<Option<(AppState, WindowId)>, StateError> {
        if self.quitting {
            return Err(StateError("application is quitting"));
        }
        let Some(closed) = self.closed.last() else {
            return Ok(None);
        };
        let mut state = self.state.clone();
        let id = closed.restore(&mut state, target)?;
        Ok(Some((state, id)))
    }

    pub fn finish_reopen(
        &mut self,
        mut state: AppState,
        cx: &mut Context<Self>,
    ) -> Result<(), StateError> {
        if self.quitting {
            return Err(StateError("application is quitting"));
        }
        let (_, changes) = state.collect_view_changes(self.viewing_foreground(), |state| {
            let tab = state
                .active_window()
                .and_then(|id| state.window(id).ok())
                .and_then(|window| window.active_tab)
                .ok_or(StateError("reopened container has no active tab"))?;
            state.navigate(tab, Navigation::ReopenTab)
        })?;
        self.closed.pop();
        self.commit(state, changes, cx);
        Ok(())
    }

    fn viewing_foreground(&self) -> Foreground {
        if self.native_windows.is_empty() {
            Foreground::SelectedWindow
        } else {
            self.foreground
                .map_or(Foreground::Hidden, Foreground::Window)
        }
    }

    pub fn register_window(&mut self, id: WindowId, handle: AnyWindowHandle, active: bool) {
        self.native_windows.insert(id, handle);
        if active {
            self.foreground = Some(id);
        }
    }

    pub fn window_activation(&mut self, id: WindowId, active: bool, cx: &mut Context<Self>) {
        if active {
            let changed = self.foreground != Some(id);
            self.foreground = Some(id);
            if (changed || self.state.active_window() != Some(id)) && self.state.window(id).is_ok()
            {
                let _ = self.edit(|state| state.focus_window(id), cx);
            }
        } else if self.foreground == Some(id) {
            self.foreground = None;
        }
    }

    pub fn activate_window(&self, id: WindowId, cx: &mut App) {
        if let Some(window) = self.native_windows.get(&id) {
            let _ = window.update(cx, |_, window, _| window.activate_window());
        }
    }

    pub fn tabs(&self, id: WindowId) -> (Vec<Tab>, Option<u64>) {
        let Ok(window) = self.state.window(id) else {
            return (Vec::new(), None);
        };
        let tabs = window
            .tabs
            .iter()
            .map(|tab| Tab {
                id: tab.id.as_u64(),
                title: tab_title(&self.state, tab),
            })
            .collect();
        (tabs, window.active_tab.map(|id| id.as_u64()))
    }

    pub fn save(&mut self, cx: &mut Context<Self>) {
        let writer = self.writer.clone();
        let history_writer = self.history_writer.clone();
        // Debounce before capturing data. Once IO starts, keep that task alive: dropping an
        // incremental batch like an obsolete whole-state snapshot could lose new record IDs.
        if self.quitting || self.saving || (writer.is_none() && history_writer.is_none()) {
            return;
        }
        self.save_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(150))
                .await;
            let Ok(Some((state, records, revision))) = this.update(cx, |this, _| {
                if this.quitting {
                    return None;
                }
                this.saving = true;
                let records = if this.history_writer.is_some() {
                    this.viewing_history.pending()
                } else {
                    Vec::new()
                };
                Some((this.state.clone(), records, this.revision))
            }) else {
                return;
            };
            let (records, history_saved, errors) = cx
                .background_executor()
                .spawn(async move {
                    let mut errors = Vec::new();
                    let mut history_saved = false;
                    // The files remain independent. Always attempt history even if workspace
                    // saving fails; only one batch runs at a time, and failed changes stay dirty.
                    if let Some(writer) = writer {
                        let result = writer.lock().map_err(|error| error.to_string()).and_then(
                            |mut writer| {
                                writer
                                    .save(revision, &state)
                                    .map_err(|error| error.to_string())
                            },
                        );
                        if let Err(error) = result {
                            errors.push(format!("Workspace: {error}"));
                        }
                    }
                    if let Some(writer) = history_writer {
                        let result = writer.lock().map_err(|error| error.to_string()).and_then(
                            |mut writer| writer.save(&records).map_err(|error| error.to_string()),
                        );
                        match result {
                            Ok(()) => history_saved = true,
                            Err(error) => errors.push(format!("Viewing history: {error}")),
                        }
                    }
                    (records, history_saved, errors)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.finish_save(revision, &records, history_saved, errors, cx);
            });
        }));
    }

    fn finish_save(
        &mut self,
        revision: u64,
        records: &[(u64, Record)],
        history_saved: bool,
        errors: Vec<String>,
        cx: &mut Context<Self>,
    ) {
        if self.quitting {
            return;
        }
        self.saving = false;
        if history_saved {
            self.viewing_history.acknowledge(records);
        }
        self.save_error = if errors.is_empty() {
            None
        } else {
            Some(errors.join("\n"))
        };
        if self.revision != revision {
            self.save(cx);
        }
        cx.notify();
    }

    fn flush_on_quit(&mut self) {
        if self.quitting {
            return;
        }
        self.quitting = true;
        self.save_task.take();
        if let Some(writer) = &self.writer {
            // GPUI allows only 100 ms for async quit hooks. Finish this write before teardown.
            match writer.lock() {
                Ok(mut writer) => {
                    if let Err(error) = writer.save(self.revision, &self.state) {
                        eprintln!("Could not save workspace on quit: {error}");
                    }
                    writer.closed = true;
                }
                Err(error) => eprintln!("Could not save workspace on quit: {error}"),
            }
        }
        if let Some(writer) = &self.history_writer {
            // Unacknowledged in-flight records remain dirty. The lock orders this final batch
            // after an append already running; closing fences one that has not started yet.
            let records = self.viewing_history.pending();
            match writer.lock() {
                Ok(mut writer) => {
                    match writer.save(&records) {
                        Ok(()) => self.viewing_history.acknowledge(&records),
                        Err(error) => eprintln!("Could not save viewing history on quit: {error}"),
                    }
                    writer.closed = true;
                }
                Err(error) => eprintln!("Could not save viewing history on quit: {error}"),
            }
        }
    }
}

pub fn project_title(project: &Project) -> String {
    project
        .directory
        .file_name()
        .unwrap_or(project.directory.as_os_str())
        .to_string_lossy()
        .into_owned()
}

pub fn tab_title(state: &AppState, tab: &kestrel::state::Tab) -> String {
    match &tab.route {
        Route::Home => "Home".into(),
        Route::Repository {
            project,
            review,
            page,
            ..
        } => {
            let name = project_title(
                state
                    .project(*project)
                    .expect("repository routes reference registered projects"),
            );
            match (page, &review.selected_branch) {
                (kestrel::state::RepositoryPage::Comparison, Some(branch)) => {
                    format!("{name} - {}", branch.label())
                }
                _ => name,
            }
        }
    }
}

pub fn set_tab_layout(
    state: &mut AppState,
    id: WindowId,
    tabs: &[u64],
    active: Option<u64>,
) -> Result<(), StateError> {
    state.set_tab_layout(id, tabs, active)
}

#[cfg(test)]
mod tests;
