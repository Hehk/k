pub mod model;
mod shortcuts;
mod tooltip;

pub use shortcuts::{TabAction, bind_keys};
use tooltip::tooltip;

#[cfg(test)]
mod tests;

use std::time::Instant;

use gpui::{
    Context, DispatchPhase, DragMoveEvent, EventEmitter, FocusHandle, KeyDownEvent, MouseButton,
    MouseUpEvent, Pixels, Point, ScrollHandle, WeakFocusHandle, Window, WindowControlArea, canvas,
    div, prelude::*, px, rgb,
};
pub use model::TITLEBAR_HEIGHT;
use model::{
    Command, DragToken, Geometry, Model, Msg, PILL_HEIGHT, PILL_PADDING, SIDE_WIDTH, TAB_GAP,
    TAB_HEIGHT, Tab, TabView, WindowCommand,
};

struct TabDrag {
    tab: Tab,
    token: DragToken,
    width: f32,
}

struct DragPreview {
    title: String,
    width: f32,
}

impl Render for DragPreview {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .w(px(self.width))
            .h(px(TAB_HEIGHT))
            .flex()
            .items_center()
            .justify_center()
            .text_center()
            .px(px(30.))
            .rounded_full()
            .bg(rgb(0xffffff))
            .shadow_md()
            .text_sm()
            .whitespace_nowrap()
            .text_ellipsis()
            .child(self.title.clone())
    }
}

pub struct TopBar {
    // TODO: Derive tabs and selection from shared active-content state when it exists.
    model: Model,
    scroll: ScrollHandle,
    focus: FocusHandle,
    previous_focus: Option<WeakFocusHandle>,
}

impl EventEmitter<WindowCommand> for TopBar {}

impl TopBar {
    pub fn new(initial: Option<(Tab, u64)>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let source = cx.entity_id().as_u64();
        let model = match initial {
            Some((tab, next_id)) => Model::with_tab(source, tab, next_id),
            None => Model::new(source),
        };
        let mut bar = Self {
            model,
            scroll: ScrollHandle::new(),
            focus: cx.focus_handle(),
            previous_focus: None,
        };
        bar.dispatch(
            Msg::LayoutChanged(Geometry {
                width: window.viewport_size().width.into(),
                ..Geometry::default()
            }),
            window,
            cx,
        );
        bar
    }

    pub fn active_tab(&self) -> Option<&Tab> {
        self.model.active_tab()
    }

    pub fn dispatch(&mut self, message: Msg, window: &mut Window, cx: &mut Context<Self>) {
        for command in self.model.update(message) {
            match command {
                Command::Reveal(id) => {
                    if let Some(index) = self.model.tab_index(id) {
                        self.scroll.scroll_to_item(index);
                    }
                }
                Command::RequestFrame {
                    generation,
                    elapsed,
                } => {
                    let start = Instant::now();
                    cx.on_next_frame(window, move |this, window, cx| {
                        this.dispatch(
                            Msg::Frame {
                                generation,
                                elapsed: elapsed + start.elapsed(),
                            },
                            window,
                            cx,
                        );
                    });
                }
                Command::Window(command) => cx.emit(command),
                Command::BeginWindowDrag => crate::start_window_drag(),
                Command::FocusDrag => {
                    self.previous_focus = window.focused(cx).map(|focus| focus.downgrade());
                    window.focus(&self.focus);
                }
                Command::RestoreFocus => {
                    let previous = self.previous_focus.take().and_then(|focus| focus.upgrade());
                    if self.focus.is_focused(window) {
                        if let Some(previous) = previous {
                            window.focus(&previous);
                        } else {
                            window.blur();
                        }
                    }
                }
                Command::StopDrag => {
                    cx.stop_active_drag(window);
                }
            }
        }
        cx.notify();
    }

    fn move_drag(
        &mut self,
        token: DragToken,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let geometry = Geometry {
            scroll: self.scroll.offset().x.into(),
            ..self.model.geometry()
        };
        if geometry != self.model.geometry() {
            self.dispatch(Msg::LayoutChanged(geometry), window, cx);
        }
        self.dispatch(
            Msg::DragMoved {
                token,
                x: position.x.into(),
                y: position.y.into(),
            },
            window,
            cx,
        );
    }

    fn render_add_button(&self, cx: &Context<Self>) -> impl IntoElement {
        div()
            .id("new-tab")
            .debug_selector(|| "new-tab".into())
            .size(px(30.))
            .flex()
            .justify_center()
            .items_center()
            .rounded_md()
            .cursor_pointer()
            .text_color(rgb(0x55575c))
            .hover(|this| this.bg(gpui::white().opacity(0.7)))
            .child("+")
            .tooltip(tooltip("New tab (⌘T)"))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(|this, _, window, cx| this.dispatch(Msg::Add, window, cx)))
    }

    fn render_close_button(&self, id: u64, cx: &Context<Self>) -> impl IntoElement {
        div()
            .id(("close-tab", id))
            .debug_selector(|| format!("close-tab-{id}"))
            .absolute()
            .right(px(5.))
            .top(px((TAB_HEIGHT - 20.) / 2.))
            .size(px(20.))
            .flex()
            .items_center()
            .justify_center()
            .rounded_full()
            .cursor_pointer()
            .text_sm()
            .text_color(rgb(0x686a70))
            .hover(|this| this.bg(rgb(0xd5d6da)).text_color(rgb(0x25262a)))
            .child("×")
            .tooltip(tooltip(
                if self.active_tab().is_some_and(|tab| tab.id == id) {
                    "Close tab (⌘W)"
                } else {
                    "Close tab"
                },
            ))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(move |this, _, window, cx| {
                cx.stop_propagation();
                this.dispatch(Msg::Close(id), window, cx);
            }))
    }

    fn render_tab(
        &self,
        tab: &TabView<'_>,
        width: f32,
        interactive: bool,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let id = tab.tab.id;
        let drag = TabDrag {
            tab: tab.tab.clone(),
            token: self.model.next_drag(),
            width,
        };
        let entity = cx.entity().downgrade();
        div()
            .id(("tab", id))
            .debug_selector(|| format!("tab-slot-{id}"))
            .tooltip(tooltip(tab.tab.title.clone()))
            .w(px(width))
            .h(px(TAB_HEIGHT))
            .flex_shrink_0()
            .relative()
            .child(
                div()
                    .absolute()
                    .left(px(tab.offset))
                    .top_0()
                    .w(px(width))
                    .h_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_center()
                    .px(px(30.))
                    .rounded_full()
                    .cursor_pointer()
                    .text_sm()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .text_color(if tab.active {
                        rgb(0x25262a)
                    } else {
                        rgb(0x686a70)
                    })
                    .when(tab.active, |this| this.bg(rgb(0xffffff)).shadow_sm())
                    .when(!tab.active, |this| {
                        this.hover(|this| this.bg(gpui::white().opacity(0.55)))
                    })
                    .opacity(if tab.hidden { 0. } else { tab.opacity })
                    .child(tab.tab.title.clone())
                    .when(tab.close_visible, |this| {
                        this.child(self.render_close_button(id, cx))
                    }),
            )
            .on_hover(cx.listener(move |this, hovered, window, cx| {
                this.dispatch(
                    Msg::Hovered {
                        tab: id,
                        hovered: *hovered,
                    },
                    window,
                    cx,
                );
            }))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(
                cx.listener(move |this, _, window, cx| this.dispatch(Msg::Select(id), window, cx)),
            )
            .when(interactive, |this| {
                this.on_drag(drag, move |drag, _, window, cx| {
                    let _ = entity.update(cx, |this, cx| {
                        this.dispatch(
                            Msg::DragStarted {
                                tab: drag.tab.id,
                                token: drag.token,
                            },
                            window,
                            cx,
                        );
                        this.move_drag(drag.token, window.mouse_position(), window, cx);
                    });
                    cx.new(|cx| {
                        let owner = entity.clone();
                        let token = drag.token;
                        cx.on_release_in(window, move |_, window, cx| {
                            let _ = owner.update(cx, |this, cx| {
                                this.dispatch(Msg::Cancelled { token }, window, cx);
                            });
                        })
                        .detach();
                        DragPreview {
                            title: drag.tab.title.clone(),
                            width: drag.width,
                        }
                    })
                })
            })
    }
}

impl Render for TopBar {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let view = self.model.view();
        let geometry = self.model.geometry();
        let entity = cx.entity().downgrade();
        let scroll = self.scroll.clone();

        div()
            .relative()
            .h(px(TITLEBAR_HEIGHT))
            .w_full()
            .flex_shrink_0()
            .flex()
            .items_center()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape"
                    && let Some(token) = this.model.drag_token()
                {
                    this.dispatch(Msg::Cancelled { token }, window, cx);
                    cx.stop_propagation();
                }
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    this.dispatch(Msg::BackgroundPressed, window, cx);
                }),
            )
            .on_drag_move::<TabDrag>(cx.listener(
                |this, event: &DragMoveEvent<TabDrag>, window, cx| {
                    let token = event.drag(cx).token;
                    let geometry = Geometry {
                        x: event.bounds.origin.x.into(),
                        y: event.bounds.origin.y.into(),
                        width: event.bounds.size.width.into(),
                        scroll: this.scroll.offset().x.into(),
                    };
                    if this.model.geometry() != geometry {
                        this.dispatch(Msg::LayoutChanged(geometry), window, cx);
                    }
                    this.move_drag(token, event.event.position, window, cx);
                },
            ))
            .child(
                canvas(
                    |_, _, _| (),
                    move |bounds, _, window, cx| {
                        let capture = entity.clone();
                        window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
                            if phase == DispatchPhase::Capture && event.button == MouseButton::Left
                            {
                                let _ = capture.update(cx, |this, cx| {
                                    if let Some(token) = this.model.drag_token() {
                                        let size = window.viewport_size();
                                        let outside = event.position.x < px(0.)
                                            || event.position.y < px(0.)
                                            || event.position.x >= size.width
                                            || event.position.y >= size.height;
                                        if !outside {
                                            this.move_drag(token, event.position, window, cx);
                                        }
                                        this.dispatch(Msg::Released { token, outside }, window, cx);
                                        cx.stop_propagation();
                                    }
                                });
                            }
                        });
                        let measured = Geometry {
                            x: bounds.origin.x.into(),
                            y: bounds.origin.y.into(),
                            width: bounds.size.width.into(),
                            scroll: scroll.offset().x.into(),
                        };
                        if geometry != measured {
                            window.defer(cx, move |window, cx| {
                                let _ = entity.update(cx, |this, cx| {
                                    if this.model.geometry() != measured {
                                        this.dispatch(Msg::LayoutChanged(measured), window, cx);
                                    }
                                });
                            });
                        }
                    },
                )
                .absolute()
                .size_full(),
            )
            .child(
                div()
                    .w(px(SIDE_WIDTH))
                    .flex_shrink_0()
                    .h_full()
                    .window_control_area(WindowControlArea::Drag),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .justify_center()
                    .items_center()
                    .when(view.tabs.len() == 1, |this| {
                        let tab = &view.tabs[0];
                        this.child(
                            div()
                                .id("single-tab")
                                .tooltip(tooltip(tab.tab.title.clone()))
                                .relative()
                                .w_full()
                                .h(px(TAB_HEIGHT))
                                .flex()
                                .items_center()
                                .justify_center()
                                .text_center()
                                .px(px(30.))
                                .text_sm()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .text_color(rgb(0x25262a))
                                .child(tab.tab.title.clone())
                                .opacity(if tab.hidden { 0. } else { 1. })
                                .when(tab.close_visible, |this| {
                                    this.child(self.render_close_button(tab.tab.id, cx))
                                }),
                        )
                    })
                    .when(view.tabs.len() > 1, |this| {
                        this.child(
                            div()
                                .id("tab-pill")
                                .w(px(view.layout.pill_width))
                                .h(px(PILL_HEIGHT))
                                .flex_shrink_0()
                                .flex()
                                .items_center()
                                .gap(px(TAB_GAP))
                                .px(px(PILL_PADDING))
                                .rounded_full()
                                .bg(rgb(0xe5e6e9))
                                .overflow_x_scroll()
                                .track_scroll(&self.scroll)
                                .children(view.tabs.iter().map(|tab| {
                                    self.render_tab(tab, view.layout.tab_width, view.draggable, cx)
                                })),
                        )
                    }),
            )
            .child(
                div()
                    .w(px(SIDE_WIDTH))
                    .flex_shrink_0()
                    .h_full()
                    .flex()
                    .items_center()
                    .justify_end()
                    .pr_4()
                    .window_control_area(WindowControlArea::Drag)
                    .child(
                        div()
                            .opacity(if view.interactive { 1. } else { 0.5 })
                            .child(self.render_add_button(cx)),
                    ),
            )
    }
}
