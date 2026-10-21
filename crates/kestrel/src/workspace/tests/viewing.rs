use super::*;
use kestrel::{
    review::{Comparison, Endpoints},
    state::{BaseSelection, Browsing},
};

fn setup(cx: &mut TestAppContext) -> (tempfile::TempDir, Entity<Workspace>, WindowId, TabId) {
    let dir = tempfile::tempdir().unwrap();
    let workspace = cx.new(|cx| {
        Workspace::new(
            LoadedWorkspace::load(Ok(Store::new(dir.path().join("state.json")))),
            cx,
        )
    });
    let (window, tab) = workspace.update(cx, |workspace, cx| {
        workspace
            .edit(
                |state| {
                    let project = project(state, "viewing-repository");
                    let window = state.new_window()?;
                    let tab = state.new_tab(window)?;
                    state.open_project(project, tab)?;
                    Ok((window, tab))
                },
                cx,
            )
            .unwrap()
    });
    (dir, workspace, window, tab)
}

#[gpui::test]
fn viewing_decisions_commit_atomically_and_are_not_in_the_workspace_snapshot(
    cx: &mut TestAppContext,
) {
    let (dir, workspace, window, tab) = setup(cx);
    workspace.update(cx, |workspace, cx| {
        let before = workspace.state().clone();
        let records = workspace.viewing_history().records().to_vec();
        assert_eq!(records.len(), 1);
        let revision = workspace.model.revision();
        assert!(
            workspace
                .edit(
                    |state| {
                        state.go_home(tab)?;
                        state.go_back(tab)?;
                        Err::<(), _>(StateError("rejected transaction"))
                    },
                    cx
                )
                .is_err()
        );
        assert_eq!(workspace.state(), &before);
        assert_eq!(workspace.viewing_history().records(), records);
        assert_eq!(workspace.model.revision(), revision);
        workspace
            .edit(|state| state.focus_window(window), cx)
            .unwrap();
        assert_eq!(workspace.viewing_history().records(), records);
        workspace.flush_on_quit();
    });
    let state = workspace.read_with(cx, |workspace, _| workspace.state().clone());
    let bytes = fs::read(dir.path().join("state.json")).unwrap();
    let encoded: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert!(encoded["state"].get("view_changes").is_none());
    let reloaded = cx.new(|cx| {
        Workspace::new(
            LoadedWorkspace::load(Ok(Store::new(dir.path().join("state.json")))),
            cx,
        )
    });
    reloaded.update(cx, |workspace, cx| {
        assert_eq!(workspace.state(), &state);
        let restored = workspace.viewing_history().records().to_vec();
        assert_eq!(restored.len(), 1);
        workspace
            .edit(|state| state.focus_window(window), cx)
            .unwrap();
        workspace.navigate(tab, Navigation::SelectTab, cx).unwrap();
        assert_eq!(workspace.viewing_history().records(), restored);
    });
}

#[gpui::test]
fn reopening_a_closed_window_records_only_its_selected_tab_after_commit(cx: &mut TestAppContext) {
    let (_dir, workspace, window, first) = setup(cx);
    workspace.update(cx, |workspace, cx| {
        workspace
            .edit(
                |state| {
                    let project = state.tab(first)?.route.project().unwrap();
                    let second = state.new_tab(window)?;
                    state.open_project(project, second)?;
                    state.select_tab(first)
                },
                cx,
            )
            .unwrap();
        assert_eq!(workspace.viewing_history().records().len(), 3);
        let selected = workspace.state().tab(first).unwrap().snapshot().unwrap();
        workspace.close_window(window, cx);
        assert_eq!(workspace.viewing_history().records().len(), 3);
        let _abandoned = workspace.reopen_state(None).unwrap().unwrap();
        assert_eq!(workspace.viewing_history().records().len(), 3);
        let (state, restored) = workspace.reopen_state(None).unwrap().unwrap();
        workspace.finish_reopen(state, cx).unwrap();
        assert_eq!(workspace.state().window(restored).unwrap().tabs.len(), 2);
        let records = workspace.viewing_history().records();
        assert_eq!(records.len(), 4);
        assert_eq!(records[3].viewed.snapshot, selected.snapshot);
    });
}

#[gpui::test]
fn a_saved_view_opens_in_the_requested_tab_or_a_new_tab_after_its_source_is_closed(
    cx: &mut TestAppContext,
) {
    let (_dir, workspace, window, source) = setup(cx);
    workspace.update(cx, |workspace, cx| {
        workspace
            .edit(
                |state| {
                    state.select_branch(source, "refs/heads/review".into())?;
                    let old = "1111111111111111111111111111111111111111".parse().unwrap();
                    state.set_comparison(
                        source,
                        BaseSelection::Default,
                        Comparison {
                            branch: "refs/heads/review".into(),
                            base: "refs/heads/main".into(),
                            base_tip: old,
                            commits: Endpoints {
                                old,
                                new: "2222222222222222222222222222222222222222".parse().unwrap(),
                            },
                        },
                    )?;
                    state.set_browsing(
                        source,
                        &"refs/heads/review".into(),
                        Browsing {
                            file: Some("old.rs".into()),
                            first_visible_line: 30,
                            ..Default::default()
                        },
                    )
                },
                cx,
            )
            .unwrap();
        let record = workspace
            .viewing_history()
            .records()
            .last()
            .unwrap()
            .clone();
        workspace.close_window(window, cx);
        assert!(workspace.state().tab(source).is_err());
        let (window, home) = workspace
            .edit(
                |state| {
                    let window = state.new_window()?;
                    Ok((window, state.new_tab(window)?))
                },
                cx,
            )
            .unwrap();
        workspace
            .open_view(record.id, OpenViewIn::NewTab(window), cx)
            .unwrap();
        let new = workspace
            .state()
            .window(window)
            .unwrap()
            .tabs
            .last()
            .unwrap()
            .id;
        assert_ne!(new, source);
        assert_eq!(
            workspace.state().window(window).unwrap().active_tab,
            Some(new)
        );
        assert_eq!(
            workspace
                .state()
                .tab(new)
                .unwrap()
                .snapshot()
                .unwrap()
                .snapshot,
            record.viewed.snapshot
        );
        assert_eq!(workspace.state().tab(home).unwrap().route, Route::Home);
        workspace.navigate(home, Navigation::SelectTab, cx).unwrap();
        workspace
            .open_view(record.id, OpenViewIn::Tab(home), cx)
            .unwrap();
        assert_eq!(
            workspace.state().window(window).unwrap().active_tab,
            Some(home)
        );
        assert_eq!(
            workspace
                .state()
                .tab(home)
                .unwrap()
                .snapshot()
                .unwrap()
                .snapshot,
            record.viewed.snapshot
        );
        assert_eq!(workspace.state().windows().len(), 1);
        assert_eq!(workspace.state().window(window).unwrap().tabs.len(), 2);
        let before = workspace.state().clone();
        let records = workspace.viewing_history().records().to_vec();
        assert!(
            workspace
                .open_view(ViewingId(u64::MAX), OpenViewIn::NewTab(window), cx)
                .is_err()
        );
        assert_eq!(workspace.state(), &before);
        assert_eq!(workspace.viewing_history().records(), records);
    });
}

#[gpui::test]
fn tab_bar_selection_records_but_reordering_and_automatic_close_selection_do_not(
    cx: &mut TestAppContext,
) {
    let (_dir, workspace, window, first) = setup(cx);
    workspace.update(cx, |workspace, cx| {
        let second = workspace
            .edit(
                |state| {
                    let project = state.tab(first)?.route.project().unwrap();
                    let second = state.new_tab(window)?;
                    state.open_project(project, second)?;
                    Ok(second)
                },
                cx,
            )
            .unwrap();
        assert_eq!(workspace.viewing_history().records().len(), 2);
        workspace
            .edit(
                |state| {
                    set_tab_layout(
                        state,
                        window,
                        &[first.as_u64(), second.as_u64()],
                        Some(first.as_u64()),
                    )
                },
                cx,
            )
            .unwrap();
        assert_eq!(workspace.viewing_history().records().len(), 3);
        workspace
            .edit(
                |state| {
                    set_tab_layout(
                        state,
                        window,
                        &[second.as_u64(), first.as_u64()],
                        Some(first.as_u64()),
                    )
                },
                cx,
            )
            .unwrap();
        assert_eq!(workspace.viewing_history().records().len(), 3);
        workspace
            .edit(
                |state| set_tab_layout(state, window, &[second.as_u64()], Some(second.as_u64())),
                cx,
            )
            .unwrap();
        assert_eq!(workspace.viewing_history().records().len(), 3);
    });
}
