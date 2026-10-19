pub(crate) mod line;

use std::{cell::RefCell, collections::BTreeMap, ops::Range, rc::Rc};

use gpui::{
    Action, AnyElement, App, Bounds, Context, Div, EventEmitter, FocusHandle, KeyBinding,
    MouseButton, Pixels, Point, ShapedLine, SharedString, Stateful, Window, canvas, div,
    prelude::*,
};
use unicode_segmentation::UnicodeSegmentation;

use crate::appearance::Theme;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RowSource {
    Code(Range<usize>),
    Boundary(usize),
}

#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = diff_selection, no_json)]
enum Select {
    Copy,
    All,
    Clear,
}

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("cmd-c", Select::Copy, Some("DiffSelection")),
        KeyBinding::new("cmd-a", Select::All, Some("DiffSelection")),
        KeyBinding::new("escape", Select::Clear, Some("DiffSelection")),
    ]);
}

pub const ROW_HEIGHT: f32 = 22.;

pub struct CopySelection(pub String);

struct HitRow {
    bounds: Bounds<Pixels>,
    origin: Point<Pixels>,
    line: ShapedLine,
    source: RowSource,
    gutter_right: Pixels,
}

pub struct DiffSelection {
    text: SharedString,
    gutter_width: Pixels,
    anchor: usize,
    cursor: usize,
    dragging: bool,
    focus: FocusHandle,
    hits: Rc<RefCell<BTreeMap<usize, HitRow>>>,
}

impl EventEmitter<CopySelection> for DiffSelection {}

impl DiffSelection {
    pub fn new(text: SharedString, gutter_width: Pixels, cx: &mut Context<Self>) -> Self {
        Self {
            text,
            gutter_width,
            anchor: 0,
            cursor: 0,
            dragging: false,
            focus: cx.focus_handle(),
            hits: Rc::new(RefCell::new(BTreeMap::new())),
        }
    }

    fn range(&self) -> Range<usize> {
        self.anchor.min(self.cursor)..self.anchor.max(self.cursor)
    }

    fn hit(&self, position: Point<Pixels>) -> Option<usize> {
        let hits = self.hits.borrow();
        let row = hits
            .values()
            .find(|row| position.y < row.bounds.bottom())
            .or_else(|| hits.values().next_back())?;
        Some(match &row.source {
            RowSource::Boundary(offset) => (*offset).min(self.text.len()),
            RowSource::Code(source) => {
                let index = row.line.closest_index_for_x(position.x - row.origin.x);
                let local = if position.x < row.gutter_right {
                    0
                } else {
                    index.min(source.len())
                };
                let text = &self.text[source.clone()];
                source.start
                    + text
                        .grapheme_indices(true)
                        .map(|(index, _)| index)
                        .chain(std::iter::once(text.len()))
                        .min_by_key(|index| index.abs_diff(local))
                        .unwrap_or(0)
            }
        })
    }

    fn extend(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        if let Some(cursor) = self.hit(position)
            && cursor != self.cursor
        {
            self.cursor = cursor;
            cx.notify();
        }
    }

    fn action(&mut self, action: &Select, _: &mut Window, cx: &mut Context<Self>) {
        match action {
            Select::Copy if !self.range().is_empty() => {
                cx.emit(CopySelection(self.text[self.range()].to_owned()))
            }
            Select::Copy => {}
            Select::All => {
                self.anchor = 0;
                self.cursor = self.text.len();
                self.dragging = false;
            }
            Select::Clear => {
                self.anchor = self.cursor;
                self.dragging = false;
            }
        }
        cx.notify();
    }

    pub fn wrap(&mut self, content: AnyElement, cx: &Context<Self>) -> Stateful<Div> {
        // Hit geometry is rebuilt from painted rows each frame, never retained as a layout cache.
        self.hits.borrow_mut().clear();
        let selection = cx.entity();
        div()
            .id("selectable-diff")
            .debug_selector(|| "selectable-diff".into())
            .relative()
            .size_full()
            .key_context("DiffSelection")
            .track_focus(&self.focus)
            .cursor_text()
            .on_action(cx.listener(Self::action))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &gpui::MouseDownEvent, window, cx| {
                    window.focus(&this.focus);
                    if let Some(cursor) = this.hit(event.position) {
                        this.cursor = cursor;
                        if !event.modifiers.shift {
                            this.anchor = cursor;
                        }
                        this.dragging = true;
                        cx.notify();
                    }
                }),
            )
            .on_mouse_move(cx.listener(|this, event: &gpui::MouseMoveEvent, _, cx| {
                if this.dragging && event.pressed_button == Some(MouseButton::Left) {
                    this.extend(event.position, cx);
                } else {
                    this.dragging = false;
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, event: &gpui::MouseUpEvent, _, cx| {
                    if this.dragging {
                        this.extend(event.position, cx);
                    }
                    this.dragging = false;
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, event: &gpui::MouseUpEvent, _, cx| {
                    if this.dragging {
                        this.extend(event.position, cx);
                    }
                    this.dragging = false;
                }),
            )
            .on_scroll_wheel(cx.listener(|_, _, _, cx| cx.notify()))
            .child(content)
            .child(
                canvas(
                    |_, _, _| (),
                    move |bounds, _, window, cx| {
                        // Wheel scrolling can move text under a stationary selection endpoint.
                        if bounds.contains(&window.mouse_position()) {
                            selection.update(cx, |selection, cx| {
                                if selection.dragging {
                                    selection.extend(window.mouse_position(), cx);
                                }
                            });
                        }
                    },
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            )
    }

    pub fn row(
        &self,
        index: usize,
        text: SharedString,
        source: RowSource,
        window: &Window,
        cx: &App,
    ) -> Div {
        let selected = self.range();
        let theme = Theme::get(window, cx);
        let color = if self.focus.is_focused(window) {
            theme.selection
        } else {
            theme.control
        };
        let selected = match &source {
            RowSource::Code(range) => {
                let start = selected.start.max(range.start);
                let end = selected.end.min(range.end);
                (start < end)
                    .then_some(start.saturating_sub(range.start)..end.saturating_sub(range.start))
            }
            RowSource::Boundary(_) => None,
        };
        let newline = matches!(&source, RowSource::Code(range) if self.range().contains(&range.end) && self.text.as_bytes().get(range.end) == Some(&b'\n'));
        div().h_full().child(line::Line {
            index,
            text,
            source,
            selected,
            newline,
            selection_color: color,
            gutter_width: self.gutter_width,
            hits: Rc::clone(&self.hits),
        })
    }
}

#[cfg(test)]
mod tests;
