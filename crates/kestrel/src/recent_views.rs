use gpui::{
    App, Bounds, Context, Entity, EventEmitter, FocusHandle, KeyDownEvent, Pixels, ScrollHandle,
    Subscription, WeakFocusHandle, Window, deferred, div, prelude::*, px,
};
use kestrel::{
    review::RefName,
    state::{BaseSelection, Destination},
    viewing_history::{RECENT_LIMIT, Record, ViewingId},
};

use crate::{appearance::Theme, controls::tooltip, top_bar::TITLEBAR_HEIGHT, workspace::Workspace};

#[cfg(test)]
mod tests;

pub enum RecentEvent {
    Dismiss,
    Open { id: ViewingId, new_tab: bool },
}

struct Entry {
    id: ViewingId,
    title: String,
    detail: String,
    directory: String,
    tooltip: String,
}

fn reference_label(reference: &RefName) -> String {
    let label = reference.label();
    if reference.0.starts_with(b"refs/remotes/") {
        format!("{label} (remote)")
    } else {
        label
    }
}

impl Entry {
    fn new(record: &Record) -> Self {
        let labels = &record.viewed.labels;
        let (title, detail, identity) = match &record.viewed.snapshot.destination {
            Destination::Comparison {
                branch,
                base,
                comparison,
                ..
            } => {
                let title = format!("{} — {}", labels.repository, reference_label(branch));
                let (detail, identity) = if let Some(comparison) = comparison {
                    let head = comparison.commits.new.to_string();
                    let base = comparison.base_tip.to_string();
                    (
                        format!(
                            "{} / {} · vs {}",
                            &head[..7],
                            &base[..7],
                            reference_label(&comparison.base)
                        ),
                        format!(
                            "{}\nBase: {}\nHead: {head}\nBase tip: {base}\nMerge base: {}",
                            String::from_utf8_lossy(&branch.0),
                            String::from_utf8_lossy(&comparison.base.0),
                            comparison.commits.old
                        ),
                    )
                } else {
                    let (label, base) = match base {
                        BaseSelection::Default => ("default base".into(), "Default".into()),
                        BaseSelection::Explicit(base) => (
                            reference_label(base),
                            String::from_utf8_lossy(&base.0).into_owned(),
                        ),
                    };
                    (
                        format!("Not yet resolved · vs {label}"),
                        format!("{}\nBase: {base}", String::from_utf8_lossy(&branch.0)),
                    )
                };
                (title, detail, identity)
            }
            Destination::Branches(_) => (
                labels.repository.clone(),
                "Branches".into(),
                "Branch list".into(),
            ),
            Destination::Home => unreachable!("utility pages are excluded from recents"),
        };
        let tooltip = format!(
            "{}\n{identity}{}",
            labels.directory,
            labels
                .file
                .as_ref()
                .map(|file| format!("\nFile: {file}"))
                .unwrap_or_default()
        );
        Self {
            id: record.id,
            title,
            detail,
            directory: labels.directory.clone(),
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
    scroll: ScrollHandle,
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
        // Keep menu order stable while it is open. Activation still resolves the record by ID.
        let entries = workspace
            .read(cx)
            .viewing_history()
            .recent(RECENT_LIMIT)
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
            scroll: ScrollHandle::new(),
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
            self.scroll.scroll_to_item(index);
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
        let width = (f32::from(viewport.width) - 16.).clamp(0., 440.);
        let left = f32::from(self.trigger.left())
            .min(f32::from(viewport.width) - width - 8.)
            .max(8.);
        let available = (f32::from(viewport.height) - TITLEBAR_HEIGHT - 12.).max(0.);
        let height = if self.entries.is_empty() {
            128.
        } else {
            62. * self.entries.len() as f32 + 62.
        };
        deferred(
            div()
                .id("recent-views")
                .debug_selector(|| "recent-views".into())
                .absolute()
                .left(px(left))
                .top(px(TITLEBAR_HEIGHT + 4.))
                .w(px(width))
                .h(px(height.min(available)))
                .occlude()
                .track_focus(&self.focus)
                .flex()
                .flex_col()
                .p_1()
                .rounded_lg()
                .border_1()
                .border_color(theme.border)
                .bg(theme.paper)
                .shadow_lg()
                .on_key_down(cx.listener(Self::key_down))
                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_mouse_down_out(cx.listener(|this, event: &gpui::MouseDownEvent, _, cx| {
                    // Let the clock's click toggle the menu rather than close-and-immediately-reopen.
                    if !this.trigger.contains(&event.position) {
                        cx.emit(RecentEvent::Dismiss);
                    }
                }))
                .child(
                    div()
                        .flex_shrink_0()
                        .px_2()
                        .py_1()
                        .text_sm()
                        .text_color(theme.muted)
                        .child("Recently viewed"),
                )
                .child(
                    div()
                        .id("recent-items")
                        .flex_1()
                        .min_h_0()
                        .overflow_y_scroll()
                        .track_scroll(&self.scroll)
                        .when(self.entries.is_empty(), |list| {
                            list.child(
                                div()
                                    .p_3()
                                    .text_sm()
                                    .text_color(theme.muted)
                                    .child("Nothing viewed yet. Open a repository to get started."),
                            )
                        })
                        .children(self.entries.iter().enumerate().map(|(index, entry)| {
                            let id = entry.id;
                            div()
                                .id(("recent-view", id.0))
                                .debug_selector(move || format!("recent-view-{}", id.0))
                                .h(px(62.))
                                .flex_shrink_0()
                                .px_2()
                                .py_1()
                                .flex()
                                .flex_col()
                                .justify_center()
                                .rounded_md()
                                .cursor_pointer()
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
                                .child(
                                    div()
                                        .text_sm()
                                        .text_color(theme.text)
                                        .truncate()
                                        .child(entry.title.clone()),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(theme.muted)
                                        .truncate()
                                        .child(entry.detail.clone()),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(theme.muted)
                                        .truncate()
                                        .child(entry.directory.clone()),
                                )
                        })),
                )
                .child(
                    div()
                        .flex_shrink_0()
                        .px_2()
                        .py_1()
                        .text_xs()
                        .text_color(theme.muted)
                        .child("⌘-click or ⌘Return to open in a new tab"),
                ),
        )
        .with_priority(2)
    }
}
