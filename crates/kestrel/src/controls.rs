mod tooltip;

pub use tooltip::tooltip;

use gpui::{Div, SharedString, Stateful, div, prelude::*};

use crate::appearance::Theme;

/// Escape external labels for single-line display, never for storage, lookup, or copying.
/// Backslashes stay literal so composing already-formatted labels is harmless.
pub fn single_line(text: &str) -> String {
    let mut label = String::with_capacity(text.len());
    for ch in text.chars() {
        if ch.is_control() || matches!(ch, '\u{2028}' | '\u{2029}') {
            label.extend(ch.escape_default());
        } else {
            label.push(ch);
        }
    }
    label
}

#[cfg(test)]
mod tests;

pub fn control(id: &'static str, label: impl Into<SharedString>, theme: Theme) -> Stateful<Div> {
    enabled_control(id, label, true, theme)
}

pub fn enabled_control(
    id: &'static str,
    label: impl Into<SharedString>,
    enabled: bool,
    theme: Theme,
) -> Stateful<Div> {
    div()
        .id(id)
        .debug_selector(move || id.into())
        .px_3()
        .py_1()
        .rounded_md()
        .bg(theme.control)
        .text_color(if enabled { theme.text } else { theme.disabled })
        .when(enabled, |this| {
            this.hover(|style| style.bg(theme.hover))
                .active(|style| style.bg(theme.pressed))
        })
        .cursor_default()
        .child(label.into())
}
