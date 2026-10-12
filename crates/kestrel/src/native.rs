#[cfg(all(target_os = "macos", not(test)))]
mod mac {
    use std::ffi::CString;

    use crate::commands::MenuState;
    use objc::{
        class, msg_send,
        runtime::{NO, Object, YES},
        sel, sel_impl,
    };

    unsafe fn string(value: &str) -> *mut Object {
        let value = CString::new(value).expect("menu titles contain no NUL");
        unsafe { msg_send![class!(NSString), stringWithUTF8String: value.as_ptr()] }
    }

    pub fn update_menus(states: &[MenuState]) {
        // SAFETY: Called on GPUI's main thread. AppKit owns all menu objects;
        // selectors and argument types below follow NSApplication/NSMenu/NSMenuItem.
        unsafe {
            let app: *mut Object = msg_send![class!(NSApplication), sharedApplication];
            let main: *mut Object = msg_send![app, mainMenu];
            if main.is_null() {
                return;
            }
            for state in states {
                let parent: *mut Object = msg_send![main, itemWithTitle: string(&state.menu)];
                let menu: *mut Object = msg_send![parent, submenu];
                if menu.is_null() {
                    continue;
                }
                // GPUI 0.2.2 validates by action type, not action value or state.
                let _: () = msg_send![menu, setAutoenablesItems: NO];
                let mut item: *mut Object = msg_send![menu, itemWithTitle: string(&state.label)];
                if item.is_null() && state.label == "Enter Full Screen" {
                    item = msg_send![menu, itemWithTitle: string("Exit Full Screen")];
                }
                if !item.is_null() {
                    let _: () = msg_send![item, setTitle: string(&state.title)];
                    let _: () = msg_send![item, setEnabled: if state.enabled { YES } else { NO }];
                }
            }
        }
    }

    pub fn restore_minimized_window() {
        // SAFETY: AppKit owns the window array; this runs on the main thread.
        unsafe {
            let app: *mut Object = msg_send![class!(NSApplication), sharedApplication];
            let windows: *mut Object = msg_send![app, windows];
            let count: usize = msg_send![windows, count];
            for index in 0..count {
                let window: *mut Object = msg_send![windows, objectAtIndex: index];
                let minimized: objc::runtime::BOOL = msg_send![window, isMiniaturized];
                if minimized == YES {
                    let _: () = msg_send![window, deminiaturize: std::ptr::null_mut::<Object>()];
                    let _: () =
                        msg_send![window, makeKeyAndOrderFront: std::ptr::null_mut::<Object>()];
                    break;
                }
            }
        }
    }

    pub fn bring_all_to_front() {
        // SAFETY: NSApplication's documented action, invoked on the main thread.
        unsafe {
            let app: *mut Object = msg_send![class!(NSApplication), sharedApplication];
            let _: () = msg_send![app, arrangeInFront: std::ptr::null_mut::<Object>()];
        }
    }
}

#[cfg(all(target_os = "macos", not(test)))]
pub use mac::*;

#[cfg(any(not(target_os = "macos"), test))]
pub fn update_menus(_: &[crate::commands::MenuState]) {}

#[cfg(any(not(target_os = "macos"), test))]
pub fn bring_all_to_front() {}

#[cfg(any(not(target_os = "macos"), test))]
pub fn restore_minimized_window() {}
