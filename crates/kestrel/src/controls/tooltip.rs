use gpui::{AnyView, App, Context, SharedString, Window, div, prelude::*, px};

use crate::appearance::Theme;

struct Tooltip(SharedString);

impl Render for Tooltip {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::get(window, cx);
        div()
            .id("tooltip")
            .debug_selector(|| format!("tooltip-{}", self.0))
            .max_w(px(400.))
            .px_2()
            .py_1()
            .rounded_md()
            .border_1()
            .border_color(theme.border)
            .bg(theme.paper)
            .shadow_md()
            .text_sm()
            .text_color(theme.text)
            .child(self.0.clone())
    }
}

pub fn tooltip(text: impl Into<SharedString>) -> impl Fn(&mut Window, &mut App) -> AnyView {
    let text = text.into();
    move |_, cx| cx.new(|_| Tooltip(text.clone())).into()
}
