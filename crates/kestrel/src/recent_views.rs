use gpui::{
    App, Bounds, Context, Entity, EventEmitter, FocusHandle, KeyDownEvent, Pixels, Subscription,
    WeakFocusHandle, Window, deferred, div, prelude::*, px,
};
use kestrel::{
    state::{BaseSelection, Destination},
    viewing_history::{RECENT_LIMIT, Record, ViewingId},
};

use crate::{
    appearance::Theme,
    controls::{single_line, tooltip},
    top_bar::TITLEBAR_HEIGHT,
    workspace::Workspace,
};

#[cfg(test)]
mod tests;

pub enum RecentEvent {
    Dismiss,
    Open { id: ViewingId, new_tab: bool },
}

const ROW_HEIGHT: f32 = 28.;
const HEADER_HEIGHT: f32 = 28.;
const MENU_PADDING: f32 = 8.;
const MENU_CHROME: f32 = HEADER_HEIGHT + 2. * MENU_PADDING + 2.;

fn visible_limit(viewport_height: f32) -> usize {
    let available = viewport_height - TITLEBAR_HEIGHT - 12. - MENU_CHROME;
    ((available / ROW_HEIGHT).floor().max(0.) as usize).min(RECENT_LIMIT)
}

fn label(text: impl IntoElement) -> gpui::Div {
    // Keep width-sensitive measurement: GPUI caches nowrap text before flex assigns its final width.
    div()
        .flex_1()
        .min_w_0()
        .text_sm()
        .text_ellipsis()
        .line_clamp(1)
        .child(text)
}

struct Entry {
    id: ViewingId,
    title: String,
    tooltip: String,
}

impl Entry {
    fn new(record: &Record) -> Self {
        let labels = &record.viewed.labels;
        let (title, identity) = match &record.viewed.snapshot.destination {
            Destination::Comparison {
                branch,
                base,
                comparison,
                ..
            } => {
                let title = format!(
                    "{} - {}",
                    single_line(&labels.repository),
                    single_line(&branch.label())
                );
                let identity = if let Some(comparison) = comparison {
                    format!(
                        "{}\nBase: {}\nHead: {}\nBase tip: {}\nMerge base: {}",
                        single_line(&String::from_utf8_lossy(&branch.0)),
                        single_line(&String::from_utf8_lossy(&comparison.base.0)),
                        comparison.commits.new,
                        comparison.base_tip,
                        comparison.commits.old
                    )
                } else {
                    let base = match base {
                        BaseSelection::Default => "Default".into(),
                        BaseSelection::Explicit(base) => {
                            single_line(&String::from_utf8_lossy(&base.0))
                        }
                    };
                    format!(
                        "{}\nBase: {base}\nNot yet resolved",
                        single_line(&String::from_utf8_lossy(&branch.0))
                    )
                };
                (title, identity)
            }
            Destination::Branches(_) => (single_line(&labels.repository), "Branch list".into()),
            Destination::Home => unreachable!("utility pages are excluded from recents"),
        };
        let tooltip = format!(
            "{title}\n{}\n{identity}{}\n\nClick or Return to open\n⌘-click or ⌘Return to open in a new tab",
            single_line(&labels.directory),
            labels
                .file
                .as_ref()
                .map(|file| format!("\nFile: {}", single_line(file)))
                .unwrap_or_default()
        );
        Self {
            id: record.id,
            title,
            tooltip,
        }
    }
}

pub struct RecentViews {
    entries: Vec<Entry>,
    selected: Option<usize>,
    focus: FocusHandle,
    previous_focus: Option<WeakFocusHandle>,
    trigger: Bounds<Pixels>,
    _blur: Subscription,
}

impl EventEmitter<RecentEvent> for RecentViews {}

impl RecentViews {
    pub fn new(
        workspace: Entity<Workspace>,
        trigger: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        // Resize dismisses the menu. Limit only this presentation so every row fits without scrolling.
        let limit = visible_limit(window.viewport_size().height.into());
        // Keep menu order stable while it is open. Activation still resolves the record by ID.
        let entries = workspace
            .read(cx)
            .viewing_history()
            .recent(limit)
            .map(Entry::new)
            .collect::<Vec<_>>();
        let selected = (!entries.is_empty()).then_some(0);
        let previous_focus = window.focused(cx).map(|focus| focus.downgrade());
        let focus = cx.focus_handle();
        let blur = cx.on_focus_out(&focus, window, |_, _, _, cx| cx.emit(RecentEvent::Dismiss));
        window.focus(&focus);
        Self {
            entries,
            selected,
            focus,
            previous_focus,
            trigger,
            _blur: blur,
        }
    }

    pub fn restore_focus(&self, fallback: &FocusHandle, window: &mut Window, cx: &App) {
        // An outside click may already have focused another control; do not steal it back.
        if self.focus.contains_focused(window, cx) {
            let previous = self
                .previous_focus
                .as_ref()
                .and_then(|focus| focus.upgrade());
            window.focus(previous.as_ref().unwrap_or(fallback));
        }
    }

    fn select(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.selected != Some(index) {
            self.selected = Some(index);
            cx.notify();
        }
    }

    fn key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        match event.keystroke.key.as_str() {
            "escape" => cx.emit(RecentEvent::Dismiss),
            "tab"
                if !event.keystroke.modifiers.control
                    && !event.keystroke.modifiers.platform
                    && !event.keystroke.modifiers.alt =>
            {
                cx.emit(RecentEvent::Dismiss)
            }
            "up" | "down" if !self.entries.is_empty() => {
                let count = self.entries.len();
                let step = if event.keystroke.key == "up" {
                    count - 1
                } else {
                    1
                };
                self.select((self.selected.unwrap_or(0) + step) % count, cx);
            }
            "home" if !self.entries.is_empty() => self.select(0, cx),
            "end" if !self.entries.is_empty() => self.select(self.entries.len() - 1, cx),
            "enter" => {
                if let Some(index) = self.selected {
                    cx.emit(RecentEvent::Open {
                        id: self.entries[index].id,
                        new_tab: event.keystroke.modifiers.platform,
                    });
                }
            }
            _ => return,
        }
        cx.stop_propagation();
    }
}

impl Render for RecentViews {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::get(window, cx);
        let viewport = window.viewport_size();
        let width = (f32::from(viewport.width) - 16.).clamp(0., 320.);
        let left = f32::from(self.trigger.left())
            .min(f32::from(viewport.width) - width - 8.)
            .max(8.);
        let available = (f32::from(viewport.height) - TITLEBAR_HEIGHT - 12.).max(0.);
        let height = MENU_CHROME + ROW_HEIGHT * self.entries.len().max(1) as f32;
        deferred(
            div()
                .id("recent-views")
                .debug_selector(|| "recent-views".into())
                .absolute()
                .left(px(left))
                .top(px(TITLEBAR_HEIGHT + 4.))
                .w(px(width))
                .h(px(height.min(available)))
                .overflow_hidden()
                .occlude()
                .track_focus(&self.focus)
                .flex()
                .flex_col()
                .p(px(MENU_PADDING))
                .rounded_lg()
                .border_1()
                .border_color(theme.border)
                .bg(theme.paper)
                .shadow_lg()
                .on_key_down(cx.listener(Self::key_down))
                .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_mouse_down_out(cx.listener(|this, event: &gpui::MouseDownEvent, _, cx| {
                    // Let the clock's click toggle the menu rather than close-and-immediately-reopen.
                    if !this.trigger.contains(&event.position) {
                        cx.emit(RecentEvent::Dismiss);
                    }
                }))
                .child(
                    div()
                        .h(px(HEADER_HEIGHT))
                        .flex_shrink_0()
                        .px_3()
                        .flex()
                        .items_center()
                        .text_sm()
                        .text_color(theme.muted)
                        .child("Recently viewed"),
                )
                .child(
                    div()
                        .id("recent-items")
                        .flex()
                        .flex_col()
                        .when(self.entries.is_empty(), |list| {
                            list.child(
                                div()
                                    .h(px(ROW_HEIGHT))
                                    .px_3()
                                    .flex()
                                    .items_center()
                                    .text_color(theme.muted)
                                    .child(label("No recent items to show.")),
                            )
                        })
                        .children(self.entries.iter().enumerate().map(|(index, entry)| {
                            let id = entry.id;
                            div()
                                .id(("recent-view", id.0))
                                .debug_selector(move || format!("recent-view-{}", id.0))
                                .h(px(ROW_HEIGHT))
                                .flex_shrink_0()
                                .px_3()
                                .flex()
                                .items_center()
                                .rounded_md()
                                .cursor_default()
                                .text_color(theme.text)
                                .when(self.selected == Some(index), |row| row.bg(theme.selection))
                                .tooltip(tooltip(entry.tooltip.clone()))
                                .on_mouse_move(
                                    cx.listener(move |this, _, _, cx| this.select(index, cx)),
                                )
                                .on_click(cx.listener(move |_, event: &gpui::ClickEvent, _, cx| {
                                    cx.emit(RecentEvent::Open {
                                        id,
                                        new_tab: event.modifiers().platform,
                                    })
                                }))
                                .child(label(entry.title.clone()))
                        })),
                ),
        )
        .with_priority(2)
    }
}
