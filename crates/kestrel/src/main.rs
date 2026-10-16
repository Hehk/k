mod appearance;
mod branch_view;
mod commands;
mod comparison_view;
mod controls;
#[cfg(any(debug_assertions, feature = "frame-metrics"))]
mod debug_panel;
mod diff_selection;
mod native;
mod repository_model;
mod repository_view;
mod text_input;
mod top_bar;
mod workspace;
mod workspace_window;

use gpui::{App, AppContext, Application, TitlebarOptions, WindowBounds, WindowOptions, point, px};
use kestrel::storage::Store;
use top_bar::TITLEBAR_HEIGHT;
use workspace::{LoadedWorkspace, Session, Workspace};
use workspace_window::Kestrel;

use commands::new_window;

const TRAFFIC_LIGHT_HEIGHT: f32 = 14.;

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
    #[cfg(any(debug_assertions, feature = "frame-metrics"))]
    debug_panel::install();
    let loaded = LoadedWorkspace::load(Store::for_app());
    let app = Application::new();
    app.on_reopen(|cx| {
        if cx.windows().is_empty() {
            new_window(cx);
        } else {
            native::restore_minimized_window();
        }
    });
    app.run(move |cx: &mut App| {
        appearance::install(cx);
        let workspace = cx.new(|cx| Workspace::new(loaded, cx));
        cx.set_global(Session(workspace.clone()));
        commands::install(cx);
        let windows = workspace
            .read(cx)
            .state()
            .windows()
            .iter()
            .map(|window| window.id)
            .collect::<Vec<_>>();
        let active = workspace.read(cx).state().active_window();
        if windows.is_empty() {
            new_window(cx);
        } else {
            for id in windows {
                if let Err(error) = Kestrel::open(workspace.clone(), id, cx) {
                    eprintln!("Could not restore window: {error}");
                }
            }
            if let Some(active) = active {
                workspace.update(cx, |workspace, cx| workspace.activate_window(active, cx));
            }
        }
        cx.activate(true);
    });
}
