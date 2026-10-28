use super::*;
use gpui::{Modifiers, point};

use super::structural::{commit, redraw};

#[gpui::test]
fn horizontal_reach_uses_display_width_in_structural_and_text_fallback(cx: &mut TestAppContext) {
    for (first, second) in [
        ("é".repeat(100), "x".repeat(150)),
        ("a\u{301}".repeat(100), "x".repeat(150)),
        ("a\u{301}".repeat(250), "😀界".repeat(100)),
        ("é".repeat(100), "\tx".repeat(100)),
    ] {
        let accented_counterexample = first == "é".repeat(100) && second == "x".repeat(150);
        let first = format!("// {first}");
        let second = format!("// {second}");
        let (dir, _, _, repository) = setup(cx);
        let git = git2::Repository::init(dir.path().join("repo")).unwrap();
        let main = commit(&git, "refs/heads/main", None, "");
        let text = format!("{first}\n{second}\n{}", "// short\n".repeat(200));
        commit(&git, "refs/heads/topic", Some(main), &text);
        let (view, cx) = cx.add_window_view(|_, cx| ComparisonView::new(repository, cx));
        cx.simulate_resize(gpui::size(px(800.), px(600.)));
        view.update(cx, |view, cx| {
            view.load_review(Some("refs/heads/main".into()), false, cx)
        });
        for structural in [true, false] {
            if !structural {
                view.update(cx, |view, cx| {
                    view.engine = Arc::new(Engine::new("/missing/difft".into()));
                    view.load_review(None, false, cx);
                });
            }
            redraw(cx);
            let (indices, scroll) = view.read_with(cx, |view, _| {
                let loaded = view.patch.data().expect("loaded patch");
                assert_eq!(
                    loaded.status.ends_with("Structural"),
                    structural,
                    "{}",
                    loaded.status
                );
                let PreparedPatch::Rows { rows, widest, .. } = &loaded.content else {
                    panic!("expected rows")
                };
                assert!(widest.get().is_some(), "width is cached on first display");
                let indices = [&first, &second].map(|text| {
                    rows.iter()
                        .position(|row| row.text.as_ref() == text)
                        .unwrap()
                });
                (indices, view.scroll.clone())
            });
            assert_eq!(indices, [1, 2]);
            let source_selectors = ["diff-source-1", "diff-source-2"];
            let gutter_selectors = ["diff-gutter-1", "diff-gutter-2"];
            let sources = source_selectors.map(|selector| cx.debug_bounds(selector).unwrap());
            let widest = if sources[0].size.width > sources[1].size.width {
                0
            } else {
                1
            };
            let gutter = cx.debug_bounds(gutter_selectors[widest]).unwrap();
            let viewport_width = scroll.0.borrow().last_item_size.unwrap().item.width;
            let right = gutter.left() + viewport_width;
            assert!(sources[widest].right() > right, "fixture must overflow");
            let content_width = scroll.0.borrow().last_item_size.unwrap().contents.width;
            assert!(content_width >= sources[widest].size.width + px(DIFF_GUTTER_WIDTH));
            if accented_counterexample {
                assert!(first.len() > second.len());
                assert!(sources[0].size.width < sources[1].size.width);
            }
            assert!(
                cx.debug_bounds("diff-source-100").is_none(),
                "list remains virtualized"
            );
            cx.simulate_event(gpui::ScrollWheelEvent {
                position: point(gutter.right() + px(10.), gutter.center().y),
                delta: gpui::ScrollDelta::Pixels(point(px(-100_000.), px(0.))),
                modifiers: Modifiers::none(),
                touch_phase: gpui::TouchPhase::Moved,
            });
            redraw(cx);
            let reached = cx.debug_bounds(source_selectors[widest]).unwrap();
            assert!(scroll.0.borrow().base_handle.offset().x < px(0.));
            assert!(
                (reached.right() - right).abs() <= px(1.),
                "longest row end must reach the viewport edge"
            );
            assert_eq!(cx.debug_bounds(gutter_selectors[widest]).unwrap(), gutter);
            redraw(cx);
            assert_eq!(
                scroll.0.borrow().last_item_size.unwrap().contents.width,
                content_width
            );
        }
    }
}
