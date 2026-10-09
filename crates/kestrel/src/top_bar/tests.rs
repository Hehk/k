use super::*;
use gpui::{Entity, Modifiers, Point, TestAppContext, VisualTestContext, point};

struct Host {
    bar: Entity<TopBar>,
    content_focus: FocusHandle,
    escapes: usize,
    releases: usize,
}

impl Host {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let content_focus = cx.focus_handle();
        window.focus(&content_focus);
        let bar = cx.new(|cx| TopBar::new(None, window, cx));
        bar.update(cx, |bar, cx| bar.dispatch(Msg::Add, window, cx));
        Self {
            bar,
            content_focus,
            escapes: 0,
            releases: 0,
        }
    }
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .key_context("Workspace")
            .on_action(cx.listener(|this, action: &TabAction, window, cx| {
                this.bar
                    .update(cx, |bar, cx| bar.dispatch(action.message(), window, cx));
            }))
            .size_full()
            .flex()
            .flex_col()
            .child(self.bar.clone())
            .child(
                div()
                    .id("test-content")
                    .debug_selector(|| "test-content".into())
                    .flex_1()
                    .track_focus(&self.content_focus)
                    .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                        if event.keystroke.key == "escape" {
                            this.escapes += 1;
                            cx.stop_propagation();
                        }
                    }))
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            this.releases += 1;
                            cx.stop_propagation();
                        }),
                    ),
            )
    }
}

#[gpui::test]
fn component_captures_content_drop_and_restores_sibling_focus(cx: &mut TestAppContext) {
    let (host, cx) = cx.add_window_view(Host::new);
    let bar = host.read_with(cx, |host, _| host.bar.clone());
    let previous = host.read_with(cx, |host, _| host.content_focus.clone());
    redraw(cx);
    let content = cx.debug_bounds("test-content").unwrap().center();
    cx.simulate_click(content, Modifiers::none());
    assert_eq!(host.read_with(cx, |host, _| host.releases), 1);
    drag_start(cx);
    assert!(cx.update(|window, cx| bar.read(cx).focus.is_focused(window)));
    cx.simulate_mouse_up(content, MouseButton::Left, Modifiers::none());
    assert!(cx.update(|window, _| previous.is_focused(window)));
    assert_eq!(host.read_with(cx, |host, _| host.releases), 1);
    assert!(bar.read_with(cx, |bar, _| bar.model.drag_token().is_none()));
    assert_eq!(cx.windows().len(), 1);
    assert!(!cx.read(|cx| cx.has_active_drag()));
    cx.simulate_click(content, Modifiers::none());
    assert_eq!(host.read_with(cx, |host, _| host.releases), 2);
}

#[gpui::test]
fn component_handles_escape_only_during_its_drag(cx: &mut TestAppContext) {
    let (host, cx) = cx.add_window_view(Host::new);
    let previous = host.read_with(cx, |host, _| host.content_focus.clone());
    redraw(cx);
    cx.simulate_keystrokes("escape");
    assert_eq!(host.read_with(cx, |host, _| host.escapes), 1);
    drag_start(cx);
    cx.simulate_keystrokes("escape");
    assert!(!cx.read(|cx| cx.has_active_drag()));
    assert!(cx.update(|window, _| previous.is_focused(window)));
    assert_eq!(host.read_with(cx, |host, _| host.escapes), 1);
    cx.simulate_keystrokes("escape");
    assert_eq!(host.read_with(cx, |host, _| host.escapes), 2);
}

#[gpui::test]
fn restoring_drag_focus_does_not_override_a_new_focus_owner(cx: &mut TestAppContext) {
    let (host, cx) = cx.add_window_view(Host::new);
    redraw(cx);
    drag_start(cx);
    let other = cx.update(|window, cx| {
        let other = cx.focus_handle();
        window.focus(&other);
        other
    });
    let content = cx.debug_bounds("test-content").unwrap().center();
    cx.simulate_mouse_up(content, MouseButton::Left, Modifiers::none());
    assert!(cx.update(|window, _| other.is_focused(window)));
    let bar = host.read_with(cx, |host, _| host.bar.clone());
    assert!(bar.read_with(cx, |bar, _| bar.previous_focus.is_none()));
}

fn setup(cx: &mut TestAppContext, count: usize) -> (Entity<TopBar>, &mut VisualTestContext) {
    cx.update(bind_keys);
    let (root, cx) = cx.add_window_view(|window, cx| crate::Kestrel::new(None, window, cx));
    let bar = root.read_with(cx, |root, _| root.top_bar.clone());
    cx.update(|window, cx| {
        bar.update(cx, |bar, cx| {
            for _ in 1..count {
                bar.dispatch(Msg::Add, window, cx);
            }
        })
    });
    redraw(cx);
    (bar, cx)
}

fn redraw(cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        window.draw(cx).clear();
    });
    cx.run_until_parked();
}

fn drag_start(cx: &mut VisualTestContext) {
    let source = cx.debug_bounds("tab-slot-1").unwrap().center();
    cx.simulate_mouse_down(source, MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_move(
        source + point(px(10.), px(0.)),
        MouseButton::Left,
        Modifiers::none(),
    );
    redraw(cx);
}

fn order(bar: &Entity<TopBar>, cx: &VisualTestContext) -> Vec<u64> {
    bar.read_with(cx, |bar, _| {
        bar.model.view().tabs.iter().map(|tab| tab.tab.id).collect()
    })
}

#[gpui::test]
fn click_updates_selection_and_content(cx: &mut TestAppContext) {
    let (bar, cx) = setup(cx, 2);
    let source = cx.debug_bounds("tab-slot-1").unwrap().center();
    cx.simulate_click(source, Modifiers::none());
    redraw(cx);
    assert_eq!(bar.read_with(cx, |bar, _| bar.active_tab().unwrap().id), 1);
    assert!(cx.debug_bounds("content-title-Project 1").is_some());
    let add = cx.debug_bounds("new-tab").unwrap().center();
    cx.simulate_click(add, Modifiers::none());
    redraw(cx);
    assert_eq!(bar.read_with(cx, |bar, _| bar.active_tab().unwrap().id), 3);
    assert!(cx.debug_bounds("content-title-Project 3").is_some());
}

#[gpui::test]
fn real_drag_events_use_fixed_slots_and_content_release(cx: &mut TestAppContext) {
    let (bar, cx) = setup(cx, 3);
    let target = cx.debug_bounds("tab-slot-2").unwrap();
    drag_start(cx);
    assert!(bar.read_with(cx, |bar, _| bar.model.view().tabs[0].hidden));
    let pointer = Point {
        x: target.left() + px(1.),
        y: target.center().y,
    };
    cx.simulate_mouse_move(pointer, MouseButton::Left, Modifiers::none());
    redraw(cx);
    assert_eq!(order(&bar, cx), vec![2, 1, 3]);
    let source_slot = cx.debug_bounds("tab-slot-1").unwrap();
    assert_eq!(source_slot, target);
    for _ in 0..3 {
        cx.simulate_mouse_move(pointer, MouseButton::Left, Modifiers::none());
        redraw(cx);
        assert_eq!(order(&bar, cx), vec![2, 1, 3]);
    }
    cx.simulate_mouse_up(
        point(px(500.), px(400.)),
        MouseButton::Left,
        Modifiers::none(),
    );
    redraw(cx);
    assert!(bar.read_with(cx, |bar, _| bar.model.drag_token().is_none()));
    assert!(bar.read_with(cx, |bar, _| {
        bar.model.view().tabs.iter().all(|tab| !tab.hidden)
    }));
    assert_eq!(cx.windows().len(), 1);
}

#[gpui::test]
fn escape_cancels_model_and_gpui_preview(cx: &mut TestAppContext) {
    let (bar, cx) = setup(cx, 2);
    let previous = cx.update(|window, cx| window.focused(cx).unwrap());
    drag_start(cx);
    assert!(cx.read(|cx| cx.has_active_drag()));
    cx.simulate_keystrokes("escape");
    assert!(cx.update(|window, _| previous.is_focused(window)));
    redraw(cx);
    assert!(!cx.read(|cx| cx.has_active_drag()));
    assert!(bar.read_with(cx, |bar, _| bar.model.drag_token().is_none()));
    assert!(bar.read_with(cx, |bar, _| {
        bar.model.view().tabs.iter().all(|tab| !tab.hidden)
    }));
}

#[gpui::test]
fn outside_release_runs_window_command_and_returns_success(cx: &mut TestAppContext) {
    let (bar, cx) = setup(cx, 2);
    drag_start(cx);
    cx.simulate_mouse_up(
        point(px(-30.), px(100.)),
        MouseButton::Left,
        Modifiers::none(),
    );
    redraw(cx);
    assert_eq!(cx.windows().len(), 2);
    assert_eq!(order(&bar, cx), vec![2]);
    assert_eq!(bar.read_with(cx, |bar, _| bar.active_tab().unwrap().id), 2);
}

#[gpui::test]
fn last_tab_transfer_closes_only_the_source_window(cx: &mut TestAppContext) {
    let (bar, cx) = setup(cx, 1);
    let source = cx.windows()[0];
    cx.update(|window, cx| {
        bar.update(cx, |bar, cx| {
            let token = bar.model.next_drag();
            bar.dispatch(Msg::DragStarted { tab: 1, token }, window, cx);
            bar.dispatch(
                Msg::Released {
                    token,
                    outside: true,
                },
                window,
                cx,
            );
        })
    });
    cx.run_until_parked();
    let windows = cx.windows();
    assert_eq!(windows.len(), 1);
    assert!(windows[0] != source);
    assert!(bar.read_with(cx, |bar, _| bar.active_tab().is_none()));
    let destination = windows[0]
        .downcast::<crate::Kestrel>()
        .unwrap()
        .root(cx)
        .unwrap();
    destination.read_with(cx, |root, cx| {
        assert_eq!(root.top_bar.read(cx).active_tab().unwrap().id, 1);
        assert_ne!(root.top_bar.entity_id(), bar.entity_id());
    });
}

#[gpui::test]
fn close_button_appears_on_hover_and_closes_without_selecting_or_dragging(cx: &mut TestAppContext) {
    let (bar, cx) = setup(cx, 3);
    assert!(!bar.read_with(cx, |bar, _| bar.model.view().tabs[0].close_visible));
    assert!(cx.debug_bounds("close-tab-3").is_some());
    let slot = cx.debug_bounds("tab-slot-1").unwrap();
    cx.simulate_mouse_move(slot.center(), None, Modifiers::none());
    redraw(cx);
    let close = cx.debug_bounds("close-tab-1").unwrap();
    assert_eq!(cx.debug_bounds("tab-slot-1").unwrap(), slot);
    assert!(close.right() < slot.right());
    cx.simulate_mouse_move(point(px(500.), px(400.)), None, Modifiers::none());
    redraw(cx);
    assert!(!bar.read_with(cx, |bar, _| bar.model.view().tabs[0].close_visible));
    cx.simulate_mouse_move(slot.center(), None, Modifiers::none());
    redraw(cx);
    let close = cx.debug_bounds("close-tab-1").unwrap().center();
    cx.simulate_mouse_down(close, MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_move(
        close + point(px(4.), px(0.)),
        MouseButton::Left,
        Modifiers::none(),
    );
    assert!(!cx.read(|cx| cx.has_active_drag()));
    cx.simulate_mouse_up(close, MouseButton::Left, Modifiers::none());
    redraw(cx);
    assert_eq!(order(&bar, cx), vec![2, 3]);
    assert_eq!(bar.read_with(cx, |bar, _| bar.active_tab().unwrap().id), 3);
}

#[gpui::test]
fn active_close_selects_neighbor_and_hides_last_tabs_close_button(cx: &mut TestAppContext) {
    let (bar, cx) = setup(cx, 2);
    let close = cx.debug_bounds("close-tab-2").unwrap().center();
    cx.simulate_click(close, Modifiers::none());
    redraw(cx);
    assert_eq!(order(&bar, cx), vec![1]);
    assert!(cx.debug_bounds("content-title-Project 1").is_some());
    assert!(!bar.read_with(cx, |bar, _| bar.model.view().tabs[0].close_visible));
    assert!(cx.debug_bounds("close-tab-1").is_none());
    assert_eq!(cx.windows().len(), 1);
}

#[gpui::test]
fn initial_drag_motion_crossing_slot_reorders_immediately(cx: &mut TestAppContext) {
    let (bar, cx) = setup(cx, 3);
    let source = cx.debug_bounds("tab-slot-1").unwrap();
    let target = cx.debug_bounds("tab-slot-2").unwrap();
    let down = point(source.right() - px(1.), source.center().y);
    let to = point(target.left() + px(3.), target.center().y);
    cx.simulate_mouse_down(down, MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_move(to, MouseButton::Left, Modifiers::none());
    redraw(cx);
    assert_eq!(order(&bar, cx), vec![2, 1, 3]);
    cx.simulate_mouse_up(to, MouseButton::Left, Modifiers::none());
    redraw(cx);
    assert_eq!(order(&bar, cx), vec![2, 1, 3]);
}

#[gpui::test]
fn release_uses_final_pointer_position(cx: &mut TestAppContext) {
    let (bar, cx) = setup(cx, 3);
    let target = cx.debug_bounds("tab-slot-3").unwrap().center();
    drag_start(cx);
    cx.simulate_mouse_up(target, MouseButton::Left, Modifiers::none());
    redraw(cx);
    assert_eq!(order(&bar, cx), vec![2, 3, 1]);
}

#[gpui::test]
fn secondary_release_cancels_model_when_gpui_dismisses_preview(cx: &mut TestAppContext) {
    let (bar, cx) = setup(cx, 3);
    drag_start(cx);
    let pointer = cx.debug_bounds("tab-slot-1").unwrap().center();
    cx.simulate_mouse_down(pointer, MouseButton::Right, Modifiers::none());
    cx.simulate_mouse_up(pointer, MouseButton::Right, Modifiers::none());
    redraw(cx);
    assert!(!cx.read(|cx| cx.has_active_drag()));
    assert!(bar.read_with(cx, |bar, _| bar.model.drag_token().is_none()));
    assert!(bar.read_with(cx, |bar, _| {
        bar.model.view().tabs.iter().all(|tab| !tab.hidden)
    }));
    cx.simulate_mouse_up(
        point(px(-30.), px(100.)),
        MouseButton::Left,
        Modifiers::none(),
    );
    assert_eq!(cx.windows().len(), 1);
    cx.simulate_keystrokes("cmd-t");
    assert_eq!(order(&bar, cx), vec![1, 2, 3, 4]);
}

#[gpui::test]
fn framework_drag_cancellation_restores_previous_focus(cx: &mut TestAppContext) {
    let (host, cx) = cx.add_window_view(Host::new);
    let bar = host.read_with(cx, |host, _| host.bar.clone());
    let previous = host.read_with(cx, |host, _| host.content_focus.clone());
    redraw(cx);
    drag_start(cx);
    cx.update(|window, cx| {
        cx.stop_active_drag(window);
    });
    redraw(cx);
    assert!(bar.read_with(cx, |bar, _| bar.model.drag_token().is_none()));
    assert!(cx.update(|window, _| previous.is_focused(window)));
}

#[gpui::test]
fn tab_shortcuts_add_select_cycle_and_close(cx: &mut TestAppContext) {
    let (bar, cx) = setup(cx, 1);
    cx.simulate_keystrokes("cmd-t cmd-t cmd-1");
    assert_eq!(order(&bar, cx), vec![1, 2, 3]);
    assert_eq!(bar.read_with(cx, |bar, _| bar.active_tab().unwrap().id), 1);
    cx.simulate_keystrokes("ctrl-shift-tab");
    assert_eq!(bar.read_with(cx, |bar, _| bar.active_tab().unwrap().id), 3);
    cx.simulate_keystrokes("ctrl-tab");
    assert_eq!(bar.read_with(cx, |bar, _| bar.active_tab().unwrap().id), 1);
    cx.simulate_keystrokes("cmd-shift-] cmd-shift-]");
    assert_eq!(bar.read_with(cx, |bar, _| bar.active_tab().unwrap().id), 3);
    cx.simulate_keystrokes("cmd-shift-[");
    assert_eq!(bar.read_with(cx, |bar, _| bar.active_tab().unwrap().id), 2);
    cx.simulate_keystrokes("cmd-}");
    assert_eq!(bar.read_with(cx, |bar, _| bar.active_tab().unwrap().id), 3);
    cx.simulate_keystrokes("cmd-{");
    assert_eq!(bar.read_with(cx, |bar, _| bar.active_tab().unwrap().id), 2);
    cx.simulate_keystrokes("cmd-9");
    assert_eq!(bar.read_with(cx, |bar, _| bar.active_tab().unwrap().id), 2);
    cx.simulate_keystrokes("cmd-w");
    assert_eq!(order(&bar, cx), vec![1, 3]);
    cx.simulate_keystrokes("cmd-2");
    assert_eq!(bar.read_with(cx, |bar, _| bar.active_tab().unwrap().id), 3);
    cx.simulate_keystrokes("cmd-w");
    assert_eq!(order(&bar, cx), vec![1]);
    cx.simulate_keystrokes("cmd-w");
    assert!(cx.windows().is_empty());
}

#[gpui::test]
fn workspace_shortcuts_bubble_from_focused_sibling_content(cx: &mut TestAppContext) {
    cx.update(bind_keys);
    let (host, cx) = cx.add_window_view(Host::new);
    let bar = host.read_with(cx, |host, _| host.bar.clone());
    let focus = host.read_with(cx, |host, _| host.content_focus.clone());
    redraw(cx);
    cx.simulate_keystrokes("cmd-t cmd-1 ctrl-tab");
    assert_eq!(order(&bar, cx), vec![1, 2, 3]);
    assert_eq!(bar.read_with(cx, |bar, _| bar.active_tab().unwrap().id), 2);
    assert!(cx.update(|window, _| focus.is_focused(window)));
    cx.simulate_keystrokes("cmd-w");
    assert_eq!(order(&bar, cx), vec![1, 3]);
}

#[gpui::test]
fn cmd_nine_selects_ninth_not_last_tab(cx: &mut TestAppContext) {
    let (bar, cx) = setup(cx, 10);
    cx.simulate_keystrokes("cmd-9");
    assert_eq!(bar.read_with(cx, |bar, _| bar.active_tab().unwrap().id), 9);
}

#[gpui::test]
fn full_title_and_button_tooltips_use_gpui_hover_delay(cx: &mut TestAppContext) {
    let (_, cx) = setup(cx, 12);
    cx.simulate_resize(gpui::size(px(600.), px(500.)));
    redraw(cx);
    redraw(cx);
    let add = cx.debug_bounds("new-tab").unwrap().center();
    cx.simulate_mouse_move(add, None, Modifiers::none());
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(1));
    redraw(cx);
    assert!(cx.debug_bounds("tooltip-New tab (⌘T)").is_some());
    cx.simulate_keystrokes("cmd-1");
    redraw(cx);
    let slot = cx.debug_bounds("tab-slot-1").unwrap().center();
    cx.simulate_mouse_move(slot, None, Modifiers::none());
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(1));
    redraw(cx);
    assert!(cx.debug_bounds("tooltip-Project 1").is_some());
    let close = cx.debug_bounds("close-tab-1").unwrap().center();
    cx.simulate_mouse_move(close, None, Modifiers::none());
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(1));
    redraw(cx);
    assert!(cx.debug_bounds("tooltip-Close tab (⌘W)").is_some());
}

#[gpui::test]
fn measured_scroll_and_resize_feed_back_into_model(cx: &mut TestAppContext) {
    let (bar, cx) = setup(cx, 12);
    cx.simulate_resize(gpui::size(px(600.), px(500.)));
    redraw(cx);
    redraw(cx);
    bar.read_with(cx, |bar, _| {
        assert_eq!(bar.model.geometry().width, 600.);
        assert_eq!(bar.model.view().layout.tab_width, 100.);
        assert_eq!(
            bar.model.geometry().scroll,
            f32::from(bar.scroll.offset().x)
        );
        assert!(bar.model.geometry().scroll < 0.);
    });
    cx.update(|window, cx| bar.update(cx, |bar, cx| bar.dispatch(Msg::Select(1), window, cx)));
    redraw(cx);
    redraw(cx);
    bar.read_with(cx, |bar, _| {
        assert_eq!(
            bar.model.geometry().scroll,
            f32::from(bar.scroll.offset().x)
        );
        assert!(bar.scroll.bounds_for_item(0).is_some());
    });
    let first = cx.debug_bounds("tab-slot-1").unwrap();
    assert!(first.left() >= px(SIDE_WIDTH));
    cx.simulate_mouse_move(first.center(), None, Modifiers::none());
    cx.simulate_event(gpui::ScrollWheelEvent {
        position: first.center(),
        delta: gpui::ScrollDelta::Pixels(point(px(-120.), px(0.))),
        ..Default::default()
    });
    redraw(cx);
    bar.read_with(cx, |bar, _| {
        assert!(bar.model.geometry().scroll < 0.);
        assert_eq!(
            bar.model.geometry().scroll,
            f32::from(bar.scroll.offset().x)
        );
    });
}
