use super::*;
use std::{cell::Cell, rc::Rc};

use crate::commands::Command;
use gpui::MouseDownEvent;

fn zoom_requests(cx: &mut VisualTestContext) -> Rc<Cell<usize>> {
    let requests = Rc::new(Cell::new(0));
    let received = requests.clone();
    cx.update(|_, cx| {
        cx.on_action(move |command: &Command, _| {
            if *command == Command::Zoom {
                received.set(received.get() + 1);
            }
        });
    });
    requests
}

fn click(cx: &mut VisualTestContext, position: Point<Pixels>, button: MouseButton, count: usize) {
    cx.simulate_event(MouseDownEvent {
        position,
        button,
        click_count: count,
        ..Default::default()
    });
    cx.simulate_event(MouseUpEvent {
        position,
        button,
        click_count: count,
        ..Default::default()
    });
    redraw(cx);
}

#[gpui::test]
fn background_double_click_requests_native_zoom_each_time(cx: &mut TestAppContext) {
    let (_, cx) = setup(cx, 2);
    let requests = zoom_requests(cx);
    let background = point(px(100.), px(2.));

    for expected in 1..=2 {
        click(cx, background, MouseButton::Left, 1);
        assert_eq!(requests.get(), expected - 1);
        click(cx, background, MouseButton::Left, 2);
        assert_eq!(requests.get(), expected);
    }
    click(cx, background, MouseButton::Left, 3);
    click(cx, background, MouseButton::Right, 2);
    assert_eq!(requests.get(), 2);
}

#[gpui::test]
fn single_tab_title_double_click_requests_zoom(cx: &mut TestAppContext) {
    let (bar, cx) = setup(cx, 1);
    let requests = zoom_requests(cx);
    let title =
        cx.update(|window, _| point(window.viewport_size().width / 2., px(TITLEBAR_HEIGHT / 2.)));
    click(cx, title, MouseButton::Left, 1);
    click(cx, title, MouseButton::Left, 2);
    assert_eq!(requests.get(), 1);
    assert_eq!(order(&bar, cx), [1]);
}

#[gpui::test]
fn tab_and_add_button_double_clicks_do_not_zoom(cx: &mut TestAppContext) {
    let (bar, cx) = setup(cx, 2);
    let requests = zoom_requests(cx);
    let tab = cx.debug_bounds("tab-slot-1").unwrap().center();
    click(cx, tab, MouseButton::Left, 1);
    click(cx, tab, MouseButton::Left, 2);
    assert_eq!(bar.read_with(cx, |bar, _| bar.active_tab().unwrap().id), 1);

    let add = cx.debug_bounds("new-tab").unwrap().center();
    click(cx, add, MouseButton::Left, 1);
    click(cx, add, MouseButton::Left, 2);
    assert_eq!(order(&bar, cx), [1, 2, 3, 4]);
    assert_eq!(requests.get(), 0);
}

#[gpui::test]
fn close_button_second_click_does_not_zoom(cx: &mut TestAppContext) {
    let (bar, cx) = setup(cx, 3);
    let requests = zoom_requests(cx);
    let close = cx.debug_bounds("close-tab-3").unwrap().center();
    click(cx, close, MouseButton::Left, 2);
    assert_eq!(order(&bar, cx), [1, 2]);
    assert_eq!(requests.get(), 0);
}

#[gpui::test]
fn fullscreen_background_double_click_does_not_zoom(cx: &mut TestAppContext) {
    let (_, cx) = setup(cx, 1);
    let requests = zoom_requests(cx);
    cx.update(|window, _| window.toggle_fullscreen());
    click(cx, point(px(100.), px(2.)), MouseButton::Left, 2);
    assert_eq!(requests.get(), 0);
    assert!(cx.update(|window, _| window.is_fullscreen()));
}

#[gpui::test]
fn background_double_click_during_tab_drag_does_not_zoom(cx: &mut TestAppContext) {
    let (_, cx) = setup(cx, 2);
    let requests = zoom_requests(cx);
    drag_start(cx);
    cx.simulate_event(MouseDownEvent {
        position: point(px(100.), px(2.)),
        button: MouseButton::Left,
        click_count: 2,
        ..Default::default()
    });
    assert_eq!(requests.get(), 0);
    assert!(cx.read(|cx| cx.has_active_drag()));
    cx.simulate_keystrokes("escape");
}
