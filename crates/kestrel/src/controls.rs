use gpui::{Div, SharedString, Stateful, div, prelude::*};

use crate::appearance::Theme;

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
