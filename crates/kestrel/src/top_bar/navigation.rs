use gpui::{Bounds, Context, PathBuilder, canvas, div, point, prelude::*, px, size};

use super::{
    TopBar,
    model::{SIDE_WIDTH, TITLEBAR_HEIGHT, WindowCommand},
};
use crate::{appearance::Theme, controls::tooltip};

// TODO: GPUI 0.2.2 has no accessibility-node API. Tooltips are not VoiceOver names;
// expose these controls and the recent rows when that platform support is available.
pub const NAVIGATION_WIDTH: f32 = 96.;
const BUTTON: f32 = 28.;
const GAP: f32 = 2.;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NavigationState {
    pub enabled: bool,
    pub back: bool,
    pub forward: bool,
    pub recent_open: bool,
}

#[derive(Clone, Copy)]
enum Icon {
    Back,
    Forward,
    Clock,
}

impl TopBar {
    pub fn set_navigation(&mut self, state: NavigationState, cx: &mut Context<Self>) {
        if self.navigation != state {
            self.navigation = state;
            cx.notify();
        }
    }

    pub fn clock_bounds(&self) -> Bounds<gpui::Pixels> {
        let geometry = self.model.geometry();
        Bounds::new(
            point(
                px(geometry.x + SIDE_WIDTH + 2. * (BUTTON + GAP)),
                px(geometry.y + (TITLEBAR_HEIGHT - BUTTON) / 2.),
            ),
            size(px(BUTTON), px(BUTTON)),
        )
    }

    pub(super) fn render_navigation(&self, theme: Theme, cx: &Context<Self>) -> impl IntoElement {
        let enabled = self.navigation.enabled && self.interactive();
        div()
            .w(px(NAVIGATION_WIDTH))
            .h_full()
            .flex_shrink_0()
            .flex()
            .tab_group()
            .items_center()
            .gap(px(GAP))
            .pr(px(8.))
            .children(
                [
                    (
                        "navigate-back",
                        "Back (⌘[)",
                        Icon::Back,
                        WindowCommand::Back,
                        enabled && self.navigation.back,
                    ),
                    (
                        "navigate-forward",
                        "Forward (⌘])",
                        Icon::Forward,
                        WindowCommand::Forward,
                        enabled && self.navigation.forward,
                    ),
                    (
                        "recently-viewed",
                        "Recently viewed",
                        Icon::Clock,
                        WindowCommand::RecentlyViewed,
                        enabled,
                    ),
                ]
                .into_iter()
                .map(|(id, label, icon, command, enabled)| {
                    let key_command = command.clone();
                    div()
                        .id(id)
                        .debug_selector(move || id.into())
                        .size(px(BUTTON))
                        .flex_shrink_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_md()
                        .border_1()
                        .border_color(gpui::transparent_black())
                        .tooltip(tooltip(label))
                        .when(
                            id == "recently-viewed" && self.navigation.recent_open,
                            |button| button.bg(theme.pressed),
                        )
                        .when(enabled, |button| {
                            button
                                .tab_index(0)
                                .focus(|button| button.border_color(theme.accent))
                                .hover(|button| button.bg(theme.hover))
                                .active(|button| button.bg(theme.pressed))
                        })
                        .cursor_default()
                        .on_mouse_down(gpui::MouseButton::Left, |_, window, cx| {
                            window.prevent_default();
                            cx.stop_propagation();
                        })
                        .when(enabled, |button| {
                            button
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if this.interactive() {
                                        cx.emit(command.clone());
                                    }
                                }))
                                .on_key_down(cx.listener(
                                    move |this, event: &gpui::KeyDownEvent, _, cx| {
                                        if this.interactive()
                                            && matches!(
                                                event.keystroke.key.as_str(),
                                                "enter" | "space"
                                            )
                                        {
                                            cx.emit(key_command.clone());
                                            cx.stop_propagation();
                                        }
                                    },
                                ))
                        })
                        .child(
                            canvas(
                                |_, _, _| (),
                                move |bounds, _, window, _| {
                                    let origin = bounds.origin;
                                    let p = |x, y| origin + point(px(x), px(y));
                                    let mut path = PathBuilder::stroke(px(1.5));
                                    match icon {
                                        Icon::Back | Icon::Forward => {
                                            let (tip, tail) = match icon {
                                                Icon::Back => (5., 11.),
                                                _ => (11., 5.),
                                            };
                                            path.move_to(p(tail, 2.));
                                            path.line_to(p(tip, 8.));
                                            path.line_to(p(tail, 14.));
                                        }
                                        Icon::Clock => {
                                            for index in 0..=32 {
                                                let angle =
                                                    index as f32 * std::f32::consts::TAU / 32.;
                                                let point = p(
                                                    8. + 6.5 * angle.cos(),
                                                    8. + 6.5 * angle.sin(),
                                                );
                                                if index == 0 {
                                                    path.move_to(point);
                                                } else {
                                                    path.line_to(point);
                                                }
                                            }
                                            path.move_to(p(8., 3.5));
                                            path.line_to(p(8., 8.));
                                            path.line_to(p(11., 9.5));
                                        }
                                    }
                                    if let Ok(path) = path.build() {
                                        window.paint_path(
                                            path,
                                            if enabled { theme.text } else { theme.disabled },
                                        );
                                    }
                                },
                            )
                            .size(px(16.)),
                        )
                }),
            )
    }
}
