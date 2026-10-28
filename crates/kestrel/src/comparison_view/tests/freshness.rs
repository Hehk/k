use super::*;

#[gpui::test]
fn older_cached_snapshot_notifications_do_not_undo_load_latest(cx: &mut TestAppContext) {
    let (dir, workspace, _, repository) = setup(cx);
    let git = git2::Repository::init(dir.path().join("repo")).unwrap();
    let first = super::retry::commit(&git, None);
    let second = super::retry::commit(&git, Some(first));
    for reference in ["refs/heads/main", "refs/heads/topic"] {
        git.reference(reference, first, true, "fixture").unwrap();
    }
    let (view, cx) = cx.add_window_view(|_, cx| ComparisonView::new(repository.clone(), cx));
    cx.run_until_parked();
    view.update(cx, |view, cx| {
        view.load_review(Some("refs/heads/main".into()), false, cx);
    });
    cx.run_until_parked();
    git.reference("refs/heads/topic", second, true, "fixture")
        .unwrap();
    view.update(cx, |view, cx| view.load_review(None, true, cx));
    cx.run_until_parked();
    let observed = repository.read_with(cx, |repository, cx| {
        let snapshot = repository.snapshot(cx).data().unwrap();
        assert_eq!(
            snapshot
                .branches
                .iter()
                .find(|branch| branch.name == RefName::from("refs/heads/topic"))
                .unwrap()
                .tip
                .to_string(),
            first.to_string()
        );
        repository.snapshot_read(cx)
    });
    let before = workspace.read_with(cx, |workspace, _| workspace.state().clone());
    repository.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();
    view.read_with(cx, |view, _| assert_eq!(view.freshness(), Some(true)));

    // A failed refresh retains the cache; its notification is not a new observation.
    drop(git);
    std::fs::rename(dir.path().join("repo"), dir.path().join("unavailable")).unwrap();
    repository.update(cx, |repository, cx| repository.refresh(cx));
    cx.run_until_parked();
    repository.read_with(cx, |repository, cx| {
        assert!(repository.snapshot(cx).error().is_some());
        assert_eq!(repository.snapshot_read(cx), observed);
    });
    view.read_with(cx, |view, _| {
        assert_eq!(view.freshness(), Some(true));
        assert_eq!(
            view.review
                .data()
                .unwrap()
                .comparison
                .commits
                .new
                .to_string(),
            second.to_string()
        );
    });
    workspace.read_with(cx, |workspace, _| assert_eq!(workspace.state(), &before));
}

#[gpui::test]
fn newer_snapshot_before_review_completion_marks_the_pins_stale(cx: &mut TestAppContext) {
    snapshot_and_review_completion(cx, true, false);
}

#[gpui::test]
fn newer_snapshot_after_review_completion_marks_the_pins_stale(cx: &mut TestAppContext) {
    snapshot_and_review_completion(cx, false, false);
}

#[gpui::test]
fn overlapping_ref_reads_reconcile_after_review_completion(cx: &mut TestAppContext) {
    snapshot_and_review_completion(cx, true, true);
}

fn snapshot_and_review_completion(
    cx: &mut TestAppContext,
    snapshot_first: bool,
    overlapping: bool,
) {
    let (dir, workspace, tab, repository) = setup(cx);
    let git = git2::Repository::init(dir.path().join("repo")).unwrap();
    let first = super::retry::commit(&git, None);
    let second = super::retry::commit(&git, Some(first));
    for reference in ["refs/heads/main", "refs/heads/topic"] {
        git.reference(reference, first, true, "fixture").unwrap();
    }
    let pinned = kestrel::git::Repository::open(git.workdir().unwrap())
        .unwrap()
        .compare(&"refs/heads/topic".into(), &"refs/heads/main".into())
        .unwrap();
    let (view, cx) = cx.add_window_view(|_, cx| ComparisonView::new(repository.clone(), cx));
    cx.run_until_parked();
    let (revision, key) = view.update(cx, |view, cx| {
        view.load_review(Some("refs/heads/main".into()), false, cx);
        view.review_task.take();
        (view.review_revision, view.current_key(cx).unwrap())
    });
    let mut loaded = data("refs/heads/main");
    loaded.comparison = pinned.clone();
    git.reference("refs/heads/topic", second, true, "fixture")
        .unwrap();
    if snapshot_first {
        repository.update(cx, |repository, cx| repository.refresh(cx));
        cx.run_until_parked();
    }
    if overlapping {
        // Model a freshness read paused across the snapshot's entire read interval.
        loaded.freshness_read.finished = std::time::Instant::now();
    }
    let freshness_read = loaded.freshness_read;
    view.update(cx, |view, cx| {
        view.finish_review(
            revision,
            key,
            BaseSelection::Explicit("refs/heads/main".into()),
            Ok(loaded),
            cx,
        );
        if snapshot_first && !overlapping {
            assert_eq!(view.freshness(), Some(false));
        }
    });
    if !snapshot_first {
        repository.update(cx, |repository, cx| repository.refresh(cx));
    }
    cx.run_until_parked();
    repository.read_with(cx, |repository, cx| {
        let snapshot = repository.snapshot(cx).data().unwrap();
        assert!(repository.snapshot_read(cx).unwrap().started > freshness_read.finished);
        assert_eq!(
            snapshot
                .branches
                .iter()
                .find(|branch| branch.name == pinned.branch)
                .unwrap()
                .tip
                .to_string(),
            second.to_string()
        );
    });
    view.read_with(cx, |view, _| {
        assert_eq!(view.freshness(), Some(false));
        assert_eq!(view.review.data().unwrap().comparison, pinned);
    });
    workspace.read_with(cx, |workspace, _| {
        assert_eq!(
            workspace
                .state()
                .tab(tab)
                .unwrap()
                .route
                .review()
                .unwrap()
                .active_review()
                .unwrap()
                .comparison
                .as_ref(),
            Some(&pinned)
        );
    });
}
