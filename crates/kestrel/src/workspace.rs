use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};

use gpui::{AnyWindowHandle, App, Context, Entity, Global, Subscription, Task};
use kestrel::{
    state::{AppState, Project, Route, StateError, WindowId},
    storage::{self, Store},
};

use crate::top_bar::model::Tab;

pub struct Session(pub Entity<Workspace>);
impl Global for Session {}

pub struct LoadedWorkspace {
    state: AppState,
    writer: Option<Writer>,
    notice: Option<String>,
}

impl LoadedWorkspace {
    pub fn load(store: Result<Store, storage::Error>) -> Self {
        let result = store.and_then(|mut store| {
            let loaded = store.load()?;
            Ok(Self {
                state: loaded.state,
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
        result.unwrap_or_else(|error| Self {
            state: AppState::default(),
            writer: None,
            notice: Some(format!(
                "Saved state could not be loaded: {error}. Saving is disabled; restart to retry."
            )),
        })
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

pub struct Workspace {
    state: AppState,
    writer: Option<Arc<Mutex<Writer>>>,
    revision: u64,
    save_task: Option<Task<()>>,
    save_error: Option<String>,
    notice: Option<String>,
    quitting: bool,
    native_windows: HashMap<WindowId, AnyWindowHandle>,
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
            writer: loaded.writer.map(|writer| Arc::new(Mutex::new(writer))),
            revision: 0,
            save_task: None,
            save_error: None,
            notice: loaded.notice,
            quitting: false,
            native_windows: HashMap::new(),
            _subscriptions: vec![quit, closed],
        }
    }

    pub fn state(&self) -> &AppState {
        &self.state
    }

    pub fn notice(&self) -> Option<&str> {
        self.notice.as_deref()
    }

    pub fn save_error(&self) -> Option<&str> {
        self.save_error.as_deref()
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
        let result = change(&mut state)?;
        self.replace(state, cx);
        Ok(result)
    }

    pub fn replace(&mut self, state: AppState, cx: &mut Context<Self>) {
        if self.quitting || self.state == state {
            return;
        }
        self.state = state;
        self.revision += 1;
        self.save(cx);
        cx.notify();
    }

    pub fn close_window(&mut self, id: WindowId, cx: &mut Context<Self>) {
        self.native_windows.remove(&id);
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

    pub fn register_window(&mut self, id: WindowId, handle: AnyWindowHandle) {
        self.native_windows.insert(id, handle);
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
        let Some(writer) = self.writer.clone() else {
            return;
        };
        let state = self.state.clone();
        let revision = self.revision;
        self.save_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(150))
                .await;
            let result = cx
                .background_executor()
                .spawn(async move {
                    let mut writer = writer.lock().map_err(|error| error.to_string())?;
                    writer
                        .save(revision, &state)
                        .map_err(|error| error.to_string())
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.revision == revision && !this.quitting {
                    this.save_error = result.err();
                    cx.notify();
                }
            });
        }));
    }

    fn flush_on_quit(&mut self) {
        self.quitting = true;
        self.save_task.take();
        if let Some(writer) = &self.writer {
            // GPUI allows only 100 ms for async quit hooks. Finish this write before teardown.
            let mut writer = match writer.lock() {
                Ok(writer) => writer,
                Err(error) => {
                    eprintln!("Could not save workspace on quit: {error}");
                    return;
                }
            };
            if let Err(error) = writer.save(self.revision, &self.state) {
                eprintln!("Could not save workspace on quit: {error}");
            }
            writer.closed = true;
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
        Route::Repository { project, review } => {
            let name = project_title(
                state
                    .project(*project)
                    .expect("repository routes reference registered projects"),
            );
            match &review.selected_branch {
                Some(branch) => format!("{name} - {}", branch.label()),
                None => name,
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
    let previous = state
        .window(id)?
        .tabs
        .iter()
        .map(|tab| tab.id)
        .collect::<Vec<_>>();
    let tabs = tabs
        .iter()
        .map(|id| {
            previous
                .iter()
                .find(|tab| tab.as_u64() == *id)
                .copied()
                .ok_or(StateError("tab is not in this window"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    if tabs
        .iter()
        .enumerate()
        .any(|(index, id)| tabs[..index].contains(id))
    {
        return Err(StateError("duplicate tab"));
    }
    let selected = active.and_then(|id| tabs.iter().find(|tab| tab.as_u64() == id).copied());
    if (tabs.is_empty() && active.is_some()) || (!tabs.is_empty() && selected.is_none()) {
        return Err(StateError("invalid active tab"));
    }
    for tab in previous {
        if !tabs.contains(&tab) {
            state.close_tab(tab);
        }
    }
    for (index, tab) in tabs.iter().enumerate() {
        if state.window(id)?.tabs[index].id != *tab {
            state.move_tab(*tab, id, index)?;
        }
    }
    if let Some(tab) = selected {
        state.select_tab(tab)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
