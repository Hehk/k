use super::*;
use crate::comparison_view::PreparedPatch;
use gpui::{Context, Modifiers, Render, TestAppContext, VisualTestContext, Window, point};
use kestrel::review::{DiffLine, FilePatch, Hunk};

struct ReadingTest {
    rows: Arc<[DiffRow]>,
    widest: Option<usize>,
    selection: Entity<DiffSelection>,
}

impl Render for ReadingTest {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        view(
            Arc::clone(&self.rows),
            self.widest,
            Theme::get(window, cx),
            &self.selection,
            cx,
        )
    }
}

fn redraw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear());
}

#[gpui::test]
fn gutters_and_section_headings_stay_anchored_while_code_scrolls(cx: &mut TestAppContext) {
    let PreparedPatch::Rows { rows, widest, text } =
        PreparedPatch::new(FilePatch::Hunks(vec![Hunk {
            header: "@@ -0,0 +1,3000 @@".into(),
            lines: (1..=3000)
                .map(|new_line| DiffLine::Added {
                    new_line,
                    text: format!("{}\n", "wide source ".repeat(100)),
                })
                .collect(),
        }]))
    else {
        panic!()
    };
    let (_, cx) = cx.add_window_view(|_, cx| ReadingTest {
        rows,
        widest,
        selection: cx.new(|cx| DiffSelection::new(text, px(GUTTER_WIDTH), cx)),
    });
    redraw(cx);
    let gutter = cx.debug_bounds("diff-gutter-1").unwrap();
    let source = cx.debug_bounds("diff-source-1").unwrap();
    let heading = cx.debug_bounds("diff-section-0").unwrap();
    assert_eq!(gutter.size.width, px(GUTTER_WIDTH));
    assert_eq!(gutter.top(), cx.debug_bounds("diff-line-1").unwrap().top());
    assert!(source.left() >= gutter.right());
    assert!(cx.debug_bounds("diff-line-2500").is_none());
    let position = point(gutter.right() + px(10.), gutter.center().y);
    cx.simulate_event(gpui::ScrollWheelEvent {
        position,
        delta: gpui::ScrollDelta::Pixels(point(px(-300.), px(0.))),
        modifiers: Modifiers::none(),
        touch_phase: gpui::TouchPhase::Moved,
    });
    redraw(cx);
    assert_eq!(cx.debug_bounds("diff-gutter-1").unwrap(), gutter);
    assert_eq!(cx.debug_bounds("diff-section-0").unwrap(), heading);
    assert_eq!(
        cx.debug_bounds("diff-source-1").unwrap().left(),
        source.left() - px(300.)
    );
    cx.simulate_event(gpui::ScrollWheelEvent {
        position,
        delta: gpui::ScrollDelta::Pixels(point(px(0.), px(-2200.))),
        modifiers: Modifiers::none(),
        touch_phase: gpui::TouchPhase::Moved,
    });
    redraw(cx);
    let shifted = cx.debug_bounds("diff-gutter-102").unwrap();
    assert_eq!(shifted.left(), gutter.left());
    assert_eq!(
        shifted.top(),
        cx.debug_bounds("diff-line-102").unwrap().top()
    );
    assert_eq!(
        shifted.center().y,
        cx.debug_bounds("diff-source-102").unwrap().center().y
    );
    assert!(cx.debug_bounds("diff-line-2500").is_none());
    cx.simulate_resize(gpui::size(px(600.), px(400.)));
    redraw(cx);
    assert_eq!(
        cx.debug_bounds("diff-gutter-102").unwrap().left(),
        shifted.left()
    );
}
