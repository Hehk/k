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
