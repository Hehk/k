use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use gpui::{AnyWindowHandle, App, AppContext, Context, Entity, Global, Subscription, Task};
use kestrel::{
    model::{self, Effect, Event, Model, Persistence, SaveBatch, SaveResult},
    state::{AppState, Navigation, Project, Route, StateError, TabId, WindowId},
    storage::{self, Store},
    viewing_history::{Foreground, Record, ViewingHistory, ViewingId, storage as history_storage},
};

use crate::{repository_model::RepositorySnapshot, top_bar::model::Tab};

pub use kestrel::model::OpenViewIn;

const REPOSITORY_POLL_INTERVAL: Duration = Duration::from_secs(120);

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

pub struct Workspace {
    model: Model,
    writer: Option<Arc<Mutex<Writer>>>,
    history_writer: Option<Arc<Mutex<HistoryWriter>>>,
    save_task: Option<Task<()>>,
    notice: Option<String>,
    native_windows: HashMap<WindowId, AnyWindowHandle>,
    repository_snapshots: HashMap<PathBuf, Entity<RepositorySnapshot>>,
    watching_repositories: bool,
    repository_poll_task: Option<Task<()>>,
    // Native focus can be absent while the persisted active window still names the last window.
    foreground: Option<WindowId>,
    _subscriptions: Vec<Subscription>,
}

impl Workspace {
    pub fn new(loaded: LoadedWorkspace, cx: &mut Context<Self>) -> Self {
        let quit = cx.on_app_quit(|this, cx| {
            this.watching_repositories = false;
            this.synchronize_repository_watches(cx);
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
            model: Model::new(
                loaded.state,
                loaded.viewing_history,
                Persistence {
                    workspace: loaded.writer.is_some(),
                    history: loaded.history_writer.is_some(),
                },
            ),
            writer: loaded.writer.map(|writer| Arc::new(Mutex::new(writer))),
            history_writer: loaded
                .history_writer
                .map(|writer| Arc::new(Mutex::new(writer))),
            save_task: None,
            notice: loaded.notice,
            native_windows: HashMap::new(),
            repository_snapshots: HashMap::new(),
            watching_repositories: false,
            repository_poll_task: None,
            foreground: None,
            _subscriptions: vec![quit, closed],
        }
    }

    pub fn start_watching_repositories(&mut self, cx: &mut Context<Self>) {
        if self.model.stopped() || self.watching_repositories {
            return;
        }
        self.watching_repositories = true;
        self.refresh_repositories(cx);
        self.synchronize_repository_watches(cx);
        self.repository_poll_task = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(REPOSITORY_POLL_INTERVAL)
                    .await;
                let running = this.update(cx, |this, cx| {
                    if this.model.stopped() {
                        return false;
                    }
                    this.refresh_repositories(cx);
                    true
                });
                if !matches!(running, Ok(true)) {
                    break;
                }
            }
        }));
    }

    pub fn refresh_repositories(&mut self, cx: &mut Context<Self>) {
        if self.model.stopped() {
            return;
        }
        for project in self.state().projects().to_vec() {
            if let Some(snapshot) = self.repository_snapshots.get(&project.common_dir) {
                snapshot.update(cx, |snapshot, cx| {
                    // The two-minute sweep bounds automatic retries; focus can recover sooner.
                    // Stopped watchers stay stopped, including cached inactive repositories.
                    snapshot.retry_watch(cx);
                    snapshot.refresh(cx);
                });
            } else {
                self.repository_snapshot(&project, cx);
            }
        }
    }

    fn repository_is_open(&self, project: &Project) -> bool {
        self.state().windows().iter().any(|window| {
            window
                .tabs
                .iter()
                .any(|tab| tab.route.project() == Some(project.id))
        })
    }

    fn synchronize_repository_watches(&mut self, cx: &mut Context<Self>) {
        let projects = self.model.state().projects();
        self.repository_snapshots
            .retain(|common, _| projects.iter().any(|project| &project.common_dir == common));
        for project in projects {
            if let Some(snapshot) = self.repository_snapshots.get(&project.common_dir) {
                let watching = self.watching_repositories && self.repository_is_open(project);
                snapshot.update(cx, |snapshot, cx| {
                    snapshot.synchronize_project(project, cx);
                    snapshot.set_watching(watching, cx);
                });
            }
        }
    }

    pub fn repository_snapshot(
        &mut self,
        project: &Project,
        cx: &mut Context<Self>,
    ) -> Entity<RepositorySnapshot> {
        if let Some(snapshot) = self.repository_snapshots.get(&project.common_dir) {
            return snapshot.clone();
        }
        let snapshot = cx.new(|cx| RepositorySnapshot::new(project.clone(), cx));
        if self.watching_repositories && self.repository_is_open(project) {
            snapshot.update(cx, |snapshot, cx| snapshot.set_watching(true, cx));
        }
        self.repository_snapshots
            .insert(project.common_dir.clone(), snapshot.clone());
        snapshot
    }

    pub fn state(&self) -> &AppState {
        self.model.state()
    }

    pub fn viewing_history(&self) -> &ViewingHistory {
        self.model.history()
    }

    pub fn open_view(
        &mut self,
        id: ViewingId,
        target: OpenViewIn,
        cx: &mut Context<Self>,
    ) -> Result<(), StateError> {
        self.dispatch(Event::OpenView { id, target }, cx)
    }

    pub fn notice(&self) -> Option<&str> {
        self.notice.as_deref()
    }

    pub fn save_error(&self) -> Option<&str> {
        self.model.save_error()
    }

    pub fn navigate(
        &mut self,
        tab: TabId,
        action: Navigation,
        cx: &mut Context<Self>,
    ) -> Result<(), StateError> {
        self.dispatch(Event::Navigate { tab, action }, cx)
    }

    pub fn edit<R>(
        &mut self,
        change: impl FnOnce(&mut AppState) -> Result<R, StateError>,
        cx: &mut Context<Self>,
    ) -> Result<R, StateError> {
        let (value, effects) = self.model.edit(self.context(), change)?;
        self.run_effects(effects, cx);
        Ok(value)
    }

    /// Commit structural edits to current state, not workspace loading/restoration.
    /// See [`Event::Replace`] for the history-binding identity constraint.
    pub fn replace(&mut self, state: AppState, cx: &mut Context<Self>) {
        self.dispatch(Event::Replace(state), cx)
            .expect("replacement cannot fail");
    }

    fn dispatch(&mut self, event: Event, cx: &mut Context<Self>) -> Result<(), StateError> {
        let effects = self.model.transition(event, self.context())?;
        self.run_effects(effects, cx);
        Ok(())
    }

    pub fn close_window(&mut self, id: WindowId, cx: &mut Context<Self>) {
        self.native_windows.remove(&id);
        if self.foreground == Some(id) {
            self.foreground = None;
        }
        if !self.model.stopped() {
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
        self.model.can_reopen()
    }

    pub fn reopen_state(
        &self,
        target: Option<WindowId>,
    ) -> Result<Option<(AppState, WindowId)>, StateError> {
        self.model.reopen_state(target)
    }

    pub fn finish_reopen(
        &mut self,
        state: AppState,
        cx: &mut Context<Self>,
    ) -> Result<(), StateError> {
        self.dispatch(Event::Reopen(state), cx)
    }

    fn context(&self) -> model::Context {
        model::Context {
            foreground: if self.native_windows.is_empty() {
                Foreground::SelectedWindow
            } else {
                self.foreground
                    .map_or(Foreground::Hidden, Foreground::Window)
            },
            timestamp_ms: match SystemTime::now().duration_since(UNIX_EPOCH) {
                Ok(duration) => duration.as_millis() as i64,
                Err(error) => -(error.duration().as_millis() as i64),
            },
        }
    }

    pub fn register_window(&mut self, id: WindowId, handle: AnyWindowHandle, active: bool) {
        self.native_windows.insert(id, handle);
        if active {
            self.foreground = Some(id);
        }
    }

    pub fn window_activation(&mut self, id: WindowId, active: bool, cx: &mut Context<Self>) {
        if self.model.stopped() || self.state().window(id).is_err() {
            return;
        }
        if active {
            let changed = self.foreground != Some(id);
            self.foreground = Some(id);
            if changed || self.state().active_window() != Some(id) {
                let _ = self.dispatch(Event::FocusWindow(id), cx);
            }
            if changed && self.watching_repositories {
                self.refresh_repositories(cx);
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
        let Ok(window) = self.state().window(id) else {
            return (Vec::new(), None);
        };
        let tabs = window
            .tabs
            .iter()
            .map(|tab| Tab {
                id: tab.id.as_u64(),
                title: tab_title(self.state(), tab),
            })
            .collect();
        (tabs, window.active_tab.map(|id| id.as_u64()))
    }

    pub fn save(&mut self, cx: &mut Context<Self>) {
        self.dispatch(Event::SaveRequested, cx)
            .expect("save request cannot fail");
    }

    fn run_effects(&mut self, effects: Vec<Effect>, cx: &mut Context<Self>) {
        for effect in effects {
            match effect {
                Effect::Changed => {
                    self.synchronize_repository_watches(cx);
                    cx.notify();
                }
                Effect::ScheduleSave(id) => {
                    self.save_task = Some(cx.spawn(async move |this, cx| {
                        cx.background_executor()
                            .timer(Duration::from_millis(150))
                            .await;
                        let _ = this.update(cx, |this, cx| this.dispatch(Event::SaveDue(id), cx));
                    }));
                }
                Effect::Save(batch) => {
                    let writer = self.writer.clone();
                    let history_writer = self.history_writer.clone();
                    self.save_task = Some(cx.spawn(async move |this, cx| {
                        let result = cx
                            .background_executor()
                            .spawn(
                                async move { write_batch(*batch, writer, history_writer, false) },
                            )
                            .await;
                        let _ = this.update(cx, |this, cx| this.dispatch(Event::Saved(result), cx));
                    }));
                }
                Effect::Flush(_) => unreachable!("quit effects are handled synchronously"),
            }
        }
    }

    fn flush_on_quit(&mut self) {
        self.repository_poll_task.take();
        let effects = self
            .model
            .transition(Event::Quit, self.context())
            .expect("quit cannot fail");
        for effect in effects {
            let Effect::Flush(batch) = effect else {
                unreachable!("quit only emits the final flush");
            };
            self.save_task.take();
            // GPUI allows only 100 ms for async quit hooks. The mutexes order this
            // final write after in-flight IO; closing fences workers not yet started.
            let result = write_batch(
                *batch,
                self.writer.clone(),
                self.history_writer.clone(),
                true,
            );
            self.model
                .transition(Event::Saved(result), self.context())
                .expect("save completion cannot fail");
            if let Some(error) = self.save_error() {
                eprintln!("Could not save on quit: {error}");
            }
        }
    }
}

fn write_batch(
    batch: SaveBatch,
    writer: Option<Arc<Mutex<Writer>>>,
    history_writer: Option<Arc<Mutex<HistoryWriter>>>,
    closing: bool,
) -> SaveResult {
    let workspace = batch.workspace.map(|state| {
        writer
            .as_ref()
            .expect("workspace persistence has a writer")
            .lock()
            .map_err(|error| error.to_string())
            .and_then(|mut writer| {
                let result = writer
                    .save(batch.revision, &state)
                    .map_err(|error| error.to_string());
                writer.closed |= closing;
                result
            })
    });
    // Independent files: always attempt history even if the workspace write failed.
    let history = batch.history.map(|records| {
        history_writer
            .as_ref()
            .expect("history persistence has a writer")
            .lock()
            .map_err(|error| error.to_string())
            .and_then(|mut writer| {
                let result = writer.save(&records).map_err(|error| error.to_string());
                writer.closed |= closing;
                result
            })
    });
    SaveResult {
        id: batch.id,
        workspace,
        history,
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
