use std::{cell::RefCell, collections::BTreeMap, ops::Range, rc::Rc};

use gpui::{
    App, Bounds, ContentMask, Element, ElementId, GlobalElementId, Hsla, InspectorElementId,
    IntoElement, LayoutId, Pixels, Point, ShapedGlyph, ShapedLine, SharedString, Style, TextRun,
    Window, fill, point, px, size,
};

use super::{HitRow, ROW_HEIGHT, RowSource};

pub(super) struct Line {
    pub index: usize,
    pub text: SharedString,
    pub source: RowSource,
    pub selected: Option<Range<usize>>,
    pub newline: bool,
    pub selection_color: Hsla,
    pub gutter_width: Pixels,
    pub hits: Rc<RefCell<BTreeMap<usize, HitRow>>>,
}

pub fn shape(text: SharedString, window: &Window) -> ShapedLine {
    let style = window.text_style();
    let run = style.to_run(text.len());
    window.text_system().shape_line(
        text,
        style.font_size.to_pixels(window.rem_size()),
        &[run],
        None,
    )
}

// ASCII is left-to-right, so its shaped glyph positions can be searched directly.
// Other scripts retain the platform's glyph ordering and use conservative clipping.
fn visible_glyphs(
    glyphs: &[ShapedGlyph],
    ascii: bool,
    left: Pixels,
    right: Pixels,
) -> &[ShapedGlyph] {
    if !ascii {
        return glyphs;
    }
    let start = glyphs.partition_point(|glyph| glyph.position.x < left);
    let end = glyphs.partition_point(|glyph| glyph.position.x <= right);
    &glyphs[start..end]
}

pub fn paint(
    line: &ShapedLine,
    origin: Point<Pixels>,
    height: Pixels,
    color: Hsla,
    window: &mut Window,
    cx: &mut App,
) {
    let clip = window.content_mask().bounds;
    let baseline = point(
        px(0.),
        (height - line.ascent - line.descent) / 2. + line.ascent,
    );
    let ascii = line.text.is_ascii();
    // Keep the visible glyphs in one scene layer, rather than inserting a layer per glyph.
    window.paint_layer(Bounds::new(origin, size(line.width, height)), |window| {
        for run in &line.runs {
            let font_bounds = cx.text_system().bounding_box(run.font_id, line.font_size);
            let margin = font_bounds.size.width + font_bounds.origin.x.abs();
            let left = clip.left() - origin.x - margin;
            let right = clip.right() - origin.x + margin;
            for glyph in visible_glyphs(&run.glyphs, ascii, left, right) {
                if glyph.position.x < left || glyph.position.x > right {
                    continue;
                }
                let position = point(origin.x + glyph.position.x, origin.y) + baseline;
                let _ = if glyph.is_emoji {
                    window.paint_emoji(position, run.font_id, glyph.id, line.font_size)
                } else {
                    window.paint_glyph(position, run.font_id, glyph.id, line.font_size, color)
                };
            }
        }
    });
}

impl IntoElement for Line {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl Element for Line {
    type RequestLayoutState = ShapedLine;
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }
    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ShapedLine) {
        let style = window.text_style();
        let run = style.to_run(self.text.len());
        let runs = if let Some(selected) = &self.selected {
            vec![
                TextRun {
                    len: selected.start,
                    ..run.clone()
                },
                TextRun {
                    len: selected.len(),
                    background_color: Some(self.selection_color),
                    ..run.clone()
                },
                TextRun {
                    len: self.text.len() - selected.end,
                    ..run
                },
            ]
        } else {
            vec![run]
        };
        let line = window.text_system().shape_line(
            self.text.clone(),
            style.font_size.to_pixels(window.rem_size()),
            &runs,
            None,
        );
        let layout = window.request_layout(
            Style {
                size: size(line.width.into(), px(ROW_HEIGHT).into()),
                flex_shrink: 0.,
                ..Default::default()
            },
            [],
            cx,
        );
        (layout, line)
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut ShapedLine,
        _: &mut Window,
        _: &mut App,
    ) {
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        line: &mut ShapedLine,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        let viewport = window.content_mask().bounds;
        let gutter_right = viewport.left() + self.gutter_width;
        let clip = Bounds::from_corners(
            point(gutter_right.min(viewport.right()), viewport.top()),
            viewport.bottom_right(),
        );
        let color = window.text_style().color;
        window.with_content_mask(Some(ContentMask { bounds: clip }), |window| {
            if let Some(selected) = &self.selected {
                if line.text.is_ascii() {
                    let start = if selected.start == 0 {
                        px(0.)
                    } else {
                        line.x_for_index(selected.start)
                    };
                    let end = if selected.end == line.len() {
                        line.width
                    } else {
                        line.x_for_index(selected.end)
                    };
                    window.paint_quad(fill(
                        Bounds::new(
                            point(bounds.left() + start, bounds.top()),
                            size(end - start, bounds.size.height),
                        ),
                        self.selection_color,
                    ));
                } else {
                    // Let GPUI handle backgrounds spanning complex-script glyph clusters.
                    let _ = line.paint_background(bounds.origin, bounds.size.height, window, cx);
                }
            }
            if self.newline {
                window.paint_quad(fill(
                    Bounds::new(
                        point(bounds.left() + line.width, bounds.top()),
                        size(px(7.), bounds.size.height),
                    ),
                    self.selection_color,
                ));
            }
            paint(line, bounds.origin, bounds.size.height, color, window, cx);
        });
        let visible = bounds.intersect(&viewport);
        if visible.size.height > px(0.) {
            self.hits.borrow_mut().insert(
                self.index,
                HitRow {
                    bounds: visible,
                    origin: bounds.origin,
                    line: line.clone(),
                    source: self.source.clone(),
                    gutter_right,
                },
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[gpui::test]
    fn clipping_limits_ascii_work_and_preserves_other_glyph_orders(cx: &mut gpui::TestAppContext) {
        let cx = cx.add_empty_window();
        let prototype = cx.update(|window, _| shape("a".into(), window).runs[0].glyphs[0].clone());
        let glyphs = (0..10000)
            .map(|index| ShapedGlyph {
                id: prototype.id,
                position: point(px(index as f32 * 8.), px(0.)),
                index,
                is_emoji: false,
            })
            .collect::<Vec<_>>();
        let visible = visible_glyphs(&glyphs, true, px(800.), px(1600.));
        assert_eq!(visible.len(), 101);
        assert_eq!(visible[0].index, 100);
        assert_eq!(visible[100].index, 200);
        assert!(visible_glyphs(&glyphs, true, px(-20.), px(-1.)).is_empty());
        assert!(visible_glyphs(&glyphs, true, px(80000.), px(81000.)).is_empty());
        assert_eq!(
            visible_glyphs(&glyphs, false, px(800.), px(1600.)).len(),
            glyphs.len()
        );
    }
}
