#[cfg(all(target_os = "macos", not(test)))]
mod mac {
    use std::{cell::Cell, ffi::CString, ptr};

    use block::ConcreteBlock;
    use gpui::{App, Global, Rgba};
    use objc::{
        class, msg_send,
        runtime::{Object, YES},
        sel, sel_impl,
    };

    use super::super::Preferences;

    unsafe fn string(value: &str) -> *mut Object {
        let value = CString::new(value).expect("notification names contain no NUL");
        unsafe { msg_send![class!(NSString), stringWithUTF8String: value.as_ptr()] }
    }

    pub fn preferences() -> Preferences {
        // SAFETY: Called on the AppKit main thread. Resolve the dynamic color in
        // the application's effective appearance, converting to explicit sRGB
        // before reading CGFloat components. AppKit restores drawing appearance.
        unsafe {
            let workspace: *mut Object = msg_send![class!(NSWorkspace), sharedWorkspace];
            let increase_contrast: objc::runtime::BOOL =
                msg_send![workspace, accessibilityDisplayShouldIncreaseContrast];
            let reduce_transparency: objc::runtime::BOOL =
                msg_send![workspace, accessibilityDisplayShouldReduceTransparency];
            let accent = Cell::new(Preferences::default().accent);
            let resolve = ConcreteBlock::new(|| {
                let color: *mut Object = msg_send![class!(NSColor), controlAccentColor];
                let space: *mut Object = msg_send![class!(NSColorSpace), sRGBColorSpace];
                let color: *mut Object = msg_send![color, colorUsingColorSpace: space];
                if !color.is_null() {
                    let r: f64 = msg_send![color, redComponent];
                    let g: f64 = msg_send![color, greenComponent];
                    let b: f64 = msg_send![color, blueComponent];
                    accent.set(
                        Rgba {
                            r: r as f32,
                            g: g as f32,
                            b: b as f32,
                            a: 1.,
                        }
                        .into(),
                    );
                }
            });
            let app: *mut Object = msg_send![class!(NSApplication), sharedApplication];
            let appearance: *mut Object = msg_send![app, effectiveAppearance];
            let _: () = msg_send![appearance, performAsCurrentDrawingAppearance: &*resolve];
            Preferences {
                accent: accent.get(),
                increase_contrast: increase_contrast == YES,
                reduce_transparency: reduce_transparency == YES,
            }
        }
    }

    struct Observation {
        center: *mut Object,
        observer: *mut Object,
    }

    impl Drop for Observation {
        fn drop(&mut self) {
            // SAFETY: Both objects were retained at registration; this global is
            // dropped on GPUI's main thread. Remove the block before releasing it.
            unsafe {
                let _: () = msg_send![self.center, removeObserver: self.observer];
                let _: () = msg_send![self.observer, release];
                let _: () = msg_send![self.center, release];
            }
        }
    }

    struct Observations {
        _items: Vec<Observation>,
    }
    impl Global for Observations {}

    pub fn observe(cx: &mut App) {
        // SAFETY: Foundation retains the copied blocks. Notifications are delivered
        // on the main queue; defer GPUI updates to avoid reentering an App borrow.
        unsafe {
            let center: *mut Object = msg_send![class!(NSNotificationCenter), defaultCenter];
            let workspace: *mut Object = msg_send![class!(NSWorkspace), sharedWorkspace];
            let workspace_center: *mut Object = msg_send![workspace, notificationCenter];
            let queue: *mut Object = msg_send![class!(NSOperationQueue), mainQueue];
            let mut items = Vec::new();
            for (center, name) in [
                (center, "NSSystemColorsDidChangeNotification"),
                (
                    workspace_center,
                    "NSWorkspaceAccessibilityDisplayOptionsDidChangeNotification",
                ),
            ] {
                let app = cx.to_async();
                let executor = cx.foreground_executor().clone();
                let block = ConcreteBlock::new(move |_: *mut Object| {
                    let app = app.clone();
                    executor
                        .spawn(async move {
                            let _ = app.update(super::super::refresh);
                        })
                        .detach();
                })
                .copy();
                let observer: *mut Object = msg_send![center,
                    addObserverForName: string(name)
                    object: ptr::null_mut::<Object>()
                    queue: queue
                    usingBlock: &*block
                ];
                let _: *mut Object = msg_send![observer, retain];
                let _: *mut Object = msg_send![center, retain];
                items.push(Observation { center, observer });
            }
            cx.set_global(Observations { _items: items });
        }
    }
}

#[cfg(all(target_os = "macos", not(test)))]
pub use mac::{observe, preferences};

#[cfg(any(not(target_os = "macos"), test))]
pub fn preferences() -> super::Preferences {
    super::Preferences::default()
}

#[cfg(any(not(target_os = "macos"), test))]
pub fn observe(_: &mut gpui::App) {}
