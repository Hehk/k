use crate::workspace::{LoadedWorkspace, Workspace};
use gpui::{AppContext, TestAppContext};
use kestrel::{git::Repository, storage::Store};

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
