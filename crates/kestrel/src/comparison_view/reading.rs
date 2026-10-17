use std::sync::Arc;

use gpui::{AnyElement, App, Entity, div, prelude::*, px, uniform_list};

use crate::{
    appearance::Theme,
    diff_selection::{DiffSelection, ROW_HEIGHT},
};

use super::{DiffRow, gutter::DiffGutter, token_highlight};

#[cfg(test)]
mod tests;

pub(super) const NUMBER_WIDTH: f32 = 52.;
pub(super) const MARKER_WIDTH: f32 = 20.;
pub(super) const GUTTER_WIDTH: f32 = 2. * NUMBER_WIDTH + MARKER_WIDTH;
pub(super) const FONT: &str = if cfg!(target_os = "macos") {
    "Menlo"
} else {
    "monospace"
};

pub(super) fn view(
    rows: Arc<[DiffRow]>,
    widest: Option<usize>,
    theme: Theme,
    selection: &Entity<DiffSelection>,
    cx: &mut App,
) -> AnyElement {
    let decoration = DiffGutter(Arc::clone(&rows));
    let row_selection = selection.clone();
    let list = uniform_list("diff-lines", rows.len(), move |range, window, cx| {
        range
            .map(|index| {
                let row = &rows[index];
                div()
                    .debug_selector(move || format!("diff-line-{index}"))
                    .h(px(ROW_HEIGHT))
                    .pl(px(GUTTER_WIDTH + 8.))
                    .pr_4()
                    .flex()
                    .items_center()
                    .bg(theme.diff(row.tone))
                    .font_family(FONT)
                    .font_weight(gpui::FontWeight::NORMAL)
                    .text_size(px(13.))
                    .child(row_selection.update(cx, |selection, cx| {
                        selection
                            .row(
                                index,
                                if row.is_source() {
                                    row.text.clone()
                                } else {
                                    "".into()
                                },
                                row.source.clone(),
                                row.highlights
                                    .iter()
                                    .map(|(range, tone)| {
                                        (
                                            range.clone(),
                                            token_highlight(theme, *tone)
                                                .background_color
                                                .expect("tokens use background emphasis"),
                                        )
                                    })
                                    .collect(),
                                window,
                                cx,
                            )
                            .debug_selector(move || format!("diff-source-{index}"))
                    }))
            })
            .collect::<Vec<_>>()
    })
    .with_width_from_item(widest)
    .with_decoration(decoration)
    .with_horizontal_sizing_behavior(gpui::ListHorizontalSizingBehavior::Unconstrained)
    .size_full();
    let list = selection.update(cx, |selection, cx| {
        selection
            .wrap(list.into_any_element(), cx)
            .flex()
            .flex_1()
            .min_w_0()
    });
    div()
        .debug_selector(|| "diff-reading-surface".into())
        .size_full()
        .flex()
        .flex_col()
        .border_1()
        .border_color(theme.border)
        .rounded_md()
        .overflow_hidden()
        .child(
            div()
                .h(px(28.))
                .flex_shrink_0()
                .flex()
                .items_center()
                .bg(theme.metadata)
                .border_b_1()
                .border_color(theme.border)
                .text_xs()
                .text_color(theme.muted)
                .child(number("Before".into()))
                .child(number("After".into()))
                .child(div().w(px(MARKER_WIDTH)).flex_shrink_0())
                .child(div().pl_2().child("Code")),
        )
        .child(div().flex().flex_1().min_w_0().min_h_0().child(list))
        .into_any_element()
}

fn number(text: gpui::SharedString) -> gpui::Div {
    div()
        .w(px(NUMBER_WIDTH))
        .flex_shrink_0()
        .pr_2()
        .text_right()
        .child(text)
}
