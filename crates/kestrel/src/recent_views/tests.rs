use super::*;

fn fixture(index: usize) -> Record {
    serde_json::from_str(
        include_str!("../viewing_history/storage/fixtures/v1.jsonl")
            .lines()
            .nth(index + 1)
            .unwrap(),
    )
    .unwrap()
}

#[test]
fn rows_show_tab_names_with_identity_and_shortcuts_in_tooltips() {
    let branches = Entry::new(&fixture(0));
    assert_eq!(branches.title, "example");
    assert!(
        branches
            .tooltip
            .contains("/repositories/example\nBranch list")
    );
    let comparison = Entry::new(&fixture(1));
    assert_eq!(comparison.title, "example - topic-�");
    assert!(comparison.tooltip.starts_with(&comparison.title));
    assert!(comparison.tooltip.contains("refs/heads/topic-�"));
    assert!(
        comparison
            .tooltip
            .contains("Base: refs/remotes/origin/main")
    );
    assert!(
        comparison
            .tooltip
            .contains("Head: 2222222222222222222222222222222222222222")
    );
    assert!(comparison.tooltip.contains("File: src/file-�.rs"));
    for entry in [branches, comparison] {
        assert!(entry.tooltip.contains("Click or Return to open"));
        assert!(
            entry
                .tooltip
                .contains("⌘-click or ⌘Return to open in a new tab")
        );
    }
}

#[test]
fn the_menu_keeps_only_as_many_rows_as_fit_without_scrolling() {
    assert_eq!(visible_limit(760.), 10);
    assert_eq!(visible_limit(400.), 10);
    assert_eq!(visible_limit(280.), 6);
    assert_eq!(visible_limit(160.), 2);
    assert_eq!(visible_limit(80.), 0);
}

#[gpui::test]
fn long_labels_end_with_an_ellipsis_and_short_labels_stay_intact(cx: &mut gpui::TestAppContext) {
    struct Label {
        title: String,
        layout: gpui::TextLayout,
    }

    impl Render for Label {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let text = gpui::StyledText::new(self.title.clone());
            self.layout = text.layout().clone();
            div().w_full().h(px(ROW_HEIGHT)).flex().child(label(text))
        }
    }

    let (view, cx) = cx.add_window_view(|_, _| Label {
        title: String::new(),
        layout: gpui::TextLayout::default(),
    });
    for width in [280., 140., 60.] {
        cx.simulate_resize(gpui::size(px(width), px(100.)));
        for title in [
            "example - very-long-branch-name/日本語/🦅".repeat(4),
            "Home".into(),
        ] {
            view.update(cx, |view, cx| {
                view.title = title.clone();
                cx.notify();
            });
            cx.run_until_parked();
            cx.update(|window, cx| window.draw(cx).clear());
            view.read_with(cx, |view, _| {
                let displayed = view.layout.text();
                if title == "Home" {
                    assert_eq!(displayed, title);
                } else {
                    assert!(displayed.ends_with('…'), "{width}: {displayed}");
                    assert!(title.starts_with(displayed.trim_end_matches('…')));
                }
                assert!(view.layout.bounds().size.width <= px(width));
                assert!(view.layout.bounds().size.height <= px(ROW_HEIGHT));
            });
        }
    }
}

#[test]
fn unresolved_rows_keep_the_requested_base_instead_of_inventing_pins() {
    let mut record = fixture(1);
    let Destination::Comparison { comparison, .. } = &mut record.viewed.snapshot.destination else {
        unreachable!()
    };
    *comparison = None;
    let row = Entry::new(&record);
    assert_eq!(row.title, "example - topic-�");
    assert!(row.tooltip.contains("Not yet resolved"));
    assert!(row.tooltip.contains("Base: refs/remotes/origin/main"));
    assert!(!row.tooltip.contains("Head:"));
}
