use std::{collections::HashMap, path::PathBuf};

use gpui::{
    App, Context, Entity, FocusHandle, PathPromptOptions, Subscription, Task, Window, WindowBounds,
    WindowHandle, div, prelude::*, px, size,
};
use kestrel::{
    git::{Repository, RepositoryInfo},
    query::{Query, QueryResult},
    state::{
        Navigation, Project, ProjectId, RepositoryPage, Route, StateError, Tab, TabId, WindowId,
    },
};

use crate::{
    appearance::{self, Preferences, Theme},
    commands::{self, Command},
    controls::control as button,
    recent_views::{RecentEvent, RecentViews},
    repository_view::RepositoryView,
    top_bar::{
        NavigationState, TabAction, TopBar,
        model::{Msg, WindowCommand},
    },
    workspace::{OpenViewIn, Workspace, project_title, set_tab_layout, tab_title},
};

#[derive(Clone, Debug, PartialEq, Eq)]
struct OpenTarget {
    tab: TabId,
    activation: u64,
}

struct RecentMenu {
    view: Entity<RecentViews>,
    target: OpenTarget,
    _subscription: Subscription,
}

pub struct Kestrel {
    #[cfg(any(debug_assertions, feature = "frame-metrics"))]
    pub(crate) debug_panel: Entity<crate::debug_panel::DebugPanel>,
    pub(crate) top_bar: Entity<TopBar>,
    workspace: Entity<Workspace>,
    id: WindowId,
    focus: FocusHandle,
    selection: Option<(TabId, ProjectId)>,
    selection_directory: Option<PathBuf>,
    repository: Query<Entity<RepositoryView>>,
    repository_views: HashMap<TabId, (Project, Entity<RepositoryView>)>,
    repository_task: Option<Task<()>>,
    request: u64,
    open_task: Option<Task<()>>,
    opening: Option<OpenTarget>,
    error: Option<String>,
    recent: Option<RecentMenu>,
    _subscriptions: Vec<Subscription>,
}

impl Kestrel {
    fn new(
        workspace: Entity<Workspace>,
        id: WindowId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let (tabs, active) = workspace.read(cx).tabs(id);
        let top_bar = cx.new(|cx| TopBar::for_workspace(tabs, active, window, cx));
        let focus = cx.focus_handle();
        window.focus(&focus);
        let subscriptions = vec![
            cx.observe_window_appearance(window, |_, window, cx| {
                appearance::refresh(cx);
                window.refresh();
            }),
            cx.observe_global_in::<Preferences>(window, |_, window, _| window.refresh()),
            cx.subscribe_in(&top_bar, window, Self::handle_window_command),
            cx.observe_in(&workspace, window, |this, _, window, cx| {
                if this.workspace.read(cx).state().window(this.id).is_err() {
                    window.remove_window();
                    return;
                }
                let (tabs, active) = this.workspace.read(cx).tabs(this.id);
                this.top_bar
                    .update(cx, |bar, cx| bar.synchronize(tabs, active, window, cx));
                if this
                    .opening
                    .as_ref()
                    .is_some_and(|target| !this.accepts_open(target, cx))
                {
                    this.open_task.take();
                    this.opening = None;
                }
                if this
                    .recent
                    .as_ref()
                    .is_some_and(|menu| this.open_target(cx).as_ref() != Some(&menu.target))
                {
                    this.close_recent(window, cx);
                }
                let previous = this.selection;
                this.load_project(cx);
                if previous != this.selection && this.top_bar.read(cx).interactive() {
                    window.focus(&this.focus);
                }
                cx.notify();
            }),
            cx.observe_window_bounds(window, |this, window, cx| {
                this.close_recent(window, cx);
                commands::refresh_later(cx);
            }),
            cx.observe_window_activation(window, |this, window, cx| {
                commands::refresh_later(cx);
                window.refresh();
                if !window.is_window_active() {
                    this.close_recent(window, cx);
                }
                this.workspace.update(cx, |workspace, cx| {
                    workspace.window_activation(this.id, window.is_window_active(), cx);
                });
            }),
        ];
        let mut this = Self {
            #[cfg(any(debug_assertions, feature = "frame-metrics"))]
            debug_panel: cx.new(|_| crate::debug_panel::DebugPanel::default()),
            top_bar,
            workspace,
            id,
            focus,
            selection: None,
            selection_directory: None,
            repository: Query::new(),
            repository_views: HashMap::new(),
            repository_task: None,
            request: 0,
            open_task: None,
            opening: None,
            error: None,
            recent: None,
            _subscriptions: subscriptions,
        };
        this.load_project(cx);
        this
    }

    pub fn open(
        workspace: Entity<Workspace>,
        id: WindowId,
        cx: &mut App,
    ) -> gpui::Result<WindowHandle<Self>> {
        let bounds =
            WindowBounds::Windowed(gpui::Bounds::centered(None, size(px(1100.), px(760.)), cx));
        let shared = workspace.clone();
        let handle = cx.open_window(crate::window_options(Some(bounds)), |window, cx| {
            cx.new(|cx| Self::new(shared, id, window, cx))
        })?;
        handle.update(cx, |_, window, cx| {
            workspace.update(cx, |workspace, _| {
                workspace.register_window(id, handle.into(), window.is_window_active());
            });
        })?;
        Ok(handle)
    }

    pub fn new_window(
        workspace: Entity<Workspace>,
        cx: &mut App,
    ) -> gpui::Result<WindowHandle<Self>> {
        let mut state = workspace.read(cx).state().clone();
        let id = state.new_window()?;
        state.new_tab(id)?;
        let handle = Self::open(workspace.clone(), id, cx)?;
        workspace.update(cx, |workspace, cx| workspace.replace(state, cx));
        Ok(handle)
    }

    pub fn id(&self) -> WindowId {
        self.id
    }

    pub fn tab_count(&self, cx: &App) -> usize {
        self.workspace
            .read(cx)
            .state()
            .window(self.id)
            .map_or(0, |window| window.tabs.len())
    }

    pub fn commands_allowed(&self, cx: &App) -> bool {
        self.opening.is_none() && self.top_bar.read(cx).interactive()
    }

    pub fn tab_action_allowed(&self, action: &TabAction, cx: &App) -> bool {
        self.commands_allowed(cx) && self.top_bar.read(cx).allows(action)
    }

    fn active_tab<'a>(&self, cx: &'a App) -> Option<&'a Tab> {
        let state = self.workspace.read(cx).state();
        state.tab(state.window(self.id).ok()?.active_tab?).ok()
    }

    pub fn navigation_allowed(&self, command: Command, cx: &App) -> bool {
        self.commands_allowed(cx)
            && self.active_tab(cx).is_some_and(|tab| match command {
                Command::Back => tab.can_go_back(),
                Command::Forward => tab.can_go_forward(),
                _ => false,
            })
    }

    pub fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.workspace
            .update(cx, |workspace, cx| workspace.close_window(self.id, cx));
        window.remove_window();
    }

    pub fn command(&mut self, command: Command, window: &mut Window, cx: &mut Context<Self>) {
        if !self.commands_allowed(cx) {
            return;
        }
        match command {
            Command::Back | Command::Forward if self.navigation_allowed(command, cx) => {
                let Some(tab) = self.active_tab(cx).map(|tab| tab.id) else {
                    return;
                };
                let action = if command == Command::Back {
                    Navigation::Back
                } else {
                    Navigation::Forward
                };
                let result = self
                    .workspace
                    .update(cx, |workspace, cx| workspace.navigate(tab, action, cx));
                self.error = result.err().map(|error| error.to_string());
                window.focus(&self.focus);
                cx.notify();
            }
            Command::RecentlyViewed => self.toggle_recent(window, cx),
            Command::CloseWindow => self.close(window, cx),
            Command::Minimize if !window.is_fullscreen() => window.minimize_window(),
            Command::Zoom if !window.is_fullscreen() => window.zoom_window(),
            Command::FullScreen => window.toggle_fullscreen(),
            Command::DetachTab if self.tab_count(cx) > 1 => {
                if let Some(tab) = self.top_bar.read(cx).active_tab().map(|tab| tab.id) {
                    self.error = self.detach(tab, cx).err().map(|error| error.to_string());
                    cx.notify();
                }
            }
            _ => {}
        }
    }

    pub fn reopen(
        workspace: Entity<Workspace>,
        target: Option<WindowId>,
        cx: &mut App,
    ) -> gpui::Result<()> {
        let Some((state, id)) = workspace.read(cx).reopen_state(target)? else {
            return Ok(());
        };
        if workspace.read(cx).state().window(id).is_err() {
            Self::open(workspace.clone(), id, cx)?;
        }
        workspace.update(cx, |workspace, cx| {
            workspace.finish_reopen(state, cx)?;
            workspace.activate_window(id, cx);
            Ok::<_, StateError>(())
        })?;
        Ok(())
    }

    fn handle_window_command(
        &mut self,
        bar: &Entity<TopBar>,
        command: &WindowCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match command {
            WindowCommand::Back => self.command(Command::Back, window, cx),
            WindowCommand::Forward => self.command(Command::Forward, window, cx),
            WindowCommand::RecentlyViewed => self.command(Command::RecentlyViewed, window, cx),
            WindowCommand::NewTab => {
                let result = self.workspace.update(cx, |workspace, cx| {
                    workspace.edit(|state| state.new_tab(self.id), cx)
                });
                self.error = result.err().map(|error| error.to_string());
                cx.notify();
            }
            WindowCommand::LayoutChanged { tabs, active } => {
                if self.workspace.read(cx).state().window(self.id).is_ok() {
                    let result = self.workspace.update(cx, |workspace, cx| {
                        workspace.edit(|state| set_tab_layout(state, self.id, tabs, *active), cx)
                    });
                    if let Err(error) = result {
                        self.error = Some(error.to_string());
                        let (tabs, active) = self.workspace.read(cx).tabs(self.id);
                        bar.update(cx, |bar, cx| bar.synchronize(tabs, active, window, cx));
                        cx.notify();
                    }
                }
            }
            WindowCommand::OpenDetached { request, tab, .. } => {
                let result = self.detach(tab.id, cx);
                let success = result.is_ok();
                self.error = result.err().map(|error| error.to_string());
                bar.update(cx, |bar, cx| {
                    bar.dispatch(
                        Msg::DetachCompleted {
                            request: *request,
                            success,
                        },
                        window,
                        cx,
                    )
                });
                cx.notify();
            }
            WindowCommand::Close => self.close(window, cx),
        }
    }

    fn detach(&mut self, tab: u64, cx: &mut Context<Self>) -> gpui::Result<()> {
        let mut state = self.workspace.read(cx).state().clone();
        let tab = state
            .window(self.id)?
            .tabs
            .iter()
            .find(|item| item.id.as_u64() == tab)
            .map(|tab| tab.id)
            .ok_or(StateError("tab is not in this window"))?;
        let target = state.new_window()?;
        state.move_tab(tab, target, 0)?;
        Self::open(self.workspace.clone(), target, cx)?;
        self.workspace
            .update(cx, |workspace, cx| workspace.replace(state, cx));
        Ok(())
    }

    fn toggle_recent(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.recent.is_some() {
            self.close_recent(window, cx);
            return;
        }
        let Some(target) = self.open_target(cx) else {
            return;
        };
        let trigger = self.top_bar.read(cx).clock_bounds();
        let workspace = self.workspace.clone();
        let view = cx.new(|cx| RecentViews::new(workspace, trigger, window, cx));
        let subscription = cx.subscribe_in(&view, window, |this, source, event, window, cx| {
            let Some(menu) = this.recent.as_ref().filter(|menu| &menu.view == source) else {
                return;
            };
            let target = menu.target.clone();
            match event {
                RecentEvent::Dismiss => this.close_recent(window, cx),
                RecentEvent::Open { id, new_tab } => {
                    let allowed =
                        this.commands_allowed(cx) && this.open_target(cx).as_ref() == Some(&target);
                    this.close_recent(window, cx);
                    if allowed {
                        let target = if *new_tab {
                            OpenViewIn::NewTab(this.id)
                        } else {
                            OpenViewIn::Tab(target.tab)
                        };
                        let result = this
                            .workspace
                            .update(cx, |workspace, cx| workspace.open_view(*id, target, cx));
                        this.error = result.err().map(|error| error.to_string());
                        window.focus(&this.focus);
                        cx.notify();
                    }
                }
            }
        });
        self.recent = Some(RecentMenu {
            view,
            target,
            _subscription: subscription,
        });
        cx.notify();
    }

    fn close_recent(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(menu) = self.recent.take() {
            menu.view.read(cx).restore_focus(&self.focus, window, cx);
            cx.notify();
        }
    }

    fn open_saved(&mut self, project: ProjectId, cx: &mut Context<Self>) {
        let Some(target) = self.open_target(cx) else {
            return;
        };
        let result = self.workspace.update(cx, |workspace, cx| {
            workspace.navigate(target.tab, Navigation::OpenProject(project), cx)
        });
        self.error = result.err().map(|error| error.to_string());
        cx.notify();
    }

    fn prompt_repository(&mut self, cx: &mut Context<Self>) {
        if self.opening.is_some() {
            return;
        }
        let Some(target) = self.open_target(cx) else {
            return;
        };
        let selected = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Open repository".into()),
        });
        self.opening = Some(target.clone());
        self.error = None;
        self.open_task = Some(cx.spawn(async move |this, cx| {
            let result = async {
                let paths = selected
                    .await
                    .map_err(|error| error.to_string())?
                    .map_err(|error| error.to_string())?;
                let Some(path) = paths.and_then(|paths| paths.into_iter().next()) else {
                    return Ok(None);
                };
                cx.background_executor()
                    .spawn(async move { discover(path) })
                    .await
                    .map(Some)
            }
            .await;
            let _ = this.update(cx, |this, cx| {
                this.finish_open(target, result, cx);
            });
        }));
        cx.notify();
    }

    fn open_target(&self, cx: &App) -> Option<OpenTarget> {
        let state = self.workspace.read(cx).state();
        let tab = state.tab(state.window(self.id).ok()?.active_tab?).ok()?;
        Some(OpenTarget {
            tab: tab.id,
            activation: tab.activation(),
        })
    }

    fn accepts_open(&self, target: &OpenTarget, cx: &App) -> bool {
        self.workspace
            .read(cx)
            .state()
            .window(self.id)
            .is_ok_and(|window| {
                window
                    .tabs
                    .iter()
                    .any(|tab| tab.id == target.tab && tab.activation() == target.activation)
            })
    }

    fn finish_open(
        &mut self,
        target: OpenTarget,
        result: Result<Option<RepositoryInfo>, String>,
        cx: &mut Context<Self>,
    ) {
        if self.opening.as_ref() != Some(&target) {
            return;
        }
        self.opening = None;
        if self.accepts_open(&target, cx) {
            match result {
                Ok(Some(info)) => self.open_repository(target, info, cx),
                Ok(None) => {}
                Err(error) => self.error = Some(error),
            }
        }
        cx.notify();
    }

    fn open_repository(
        &mut self,
        target: OpenTarget,
        info: RepositoryInfo,
        cx: &mut Context<Self>,
    ) {
        if !self.accepts_open(&target, cx) {
            return;
        }
        let previous = self
            .workspace
            .read(cx)
            .state()
            .projects()
            .iter()
            .find(|project| {
                project.common_dir == info.common_dir && project.directory != info.directory
            })
            .cloned();
        let Some(previous) = previous else {
            self.apply_open(target, info, None, cx);
            return;
        };
        self.opening = Some(target.clone());
        self.open_task = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let discovered = discover(info.directory)?;
                    if discovered.common_dir != info.common_dir {
                        return Err("This path now belongs to a different repository.".into());
                    }
                    let recovery = crate::repository_model::open(&previous)
                        .is_err()
                        .then_some(previous);
                    Ok((discovered, recovery))
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.opening.as_ref() != Some(&target) {
                    return;
                }
                this.opening = None;
                if this.accepts_open(&target, cx) {
                    match result {
                        Ok((info, recovery)) => this.apply_open(target, info, recovery, cx),
                        Err(error) => this.error = Some(error),
                    }
                }
                cx.notify();
            });
        }));
    }

    fn apply_open(
        &mut self,
        target: OpenTarget,
        info: RepositoryInfo,
        recovery: Option<Project>,
        cx: &mut Context<Self>,
    ) {
        let result = self.workspace.update(cx, |workspace, cx| {
            workspace.edit(
                |state| {
                    let project = state.register_project(&info)?;
                    if let Some(previous) = recovery {
                        state.recover_project(&previous, &info)?;
                    }
                    state.navigate(target.tab, Navigation::OpenProject(project))
                },
                cx,
            )
        });
        self.error = result.err().map(|error| error.to_string());
    }

    fn load_project(&mut self, cx: &mut Context<Self>) {
        let active = self
            .workspace
            .read(cx)
            .state()
            .window(self.id)
            .ok()
            .and_then(|window| window.active_tab)
            .and_then(|tab| {
                self.workspace
                    .read(cx)
                    .state()
                    .tab(tab)
                    .ok()
                    .and_then(|tab| tab.route.project().map(|project| (tab.id, project)))
            });
        let state = self.workspace.read(cx).state();
        self.repository_views.retain(|tab, (project, _)| {
            state
                .project(project.id)
                .is_ok_and(|current| current.directory == project.directory)
                && state.window(self.id).is_ok_and(|window| {
                    window
                        .tabs
                        .iter()
                        .any(|item| item.id == *tab && item.route.project() == Some(project.id))
                })
        });
        let directory = active.and_then(|(_, project)| {
            state
                .project(project)
                .ok()
                .map(|project| project.directory.clone())
        });
        if self.selection == active && self.selection_directory == directory {
            return;
        }
        self.selection = active;
        self.selection_directory = directory;
        self.request += 1;
        self.repository_task.take();
        self.repository = Query::new();
        let Some((tab, project)) = active else { return };
        if let Some((_, view)) = self.repository_views.get(&tab) {
            self.repository.complete(Ok(view.clone()));
            return;
        }
        self.repository.start();
        let project = self
            .workspace
            .read(cx)
            .state()
            .project(project)
            .expect("repository routes reference registered projects")
            .clone();
        let request = self.request;
        self.repository_task = Some(cx.spawn(async move |this, cx| {
            let view_project = project.clone();
            let result = cx
                .background_executor()
                .spawn(async move {
                    let info = discover(project.directory)?;
                    if info.common_dir != project.common_dir {
                        return Err("This path now belongs to a different repository.".into());
                    }
                    Ok(())
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.request == request {
                    let result = match result {
                        Ok(()) => {
                            let workspace = this.workspace.clone();
                            let window = this.id;
                            let project = view_project.clone();
                            let view = cx.new(|cx| {
                                RepositoryView::new(workspace, window, tab, view_project, cx)
                            });
                            this.repository_views.insert(tab, (project, view.clone()));
                            Ok(view)
                        }
                        Err(error) => Err(error),
                    };
                    this.repository.complete(result);
                    cx.notify();
                }
            });
        }));
    }
}

fn discover(path: PathBuf) -> Result<RepositoryInfo, String> {
    Repository::open(path)
        .and_then(|repository| repository.info())
        .map_err(|error| error.to_string())
}

impl Render for Kestrel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        commands::refresh_later(cx);
        let theme = Theme::get(window, cx);
        let workspace = self.workspace.read(cx);
        let tab = self.active_tab(cx);
        let comparison = tab.is_some_and(|tab| {
            matches!(
                tab.route,
                Route::Repository {
                    page: RepositoryPage::Comparison,
                    ..
                }
            )
        });
        let project = tab
            .and_then(|tab| tab.route.project())
            .and_then(|id| workspace.state().project(id).ok());
        let title = tab
            .map(|tab| tab_title(workspace.state(), tab))
            .unwrap_or_else(|| "Home".into());
        let directory = project.map(|project| project.directory.display().to_string());
        let native_title = match &directory {
            Some(directory) => format!("{title} — {directory} — Kestrel"),
            None => format!("{title} — Kestrel"),
        };
        window.set_window_title(&native_title);
        let recent = workspace
            .state()
            .projects()
            .iter()
            .map(|project| (project.id, project_title(project)))
            .collect::<Vec<_>>();
        let notice = workspace.notice().map(str::to_owned);
        let save_error = workspace.save_error().map(str::to_owned);
        let navigation = NavigationState {
            enabled: self.commands_allowed(cx),
            back: self.navigation_allowed(Command::Back, cx),
            forward: self.navigation_allowed(Command::Forward, cx),
            recent_open: self.recent.is_some(),
        };
        self.top_bar
            .update(cx, |bar, cx| bar.set_navigation(navigation, cx));
        let root =
            div()
                .key_context("Workspace")
                .track_focus(&self.focus)
                .on_key_down(|event, window, cx| {
                    if event.keystroke.key == "tab"
                        && !event.keystroke.modifiers.control
                        && !event.keystroke.modifiers.platform
                        && !event.keystroke.modifiers.alt
                    {
                        if event.keystroke.modifiers.shift {
                            window.focus_prev();
                        } else {
                            window.focus_next();
                        }
                        cx.stop_propagation();
                    }
                })
                .on_action(cx.listener(|this, action: &TabAction, window, cx| {
                    if !this.tab_action_allowed(action, cx) {
                        return;
                    }
                    this.top_bar
                        .update(cx, |bar, cx| bar.dispatch(action.message(), window, cx));
                }))
                .relative()
                .flex()
                .flex_col()
                .size_full()
                .bg(theme.shell)
                .text_color(theme.text)
                .child(self.top_bar.clone())
                .when_some(notice, |this, notice| {
                    this.child(div().p_2().text_sm().bg(theme.warning).child(notice))
                })
                .when_some(save_error, |this, error| {
                    this.child(
                        div()
                            .p_2()
                            .flex()
                            .gap_3()
                            .text_sm()
                            .bg(theme.warning)
                            .child(format!("Workspace is not saved: {error}"))
                            .child(button("retry-save", "Retry", theme).on_click(cx.listener(
                                |this, _, _, cx| {
                                    this.workspace
                                        .update(cx, |workspace, cx| workspace.save(cx));
                                },
                            ))),
                    )
                })
                .child(
                    div()
                        .id("content-frame")
                        .relative()
                        .flex_1()
                        .min_h_0()
                        .mx_2()
                        .mb_2()
                        .p_2()
                        .flex()
                        .flex_col()
                        .gap_3()
                        .rounded_lg()
                        .border_1()
                        .border_color(theme.border)
                        .bg(theme.paper)
                        .when(!comparison, |this| this.child(div().text_lg().child(title)))
                        .when_some(
                            directory.clone().filter(|_| !comparison),
                            |this, directory| {
                                this.child(div().text_sm().text_color(theme.muted).child(directory))
                            },
                        )
                        .when(directory.is_some(), |this| match self.repository.result() {
                            QueryResult::Pending => this.child("Opening repository…"),
                            QueryResult::Success(view) => {
                                this.child(div().flex_1().min_h_0().child(view.clone()))
                            }
                            QueryResult::Error { error, .. } => this
                                .child(
                                    div()
                                        .text_color(theme.error)
                                        .child(format!("Repository unavailable: {error}")),
                                )
                                .child(button("retry-repository", "Retry", theme).on_click(
                                    cx.listener(|this, _, _, cx| {
                                        this.selection = None;
                                        this.load_project(cx);
                                        cx.notify();
                                    }),
                                ))
                                .child(button("close-tab", "Close tab", theme).on_click(
                                    cx.listener(|this, _, window, cx| {
                                        this.top_bar.update(cx, |bar, cx| {
                                            bar.dispatch(Msg::CloseActive, window, cx)
                                        });
                                    }),
                                )),
                        })
                        .when_some(self.error.clone(), |this, error| {
                            this.child(div().text_color(theme.error).child(error))
                        })
                        .when(directory.is_none(), |this| {
                            this.child(
                                button(
                                    "open-repository",
                                    if self.opening.is_some() {
                                        "Opening…"
                                    } else {
                                        "Open repository…"
                                    },
                                    theme,
                                )
                                .on_click(cx.listener(|this, _, _, cx| this.prompt_repository(cx))),
                            )
                            .children(recent.into_iter().map(|(id, title)| {
                                div()
                                    .id(("recent-project", id.as_u64()))
                                    .cursor_pointer()
                                    .child(title)
                                    .on_click(
                                        cx.listener(move |this, _, _, cx| this.open_saved(id, cx)),
                                    )
                            }))
                        }),
                );
        let root = root.when_some(
            self.recent.as_ref().map(|menu| menu.view.clone()),
            |root, menu| root.child(menu),
        );
        #[cfg(any(debug_assertions, feature = "frame-metrics"))]
        let root = root.child(self.debug_panel.clone());
        root
    }
}

#[cfg(test)]
mod tests;
