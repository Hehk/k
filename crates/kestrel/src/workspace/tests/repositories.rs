use super::*;
use crate::repository_model::RepositorySnapshot;
use kestrel::git::Repository;

struct Fixture {
    dir: tempfile::TempDir,
    git: git2::Repository,
    workspace: Entity<Workspace>,
    snapshot: Entity<RepositorySnapshot>,
    tip: git2::Oid,
}

impl Fixture {
    fn new(cx: &mut TestAppContext) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let git = git2::Repository::init(dir.path().join("repo")).unwrap();
        let tree = git
            .find_tree(git.treebuilder(None).unwrap().write().unwrap())
            .unwrap();
        let signature = git2::Signature::now("Test", "test@example.com").unwrap();
        let tip = git
            .commit(
                Some("refs/heads/main"),
                &signature,
                &signature,
                "fixture",
                &tree,
                &[],
            )
            .unwrap();
        drop(tree);
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
        let snapshot = workspace.update(cx, |workspace, cx| {
            let project = workspace
                .edit(
                    |state| {
                        let id = state.register_project(&info)?;
                        Ok(state.project(id)?.clone())
                    },
                    cx,
                )
                .unwrap();
            workspace.start_watching_repositories(cx);
            workspace.repository_snapshot(&project, cx)
        });
        cx.run_until_parked();
        Self {
            dir,
            git,
            workspace,
            snapshot,
            tip,
        }
    }

    fn add_branch(&self, name: &str) {
        self.git
            .reference(&format!("refs/heads/{name}"), self.tip, true, "fixture")
            .unwrap();
    }

    fn has_branch(&self, name: &str, cx: &mut TestAppContext) -> bool {
        let reference = format!("refs/heads/{name}");
        self.snapshot.read_with(cx, |snapshot, _| {
            snapshot
                .query()
                .data()
                .unwrap()
                .branches
                .iter()
                .any(|branch| branch.name.0 == reference.as_bytes())
        })
    }
}

fn advance(cx: &mut TestAppContext, seconds: u64) {
    cx.executor().advance_clock(Duration::from_secs(seconds));
    cx.run_until_parked();
}

#[gpui::test]
fn inactive_repositories_refresh_every_two_minutes_without_duplicate_pollers(
    cx: &mut TestAppContext,
) {
    let fixture = Fixture::new(cx);
    assert!(
        fixture
            .workspace
            .read_with(cx, |workspace, _| workspace.state().windows().is_empty())
    );
    advance(cx, 60);
    fixture.workspace.update(cx, |workspace, cx| {
        workspace.start_watching_repositories(cx)
    });
    fixture.add_branch("first");
    advance(cx, 59);
    assert!(!fixture.has_branch("first", cx));
    advance(cx, 1);
    assert!(fixture.has_branch("first", cx));

    fixture.add_branch("second");
    advance(cx, 60);
    assert!(!fixture.has_branch("second", cx));
    advance(cx, 60);
    assert!(fixture.has_branch("second", cx));
}

#[gpui::test]
fn a_poll_discovers_new_repositories_and_isolates_unavailable_ones(cx: &mut TestAppContext) {
    let fixture = Fixture::new(cx);
    let git = git2::Repository::init(fixture.dir.path().join("later")).unwrap();
    let info = Repository::open(git.workdir().unwrap())
        .unwrap()
        .info()
        .unwrap();
    let missing_directory = fixture.dir.path().canonicalize().unwrap().join("missing");
    let missing = RepositoryInfo {
        common_dir: missing_directory.join(".git"),
        directory: missing_directory,
        current_branch: None,
    };
    fixture.workspace.update(cx, |workspace, cx| {
        workspace
            .edit(
                |state| {
                    state.register_project(&info)?;
                    state.register_project(&missing)
                },
                cx,
            )
            .unwrap();
    });
    advance(cx, 119);
    assert!(!fixture.workspace.read_with(cx, |workspace, _| {
        workspace
            .repository_snapshots
            .contains_key(&info.common_dir)
    }));
    advance(cx, 1);
    fixture.workspace.read_with(cx, |workspace, cx| {
        let snapshot = workspace
            .repository_snapshots
            .get(&info.common_dir)
            .unwrap()
            .read(cx);
        assert!(snapshot.query().data().is_some());
        assert!(snapshot.query().error().is_none());
        let missing = workspace
            .repository_snapshots
            .get(&missing.common_dir)
            .unwrap()
            .read(cx);
        assert!(missing.query().error().is_some());
        assert!(!missing.query().is_fetching());
    });
}

#[gpui::test]
fn quit_cancels_polling_and_prevents_restart(cx: &mut TestAppContext) {
    let fixture = Fixture::new(cx);
    fixture.add_branch("after-quit");
    cx.quit();
    fixture.workspace.update(cx, |workspace, cx| {
        assert!(workspace.repository_poll_task.is_none());
        workspace.start_watching_repositories(cx);
        assert!(workspace.repository_poll_task.is_none());
    });
    advance(cx, 240);
    assert!(!fixture.has_branch("after-quit", cx));
}

#[gpui::test]
fn polling_does_not_keep_a_released_workspace_alive(cx: &mut TestAppContext) {
    let fixture = Fixture::new(cx);
    let weak = fixture.workspace.downgrade();
    fixture.add_branch("after-release");
    drop(fixture.workspace);
    cx.update(|_| {});
    cx.run_until_parked();
    assert!(weak.upgrade().is_none());
    advance(cx, 240);
    fixture.snapshot.read_with(cx, |snapshot, _| {
        assert!(
            !snapshot
                .query()
                .data()
                .unwrap()
                .branches
                .iter()
                .any(|branch| branch.name.0 == b"refs/heads/after-release")
        );
    });
}
