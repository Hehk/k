mod top_bar;

use gpui::{
    App, Application, Context, Entity, FocusHandle, Subscription, TitlebarOptions, Window,
    WindowBounds, WindowOptions, div, point, prelude::*, px, rgb, size,
};
use top_bar::{
    TITLEBAR_HEIGHT, TabAction, TopBar,
    model::{Msg, Tab, WindowCommand},
};

const TRAFFIC_LIGHT_HEIGHT: f32 = 14.;

struct Kestrel {
    top_bar: Entity<TopBar>,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl Kestrel {
    fn new(initial: Option<(Tab, u64)>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let top_bar = cx.new(|cx| TopBar::new(initial, window, cx));
        let focus = cx.focus_handle();
        window.focus(&focus);
        let subscriptions = vec![
            // TODO: Observe shared active-content changes instead of every tab-bar notification.
            cx.observe(&top_bar, |_, _, cx| cx.notify()),
            cx.subscribe_in(&top_bar, window, Self::handle_window_command),
        ];
        Self {
            top_bar,
            focus,
            _subscriptions: subscriptions,
        }
    }

    fn open(initial: Option<(Tab, u64)>, cx: &mut App) -> gpui::Result<gpui::WindowHandle<Self>> {
        let bounds =
            WindowBounds::Windowed(gpui::Bounds::centered(None, size(px(1100.), px(760.)), cx));
        cx.open_window(window_options(Some(bounds)), |window, cx| {
            cx.new(|cx| Self::new(initial, window, cx))
        })
    }

    fn handle_window_command(
        &mut self,
        top_bar: &Entity<TopBar>,
        command: &WindowCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match command {
            WindowCommand::OpenDetached {
                request,
                tab,
                next_tab_id,
            } => {
                let success = Self::open(Some((tab.clone(), *next_tab_id)), cx).is_ok();
                top_bar.update(cx, |bar, cx| {
                    bar.dispatch(
                        Msg::DetachCompleted {
                            request: *request,
                            success,
                        },
                        window,
                        cx,
                    );
                });
            }
            WindowCommand::Close => window.remove_window(),
        }
    }

    fn handle_tab_action(
        &mut self,
        action: &TabAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.top_bar
            .update(cx, |bar, cx| bar.dispatch(action.message(), window, cx));
    }
}

impl Render for Kestrel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let title = self
            .top_bar
            .read(cx)
            .active_tab()
            .map(|tab| tab.title.clone())
            .unwrap_or_default();

        div()
            .key_context("Workspace")
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::handle_tab_action))
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(0xf1f2f4))
            .child(self.top_bar.clone())
            .child(
                div().flex_1().pb_2().px_2().child(
                    div()
                        .id("content-frame")
                        .size_full()
                        .flex()
                        .flex_col()
                        .justify_center()
                        .items_center()
                        .gap_2()
                        .rounded_lg()
                        .border_1()
                        .border_color(rgb(0xe0e1e4))
                        .bg(rgb(0xffffff))
                        .child(
                            div()
                                .text_lg()
                                .debug_selector(|| format!("content-title-{title}"))
                                .child(title),
                        )
                        .child(
                            div()
                                .text_sm()
                                .text_color(rgb(0x777980))
                                .child("Project or review workspace"),
                        ),
                ),
            )
    }
}

#[cfg(target_os = "macos")]
fn start_window_drag() {
    use objc::{
        class, msg_send,
        runtime::{NO, Object, YES},
        sel, sel_impl,
    };

    // Native window movement is disabled so it doesn't steal tab drags. Enable
    // it temporarily here, letting AppKit handle movement from the title bar.
    //
    // SAFETY: Called synchronously from GPUI's mouse-down handler on AppKit's
    // main thread, while the current event and window are alive. These messages
    // use AppKit's NSApplication, NSWindow, and NSEvent types and signatures;
    // missing window/event pointers are checked before starting the drag.
    unsafe {
        let app: *mut Object = msg_send![class!(NSApplication), sharedApplication];
        let window: *mut Object = msg_send![app, keyWindow];
        let event: *mut Object = msg_send![app, currentEvent];
        if window.is_null() || event.is_null() {
            return;
        }
        let _: () = msg_send![window, setMovable: YES];
        let _: () = msg_send![window, performWindowDragWithEvent: event];
        let _: () = msg_send![window, setMovable: NO];
    }
}

#[cfg(not(target_os = "macos"))]
fn start_window_drag() {}

fn window_options(window_bounds: Option<WindowBounds>) -> WindowOptions {
    WindowOptions {
        window_bounds,
        is_movable: !cfg!(target_os = "macos"),
        titlebar: Some(TitlebarOptions {
            title: Some("Kestrel".into()),
            appears_transparent: true,
            traffic_light_position: Some(point(
                px(16.),
                px((TITLEBAR_HEIGHT - TRAFFIC_LIGHT_HEIGHT) / 2.),
            )),
        }),
        ..Default::default()
    }
}

fn main() {
    Application::new().run(|cx: &mut App| {
        top_bar::bind_keys(cx);
        Kestrel::open(None, cx).unwrap();
        cx.activate(true);
    });
}
