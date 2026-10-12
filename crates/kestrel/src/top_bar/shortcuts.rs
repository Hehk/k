use gpui::{Action, App, KeyBinding};

use super::model::Msg;

#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = workspace, no_json)]
pub enum TabAction {
    New,
    Close,
    Next,
    Previous,
    Last,
    MoveLeft,
    MoveRight,
    CloseOthers,
    CloseRight,
    Focus(usize),
}

impl TabAction {
    pub fn message(&self) -> Msg {
        match self {
            Self::New => Msg::Add,
            Self::Close => Msg::CloseActive,
            Self::Next => Msg::SelectNext,
            Self::Previous => Msg::SelectPrevious,
            Self::Last => Msg::SelectLast,
            Self::MoveLeft => Msg::MoveLeft,
            Self::MoveRight => Msg::MoveRight,
            Self::CloseOthers => Msg::CloseOthers,
            Self::CloseRight => Msg::CloseRight,
            Self::Focus(index) => Msg::SelectIndex(*index),
        }
    }
}

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("cmd-t", TabAction::New, None),
        KeyBinding::new("cmd-w", TabAction::Close, Some("Workspace")),
        KeyBinding::new("cmd-shift-]", TabAction::Next, Some("Workspace")),
        KeyBinding::new("cmd-shift-[", TabAction::Previous, Some("Workspace")),
        // macOS reports shifted punctuation as the resulting character.
        KeyBinding::new("cmd-}", TabAction::Next, Some("Workspace")),
        KeyBinding::new("cmd-{", TabAction::Previous, Some("Workspace")),
        KeyBinding::new("ctrl-tab", TabAction::Next, Some("Workspace")),
        KeyBinding::new("ctrl-shift-tab", TabAction::Previous, Some("Workspace")),
    ]);
    cx.bind_keys([KeyBinding::new("cmd-9", TabAction::Last, Some("Workspace"))]);
    cx.bind_keys((1..=8).map(|number| {
        KeyBinding::new(
            &format!("cmd-{number}"),
            TabAction::Focus(number - 1),
            Some("Workspace"),
        )
    }));
}
