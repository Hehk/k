use gpui::{Div, SharedString, Stateful, div, prelude::*, rgb};

pub fn control(id: &'static str, label: impl Into<SharedString>) -> Stateful<Div> {
    div()
        .id(id)
        .debug_selector(move || id.into())
        .px_3()
        .py_1()
        .rounded_md()
        .bg(rgb(0xe7e9ee))
        .cursor_pointer()
        .child(label.into())
}
