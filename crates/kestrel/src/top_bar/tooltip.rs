use gpui::{AnyView, App, Context, SharedString, Window, div, prelude::*, px, rgb};

struct Tooltip(SharedString);

impl Render for Tooltip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("tab-tooltip")
            .debug_selector(|| format!("tooltip-{}", self.0))
            .max_w(px(400.))
            .px_2()
            .py_1()
            .rounded_md()
            .border_1()
            .border_color(rgb(0xd5d6da))
            .bg(rgb(0xffffff))
            .shadow_md()
            .text_sm()
            .text_color(rgb(0x25262a))
            .child(self.0.clone())
    }
}

pub fn tooltip(text: impl Into<SharedString>) -> impl Fn(&mut Window, &mut App) -> AnyView {
    let text = text.into();
    move |_, cx| cx.new(|_| Tooltip(text.clone())).into()
}
