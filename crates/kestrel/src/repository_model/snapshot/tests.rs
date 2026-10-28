use crate::workspace::{LoadedWorkspace, Workspace};
use gpui::{AppContext, TestAppContext};
use kestrel::{git::Repository, storage::Store};

#[gpui::test]
fn recovered_snapshot_keeps_its_common_directory_watcher_and_retargets_pending_reads(
    cx: &mut TestAppContext,
) {
    let dir = tempfile::tempdir().unwrap();
    let git = git2::Repository::init(dir.path().join("repo")).unwrap();
    let tree = git
        .find_tree(git.treebuilder(None).unwrap().write().unwrap())
        .unwrap();
    let signature = git2::Signature::now("Test", "test@example.com").unwrap();
    let tip = git
        .commit(Some("HEAD"), &signature, &signature, "initial", &tree, &[])
        .unwrap();
    let linked_path = dir.path().join("linked");
    git.worktree("linked", &linked_path, None).unwrap();
    let linked = Repository::open(&linked_path).unwrap().info().unwrap();
    let main = Repository::open(git.workdir().unwrap())
        .unwrap()
        .info()
        .unwrap();
    let workspace = cx.new(|cx| {
        Workspace::new(
            LoadedWorkspace::load(Ok(Store::new(dir.path().join("state.json")))),
            cx,
        )
    });
    let (project, snapshot) = workspace.update(cx, |workspace, cx| {
        let project = workspace
            .edit(
                |state| {
                    let id = state.register_project(&linked)?;
                    let window = state.new_window()?;
                    let tab = state.new_tab(window)?;
                    state.open_project(id, tab)?;
                    Ok(state.project(id)?.clone())
                },
                cx,
            )
            .unwrap();
        workspace.start_watching_repositories(cx);
        let snapshot = workspace.repository_snapshot(&project, cx);
        (project, snapshot)
    });
    cx.run_until_parked();
    snapshot.read_with(cx, |snapshot, _| assert!(snapshot.watcher.is_watching(0)));
    std::fs::remove_dir_all(linked_path).unwrap();
    workspace.update(cx, |workspace, cx| {
        snapshot.update(cx, |snapshot, cx| snapshot.refresh(cx));
        workspace
            .edit(|state| state.recover_project(&project, &main), cx)
            .unwrap();
        assert_eq!(workspace.repository_snapshot(&project, cx), snapshot);
    });
    cx.run_until_parked();
    snapshot.read_with(cx, |snapshot, _| {
        assert_eq!(snapshot.project.directory, main.directory);
        assert!(snapshot.watcher.is_watching(0));
        assert!(snapshot.query.error().is_none());
        assert!(!snapshot.query.is_fetching());
    });

    git.reference("refs/heads/after-recovery", tip, true, "fixture")
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        cx.run_until_parked();
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(20));
        if snapshot.read_with(cx, |snapshot, _| {
            snapshot
                .query
                .data()
                .unwrap()
                .branches
                .iter()
                .any(|branch| branch.name.0 == b"refs/heads/after-recovery")
        }) {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "watcher did not refresh the recovered path"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

#[gpui::test]
fn cached_snapshots_only_watch_while_a_repository_route_is_open(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let git = git2::Repository::init(dir.path().join("repo")).unwrap();
    let info = Repository::open(git.workdir().unwrap())
        .unwrap()
        .info()
        .unwrap();
    let workspace = cx.new(|cx| {
        Workspace::new(
            LoadedWorkspace::load(Ok(Store::new(dir.path().join("state.json")))),
            cx,
        )
    });
    let (project, first, second) = workspace.update(cx, |workspace, cx| {
        workspace
            .edit(
                |state| {
                    let id = state.register_project(&info)?;
                    let window = state.new_window()?;
                    let first = state.new_tab(window)?;
                    state.open_project(id, first)?;
                    let second = state.new_tab(window)?;
                    state.open_project(id, second)?;
                    Ok((state.project(id)?.clone(), first, second))
                },
                cx,
            )
            .unwrap()
    });
    workspace.update(cx, |workspace, cx| {
        workspace.start_watching_repositories(cx)
    });
    let snapshot = workspace.update(cx, |workspace, cx| {
        workspace.repository_snapshot(&project, cx)
    });
    cx.run_until_parked();
    snapshot.read_with(cx, |snapshot, _| {
        assert!(snapshot.monitor.is_some());
        assert!(snapshot.query.data().is_some());
    });

    workspace.update(cx, |workspace, cx| {
        workspace
            .edit(
                |state| {
                    state.close_tab(first);
                    Ok(())
                },
                cx,
            )
            .unwrap();
    });
    snapshot.read_with(cx, |snapshot, _| assert!(snapshot.monitor.is_some()));
    workspace.update(cx, |workspace, cx| {
        workspace
            .edit(
                |state| {
                    state.close_tab(second);
                    Ok(())
                },
                cx,
            )
            .unwrap();
    });
    cx.run_until_parked();
    snapshot.read_with(cx, |snapshot, _| {
        assert!(snapshot.monitor.is_none());
        assert!(snapshot.query.data().is_some());
    });

    workspace.update(cx, |workspace, cx| workspace.refresh_repositories(cx));
    cx.run_until_parked();
    snapshot.read_with(cx, |snapshot, _| {
        assert!(snapshot.monitor.is_none());
        assert!(snapshot.query.data().is_some());
        assert!(!snapshot.query.is_fetching());
    });

    workspace.update(cx, |workspace, cx| {
        workspace
            .edit(
                |state| {
                    let window = state.new_window()?;
                    let tab = state.new_tab(window)?;
                    state.open_project(project.id, tab)
                },
                cx,
            )
            .unwrap();
        assert_eq!(workspace.repository_snapshot(&project, cx), snapshot);
    });
    cx.run_until_parked();
    snapshot.read_with(cx, |snapshot, _| {
        assert!(snapshot.monitor.is_some());
        assert!(snapshot.query.data().is_some());
    });
    cx.quit();
    snapshot.read_with(cx, |snapshot, _| assert!(snapshot.monitor.is_none()));
}
