use std::{ops::Range, sync::Arc};

use gpui::{
    AnyElement, App, Bounds, Pixels, Point, UniformListDecoration, Window, canvas, div, point,
    prelude::*, px,
};

use crate::{
    appearance::{DiffTone, Theme},
    diff_selection::line,
};

use super::{
    DiffRow,
    reading::{FONT, GUTTER_WIDTH, MARKER_WIDTH, NUMBER_WIDTH},
};

pub(super) struct DiffGutter(pub Arc<[DiffRow]>);

impl UniformListDecoration for DiffGutter {
    fn compute(
        &self,
        visible: Range<usize>,
        bounds: Bounds<Pixels>,
        scroll_offset: Point<Pixels>,
        row_height: Pixels,
        _: usize,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let theme = Theme::get(window, cx);
        // Follow the list vertically, but anchor source identity and section
        // headings while code scrolls horizontally.
        div()
            .relative()
            .size_full()
            .children(visible.map(|index| {
                let row = &self.0[index];
                let overlay = div()
                    .absolute()
                    .left(-scroll_offset.x)
                    .top(row_height * index)
                    .h(row_height)
                    .bg(theme.diff(row.tone));
                if row.is_source() {
                    let labels = [row.old.clone(), row.new.clone(), row.marker.into()];
                    overlay
                        .debug_selector(move || format!("diff-gutter-{index}"))
                        .w(px(GUTTER_WIDTH))
                        .border_r_1()
                        .border_color(theme.border)
                        .font_family(FONT)
                        .text_size(px(13.))
                        .child(
                            canvas(
                                move |_, window, _| labels.map(|text| line::shape(text, window)),
                                move |bounds, lines, window, cx| {
                                    for (column, shaped) in lines.iter().enumerate() {
                                        let x = if column == 2 {
                                            px(2. * NUMBER_WIDTH)
                                                + (px(MARKER_WIDTH) - shaped.width) / 2.
                                        } else {
                                            px(NUMBER_WIDTH * (column + 1) as f32 - 8.)
                                                - shaped.width
                                        };
                                        line::paint(
                                            shaped,
                                            bounds.origin + point(x, px(0.)),
                                            row_height,
                                            if column == 2 { theme.text } else { theme.muted },
                                            window,
                                            cx,
                                        );
                                    }
                                },
                            )
                            .size_full(),
                        )
                } else {
                    overlay
                        .debug_selector(move || format!("diff-section-{index}"))
                        .w(bounds.size.width)
                        .flex()
                        .items_center()
                        .px_3()
                        .text_xs()
                        .text_color(theme.muted)
                        .when(row.tone == DiffTone::Hunk, |line| {
                            line.border_t_1().border_color(theme.border)
                        })
                        .child(div().min_w_0().text_ellipsis().child(row.text.clone()))
                }
            }))
            .into_any_element()
    }
}
