use super::*;
use gpui::{WindowHandle, point, size};

fn start(cx: &mut TestAppContext) -> (tempfile::TempDir, Entity<Workspace>, WindowHandle<Kestrel>) {
    let dir = tempfile::tempdir().unwrap();
    let workspace = shared(cx, dir.path().join("state.json"));
    cx.update(|cx| {
        cx.set_global(crate::workspace::Session(workspace.clone()));
        commands::install(cx);
    });
    let handle = cx.update(|cx| Kestrel::new_window(workspace.clone(), cx).unwrap());
    handle
        .update(cx, |_, window, _| window.activate_window())
        .unwrap();
    cx.run_until_parked();
    (dir, workspace, handle)
}

fn is_open(handle: WindowHandle<Kestrel>, cx: &VisualTestContext) -> bool {
    handle
        .read_with(cx, |root, _| root.recent.is_some())
        .unwrap()
}

fn click_clock(cx: &mut VisualTestContext) {
    let clock = cx.debug_bounds("recently-viewed").unwrap();
    cx.simulate_click(clock.center(), Modifiers::none());
    redraw(cx);
}

#[gpui::test]
fn empty_recent_menu_toggles_dismisses_and_returns_keyboard_focus_without_visits(
    cx: &mut TestAppContext,
) {
    let (_dir, workspace, handle) = start(cx);
    let cx = &mut VisualTestContext::from_window(handle.into(), cx);
    redraw(cx);
    let back = cx.debug_bounds("navigate-back").unwrap();
    let forward = cx.debug_bounds("navigate-forward").unwrap();
    let clock = cx.debug_bounds("recently-viewed").unwrap();
    assert!(back.left() >= px(crate::top_bar::model::SIDE_WIDTH));
    assert!(back.right() <= forward.left() && forward.right() <= clock.left());
    assert_eq!(clock.center().y, px(crate::top_bar::TITLEBAR_HEIGHT / 2.));
    let before = workspace.read_with(cx, |workspace, _| workspace.state().clone());
    cx.simulate_keystrokes("tab");
    let trigger_focus = cx.update(|window, cx| window.focused(cx).unwrap());
    cx.simulate_keystrokes("enter");
    redraw(cx);
    assert!(is_open(handle, cx));
    cx.simulate_keystrokes("down up enter escape");
    redraw(cx);
    assert!(!is_open(handle, cx));
    cx.update(|window, _| assert!(trigger_focus.is_focused(window)));
    click_clock(cx);
    assert!(is_open(handle, cx));
    click_clock(cx);
    assert!(!is_open(handle, cx));
    click_clock(cx);
    cx.simulate_click(point(px(800.), px(500.)), Modifiers::none());
    redraw(cx);
    assert!(!is_open(handle, cx));
    workspace.read_with(cx, |workspace, _| {
        assert_eq!(workspace.state(), &before);
        assert!(workspace.viewing_history().records().is_empty());
    });
}

#[gpui::test]
fn recent_click_targets_this_tab_and_command_click_creates_a_fresh_tab(cx: &mut TestAppContext) {
    let (dir, workspace, handle) = start(cx);
    let one = repository(&dir.path().join("one"));
    let two = repository(&dir.path().join("two"));
    handle
        .update(cx, |root, _, cx| open(root, one, cx))
        .unwrap();
    cx.run_until_parked();
    handle
        .update(cx, |root, _, cx| open(root, two, cx))
        .unwrap();
    cx.run_until_parked();
    let original = active(&workspace, 0, cx);
    let one = workspace.read_with(cx, |workspace, _| {
        workspace.viewing_history().records()[0]
            .viewed
            .snapshot
            .clone()
    });
    let cx = &mut VisualTestContext::from_window(handle.into(), cx);
    redraw(cx);
    click_clock(cx);
    let row = cx.debug_bounds("recent-view-0").unwrap();
    cx.simulate_mouse_move(row.center(), None, Modifiers::none());
    redraw(cx);
    workspace.read_with(cx, |workspace, _| {
        assert_eq!(workspace.viewing_history().records().len(), 2)
    });
    cx.simulate_click(row.center(), Modifiers::none());
    redraw(cx);
    assert!(!is_open(handle, cx));
    workspace.read_with(cx, |workspace, _| {
        assert_eq!(workspace.state().windows()[0].tabs.len(), 1);
        assert_eq!(
            workspace
                .state()
                .tab(original)
                .unwrap()
                .snapshot()
                .unwrap()
                .snapshot,
            one
        );
        assert_eq!(workspace.viewing_history().records().len(), 3);
    });
    click_clock(cx);
    let row = cx.debug_bounds("recent-view-1").unwrap();
    cx.simulate_click(
        row.center(),
        Modifiers {
            platform: true,
            ..Modifiers::none()
        },
    );
    redraw(cx);
    assert!(!is_open(handle, cx));
    workspace.read_with(cx, |workspace, _| {
        let window = &workspace.state().windows()[0];
        assert_eq!(window.tabs.len(), 2);
        assert_ne!(window.active_tab, Some(original));
        assert_eq!(
            workspace
                .state()
                .tab(original)
                .unwrap()
                .snapshot()
                .unwrap()
                .snapshot,
            one
        );
        assert_eq!(workspace.viewing_history().records().len(), 4);
        assert_eq!(
            workspace
                .viewing_history()
                .records()
                .last()
                .unwrap()
                .viewed
                .labels
                .repository,
            "two"
        );
    });
}

#[gpui::test]
fn recent_limit_keyboard_scrolling_and_narrow_window_bounds_use_the_shared_projection(
    cx: &mut TestAppContext,
) {
    let (dir, workspace, handle) = start(cx);
    for index in 0..13 {
        let info = repository(&dir.path().join(format!("repo-{index}")));
        handle
            .update(cx, |root, _, cx| open(root, info, cx))
            .unwrap();
    }
    cx.run_until_parked();
    let target = workspace.read_with(cx, |workspace, _| {
        workspace
            .viewing_history()
            .recent(10)
            .last()
            .unwrap()
            .viewed
            .snapshot
            .clone()
    });
    let cx = &mut VisualTestContext::from_window(handle.into(), cx);
    cx.simulate_resize(size(px(360.), px(280.)));
    redraw(cx);
    click_clock(cx);
    let popup = cx.debug_bounds("recent-views").unwrap();
    assert!(popup.left() >= px(0.) && popup.right() <= px(360.));
    assert!(popup.top() >= px(40.) && popup.bottom() <= px(280.));
    cx.simulate_keystrokes("end");
    redraw(cx);
    assert!(popup.contains(&cx.debug_bounds("recent-view-3").unwrap().center()));
    workspace.read_with(cx, |workspace, _| {
        assert_eq!(workspace.viewing_history().records().len(), 13)
    });
    cx.simulate_keystrokes("enter");
    redraw(cx);
    assert!(!is_open(handle, cx));
    workspace.read_with(cx, |workspace, _| {
        let window = &workspace.state().windows()[0];
        assert_eq!(
            workspace
                .state()
                .tab(window.active_tab.unwrap())
                .unwrap()
                .snapshot()
                .unwrap()
                .snapshot,
            target
        );
        assert_eq!(workspace.viewing_history().records().len(), 14);
    });
}

#[gpui::test]
fn recent_selection_does_not_jump_to_an_existing_destination_in_another_window(
    cx: &mut TestAppContext,
) {
    let (dir, workspace, source) = start(cx);
    let info = repository(&dir.path().join("one"));
    source
        .update(cx, |root, _, cx| open(root, info, cx))
        .unwrap();
    cx.run_until_parked();
    let saved = workspace.read_with(cx, |workspace, _| {
        workspace.state().windows()[0].tabs[0].clone()
    });
    let target = cx.update(|cx| Kestrel::new_window(workspace.clone(), cx).unwrap());
    target
        .update(cx, |_, window, _| window.activate_window())
        .unwrap();
    cx.run_until_parked();
    let cx = &mut VisualTestContext::from_window(target.into(), cx);
    redraw(cx);
    cx.cx
        .update(|cx| cx.dispatch_action(&Command::RecentlyViewed));
    redraw(cx);
    assert!(is_open(target, cx));
    cx.simulate_keystrokes("enter");
    redraw(cx);
    workspace.read_with(cx, |workspace, _| {
        let state = workspace.state();
        assert_eq!(state.windows().len(), 2);
        assert_eq!(state.tab(saved.id).unwrap(), &saved);
        assert_eq!(state.active_window(), Some(state.windows()[1].id));
        let destination = &state.windows()[1].tabs[0];
        assert_ne!(destination.id, saved.id);
        assert_eq!(destination.route, saved.route);
        assert_eq!(workspace.viewing_history().records().len(), 2);
    });
}

#[gpui::test]
fn managed_titlebar_drag_targets_start_after_navigation(cx: &mut TestAppContext) {
    let (_dir, workspace, handle) = start(cx);
    let cx = &mut VisualTestContext::from_window(handle.into(), cx);
    redraw(cx);
    cx.simulate_keystrokes("cmd-t cmd-t");
    redraw(cx);
    let ids = workspace.read_with(cx, |workspace, _| {
        workspace.state().windows()[0]
            .tabs
            .iter()
            .map(|tab| tab.id.as_u64())
            .collect::<Vec<_>>()
    });
    assert_eq!(ids.len(), 3);
    let source = cx
        .debug_bounds(Box::leak(format!("tab-slot-{}", ids[0]).into_boxed_str()))
        .unwrap()
        .center();
    let target = cx
        .debug_bounds(Box::leak(format!("tab-slot-{}", ids[2]).into_boxed_str()))
        .unwrap()
        .center();
    assert!(cx.debug_bounds("recently-viewed").unwrap().right() < source.x);
    cx.simulate_mouse_down(source, gpui::MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_move(
        source + point(px(10.), px(0.)),
        gpui::MouseButton::Left,
        Modifiers::none(),
    );
    redraw(cx);
    cx.simulate_mouse_move(target, gpui::MouseButton::Left, Modifiers::none());
    redraw(cx);
    cx.simulate_mouse_up(target, gpui::MouseButton::Left, Modifiers::none());
    redraw(cx);
    workspace.read_with(cx, |workspace, _| {
        assert_eq!(
            workspace.state().windows()[0]
                .tabs
                .iter()
                .map(|tab| tab.id.as_u64())
                .collect::<Vec<_>>(),
            [ids[1], ids[2], ids[0]]
        );
        assert!(workspace.viewing_history().records().is_empty());
    });
    click_clock(cx);
    assert!(is_open(handle, cx));
}

#[gpui::test]
fn appearance_changes_preserve_the_popup_selection_and_history(cx: &mut TestAppContext) {
    let (dir, workspace, handle) = start(cx);
    for name in ["one", "two"] {
        let info = repository(&dir.path().join(name));
        handle
            .update(cx, |root, _, cx| open(root, info, cx))
            .unwrap();
        cx.run_until_parked();
    }
    let before = workspace.read_with(cx, |workspace, _| workspace.state().clone());
    let cx = &mut VisualTestContext::from_window(handle.into(), cx);
    redraw(cx);
    click_clock(cx);
    cx.simulate_keystrokes("down");
    redraw(cx);
    let menu = handle
        .read_with(cx, |root, _| root.recent.as_ref().unwrap().view.clone())
        .unwrap();
    for preferences in [
        Preferences {
            increase_contrast: true,
            ..Default::default()
        },
        Preferences {
            accent: gpui::rgb(0xff0000).into(),
            ..Default::default()
        },
        Preferences::default(),
    ] {
        cx.update(|_, cx| cx.set_global(preferences));
        redraw(cx);
        handle
            .read_with(cx, |root, _| {
                assert_eq!(root.recent.as_ref().unwrap().view, menu)
            })
            .unwrap();
        workspace.read_with(cx, |workspace, _| {
            assert_eq!(workspace.state(), &before);
            assert_eq!(workspace.viewing_history().records().len(), 2);
        });
    }
    cx.simulate_keystrokes("enter");
    redraw(cx);
    workspace.read_with(cx, |workspace, _| {
        assert_eq!(workspace.viewing_history().records().len(), 3);
        assert_eq!(
            workspace
                .viewing_history()
                .records()
                .last()
                .unwrap()
                .viewed
                .labels
                .repository,
            "one"
        );
    });
}

#[gpui::test]
fn navigation_double_clicks_do_not_zoom_the_window(cx: &mut TestAppContext) {
    let (_dir, _, handle) = start(cx);
    let cx = &mut VisualTestContext::from_window(handle.into(), cx);
    let zooms = std::rc::Rc::new(std::cell::Cell::new(0));
    let received = zooms.clone();
    cx.update(|_, cx| {
        cx.on_action(move |command: &Command, cx| {
            if *command == Command::Zoom {
                received.set(received.get() + 1);
                cx.stop_propagation();
            }
        })
    });
    redraw(cx);
    let double_click = |cx: &mut VisualTestContext, position| {
        cx.simulate_event(gpui::MouseDownEvent {
            position,
            button: gpui::MouseButton::Left,
            click_count: 2,
            ..Default::default()
        });
        cx.simulate_event(gpui::MouseUpEvent {
            position,
            button: gpui::MouseButton::Left,
            click_count: 2,
            ..Default::default()
        });
        redraw(cx);
    };
    for selector in ["navigate-back", "navigate-forward", "recently-viewed"] {
        let center = cx.debug_bounds(selector).unwrap().center();
        double_click(cx, center);
        assert_eq!(zooms.get(), 0);
    }
    cx.simulate_keystrokes("escape");
    redraw(cx);
    double_click(cx, point(px(80.), px(2.)));
    assert_eq!(zooms.get(), 1);
}

#[gpui::test]
fn recent_menu_closes_on_tab_switch_resize_and_focus_change(cx: &mut TestAppContext) {
    let (_dir, workspace, handle) = start(cx);
    let cx = &mut VisualTestContext::from_window(handle.into(), cx);
    redraw(cx);
    click_clock(cx);
    cx.simulate_keystrokes("cmd-t");
    redraw(cx);
    assert!(!is_open(handle, cx));
    let selected =
        workspace.read_with(cx, |workspace, _| workspace.state().windows()[0].active_tab);
    click_clock(cx);
    cx.simulate_keystrokes("ctrl-tab");
    redraw(cx);
    assert!(!is_open(handle, cx));
    workspace.read_with(cx, |workspace, _| {
        assert_ne!(workspace.state().windows()[0].active_tab, selected)
    });
    click_clock(cx);
    cx.simulate_resize(size(px(640.), px(480.)));
    redraw(cx);
    assert!(!is_open(handle, cx));
    click_clock(cx);
    handle
        .update(cx, |root, window, _| window.focus(&root.focus))
        .unwrap();
    redraw(cx);
    assert!(!is_open(handle, cx));
    workspace.read_with(cx, |workspace, _| {
        assert!(workspace.viewing_history().records().is_empty())
    });
}
