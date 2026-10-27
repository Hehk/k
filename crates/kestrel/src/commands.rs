use gpui::{
    Action, App, Global, KeyBinding, Menu, MenuItem, SharedString, SystemMenuType, WindowHandle,
};

use crate::{
    top_bar::{self, TabAction},
    workspace::Session,
    workspace_window::Kestrel,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Action)]
#[action(namespace = kestrel, no_json)]
pub enum Command {
    NewWindow,
    CloseWindow,
    CloseAllWindows,
    ReopenClosed,
    DetachTab,
    Back,
    Forward,
    RecentlyViewed,
    Minimize,
    MinimizeAll,
    Zoom,
    FullScreen,
    BringAllToFront,
    Hide,
    HideOthers,
    ShowAll,
    Quit,
}

#[derive(Default)]
struct Installed {
    pending: bool,
    menu_state: Option<Vec<MenuState>>,
}
impl Global for Installed {}

#[derive(Clone, Debug, PartialEq)]
pub struct MenuState {
    pub menu: SharedString,
    pub label: SharedString,
    pub title: SharedString,
    pub enabled: bool,
}

pub fn install(cx: &mut App) {
    cx.set_global(Installed::default());
    top_bar::bind_keys(cx);
    crate::text_input::bind_keys(cx);
    crate::diff_selection::bind_keys(cx);
    cx.bind_keys([
        KeyBinding::new("cmd-[", Command::Back, Some("Workspace")),
        KeyBinding::new("cmd-]", Command::Forward, Some("Workspace")),
        KeyBinding::new("cmd-n", Command::NewWindow, None),
        KeyBinding::new("cmd-shift-w", Command::CloseWindow, None),
        KeyBinding::new("cmd-alt-w", Command::CloseAllWindows, None),
        KeyBinding::new("cmd-shift-t", Command::ReopenClosed, None),
        KeyBinding::new("cmd-m", Command::Minimize, None),
        KeyBinding::new("cmd-alt-m", Command::MinimizeAll, None),
        KeyBinding::new("ctrl-cmd-f", Command::FullScreen, None),
        KeyBinding::new("cmd-h", Command::Hide, None),
        KeyBinding::new("cmd-alt-h", Command::HideOthers, None),
        KeyBinding::new("cmd-q", Command::Quit, None),
    ]);
    cx.on_action(|command: &Command, cx| {
        let command = *command;
        let target = active_window(cx);
        cx.defer(move |cx| execute(command, target, cx));
    });
    cx.on_action(|action: &TabAction, cx| {
        if *action == TabAction::New && active_window(cx).is_none() {
            cx.defer(new_window);
        }
    });
    cx.set_menus(menus());
    let workspace = cx.global::<Session>().0.clone();
    cx.observe(&workspace, |_, cx| refresh_later(cx)).detach();
    cx.on_window_closed(refresh_later).detach();
    refresh_later(cx);
}

pub fn menus() -> Vec<Menu> {
    vec![
        Menu {
            name: "Kestrel".into(),
            items: vec![
                MenuItem::os_submenu("Services", SystemMenuType::Services),
                MenuItem::separator(),
                MenuItem::action("Hide Kestrel", Command::Hide),
                MenuItem::action("Hide Others", Command::HideOthers),
                MenuItem::action("Show All", Command::ShowAll),
                MenuItem::separator(),
                MenuItem::action("Quit Kestrel", Command::Quit),
            ],
        },
        Menu {
            name: "File".into(),
            items: vec![
                MenuItem::action("New Window", Command::NewWindow),
                MenuItem::action("New Tab", TabAction::New),
                MenuItem::action("Reopen Closed Tab or Window", Command::ReopenClosed),
                MenuItem::separator(),
                MenuItem::action("Close Tab", TabAction::Close),
                MenuItem::action("Close Window", Command::CloseWindow),
                MenuItem::action("Close All Windows", Command::CloseAllWindows),
            ],
        },
        Menu {
            name: "Go".into(),
            items: vec![
                MenuItem::action("Back", Command::Back),
                MenuItem::action("Forward", Command::Forward),
                MenuItem::action("Recently Viewed", Command::RecentlyViewed),
            ],
        },
        Menu {
            name: "Window".into(),
            items: vec![
                MenuItem::action("Minimize", Command::Minimize),
                MenuItem::action("Minimize All", Command::MinimizeAll),
                MenuItem::action("Zoom", Command::Zoom),
                MenuItem::action("Enter Full Screen", Command::FullScreen),
                MenuItem::separator(),
                MenuItem::action("Show Previous Tab", TabAction::Previous),
                MenuItem::action("Show Next Tab", TabAction::Next),
                MenuItem::action("Select Last Tab", TabAction::Last),
                MenuItem::action("Move Tab Left", TabAction::MoveLeft),
                MenuItem::action("Move Tab Right", TabAction::MoveRight),
                MenuItem::action("Move Tab to New Window", Command::DetachTab),
                MenuItem::action("Close Other Tabs", TabAction::CloseOthers),
                MenuItem::action("Close Tabs to the Right", TabAction::CloseRight),
                MenuItem::separator(),
                MenuItem::action("Bring All to Front", Command::BringAllToFront),
            ],
        },
    ]
}

fn active_window(cx: &App) -> Option<WindowHandle<Kestrel>> {
    cx.active_window()?.downcast::<Kestrel>()
}

pub fn new_window(cx: &mut App) {
    let workspace = cx.global::<Session>().0.clone();
    if let Err(error) = Kestrel::new_window(workspace, cx) {
        eprintln!("Could not open window: {error}");
    }
    refresh_later(cx);
}

fn execute(command: Command, target: Option<WindowHandle<Kestrel>>, cx: &mut App) {
    let allowed = target
        .and_then(|handle| handle.read(cx).ok())
        .is_none_or(|root| root.commands_allowed(cx));
    if !allowed
        && !matches!(
            command,
            Command::Quit | Command::Hide | Command::HideOthers | Command::ShowAll
        )
    {
        return;
    }
    match command {
        Command::NewWindow => new_window(cx),
        Command::Quit => cx.quit(),
        Command::Hide => cx.hide(),
        Command::HideOthers => cx.hide_other_apps(),
        Command::ShowAll => cx.unhide_other_apps(),
        Command::ReopenClosed => {
            let workspace = cx.global::<Session>().0.clone();
            let target = target
                .and_then(|handle| handle.read(cx).ok())
                .map(|root| root.id());
            if let Err(error) = Kestrel::reopen(workspace, target, cx) {
                eprintln!("Could not reopen context: {error}");
            }
        }
        Command::CloseAllWindows | Command::MinimizeAll => {
            let handles = cx
                .windows()
                .into_iter()
                .filter_map(|handle| handle.downcast::<Kestrel>())
                .collect::<Vec<_>>();
            if handles
                .iter()
                .any(|handle| handle.read(cx).is_ok_and(|root| !root.commands_allowed(cx)))
            {
                return;
            }
            for handle in handles {
                let _ = handle.update(cx, |root, window, cx| {
                    if command == Command::CloseAllWindows {
                        root.close(window, cx);
                    } else if !window.is_fullscreen() {
                        window.minimize_window();
                    }
                });
            }
        }
        Command::BringAllToFront => crate::native::bring_all_to_front(),
        _ => {
            if let Some(target) = target {
                let _ = target.update(cx, |root, window, cx| root.command(command, window, cx));
            }
        }
    }
    refresh_later(cx);
}

pub fn menu_state(cx: &mut App) -> Vec<MenuState> {
    let target = active_window(cx);
    let mut fullscreen = false;
    if let Some(target) = target {
        let _ = target.update(cx, |_, window, _| fullscreen = window.is_fullscreen());
    }
    let can_minimize = cx.windows().into_iter().any(|handle| {
        handle.downcast::<Kestrel>().is_some()
            && handle
                .update(cx, |_, window, _| !window.is_fullscreen())
                .unwrap_or(false)
    });
    let cx: &App = cx;
    let root = target.and_then(|handle| handle.read(cx).ok());
    let allowed = root.is_none_or(|root| root.commands_allowed(cx));
    let window = root.is_some() && allowed;
    let all_allowed = cx
        .windows()
        .iter()
        .filter_map(|handle| handle.downcast::<Kestrel>())
        .all(|handle| handle.read(cx).is_ok_and(|root| root.commands_allowed(cx)));
    let has_windows = !cx.windows().is_empty();
    let history = cx.global::<Session>().0.read(cx).can_reopen();
    menus()
        .into_iter()
        .flat_map(|menu| {
            menu.items.into_iter().filter_map(move |item| {
                let MenuItem::Action { name, action, .. } = item else {
                    return None;
                };
                let mut title = name.clone();
                let enabled = if let Some(command) = action.as_any().downcast_ref::<Command>() {
                    match command {
                        Command::Back | Command::Forward => {
                            root.is_some_and(|root| root.navigation_allowed(*command, cx))
                        }
                        Command::RecentlyViewed => window,
                        Command::NewWindow => allowed,
                        Command::CloseWindow => window,
                        Command::CloseAllWindows => has_windows && all_allowed,
                        Command::MinimizeAll => can_minimize && all_allowed,
                        Command::ReopenClosed => allowed && history,
                        Command::DetachTab => {
                            window && root.is_some_and(|root| root.tab_count(cx) > 1)
                        }
                        Command::Minimize | Command::Zoom => window && !fullscreen,
                        Command::FullScreen => {
                            if fullscreen {
                                title = "Exit Full Screen".into();
                            }
                            window
                        }
                        Command::BringAllToFront => has_windows && allowed,
                        Command::Hide | Command::HideOthers | Command::ShowAll | Command::Quit => {
                            true
                        }
                    }
                } else if let Some(tab) = action.as_any().downcast_ref::<TabAction>() {
                    (*tab == TabAction::New && root.is_none())
                        || (allowed && root.is_some_and(|root| root.tab_action_allowed(tab, cx)))
                } else {
                    return None;
                };
                Some(MenuState {
                    menu: menu.name.clone(),
                    label: name,
                    title,
                    enabled,
                })
            })
        })
        .collect()
}

pub fn refresh_later(cx: &mut App) {
    let Some(installed) = cx.try_global::<Installed>() else {
        return;
    };
    if installed.pending {
        return;
    }
    cx.global_mut::<Installed>().pending = true;
    cx.defer(|cx| {
        let state = menu_state(cx);
        let installed = cx.global_mut::<Installed>();
        installed.pending = false;
        if installed.menu_state.as_ref() != Some(&state) {
            crate::native::update_menus(&state);
            installed.menu_state = Some(state);
        }
    });
}

#[cfg(test)]
mod tests;
