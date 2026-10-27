use super::*;

#[gpui::test]
fn filtered_list_comparison_traversal_and_forward_truncation_survive_restart(
    cx: &mut TestAppContext,
) {
    let fixture = Fixture::new();
    let main = fixture.git.refname_to_id("refs/heads/main").unwrap();
    let head = fixture.commit_files(
        "refs/heads/topic",
        Some(main),
        400,
        &[
            ("file.rs", "topic\n"),
            ("other.rs", &"added line\n".repeat(5000)),
        ],
    );
    let (workspace, handle) = fixture.start(cx);
    let tab = active(&workspace, 0, cx);
    let cx = &mut VisualTestContext::from_window(handle.into(), cx);
    let snapshot = |cx: &TestAppContext| {
        workspace.read_with(cx, |workspace, _| {
            workspace.state().tab(tab).unwrap().snapshot().unwrap()
        })
    };
    click(cx, "branch-sort");
    click(cx, "sort-name");
    click(cx, "branch-search");
    cx.simulate_input("topic");
    redraw(cx);
    let list = snapshot(cx);
    assert_eq!(list.snapshot.view.branches.query, "topic");
    assert_eq!(list.snapshot.view.branches.sort, BranchSort::Name);
    click(cx, "branch:refs/heads/topic");
    click(cx, "changed-file-1");
    let position = cx.debug_bounds("diff-line-1").unwrap().center();
    scroll(cx, position, 1100.);
    let x = snapshot(cx);
    assert_eq!(x.snapshot.view.browsing.file, Some("other.rs".into()));
    assert_eq!(x.snapshot.view.browsing.first_visible_line, 50);
    let top = cx.debug_bounds("diff-line-51").unwrap().top()
        - cx.debug_bounds("selectable-diff").unwrap().top();

    click(cx, "choose-base");
    click(cx, "base:refs/heads/Alpha");
    click(cx, "changed-file-0");
    let y = snapshot(cx);
    assert_ne!(y.id, x.id);
    let records = workspace.read_with(cx, |workspace, _| {
        workspace.viewing_history().records().to_vec()
    });
    assert_eq!(records.len(), 3);
    assert_eq!(records[1].viewed.snapshot, x.snapshot);
    assert_eq!(records[2].viewed.snapshot, y.snapshot);

    cx.simulate_keystrokes("cmd-[");
    redraw(cx);
    assert_eq!(snapshot(cx), x);
    assert_eq!(
        cx.debug_bounds("diff-line-51").unwrap().top()
            - cx.debug_bounds("selectable-diff").unwrap().top(),
        top
    );
    cx.simulate_keystrokes("cmd-[");
    redraw(cx);
    assert_eq!(snapshot(cx), list);
    cx.simulate_keystrokes("cmd-]");
    redraw(cx);
    assert_eq!(snapshot(cx), x);
    workspace.read_with(cx, |workspace, _| {
        assert!(workspace.state().tab(tab).unwrap().can_go_forward());
    });
    let latest = fixture.commit_files(
        "refs/heads/topic",
        Some(head),
        500,
        &[
            ("file.rs", "latest\n"),
            ("other.rs", &"new line\n".repeat(5000)),
        ],
    );
    click(cx, "latest-review");
    let z = snapshot(cx);
    assert_ne!(z.id, x.id);
    assert_ne!(z.id, y.id);
    let (state, records, recent) = workspace.read_with(cx, |workspace, _| {
        let current = workspace.state().tab(tab).unwrap();
        assert!(!current.can_go_forward());
        assert_eq!(
            current
                .route
                .review()
                .unwrap()
                .active_review()
                .unwrap()
                .comparison
                .as_ref()
                .unwrap()
                .commits
                .new
                .to_string(),
            latest.to_string()
        );
        let history = workspace.viewing_history();
        assert_eq!(history.records().len(), 7);
        assert_eq!(&history.records()[..records.len()], records);
        let recent = history.recent(10).cloned().collect::<Vec<_>>();
        assert_eq!(recent.len(), 4);
        assert_eq!(recent[0].viewed.snapshot, z.snapshot);
        assert_eq!(recent[1].viewed.snapshot, x.snapshot);
        assert_eq!(recent[2].viewed.snapshot, list.snapshot);
        assert_eq!(recent[3].viewed.snapshot, y.snapshot);
        (
            workspace.state().clone(),
            history.records().to_vec(),
            recent,
        )
    });
    cx.simulate_keystrokes("cmd-]");
    redraw(cx);
    assert_eq!(snapshot(cx), z);
    cx.quit();

    let mut restarted = cx.new_app();
    let reloaded = shared(&mut restarted, fixture.dir.path().join("state.json"));
    let window = state.windows()[0].id;
    let reopened = restarted.update(|cx| {
        cx.set_global(crate::workspace::Session(reloaded.clone()));
        commands::install(cx);
        Kestrel::open(reloaded.clone(), window, cx).unwrap()
    });
    let cx = &mut VisualTestContext::from_window(reopened.into(), &restarted);
    cx.update(|window, _| window.activate_window());
    redraw(cx);
    click(cx, "recently-viewed");
    cx.simulate_keystrokes("down up escape");
    redraw(cx);
    reloaded.read_with(cx, |workspace, _| {
        assert_eq!(workspace.state(), &state);
        assert_eq!(workspace.viewing_history().records(), records);
        assert_eq!(
            workspace
                .viewing_history()
                .recent(10)
                .cloned()
                .collect::<Vec<_>>(),
            recent
        );
    });
    cx.quit();
}

#[derive(Clone, Copy)]
enum Unavailable {
    Repository,
    Ref,
    Object,
    File,
}

fn reopen_after_restart(cx: &mut TestAppContext, unavailable: Unavailable) {
    let fixture = Fixture::new();
    let (workspace, handle) = fixture.start(cx);
    let source = active(&workspace, 0, cx);
    let cx = &mut VisualTestContext::from_window(handle.into(), cx);
    click(cx, "branch:refs/heads/topic");
    if matches!(unavailable, Unavailable::File) {
        workspace.update(cx, |workspace, cx| {
            workspace
                .edit(
                    |state| {
                        state.set_browsing(
                            source,
                            &"refs/heads/topic".into(),
                            kestrel::state::Browsing {
                                file: Some("missing.rs".into()),
                                first_visible_line: 12,
                                ..Default::default()
                            },
                        )
                    },
                    cx,
                )
                .unwrap();
        });
        redraw(cx);
    }
    cx.simulate_keystrokes("cmd-t cmd-1 cmd-w");
    redraw(cx);
    let records = workspace.read_with(cx, |workspace, _| {
        assert!(workspace.state().tab(source).is_err());
        workspace.viewing_history().records().to_vec()
    });
    let saved = records.last().unwrap();
    cx.quit();

    match unavailable {
        Unavailable::Repository => {
            std::fs::rename(
                fixture.git.workdir().unwrap(),
                fixture.dir.path().join("moved"),
            )
            .unwrap();
        }
        Unavailable::Ref => {
            fixture
                .git
                .find_reference("refs/heads/topic")
                .unwrap()
                .delete()
                .unwrap();
        }
        Unavailable::Object => {
            let oid = fixture.topic.to_string();
            std::fs::remove_file(
                fixture
                    .git
                    .path()
                    .join("objects")
                    .join(&oid[..2])
                    .join(&oid[2..]),
            )
            .unwrap();
        }
        Unavailable::File => {}
    }
    let mut restarted = cx.new_app();
    let reloaded = shared(&mut restarted, fixture.dir.path().join("state.json"));
    let window = reloaded.read_with(&restarted, |workspace, _| {
        assert_eq!(workspace.viewing_history().records(), records);
        workspace.state().windows()[0].id
    });
    let reopened = restarted.update(|cx| {
        cx.set_global(crate::workspace::Session(reloaded.clone()));
        commands::install(cx);
        Kestrel::open(reloaded.clone(), window, cx).unwrap()
    });
    let cx = &mut VisualTestContext::from_window(reopened.into(), &restarted);
    cx.update(|window, _| window.activate_window());
    redraw(cx);
    click(cx, "recently-viewed");
    cx.simulate_keystrokes("cmd-enter");
    redraw(cx);
    reloaded.read_with(cx, |workspace, _| {
        let tab = workspace
            .state()
            .tab(
                workspace
                    .state()
                    .window(window)
                    .unwrap()
                    .active_tab
                    .unwrap(),
            )
            .unwrap();
        assert_eq!(tab.snapshot().unwrap().snapshot, saved.viewed.snapshot);
        assert!(!tab.can_go_back());
        assert!(!tab.can_go_forward());
        assert_eq!(
            &workspace.viewing_history().records()[..records.len()],
            records
        );
        assert_eq!(
            workspace.viewing_history().records().len(),
            records.len() + 1
        );
    });
    match unavailable {
        Unavailable::Repository => {
            reopened
                .read_with(cx, |root, _| {
                    assert!(root.repository.error().is_some());
                    assert!(root.repository.data().is_none());
                })
                .unwrap();
        }
        Unavailable::Ref => assert!(cx.debug_bounds("diff-line-1").is_some()),
        Unavailable::Object => {
            assert!(cx.debug_bounds("diff-line-1").is_none());
            click(cx, "retry-review");
            assert!(cx.debug_bounds("retry-review").is_some());
        }
        Unavailable::File => {
            assert!(cx.debug_bounds("diff-line-1").is_none());
            click(cx, "retry-patch");
            assert!(cx.debug_bounds("retry-patch").is_some());
        }
    }
    reloaded.read_with(cx, |workspace, _| {
        let tab = workspace
            .state()
            .tab(
                workspace
                    .state()
                    .window(window)
                    .unwrap()
                    .active_tab
                    .unwrap(),
            )
            .unwrap();
        assert_eq!(tab.snapshot().unwrap().snapshot, saved.viewed.snapshot);
        assert_eq!(
            workspace.viewing_history().records().len(),
            records.len() + 1
        );
    });
    cx.quit();
}

#[gpui::test]
fn closed_source_reopens_missing_repository_after_restart(cx: &mut TestAppContext) {
    reopen_after_restart(cx, Unavailable::Repository);
}

#[gpui::test]
fn closed_source_reopens_deleted_ref_from_saved_pins_after_restart(cx: &mut TestAppContext) {
    reopen_after_restart(cx, Unavailable::Ref);
}

#[gpui::test]
fn closed_source_keeps_missing_object_after_restart_and_retry(cx: &mut TestAppContext) {
    reopen_after_restart(cx, Unavailable::Object);
}

#[gpui::test]
fn closed_source_keeps_missing_file_after_restart_and_retry(cx: &mut TestAppContext) {
    reopen_after_restart(cx, Unavailable::File);
}
