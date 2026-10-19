use std::{ops::Range, sync::Arc};

use gpui::{
    AnyElement, App, Bounds, Pixels, Point, UniformListDecoration, Window, canvas, div, point,
    prelude::*, px,
};

use crate::{appearance::Theme, diff_selection::line};

use super::{DIFF_FONT, DIFF_GUTTER_WIDTH, DIFF_NUMBER_WIDTH, DiffRow};

pub(super) struct DiffGutter(pub Arc<[DiffRow]>);

impl UniformListDecoration for DiffGutter {
    fn compute(
        &self,
        visible: Range<usize>,
        _: Bounds<Pixels>,
        scroll_offset: Point<Pixels>,
        row_height: Pixels,
        _: usize,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let theme = Theme::get(window, cx);
        // Share the list's vertical origin, but cancel horizontal movement.
        div()
            .relative()
            .size_full()
            .children(visible.map(|index| {
                let row = &self.0[index];
                let labels = [row.old.clone(), row.new.clone(), row.marker.into()];
                div()
                    .debug_selector(move || format!("diff-gutter-{index}"))
                    .absolute()
                    .left(-scroll_offset.x)
                    .top(row_height * index)
                    .w(px(DIFF_GUTTER_WIDTH))
                    .h(row_height)
                    .bg(theme.diff(row.tone))
                    .font_family(DIFF_FONT)
                    .text_size(px(13.))
                    .child(
                        canvas(
                            move |_, window, _| labels.map(|text| line::shape(text, window)),
                            move |bounds, lines, window, cx| {
                                for (column, shaped) in lines.iter().enumerate() {
                                    let origin = bounds.origin
                                        + point(px(DIFF_NUMBER_WIDTH * column as f32), px(0.));
                                    line::paint(
                                        shaped,
                                        origin,
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
            }))
            .into_any_element()
    }
}
