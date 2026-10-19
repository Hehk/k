use super::*;
use crate::workspace::LoadedWorkspace;
use gpui::{AppContext, TestAppContext};
use kestrel::{query::QueryResult, storage::Store};

#[gpui::test]
fn refresh_keeps_snapshot_until_replacement_is_ready(cx: &mut TestAppContext) {
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
    let (window, tab, project) = workspace.update(cx, |workspace, cx| {
        workspace
            .edit(
                |state| {
                    let project = state.register_project(&info)?;
                    let window = state.new_window()?;
                    let tab = state.new_tab(window)?;
                    state.open_project(project, tab)?;
                    Ok((window, tab, state.project(project)?.clone()))
                },
                cx,
            )
            .unwrap()
    });
    let repository = cx.new(|cx| RepositoryModel::new(workspace, window, tab, project, cx));
    repository.read_with(cx, |repository, _| {
        assert!(repository.snapshot().is_loading());
    });
    cx.run_until_parked();
    let before = repository.read_with(cx, |repository, _| {
        let QueryResult::Success(snapshot) = repository.snapshot().result() else {
            panic!("initial snapshot did not load")
        };
        assert!(!repository.snapshot().is_fetching());
        snapshot.clone()
    });

    let tree = git
        .find_tree(git.treebuilder(None).unwrap().write().unwrap())
        .unwrap();
    let signature = git2::Signature::now("Test", "test@example.com").unwrap();
    let tip = git
        .commit(
            Some("refs/heads/topic"),
            &signature,
            &signature,
            "new branch",
            &tree,
            &[],
        )
        .unwrap();
    repository.update(cx, |repository, cx| {
        repository.refresh(cx);
        assert!(repository.snapshot().is_refreshing());
        assert_eq!(repository.snapshot().data(), Some(&before));
    });
    cx.run_until_parked();
    repository.read_with(cx, |repository, _| {
        let QueryResult::Success(snapshot) = repository.snapshot().result() else {
            panic!("replacement snapshot did not load")
        };
        assert_eq!(snapshot.branches.len(), 1);
        assert_eq!(snapshot.branches[0].name.0, b"refs/heads/topic");
        assert_eq!(snapshot.branches[0].tip.to_string(), tip.to_string());
    });

    let before_failure = repository.read_with(cx, |repository, _| {
        repository.snapshot().data().unwrap().clone()
    });
    std::fs::rename(dir.path().join("repo"), dir.path().join("moved")).unwrap();
    repository.update(cx, |repository, cx| repository.refresh(cx));
    cx.run_until_parked();
    repository.read_with(cx, |repository, _| {
        let QueryResult::Error {
            data: Some(snapshot),
            error,
        } = repository.snapshot().result()
        else {
            panic!("failed refresh did not retain its snapshot")
        };
        assert_eq!(snapshot, &before_failure);
        assert!(!error.is_empty());
    });

    std::fs::rename(dir.path().join("moved"), dir.path().join("repo")).unwrap();
    repository.update(cx, |repository, cx| {
        repository.refresh(cx);
        assert!(repository.snapshot().is_refreshing());
        assert_eq!(repository.snapshot().data(), Some(&before_failure));
        assert!(repository.snapshot().error().is_some());
        repository.refresh(cx);
        assert!(repository.snapshot().is_refreshing());
    });
    cx.run_until_parked();
    repository.read_with(cx, |repository, _| {
        let QueryResult::Success(snapshot) = repository.snapshot().result() else {
            panic!("retry did not recover")
        };
        assert!(!repository.snapshot().is_fetching());
        assert!(repository.snapshot().error().is_none());
        assert_eq!(snapshot, &before_failure);
    });
}
