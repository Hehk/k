// TODO: Complete native text editing, including Cmd/Option-Delete and word movement/selection.
// Follow-up: kestrel/UI/text_input_native_editing__plan.md in Obsidian.

use std::ops::Range;

use gpui::{
    Action, App, Bounds, ClipboardItem, Context, ElementInputHandler, EntityInputHandler,
    EventEmitter, FocusHandle, Focusable, KeyBinding, MouseButton, Pixels, Point, ShapedLine,
    SharedString, TextRun, UTF16Selection, UnderlineStyle, Window, canvas, div, fill, point,
    prelude::*, px, rgb, rgba, size,
};
use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = text_input, no_json)]
enum Edit {
    Left,
    Right,
    SelectLeft,
    SelectRight,
    Home,
    End,
    SelectAll,
    Backspace,
    Delete,
    Copy,
    Cut,
    Paste,
    Submit,
}

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys(
        [
            ("left", Edit::Left),
            ("right", Edit::Right),
            ("shift-left", Edit::SelectLeft),
            ("shift-right", Edit::SelectRight),
            ("cmd-left", Edit::Home),
            ("cmd-right", Edit::End),
            ("home", Edit::Home),
            ("end", Edit::End),
            ("cmd-a", Edit::SelectAll),
            ("backspace", Edit::Backspace),
            ("delete", Edit::Delete),
            ("cmd-c", Edit::Copy),
            ("cmd-x", Edit::Cut),
            ("cmd-v", Edit::Paste),
            ("enter", Edit::Submit),
        ]
        .map(|(key, action)| KeyBinding::new(key, action, Some("TextInput"))),
    );
}

pub enum InputEvent {
    Changed(String),
    Submit,
}

pub struct TextInput {
    id: &'static str,
    placeholder: SharedString,
    text: String,
    anchor: usize,
    cursor: usize,
    marked: Option<Range<usize>>,
    focus: FocusHandle,
    selecting: bool,
    layout: Option<ShapedLine>,
    bounds: Option<Bounds<Pixels>>,
    scroll: Pixels,
}

impl EventEmitter<InputEvent> for TextInput {}
impl Focusable for TextInput {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

fn from_utf16(text: &str, offset: usize) -> usize {
    let mut units = 0;
    for (index, ch) in text.char_indices() {
        if units >= offset {
            return index;
        }
        units += ch.len_utf16();
    }
    text.len()
}

impl TextInput {
    pub fn new(id: &'static str, placeholder: &str, cx: &mut Context<Self>) -> Self {
        Self {
            id,
            placeholder: placeholder.to_owned().into(),
            text: String::new(),
            anchor: 0,
            cursor: 0,
            marked: None,
            focus: cx.focus_handle(),
            selecting: false,
            layout: None,
            bounds: None,
            scroll: px(0.),
        }
    }

    pub fn set_text(&mut self, text: &str, cx: &mut Context<Self>) {
        if self.text == text {
            return;
        }
        self.text = text.to_owned();
        self.cursor = self.text.len();
        self.anchor = self.cursor;
        self.marked = None;
        self.layout = None;
        cx.notify();
    }

    fn selection(&self) -> Range<usize> {
        self.anchor.min(self.cursor)..self.anchor.max(self.cursor)
    }
    fn utf16(&self, index: usize) -> usize {
        self.text[..index].encode_utf16().count()
    }
    fn byte_range(&self, range: Range<usize>) -> Range<usize> {
        let start = from_utf16(&self.text, range.start);
        start..from_utf16(&self.text, range.end).max(start)
    }
    fn boundary(&self, forward: bool) -> usize {
        let mut boundaries = self.text.grapheme_indices(true).map(|(index, _)| index);
        if forward {
            boundaries
                .find(|index| *index > self.cursor)
                .unwrap_or(self.text.len())
        } else {
            boundaries.rfind(|index| *index < self.cursor).unwrap_or(0)
        }
    }
    fn hit(&self, position: Point<Pixels>) -> usize {
        self.layout
            .as_ref()
            .zip(self.bounds)
            .map_or(0, |(line, bounds)| {
                line.closest_index_for_x(position.x - bounds.left() + self.scroll)
                    .min(self.text.len())
            })
    }
    fn edit(&mut self, action: &Edit, window: &mut Window, cx: &mut Context<Self>) {
        match action {
            Edit::Left | Edit::Right | Edit::SelectLeft | Edit::SelectRight => {
                let forward = matches!(action, Edit::Right | Edit::SelectRight);
                let extend = matches!(action, Edit::SelectLeft | Edit::SelectRight);
                self.cursor = if !extend && self.anchor != self.cursor {
                    if forward {
                        self.selection().end
                    } else {
                        self.selection().start
                    }
                } else {
                    self.boundary(forward)
                };
                if !extend {
                    self.anchor = self.cursor;
                }
            }
            Edit::Home | Edit::End => {
                self.cursor = if *action == Edit::Home {
                    0
                } else {
                    self.text.len()
                };
                self.anchor = self.cursor;
            }
            Edit::SelectAll => {
                self.anchor = 0;
                self.cursor = self.text.len();
            }
            Edit::Backspace | Edit::Delete => {
                if self.selection().is_empty() {
                    self.cursor = self.boundary(*action == Edit::Delete);
                }
                self.replace_text_in_range(None, "", window, cx);
            }
            Edit::Copy | Edit::Cut => {
                if !self.selection().is_empty() {
                    cx.write_to_clipboard(ClipboardItem::new_string(
                        self.text[self.selection()].to_owned(),
                    ));
                    if *action == Edit::Cut {
                        self.replace_text_in_range(None, "", window, cx);
                    }
                }
            }
            Edit::Paste => {
                if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                    self.replace_text_in_range(None, &text, window, cx);
                }
            }
            Edit::Submit => cx.emit(InputEvent::Submit),
        }
        cx.notify();
    }

    fn replace(&mut self, range: Option<Range<usize>>, text: &str) -> Range<usize> {
        let range = range
            .map(|range| self.byte_range(range))
            .or(self.marked.clone())
            .unwrap_or(self.selection());
        let text = text.replace(['\r', '\n', '\t'], " ");
        self.text.replace_range(range.clone(), &text);
        self.layout = None;
        self.cursor = range.start + text.len();
        self.anchor = self.cursor;
        self.marked = None;
        range.start..self.cursor
    }
}

impl EntityInputHandler for TextInput {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        actual: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.byte_range(range);
        *actual = Some(self.utf16(range.start)..self.utf16(range.end));
        Some(self.text[range].to_owned())
    }
    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let range = self.selection();
        Some(UTF16Selection {
            range: self.utf16(range.start)..self.utf16(range.end),
            reversed: self.cursor < self.anchor,
        })
    }
    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked
            .as_ref()
            .map(|range| self.utf16(range.start)..self.utf16(range.end))
    }
    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.marked = None;
        cx.notify();
    }
    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.replace(range, text);
        cx.emit(InputEvent::Changed(self.text.clone()));
        cx.notify();
    }
    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        selection: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let inserted = self.replace(range, text);
        if !inserted.is_empty() {
            if let Some(selection) = selection {
                let text = &self.text[inserted.clone()];
                self.anchor = inserted.start + from_utf16(text, selection.start);
                self.cursor = inserted.start + from_utf16(text, selection.end);
            }
            self.marked = Some(inserted);
        }
        cx.emit(InputEvent::Changed(self.text.clone()));
        cx.notify();
    }
    fn bounds_for_range(
        &mut self,
        range: Range<usize>,
        _: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let range = self.byte_range(range);
        let line = self.layout.as_ref()?;
        let bounds = self.bounds?;
        Some(Bounds::from_corners(
            point(
                bounds.left() + line.x_for_index(range.start) - self.scroll,
                bounds.top(),
            ),
            point(
                bounds.left() + line.x_for_index(range.end) - self.scroll,
                bounds.bottom(),
            ),
        ))
    }
    fn character_index_for_point(
        &mut self,
        position: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        self.bounds?;
        Some(self.utf16(self.hit(position)))
    }
}

impl Render for TextInput {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let id = self.id;
        let input = cx.entity();
        let paint_input = input.clone();
        div()
            .id(id)
            .debug_selector(move || id.into())
            .key_context("TextInput")
            .track_focus(&self.focus)
            .cursor_text()
            .w_full()
            .min_w_0()
            .h(px(32.))
            .px_2()
            .py_1()
            .border_1()
            .border_color(rgb(0xc8cbd2))
            .rounded_md()
            .overflow_hidden()
            .on_action(cx.listener(Self::edit))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &gpui::MouseDownEvent, window, cx| {
                    window.focus(&this.focus);
                    this.cursor = this.hit(event.position);
                    if !event.modifiers.shift {
                        this.anchor = this.cursor;
                    }
                    this.selecting = true;
                    cx.notify();
                }),
            )
            .on_mouse_move(cx.listener(|this, event: &gpui::MouseMoveEvent, _, cx| {
                if this.selecting {
                    this.cursor = this.hit(event.position);
                    cx.notify();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, _| this.selecting = false),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, _| this.selecting = false),
            )
            .child(
                canvas(
                    move |_, window, cx| {
                        let input = input.read(cx);
                        let style = window.text_style();
                        let text: SharedString = if input.text.is_empty() {
                            input.placeholder.clone()
                        } else {
                            input.text.clone().into()
                        };
                        let color = if input.text.is_empty() {
                            rgb(0x8a8d94).into()
                        } else {
                            style.color
                        };
                        let run = TextRun {
                            len: text.len(),
                            font: style.font(),
                            color,
                            background_color: None,
                            underline: None,
                            strikethrough: None,
                        };
                        let runs = if let Some(marked) = &input.marked {
                            vec![
                                TextRun {
                                    len: marked.start,
                                    ..run.clone()
                                },
                                TextRun {
                                    len: marked.len(),
                                    underline: Some(UnderlineStyle {
                                        color: Some(color),
                                        thickness: px(1.),
                                        wavy: false,
                                    }),
                                    ..run.clone()
                                },
                                TextRun {
                                    len: text.len() - marked.end,
                                    ..run
                                },
                            ]
                        } else {
                            vec![run]
                        };
                        window.text_system().shape_line(
                            text,
                            style.font_size.to_pixels(window.rem_size()),
                            &runs,
                            None,
                        )
                    },
                    move |bounds, line, window, cx| {
                        let input = paint_input.read(cx);
                        let focus = input.focus.clone();
                        let cursor = line.x_for_index(input.cursor);
                        let scroll = input
                            .scroll
                            .max(cursor - bounds.size.width + px(3.))
                            .min(cursor)
                            .max(px(0.));
                        let origin = point(bounds.left() - scroll, bounds.top());
                        let selected = input.selection();
                        if focus.is_focused(window) {
                            let selection_bounds = if selected.is_empty() {
                                Bounds::new(
                                    point(origin.x + cursor, origin.y),
                                    size(px(1.), bounds.size.height),
                                )
                            } else {
                                Bounds::from_corners(
                                    point(
                                        origin.x + line.x_for_index(selected.start),
                                        bounds.top(),
                                    ),
                                    point(
                                        origin.x + line.x_for_index(selected.end),
                                        bounds.bottom(),
                                    ),
                                )
                            };
                            window.paint_quad(fill(
                                selection_bounds,
                                if selected.is_empty() {
                                    rgba(0x3366ccff)
                                } else {
                                    rgba(0x3366cc40)
                                },
                            ));
                        }
                        window.handle_input(
                            &focus,
                            ElementInputHandler::new(bounds, paint_input.clone()),
                            cx,
                        );
                        let _ = line.paint(origin, bounds.size.height, window, cx);
                        paint_input.update(cx, |input, _| {
                            input.layout = Some(line);
                            input.bounds = Some(bounds);
                            input.scroll = scroll;
                        });
                    },
                )
                .size_full(),
            )
    }
}

#[cfg(test)]
mod tests;
