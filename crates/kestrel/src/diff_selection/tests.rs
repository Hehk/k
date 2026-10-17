use super::*;
use gpui::{ClipboardItem, Entity, Modifiers, Subscription, TestAppContext, point, px, size};

struct View {
    selection: Entity<DiffSelection>,
    highlighted: bool,
    _subscriptions: [Subscription; 2],
}

impl Render for View {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let row = self.selection.update(cx, |selection, cx| {
            selection
                .row(
                    0,
                    "a\u{301}😀z".into(),
                    RowSource::Code(0..8),
                    if self.highlighted {
                        vec![(0..7, Theme::get(window, cx).added)]
                    } else {
                        Vec::new()
                    },
                    window,
                    cx,
                )
                .h(px(22.))
                .font_family("Menlo")
                .text_size(px(13.))
        });
        self.selection.update(cx, |selection, cx| {
            selection.wrap(row.into_any_element(), cx)
        })
    }
}

#[gpui::test]
fn mouse_selection_uses_shaped_grapheme_boundaries_and_handles_outside_release(
    cx: &mut TestAppContext,
) {
    cx.update(bind_keys);
    for highlighted in [false, true] {
        let (view, cx) = cx.add_window_view(|_, cx| {
            let selection = cx.new(|cx| DiffSelection::new("a\u{301}😀z\n".into(), px(0.), cx));
            let observe = cx.observe(&selection, |_, _, cx| cx.notify());
            let copy = cx.subscribe(&selection, |_, _, event: &CopySelection, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(event.0.clone()));
            });
            View {
                selection,
                highlighted,
                _subscriptions: [observe, copy],
            }
        });
        cx.update(|window, cx| {
            window.draw(cx).clear();
        });
        let selection = view.read_with(cx, |view, _| view.selection.clone());
        let (start, end) = selection.read_with(cx, |selection, _| {
            let hits = selection.hits.borrow();
            let row = &hits[&0];
            let position = |index: usize| {
                point(
                    row.origin.x + row.line.x_for_index(index),
                    row.bounds.center().y,
                )
            };
            let bounds = Bounds::new(row.origin, size(row.line.width, row.bounds.size.height));
            for x in 0..(f32::from(bounds.size.width) as usize) {
                let offset = selection
                    .hit(point(bounds.left() + px(x as f32), row.bounds.center().y))
                    .unwrap();
                assert!([0, 3, 7, 8].contains(&offset), "split grapheme at {offset}");
            }
            (position(3), position(7))
        });
        for (from, to) in [(start, end), (end, start)] {
            cx.simulate_mouse_down(from, MouseButton::Left, Modifiers::none());
            cx.simulate_mouse_move(to, MouseButton::Left, Modifiers::none());
            cx.simulate_mouse_up(to, MouseButton::Left, Modifiers::none());
            cx.simulate_keystrokes("cmd-c");
            cx.update(|_, cx| assert_eq!(cx.read_from_clipboard().unwrap().text().unwrap(), "😀"));
        }
        cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_up(
            point(px(10000.), start.y),
            MouseButton::Left,
            Modifiers::none(),
        );
        selection.read_with(cx, |selection, _| assert!(!selection.dragging));
        cx.simulate_keystrokes("cmd-c");
        cx.update(|_, cx| assert_eq!(cx.read_from_clipboard().unwrap().text().unwrap(), "😀z"));
        cx.simulate_keystrokes("cmd-a cmd-c");
        cx.update(|_, cx| {
            assert_eq!(
                cx.read_from_clipboard().unwrap().text().unwrap(),
                "a\u{301}😀z\n"
            )
        });
    }
}
