use super::*;
use crate::repository_model::{self, RepositoryModel};

#[gpui::test]
fn reopening_surviving_worktree_recovers_cached_readers_and_restart(cx: &mut TestAppContext) {
    let fixture = Fixture::new();
    let linked_path = fixture.dir.path().join("linked");
    fixture.git.worktree("linked", &linked_path, None).unwrap();
    let linked = Repository::open(&linked_path).unwrap().info().unwrap();
    let main = Repository::open(fixture.git.workdir().unwrap())
        .unwrap()
        .info()
        .unwrap();
    let state_path = fixture.dir.path().join("state.json");
    let workspace = shared(cx, state_path.clone());
    let first = cx.update(|cx| Kestrel::new_window(workspace.clone(), cx).unwrap());
    first
        .update(cx, |root, window, cx| {
            window.activate_window();
            open(root, linked.clone(), cx);
        })
        .unwrap();
    cx.run_until_parked();
    let (project, window, tab) = workspace.read_with(cx, |workspace, _| {
        let window = &workspace.state().windows()[0];
        (
            workspace.state().projects()[0].clone(),
            window.id,
            window.active_tab.unwrap(),
        )
    });
    let reader =
        cx.new(|cx| RepositoryModel::new(workspace.clone(), window, tab, project.clone(), cx));
    let snapshot = workspace.update(cx, |workspace, cx| {
        workspace.repository_snapshot(&project, cx)
    });
    let second = cx.update(|cx| Kestrel::new_window(workspace.clone(), cx).unwrap());
    second
        .update(cx, |root, _, cx| open(root, linked.clone(), cx))
        .unwrap();
    let second_tab = active(&workspace, 1, cx);
    let comparison = Repository::open(&linked.directory)
        .unwrap()
        .compare(&"refs/heads/topic".into(), &"refs/heads/main".into())
        .unwrap();
    workspace.update(cx, |workspace, cx| {
        workspace
            .edit(
                |state| {
                    state.select_branch(second_tab, "refs/heads/topic".into())?;
                    state.set_comparison(
                        second_tab,
                        kestrel::state::BaseSelection::Explicit("refs/heads/main".into()),
                        comparison.clone(),
                    )
                },
                cx,
            )
            .unwrap();
    });
    cx.run_until_parked();
    let pinned = workspace.read_with(cx, |workspace, _| {
        workspace.state().tab(second_tab).unwrap().clone()
    });
    let old_view = second
        .read_with(cx, |root, _| root.repository.data().unwrap().clone())
        .unwrap();
    let historical = workspace.read_with(cx, |workspace, _| {
        workspace.viewing_history().records()[0].viewed.clone()
    });

    std::fs::remove_dir_all(&linked_path).unwrap();
    reader.update(cx, |reader, cx| reader.refresh(cx));
    cx.run_until_parked();
    snapshot.read_with(cx, |snapshot, _| {
        assert!(snapshot.query().error().is_some())
    });
    first
        .update(cx, |root, _, cx| open(root, main.clone(), cx))
        .unwrap();
    cx.run_until_parked();

    workspace.update(cx, |workspace, cx| {
        assert_eq!(workspace.state().projects().len(), 1);
        assert_eq!(
            workspace.state().project(project.id).unwrap().directory,
            main.directory
        );
        assert_eq!(
            workspace.state().project(project.id).unwrap().common_dir,
            linked.common_dir
        );
        assert_eq!(workspace.state().tab(second_tab).unwrap(), &pinned);
        assert_eq!(workspace.repository_snapshot(&project, cx), snapshot);
        workspace
            .edit(
                |state| {
                    historical.snapshot_for(state)?;
                    Ok(())
                },
                cx,
            )
            .unwrap();
        assert_eq!(
            workspace.state().project(project.id).unwrap().directory,
            main.directory
        );
    });
    reader.read_with(cx, |reader, cx| {
        assert_eq!(reader.project(cx).directory, main.directory);
        assert!(
            repository_model::open(reader.project(cx))
                .unwrap()
                .changes(&comparison.commits)
                .is_ok()
        );
    });
    snapshot.read_with(cx, |snapshot, _| {
        assert!(snapshot.query().error().is_none());
        assert!(snapshot.query().data().is_some());
        assert!(!snapshot.query().is_fetching());
    });
    second
        .read_with(cx, |root, cx| {
            let view = root.repository.data().unwrap();
            assert_ne!(view, &old_view);
            assert!(view.read(cx).comparison_freshness(cx).is_some());
        })
        .unwrap();

    cx.executor()
        .advance_clock(std::time::Duration::from_secs(1));
    cx.run_until_parked();
    let restored = shared(cx, state_path);
    restored.read_with(cx, |restored, _| {
        assert_eq!(
            restored.state().project(project.id).unwrap().directory,
            main.directory
        );
        assert_eq!(restored.state().tab(second_tab).unwrap(), &pinned);
    });
    let restored_window = restored.read_with(cx, |workspace, _| workspace.state().windows()[1].id);
    let restored_handle = cx.update(|cx| Kestrel::open(restored, restored_window, cx).unwrap());
    cx.run_until_parked();
    restored_handle
        .read_with(cx, |root, cx| {
            assert!(
                root.repository
                    .data()
                    .unwrap()
                    .read(cx)
                    .comparison_freshness(cx)
                    .is_some()
            );
        })
        .unwrap();
}

#[gpui::test]
fn restored_missing_worktree_recovers_but_cancelled_open_does_not(cx: &mut TestAppContext) {
    let fixture = Fixture::new();
    let linked_path = fixture.dir.path().join("linked");
    fixture.git.worktree("linked", &linked_path, None).unwrap();
    let linked = Repository::open(&linked_path).unwrap().info().unwrap();
    let main = Repository::open(fixture.git.workdir().unwrap())
        .unwrap()
        .info()
        .unwrap();
    let state_path = fixture.dir.path().join("state.json");
    let mut store = Store::new(state_path.clone());
    let mut state = store.load().unwrap().state;
    let project = state.register_project(&linked).unwrap();
    let window = state.new_window().unwrap();
    let tab = state.new_tab(window).unwrap();
    state.open_project(project, tab).unwrap();
    store.save(&state).unwrap();
    std::fs::remove_dir_all(linked_path).unwrap();

    let workspace = shared(cx, state_path);
    let handle = cx.update(|cx| Kestrel::open(workspace.clone(), window, cx).unwrap());
    cx.run_until_parked();
    handle
        .read_with(cx, |root, _| assert!(root.repository.error().is_some()))
        .unwrap();
    handle
        .update(cx, |root, _, cx| {
            open(root, main.clone(), cx);
            root.workspace.update(cx, |workspace, cx| {
                workspace.navigate(tab, Navigation::Home, cx).unwrap();
            });
        })
        .unwrap();
    cx.run_until_parked();
    workspace.read_with(cx, |workspace, _| {
        assert_eq!(
            workspace.state().project(project).unwrap().directory,
            linked.directory
        );
        assert_eq!(workspace.state().tab(tab).unwrap().route, Route::Home);
    });

    handle
        .update(cx, |root, _, cx| open(root, main.clone(), cx))
        .unwrap();
    cx.run_until_parked();
    workspace.read_with(cx, |workspace, _| {
        assert_eq!(workspace.state().projects().len(), 1);
        assert_eq!(
            workspace.state().project(project).unwrap().directory,
            main.directory
        );
    });
    handle
        .read_with(cx, |root, _| {
            assert!(root.repository.error().is_none());
            assert!(root.repository.data().is_some());
        })
        .unwrap();
}

#[gpui::test]
fn reopening_another_worktree_does_not_retarget_a_healthy_project(cx: &mut TestAppContext) {
    let fixture = Fixture::new();
    let linked_path = fixture.dir.path().join("linked");
    fixture.git.worktree("linked", &linked_path, None).unwrap();
    let linked = Repository::open(linked_path).unwrap().info().unwrap();
    let (workspace, handle) = fixture.start(cx);
    let before = workspace.read_with(cx, |workspace, _| workspace.state().projects()[0].clone());
    handle
        .update(cx, |root, _, cx| open(root, linked, cx))
        .unwrap();
    cx.run_until_parked();
    workspace.read_with(cx, |workspace, _| {
        assert_eq!(workspace.state().projects()[0], before)
    });
}
