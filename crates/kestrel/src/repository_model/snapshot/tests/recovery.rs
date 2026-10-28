use super::super::*;
use crate::{
    repository_model::RepositoryModel,
    repository_view::RepositoryView,
    workspace::{LoadedWorkspace, Workspace},
};
use gpui::{AppContext, Entity, Modifiers, TestAppContext};
use kestrel::{git::Repository, state::TabId, storage::Store};

struct Fixture {
    dir: tempfile::TempDir,
    git: git2::Repository,
    workspace: Entity<Workspace>,
    repository: Entity<RepositoryModel>,
    snapshot: Entity<RepositorySnapshot>,
    window: kestrel::state::WindowId,
    tab: TabId,
    project: Project,
    tip: git2::Oid,
}

impl Fixture {
    fn failed(cx: &mut TestAppContext) -> Self {
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
        let (window, tab, project, snapshot) = workspace.update(cx, |workspace, cx| {
            let (window, tab, project) = workspace
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
                .unwrap();
            let snapshot = workspace.repository_snapshot(&project, cx);
            (window, tab, project, snapshot)
        });
        let repository =
            cx.new(|cx| RepositoryModel::new(workspace.clone(), window, tab, project.clone(), cx));
        cx.run_until_parked();
        std::fs::rename(dir.path().join("repo"), dir.path().join("moved")).unwrap();
        workspace.update(cx, |workspace, cx| {
            workspace.start_watching_repositories(cx)
        });
        cx.run_until_parked();
        snapshot.read_with(cx, |snapshot, _| {
            assert!(snapshot.watch_error().is_some());
            assert!(snapshot.monitor.is_none());
            assert!(snapshot.query.data().is_some());
        });
        Self {
            dir,
            git,
            workspace,
            repository,
            snapshot,
            window,
            tab,
            project,
            tip,
        }
    }

    fn restore(&self) {
        std::fs::rename(self.dir.path().join("moved"), self.dir.path().join("repo")).unwrap();
    }

    fn advance(&self, seconds: u64, cx: &mut TestAppContext) {
        cx.executor().advance_clock(Duration::from_secs(seconds));
        cx.run_until_parked();
    }

    fn assert_watching(&self, generation: u64, cx: &mut TestAppContext) {
        self.snapshot.read_with(cx, |snapshot, _| {
            assert!(snapshot.watcher.is_watching(generation));
            assert!(snapshot.watch_error().is_none());
            assert!(snapshot.query.error().is_none());
        });
    }

    fn assert_native_events_resume(&self, cx: &mut TestAppContext) {
        self.git
            .reference("refs/heads/after-retry", self.tip, true, "fixture")
            .unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        loop {
            cx.run_until_parked();
            cx.executor().advance_clock(Duration::from_millis(20));
            if self.snapshot.read_with(cx, |snapshot, _| {
                snapshot
                    .query
                    .data()
                    .unwrap()
                    .branches
                    .iter()
                    .any(|branch| branch.name.0 == b"refs/heads/after-retry")
            }) {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "reattached watcher did not deliver events"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

#[gpui::test]
fn poll_retries_failed_watches_at_two_minute_intervals_without_closing_routes(
    cx: &mut TestAppContext,
) {
    let fixture = Fixture::failed(cx);
    let before = fixture
        .workspace
        .read_with(cx, |workspace, _| workspace.state().clone());
    fixture.advance(120, cx); // Generation 1 also fails while the path is absent.
    fixture.snapshot.read_with(cx, |snapshot, _| {
        assert!(snapshot.watch_error().is_some());
        assert!(snapshot.monitor.is_none());
        assert!(snapshot.query.data().is_some());
    });
    fixture.restore();
    // Ordinary workspace edits synchronize routes, but must not defeat the retry interval.
    fixture.workspace.update(cx, |workspace, cx| {
        workspace
            .edit(
                |state| {
                    state.set_branch_list(
                        fixture.tab,
                        kestrel::state::BranchList {
                            query: "main".into(),
                            ..Default::default()
                        },
                    )
                },
                cx,
            )
            .unwrap();
    });
    fixture.advance(119, cx);
    fixture.snapshot.read_with(cx, |snapshot, _| {
        assert!(snapshot.watch_error().is_some());
        assert!(snapshot.monitor.is_none());
    });
    fixture.advance(1, cx);
    fixture.assert_watching(2, cx);
    fixture.assert_native_events_resume(cx);
    fixture.workspace.read_with(cx, |workspace, _| {
        let mut expected = before;
        expected
            .set_branch_list(
                fixture.tab,
                kestrel::state::BranchList {
                    query: "main".into(),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(workspace.state(), &expected);
    });
}

#[gpui::test]
fn focus_retries_a_failed_watch_without_waiting_for_the_poll(cx: &mut TestAppContext) {
    let fixture = Fixture::failed(cx);
    fixture.restore();
    fixture.workspace.update(cx, |workspace, cx| {
        workspace.window_activation(fixture.window, true, cx)
    });
    cx.run_until_parked();
    fixture.assert_watching(1, cx);
    fixture.assert_native_events_resume(cx);
}

#[gpui::test]
fn runtime_failure_rereads_git_without_immediately_retrying_the_watcher(cx: &mut TestAppContext) {
    let fixture = Fixture::failed(cx);
    fixture.restore();
    fixture
        .repository
        .update(cx, |repository, cx| repository.refresh(cx));
    cx.run_until_parked();
    fixture.assert_watching(1, cx);
    fixture.snapshot.update(cx, |snapshot, cx| {
        snapshot.dispatch_watch(
            watch_lifecycle::Input::Failed {
                generation: 1,
                error: "runtime failure".into(),
            },
            cx,
        );
        assert!(snapshot.monitor.is_none());
        assert!(snapshot.query.is_fetching());
    });
    cx.run_until_parked();
    fixture.snapshot.read_with(cx, |snapshot, _| {
        assert_eq!(snapshot.watch_error(), Some("runtime failure"));
        assert!(snapshot.query.error().is_none());
        assert!(snapshot.monitor.is_none());
    });
    fixture.repository.update(cx, |repository, cx| {
        repository.refresh(cx);
        assert!(
            repository
                .watch_notice(cx)
                .unwrap()
                .contains("Refresh local refs also retries now")
        );
    });
    cx.run_until_parked();
    fixture.assert_watching(2, cx);
}

fn cancel_retry(cx: &mut TestAppContext, quit: bool) {
    let fixture = Fixture::failed(cx);
    fixture.restore();
    fixture
        .repository
        .update(cx, |repository, cx| repository.refresh(cx));
    fixture.snapshot.read_with(cx, |snapshot, _| {
        assert!(snapshot.monitor.is_some());
        assert!(snapshot.watch_error().is_some());
    });
    if quit {
        cx.quit();
    } else {
        fixture.workspace.update(cx, |workspace, cx| {
            workspace
                .edit(
                    |state| {
                        state.close_tab(fixture.tab);
                        Ok(())
                    },
                    cx,
                )
                .unwrap()
        });
    }
    fixture.advance(240, cx);
    fixture.snapshot.read_with(cx, |snapshot, _| {
        assert!(snapshot.monitor.is_none());
        assert!(snapshot.watch_error().is_none());
        assert!(!snapshot.watcher.is_watching(1));
    });
    // A retained model or late recovery request must not restart a released watcher.
    fixture
        .repository
        .update(cx, |repository, cx| repository.refresh(cx));
    cx.run_until_parked();
    fixture
        .snapshot
        .read_with(cx, |snapshot, _| assert!(snapshot.monitor.is_none()));
}

#[gpui::test]
fn closing_the_last_route_cancels_an_inflight_retry(cx: &mut TestAppContext) {
    cancel_retry(cx, false);
}

#[gpui::test]
fn quitting_cancels_an_inflight_retry(cx: &mut TestAppContext) {
    cancel_retry(cx, true);
}

#[gpui::test]
fn both_repository_pages_offer_manual_watch_recovery(cx: &mut TestAppContext) {
    let fixture = Fixture::failed(cx);
    let (_, visual) = cx.add_window_view(|_, cx| {
        RepositoryView::new(
            fixture.workspace.clone(),
            fixture.window,
            fixture.tab,
            fixture.project.clone(),
            cx,
        )
    });
    visual.run_until_parked();
    for selector in ["refresh-branches", "refresh-review-refs"] {
        if selector == "refresh-review-refs" {
            fixture.workspace.update(visual, |workspace, cx| {
                workspace
                    .edit(
                        |state| state.select_branch(fixture.tab, "refs/heads/main".into()),
                        cx,
                    )
                    .unwrap()
            });
        }
        visual.run_until_parked();
        visual.update(|window, cx| window.draw(cx).clear());
        assert!(visual.debug_bounds("watch-recovery-notice").is_some());
        let refresh = visual.debug_bounds(selector).unwrap();
        visual.simulate_click(refresh.center(), Modifiers::none());
        visual.run_until_parked();
        fixture.repository.read_with(visual, |repository, cx| {
            assert!(
                repository
                    .watch_notice(cx)
                    .unwrap()
                    .contains("Retrying every two minutes")
            );
            assert!(repository.snapshot(cx).error().is_some());
            assert!(repository.snapshot(cx).data().is_some());
            assert!(!repository.snapshot(cx).is_fetching());
        });
    }
    fixture.restore();
    visual.update(|window, cx| window.draw(cx).clear());
    let refresh = visual.debug_bounds("refresh-review-refs").unwrap();
    visual.simulate_click(refresh.center(), Modifiers::none());
    visual.run_until_parked();
    visual.update(|window, cx| window.draw(cx).clear());
    fixture.repository.read_with(visual, |repository, cx| {
        assert!(repository.watch_notice(cx).is_none());
        assert!(repository.snapshot(cx).error().is_none());
        assert!(repository.snapshot(cx).data().is_some());
    });
    // GPUI retains debug bounds across frames; use a fresh surface to check absence.
    let (_, recovered) = visual.add_window_view(|_, cx| {
        RepositoryView::new(
            fixture.workspace.clone(),
            fixture.window,
            fixture.tab,
            fixture.project.clone(),
            cx,
        )
    });
    recovered.run_until_parked();
    recovered.update(|window, cx| window.draw(cx).clear());
    assert!(recovered.debug_bounds("refresh-review-refs").is_some());
    assert!(recovered.debug_bounds("watch-recovery-notice").is_none());
}
