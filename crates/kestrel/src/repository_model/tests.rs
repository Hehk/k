use super::*;
use crate::workspace::LoadedWorkspace;
use gpui::{AppContext, TestAppContext};
use kestrel::{query::QueryResult, storage::Store};

struct Fixture {
    dir: tempfile::TempDir,
    git: git2::Repository,
    workspace: Entity<Workspace>,
    window: WindowId,
    tab: TabId,
    project: Project,
}

impl Fixture {
    fn new(cx: &mut TestAppContext) -> Self {
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
        Self {
            dir,
            git,
            workspace,
            window,
            tab,
            project,
        }
    }

    fn model(&self, cx: &mut TestAppContext) -> Entity<RepositoryModel> {
        cx.new(|cx| {
            RepositoryModel::new(
                self.workspace.clone(),
                self.window,
                self.tab,
                self.project.clone(),
                cx,
            )
        })
    }

    fn commit(&self) -> git2::Oid {
        let tree = self
            .git
            .find_tree(self.git.treebuilder(None).unwrap().write().unwrap())
            .unwrap();
        let signature = git2::Signature::now("Test", "test@example.com").unwrap();
        self.git
            .commit(
                Some("refs/heads/topic"),
                &signature,
                &signature,
                "new branch",
                &tree,
                &[],
            )
            .unwrap()
    }
}

#[gpui::test]
fn refresh_keeps_snapshot_until_replacement_is_ready(cx: &mut TestAppContext) {
    let fixture = Fixture::new(cx);
    let repository = fixture.model(cx);
    repository.read_with(cx, |repository, cx| {
        assert!(repository.snapshot(cx).is_loading())
    });
    cx.run_until_parked();
    let before = repository.read_with(cx, |repository, cx| {
        assert!(!repository.snapshot(cx).is_fetching());
        repository.snapshot(cx).data().unwrap().clone()
    });

    let tip = fixture.commit();
    repository.update(cx, |repository, cx| {
        repository.refresh(cx);
        assert!(repository.snapshot(cx).is_refreshing());
        assert_eq!(repository.snapshot(cx).data(), Some(&before));
    });
    cx.run_until_parked();
    let before_failure = repository.read_with(cx, |repository, cx| {
        let QueryResult::Success(snapshot) = repository.snapshot(cx).result() else {
            panic!("replacement snapshot did not load")
        };
        assert_eq!(snapshot.branches.len(), 1);
        assert_eq!(snapshot.branches[0].name.0, b"refs/heads/topic");
        assert_eq!(snapshot.branches[0].tip.to_string(), tip.to_string());
        snapshot.clone()
    });

    std::fs::rename(
        fixture.dir.path().join("repo"),
        fixture.dir.path().join("moved"),
    )
    .unwrap();
    repository.update(cx, |repository, cx| repository.refresh(cx));
    cx.run_until_parked();
    repository.read_with(cx, |repository, cx| {
        assert_eq!(repository.snapshot(cx).data(), Some(&before_failure));
        assert!(repository.snapshot(cx).error().is_some());
        assert!(!repository.snapshot(cx).is_fetching());
    });

    std::fs::rename(
        fixture.dir.path().join("moved"),
        fixture.dir.path().join("repo"),
    )
    .unwrap();
    repository.update(cx, |repository, cx| {
        repository.refresh(cx);
        assert!(repository.snapshot(cx).is_refreshing());
        assert_eq!(repository.snapshot(cx).data(), Some(&before_failure));
        assert!(repository.snapshot(cx).error().is_some());
        repository.refresh(cx);
        assert!(repository.snapshot(cx).is_refreshing());
    });
    cx.run_until_parked();
    repository.read_with(cx, |repository, cx| {
        assert!(!repository.snapshot(cx).is_fetching());
        assert!(repository.snapshot(cx).error().is_none());
        assert_eq!(repository.snapshot(cx).data(), Some(&before_failure));
    });
}

#[gpui::test]
fn explicit_startup_wires_native_events_without_changing_pinned_state(cx: &mut TestAppContext) {
    for start_before_model in [false, true] {
        verify_native_refresh(cx, start_before_model);
    }
}

fn verify_native_refresh(cx: &mut TestAppContext, start_before_model: bool) {
    let fixture = Fixture::new(cx);
    let tip = fixture.commit();
    fixture
        .git
        .reference("refs/heads/main", tip, true, "fixture")
        .unwrap();
    let comparison = Repository::open(fixture.git.workdir().unwrap())
        .unwrap()
        .compare(&"refs/heads/topic".into(), &"refs/heads/main".into())
        .unwrap();
    fixture.workspace.update(cx, |workspace, cx| {
        workspace
            .edit(
                |state| {
                    state.select_branch(fixture.tab, "refs/heads/topic".into())?;
                    state.set_comparison(
                        fixture.tab,
                        kestrel::state::BaseSelection::Explicit("refs/heads/main".into()),
                        comparison,
                    )
                },
                cx,
            )
            .unwrap();
    });
    if start_before_model {
        fixture.workspace.update(cx, |workspace, cx| {
            workspace.start_watching_repositories(cx)
        });
    }
    let repository = fixture.model(cx);
    cx.run_until_parked();
    let snapshot = repository.read_with(cx, |repository, _| repository.snapshot.clone());
    fixture.workspace.update(cx, |workspace, cx| {
        workspace.start_watching_repositories(cx);
        workspace.start_watching_repositories(cx);
    });
    cx.run_until_parked();
    std::thread::sleep(std::time::Duration::from_millis(100));
    let before = fixture
        .workspace
        .read_with(cx, |workspace, _| workspace.state().clone());
    fixture
        .git
        .reference("refs/heads/added", tip, true, "fixture")
        .unwrap();
    fixture
        .git
        .find_reference("refs/heads/topic")
        .unwrap()
        .delete()
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        cx.run_until_parked();
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(20));
        let updated = snapshot.read_with(cx, |snapshot, _| {
            snapshot
                .query
                .data()
                .unwrap()
                .branches
                .iter()
                .any(|branch| branch.name.0 == b"refs/heads/added")
        });
        if updated {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "native event did not refresh the snapshot"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    snapshot.read_with(cx, |snapshot, _| {
        assert!(
            !snapshot
                .query
                .data()
                .unwrap()
                .branches
                .iter()
                .any(|branch| branch.name.0 == b"refs/heads/topic")
        );
        assert!(
            snapshot
                .query
                .data()
                .unwrap()
                .branches
                .iter()
                .any(|branch| branch.name.0 == b"refs/heads/added")
        );
    });
    assert_eq!(
        fixture
            .workspace
            .read_with(cx, |workspace, _| workspace.state().clone()),
        before
    );
}

#[gpui::test]
fn watcher_setup_failure_preserves_readable_snapshot_data(cx: &mut TestAppContext) {
    let fixture = Fixture::new(cx);
    let repository = fixture.model(cx);
    cx.run_until_parked();
    std::fs::rename(
        fixture.dir.path().join("repo"),
        fixture.dir.path().join("moved"),
    )
    .unwrap();
    fixture.workspace.update(cx, |workspace, cx| {
        workspace.start_watching_repositories(cx)
    });
    cx.run_until_parked();
    repository.read_with(cx, |repository, cx| {
        assert!(repository.watch_error(cx).is_some());
        assert!(repository.snapshot(cx).data().is_some());
        assert!(repository.snapshot(cx).error().is_none());
    });
}

#[gpui::test]
fn tabs_and_linked_worktrees_share_snapshot_without_retaining_it_after_close(
    cx: &mut TestAppContext,
) {
    let fixture = Fixture::new(cx);
    let tip = fixture.commit();
    fixture
        .git
        .reference("refs/heads/main", tip, true, "fixture")
        .unwrap();
    fixture.git.set_head("refs/heads/main").unwrap();
    let linked = fixture.dir.path().join("linked");
    fixture.git.worktree("linked", &linked, None).unwrap();
    let info = Repository::open(&linked).unwrap().info().unwrap();
    assert_eq!(info.common_dir, fixture.project.common_dir);
    let first = fixture.model(cx);
    let (window, tab) = fixture.workspace.update(cx, |workspace, cx| {
        workspace
            .edit(
                |state| {
                    let window = state.new_window()?;
                    let tab = state.new_tab(window)?;
                    state.open_project(fixture.project.id, tab)?;
                    Ok((window, tab))
                },
                cx,
            )
            .unwrap()
    });
    let mut project = fixture.project.clone();
    project.directory = info.directory;
    let second =
        cx.new(|cx| RepositoryModel::new(fixture.workspace.clone(), window, tab, project, cx));
    let shared = first.read_with(cx, |repository, _| repository.snapshot.downgrade());
    assert_eq!(
        shared.upgrade().unwrap(),
        second.read_with(cx, |repository, _| repository.snapshot.clone())
    );
    cx.run_until_parked();
    fixture
        .git
        .reference("refs/heads/added", tip, true, "fixture")
        .unwrap();
    first.update(cx, |repository, cx| repository.refresh(cx));
    second.read_with(cx, |repository, cx| {
        assert!(repository.snapshot(cx).is_refreshing())
    });
    cx.run_until_parked();
    second.read_with(cx, |repository, cx| {
        assert!(
            repository
                .snapshot(cx)
                .data()
                .unwrap()
                .branches
                .iter()
                .any(|branch| branch.name.0 == b"refs/heads/added")
        );
    });

    drop(first);
    cx.run_until_parked();
    assert!(shared.upgrade().is_some());
    drop(second);
    cx.update(|_| {});
    cx.run_until_parked();
    assert!(shared.upgrade().is_none());
    let reopened = fixture.model(cx);
    reopened.read_with(cx, |repository, cx| {
        assert!(repository.snapshot(cx).is_loading())
    });
}
