use super::*;
use gpui::{Modifiers, VisualTestContext};

fn redraw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear());
}

fn copy_all(cx: &mut VisualTestContext, expected: &str) {
    let source = cx.debug_bounds("diff-source-1").unwrap();
    cx.simulate_click(source.center(), Modifiers::none());
    cx.simulate_keystrokes("cmd-a cmd-c");
    cx.update(|_, cx| assert_eq!(cx.read_from_clipboard().unwrap().text().unwrap(), expected));
}

#[gpui::test]
fn difftastic_uses_existing_view_and_falls_back_without_advancing_pins(cx: &mut TestAppContext) {
    cx.update(crate::diff_selection::bind_keys);
    let (dir, workspace, tab, repository) = setup(cx);
    let git = git2::Repository::init(dir.path().join("repo")).unwrap();
    let commit = |reference: &str, parent: Option<git2::Oid>, text: &str| {
        let mut tree = git.treebuilder(None).unwrap();
        tree.insert("review.rs", git.blob(text.as_bytes()).unwrap(), 0o100644)
            .unwrap();
        let tree = git.find_tree(tree.write().unwrap()).unwrap();
        let parent = parent.map(|id| git.find_commit(id).unwrap());
        let signature = git2::Signature::now("Test", "test@example.invalid").unwrap();
        git.commit(
            Some(reference),
            &signature,
            &signature,
            "fixture",
            &tree,
            &parent.iter().collect::<Vec<_>>(),
        )
        .unwrap()
    };
    let main = commit(
        "refs/heads/main",
        None,
        "fn review(enabled: bool) {\n    show();\n}\n",
    );
    let head = commit(
        "refs/heads/topic",
        Some(main),
        "fn review(enabled: bool) {\n    if enabled {\n        show();\n    }\n}\n",
    );
    let (view, cx) = cx.add_window_view(|_, cx| ComparisonView::new(repository, cx));
    view.update(cx, |view, cx| {
        view.load_review(Some("refs/heads/main".into()), false, cx)
    });
    redraw(cx);
    let (revision, key, file, cancellation) = view.read_with(cx, |view, cx| {
        let Load::Ready(loaded) = &view.patch else {
            match &view.review {
                ReviewStatus::Failed(error) => panic!("review: {error}"),
                _ => match &view.patch {
                    Load::Failed(error) => panic!("patch: {error}"),
                    _ => panic!("diff did not load"),
                },
            }
        };
        assert_eq!(loaded.status, "Difftastic · Rust · Structural");
        let PreparedPatch::Rows { rows, .. } = &loaded.content else {
            panic!()
        };
        assert!(rows.iter().any(|row| row.tone == DiffTone::Added));
        (
            view.patch_revision,
            view.current_key(cx).unwrap(),
            path(view.patch_key.as_ref().unwrap()).clone(),
            view.patch_task.as_ref().unwrap().cancellation.clone(),
        )
    });
    let pinned = workspace.read_with(cx, |workspace, _| workspace.state().clone());
    copy_all(
        cx,
        "fn review(enabled: bool) {\n    if enabled {\n        show();\n    }\n}\n",
    );
    let obsolete_selection =
        view.read_with(cx, |view, _| view.selection.as_ref().unwrap().0.clone());
    assert!(cx.debug_bounds("structural-diff").is_none());
    assert!(cx.debug_bounds("text-diff").is_none());
    assert!(cx.debug_bounds("diff-engine").is_none());
    assert!(cx.debug_bounds("diff-reading-surface").is_none());
    assert!(cx.debug_bounds("diff-section-0").is_none());
    assert_eq!(
        cx.debug_bounds("diff-gutter-1").unwrap().size.width,
        px(108.)
    );
    let title = cx.debug_bounds("diff-file-title").unwrap();
    cx.simulate_mouse_move(title.center(), None, Modifiers::none());
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(1));
    redraw(cx);
    assert!(
        cx.debug_bounds("tooltip-Difftastic · Rust · Structural")
            .is_some()
    );
    // A missing helper on reload must fall back using the same pinned blobs.
    commit("refs/heads/topic", Some(head), "fn unrelated_latest() {}\n");
    view.update(cx, |view, cx| {
        view.engine = Arc::new(Engine::new("/missing/difft".into()));
        view.load_review(None, false, cx);
    });
    redraw(cx);
    assert!(cancellation.check().is_err());
    view.read_with(cx, |view, _| {
        let Load::Ready(loaded) = &view.patch else {
            panic!()
        };
        assert!(
            loaded
                .status
                .starts_with("Text fallback · Difftastic is unavailable")
        );
        let PreparedPatch::Rows { rows, .. } = &loaded.content else {
            panic!()
        };
        assert!(rows.iter().any(|row| row.text.contains("if enabled")));
        assert!(!rows.iter().any(|row| row.text.contains("unrelated_latest")));
        assert!(rows.iter().any(|row| row.tone == DiffTone::Deleted));
    });
    cx.update(|_, cx| {
        cx.write_to_clipboard(gpui::ClipboardItem::new_string("untouched".into()));
        obsolete_selection.update(cx, |_, cx| {
            cx.emit(CopySelection("obsolete selection".into()))
        });
        assert_eq!(
            cx.read_from_clipboard().unwrap().text().unwrap(),
            "untouched"
        );
    });
    copy_all(
        cx,
        "fn review(enabled: bool) {\n    show();\n    if enabled {\n        show();\n    }\n}\n",
    );
    let current_selection =
        view.read_with(cx, |view, _| view.selection.as_ref().unwrap().0.clone());
    view.update(cx, |view, cx| {
        view.finish_patch(
            revision,
            &key,
            &file,
            Err("obsolete structural result".into()),
            cx,
        )
    });
    assert!(view.read_with(cx, |view, _| matches!(view.patch, Load::Ready(_))));
    assert_eq!(
        view.read_with(cx, |view, _| view.selection.as_ref().unwrap().0.clone()),
        current_selection
    );
    assert_eq!(
        workspace.read_with(cx, |workspace, _| workspace.state().clone()),
        pinned
    );
    view.update(cx, |view, cx| {
        view.engine = Arc::new(Engine::bundled());
        view.load_review(None, false, cx);
    });
    redraw(cx);
    view.read_with(cx, |view, _| {
        let Load::Ready(loaded) = &view.patch else {
            panic!()
        };
        assert_eq!(loaded.status, "Difftastic · Rust · Structural");
    });
    copy_all(
        cx,
        "fn review(enabled: bool) {\n    if enabled {\n        show();\n    }\n}\n",
    );
    let cancellation = view.read_with(cx, |view, _| {
        view.patch_task.as_ref().unwrap().cancellation.clone()
    });
    workspace.update(cx, |workspace, cx| {
        workspace
            .edit(
                |state| {
                    state.close_tab(tab);
                    Ok(())
                },
                cx,
            )
            .unwrap()
    });
    redraw(cx);
    assert!(cancellation.check().is_err());
    view.update(cx, |view, cx| {
        view.finish_patch(revision, &key, &file, Err("closed tab result".into()), cx)
    });
    assert!(view.read_with(cx, |view, cx| view.current_key(cx).is_none()));
}
