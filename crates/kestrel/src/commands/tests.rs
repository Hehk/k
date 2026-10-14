use super::*;
use crate::workspace::{LoadedWorkspace, Workspace};
use gpui::{AppContext, TestAppContext};
use kestrel::storage::Store;

fn setup(cx: &mut TestAppContext) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    cx.update(|cx| {
        let workspace = cx.new(|cx| {
            Workspace::new(
                LoadedWorkspace::load(Ok(Store::new(dir.path().join("state.json")))),
                cx,
            )
        });
        cx.set_global(Session(workspace));
        install(cx);
    });
    dir
}

fn open(cx: &mut TestAppContext) -> WindowHandle<Kestrel> {
    let handle = cx.update(|cx| {
        let workspace = cx.global::<Session>().0.clone();
        Kestrel::new_window(workspace, cx).unwrap()
    });
    cx.run_until_parked();
    handle
        .update(cx, |_, window, _| window.activate_window())
        .unwrap();
    handle
}

fn navigation_history(
    cx: &mut TestAppContext,
    handle: WindowHandle<Kestrel>,
    path: &std::path::Path,
) {
    git2::Repository::init(path).unwrap();
    let info = kestrel::git::Repository::open(path)
        .unwrap()
        .info()
        .unwrap();
    let window = handle.read_with(cx, |root, _| root.id()).unwrap();
    cx.update(|cx| {
        let workspace = cx.global::<Session>().0.clone();
        workspace.update(cx, |workspace, cx| {
            workspace
                .edit(
                    |state| {
                        let tab = state.window(window)?.active_tab.unwrap();
                        let project = state.register_project(&info)?;
                        state.open_project(project, tab)?;
                        state.go_home(tab)?;
                        state.go_back(tab)
                    },
                    cx,
                )
                .unwrap()
        });
    });
    cx.run_until_parked();
}

fn assert_navigation_enabled(cx: &mut TestAppContext, back: bool, forward: bool) {
    cx.update(|cx| {
        let states = menu_state(cx);
        for (label, enabled) in [("Back", back), ("Forward", forward)] {
            assert_eq!(
                states
                    .iter()
                    .find(|state| state.label == label)
                    .unwrap()
                    .enabled,
                enabled,
                "{label}"
            );
        }
    });
}

fn snapshot(cx: &mut TestAppContext) -> kestrel::state::AppState {
    cx.update(|cx| cx.global::<Session>().0.read(cx).state().clone())
}

fn menu_action(cx: &mut TestAppContext, label: &str) {
    let action = menus()
        .into_iter()
        .flat_map(|menu| menu.items)
        .find_map(|item| match item {
            MenuItem::Action { name, action, .. } if name == label => Some(action),
            _ => None,
        })
        .unwrap();
    cx.update(|cx| cx.dispatch_action(action.as_ref()));
    cx.run_until_parked();
}

#[gpui::test]
fn zero_window_commands_and_menu_enablement(cx: &mut TestAppContext) {
    let _dir = setup(cx);
    cx.update(|cx| {
        let states = menu_state(cx);
        for label in ["New Window", "New Tab", "Quit Kestrel"] {
            assert!(
                states
                    .iter()
                    .find(|state| state.label == label)
                    .unwrap()
                    .enabled
            );
        }
        for label in [
            "Close Tab",
            "Close Window",
            "Reopen Closed Tab or Window",
            "Minimize",
            "Back",
            "Forward",
        ] {
            assert!(
                !states
                    .iter()
                    .find(|state| state.label == label)
                    .unwrap()
                    .enabled
            );
        }
        cx.dispatch_action(&Command::Back);
        cx.dispatch_action(&Command::Forward);
        cx.dispatch_action(&TabAction::New);
    });
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(cx.windows().len(), 1);
        let workspace = &cx.global::<Session>().0;
        assert_eq!(workspace.read(cx).state().windows()[0].tabs.len(), 1);
        cx.dispatch_action(&Command::CloseAllWindows);
    });
    cx.run_until_parked();
    cx.update(|cx| {
        assert!(cx.windows().is_empty());
        assert!(
            menu_state(cx)
                .iter()
                .find(|state| state.label == "Reopen Closed Tab or Window")
                .unwrap()
                .enabled
        );
        cx.dispatch_action(&Command::NewWindow);
    });
    cx.run_until_parked();
    cx.update(|cx| assert_eq!(cx.windows().len(), 1));
}

#[gpui::test]
fn navigation_menus_and_shortcuts_follow_the_front_window_and_survive_reopening(
    cx: &mut TestAppContext,
) {
    let dir = setup(cx);
    let a = open(cx);
    navigation_history(cx, a, &dir.path().join("a"));
    let a_before = snapshot(cx).windows()[0].clone();
    let b = open(cx);
    navigation_history(cx, b, &dir.path().join("b"));
    let b_before = snapshot(cx).windows()[1].clone();
    assert_navigation_enabled(cx, true, true);
    menu_action(cx, "Back");
    assert_navigation_enabled(cx, false, true);
    assert_eq!(snapshot(cx).windows()[0], a_before);
    cx.simulate_keystrokes(b.into(), "cmd-]");
    cx.run_until_parked();
    let b_returned = snapshot(cx).windows()[1].tabs[0].clone();
    assert_eq!(
        b_returned.snapshot().unwrap(),
        b_before.tabs[0].snapshot().unwrap()
    );
    assert!(b_returned.activation() > b_before.tabs[0].activation());
    menu_action(cx, "Forward");
    assert_navigation_enabled(cx, true, false);
    let b_after = snapshot(cx).windows()[1].clone();
    a.update(cx, |_, window, _| window.activate_window())
        .unwrap();
    cx.run_until_parked();
    assert_navigation_enabled(cx, true, true);
    cx.simulate_keystrokes(a.into(), "cmd-[");
    cx.run_until_parked();
    assert_navigation_enabled(cx, false, true);
    assert_eq!(snapshot(cx).windows()[1], b_after);
    cx.simulate_keystrokes(a.into(), "cmd-]");
    cx.run_until_parked();
    let a_returned = snapshot(cx).windows()[0].clone();
    assert_eq!(
        a_returned.tabs[0].snapshot().unwrap(),
        a_before.tabs[0].snapshot().unwrap()
    );
    assert!(a_returned.tabs[0].activation() > a_before.tabs[0].activation());
    menu_action(cx, "Close Window");
    b.update(cx, |_, window, _| window.activate_window())
        .unwrap();
    menu_action(cx, "Reopen Closed Tab or Window");
    assert_navigation_enabled(cx, true, true);
    let restored = snapshot(cx).windows()[1].tabs[0].clone();
    let mut expected = a_returned.tabs[0].clone();
    assert_ne!(restored.id, expected.id);
    expected.id = restored.id;
    assert_eq!(restored, expected);
    menu_action(cx, "Back");
    assert_navigation_enabled(cx, false, true);
    assert_eq!(snapshot(cx).windows()[0], b_after);
}

#[gpui::test]
fn production_shortcuts_close_only_the_front_window_and_reopen_it(cx: &mut TestAppContext) {
    let _dir = setup(cx);
    let a = open(cx);
    cx.simulate_keystrokes(a.into(), "cmd-t cmd-t cmd-1");
    let a_before = snapshot(cx).windows()[0].clone();
    let b = open(cx);
    let b_before = snapshot(cx).windows()[1].clone();
    a.update(cx, |_, window, _| window.activate_window())
        .unwrap();
    cx.simulate_keystrokes(a.into(), "cmd-shift-w");
    cx.run_until_parked();
    assert_eq!(snapshot(cx).windows(), std::slice::from_ref(&b_before));
    b.update(cx, |_, window, _| window.activate_window())
        .unwrap();
    cx.simulate_keystrokes(b.into(), "cmd-shift-t");
    cx.run_until_parked();
    let after = snapshot(cx);
    assert_eq!(after.windows()[0], b_before);
    let restored = &after.windows()[1];
    assert_ne!(restored.id, a_before.id);
    assert_eq!(restored.tabs.len(), 3);
    assert_eq!(restored.active_tab, Some(restored.tabs[0].id));
    assert!(
        restored
            .tabs
            .iter()
            .all(|tab| a_before.tabs.iter().all(|old| old.id != tab.id))
    );
}

#[gpui::test]
fn keyboard_and_menu_tab_commands_have_identical_effects(cx: &mut TestAppContext) {
    let _dir = setup(cx);
    let window = open(cx);
    menu_action(cx, "New Tab");
    cx.simulate_keystrokes(window.into(), "cmd-t cmd-1 ctrl-shift-tab");
    let state = snapshot(cx);
    let tabs = state.windows()[0].tabs.clone();
    assert_eq!(tabs.len(), 3);
    assert_eq!(state.windows()[0].active_tab, Some(tabs[2].id));
    menu_action(cx, "Move Tab Left");
    let state = snapshot(cx);
    assert_eq!(
        state.windows()[0]
            .tabs
            .iter()
            .map(|tab| tab.id)
            .collect::<Vec<_>>(),
        [tabs[0].id, tabs[2].id, tabs[1].id]
    );
    cx.simulate_keystrokes(window.into(), "cmd-9");
    assert_eq!(snapshot(cx).windows()[0].active_tab, Some(tabs[1].id));
    menu_action(cx, "Close Tab");
    assert_eq!(snapshot(cx).windows()[0].tabs.len(), 2);
    cx.simulate_keystrokes(window.into(), "cmd-shift-t");
    cx.run_until_parked();
    let restored = snapshot(cx);
    assert_eq!(restored.windows()[0].tabs.len(), 3);
    assert_ne!(restored.windows()[0].active_tab, Some(tabs[1].id));
    menu_action(cx, "Close Other Tabs");
    assert_eq!(snapshot(cx).windows()[0].tabs.len(), 1);
    cx.simulate_keystrokes(window.into(), "cmd-w");
    cx.run_until_parked();
    assert!(snapshot(cx).windows().is_empty());
}

#[gpui::test]
fn full_screen_menu_tracks_state_and_detach_preserves_tab_identity(cx: &mut TestAppContext) {
    let _dir = setup(cx);
    let window = open(cx);
    cx.simulate_keystrokes(window.into(), "ctrl-cmd-f");
    cx.run_until_parked();
    cx.update(|cx| {
        let states = menu_state(cx);
        assert_eq!(
            states
                .iter()
                .find(|item| item.label == "Enter Full Screen")
                .unwrap()
                .title,
            "Exit Full Screen"
        );
        assert!(
            !states
                .iter()
                .find(|item| item.label == "Minimize")
                .unwrap()
                .enabled
        );
    });
    menu_action(cx, "Enter Full Screen");
    cx.update(|cx| {
        assert_eq!(
            menu_state(cx)
                .iter()
                .find(|item| item.label == "Enter Full Screen")
                .unwrap()
                .title,
            "Enter Full Screen"
        )
    });
    cx.simulate_keystrokes(window.into(), "cmd-t");
    let before = snapshot(cx);
    let selected = before.windows()[0].active_tab.unwrap();
    menu_action(cx, "Move Tab to New Window");
    let after = snapshot(cx);
    assert_eq!(after.windows().len(), 2);
    assert_eq!(after.windows()[1].active_tab, Some(selected));
    cx.update(|cx| assert!(!cx.global::<Session>().0.read(cx).can_reopen()));
}

#[gpui::test]
fn quit_binding_and_shutdown_preserve_open_contexts(cx: &mut TestAppContext) {
    let dir = setup(cx);
    let window = open(cx);
    cx.simulate_keystrokes(window.into(), "cmd-t cmd-n");
    cx.run_until_parked();
    let before = snapshot(cx);
    assert_eq!(before.windows().len(), 2);
    let quit = std::rc::Rc::new(std::cell::Cell::new(0));
    let observed = quit.clone();
    cx.update(|cx| {
        cx.observe_keystrokes(move |event, _, _| {
            if event.action.as_ref().is_some_and(|action| {
                action.as_any().downcast_ref::<Command>() == Some(&Command::Quit)
            }) {
                observed.set(observed.get() + 1);
            }
        })
        .detach()
    });
    let active = cx.update(|cx| cx.active_window().unwrap());
    cx.simulate_keystrokes(active, "cmd-q");
    cx.run_until_parked();
    assert_eq!(quit.get(), 1);
    // TestPlatform::quit is a no-op; explicitly deliver the OS shutdown callback.
    cx.quit();
    assert_eq!(
        Store::new(dir.path().join("state.json"))
            .load()
            .unwrap()
            .state,
        before
    );
    cx.update(|cx| assert!(cx.windows().is_empty()));
}

#[gpui::test]
fn standard_bindings_leave_system_window_and_desktop_navigation_unclaimed(cx: &mut TestAppContext) {
    let _dir = setup(cx);
    cx.update(|cx| {
        for (key, command) in [
            ("cmd-[", Command::Back),
            ("cmd-]", Command::Forward),
            ("cmd-n", Command::NewWindow),
            ("cmd-shift-w", Command::CloseWindow),
            ("cmd-alt-w", Command::CloseAllWindows),
            ("cmd-shift-t", Command::ReopenClosed),
            ("cmd-m", Command::Minimize),
            ("cmd-alt-m", Command::MinimizeAll),
            ("cmd-h", Command::Hide),
            ("cmd-alt-h", Command::HideOthers),
            ("ctrl-cmd-f", Command::FullScreen),
            ("cmd-q", Command::Quit),
        ] {
            let bindings = cx.all_bindings_for_input(&[gpui::Keystroke::parse(key).unwrap()]);
            assert_eq!(bindings.len(), 1, "{key}");
            assert!(bindings[0].action().partial_eq(&command), "{key}");
        }
        for key in ["cmd-`", "cmd-shift-`", "cmd-tab", "ctrl-left", "ctrl-right"] {
            assert!(
                cx.all_bindings_for_input(&[gpui::Keystroke::parse(key).unwrap()])
                    .is_empty(),
                "{key}"
            );
        }
    });
}

#[gpui::test]
fn menus_and_commands_are_guarded_during_a_tab_drag(cx: &mut TestAppContext) {
    use crate::top_bar::model::{DragToken, Msg};
    let dir = setup(cx);
    let window = open(cx);
    cx.simulate_keystrokes(window.into(), "cmd-t");
    navigation_history(cx, window, &dir.path().join("repo"));
    assert_navigation_enabled(cx, true, true);
    let token = window
        .update(cx, |root, window, cx| {
            let bar = root.top_bar.clone();
            let token = DragToken {
                source: bar.entity_id().as_u64(),
                serial: 1,
            };
            bar.update(cx, |bar, cx| {
                bar.dispatch(
                    Msg::DragStarted {
                        tab: bar.active_tab().unwrap().id,
                        token,
                    },
                    window,
                    cx,
                );
            });
            assert!(!root.commands_allowed(cx));
            token
        })
        .unwrap();
    cx.run_until_parked();
    let before = snapshot(cx);
    cx.update(|cx| {
        let states = menu_state(cx);
        for label in [
            "New Tab",
            "Close Tab",
            "Close Window",
            "Close All Windows",
            "Move Tab Left",
            "Move Tab to New Window",
            "Back",
            "Forward",
        ] {
            assert!(
                !states
                    .iter()
                    .find(|item| item.label == label)
                    .unwrap()
                    .enabled,
                "{label}"
            );
        }
    });
    cx.simulate_keystrokes(
        window.into(),
        "cmd-t cmd-w cmd-9 cmd-shift-w cmd-alt-w cmd-[ cmd-]",
    );
    cx.run_until_parked();
    menu_action(cx, "Back");
    menu_action(cx, "Forward");
    assert_eq!(snapshot(cx), before);
    window
        .update(cx, |root, window, cx| {
            root.top_bar.update(cx, |bar, cx| {
                bar.dispatch(Msg::Cancelled { token }, window, cx)
            });
        })
        .unwrap();
    cx.run_until_parked();
    cx.simulate_keystrokes(window.into(), "cmd-t");
    assert_eq!(snapshot(cx).windows()[0].tabs.len(), 3);
}

#[test]
fn menus_expose_the_expected_commands_once() {
    let menus = menus();
    assert_eq!(
        menus
            .iter()
            .map(|menu| menu.name.as_ref())
            .collect::<Vec<_>>(),
        ["Kestrel", "File", "Go", "Window"]
    );
    let mut names = std::collections::HashSet::new();
    for item in menus.into_iter().flat_map(|menu| menu.items) {
        if let MenuItem::Action { name, .. } = item {
            assert!(
                names.insert(name.to_string()),
                "duplicate menu command {name}"
            );
        }
    }
    for name in [
        "Back",
        "Forward",
        "New Window",
        "New Tab",
        "Close Tab",
        "Close Window",
        "Close All Windows",
        "Reopen Closed Tab or Window",
        "Minimize",
        "Minimize All",
        "Hide Kestrel",
        "Hide Others",
        "Show All",
        "Enter Full Screen",
        "Zoom",
        "Bring All to Front",
        "Move Tab to New Window",
        "Move Tab Left",
        "Move Tab Right",
        "Close Other Tabs",
        "Close Tabs to the Right",
    ] {
        assert!(names.contains(name), "missing {name}");
    }
}
