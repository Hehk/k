use super::*;
use gpui::{Modifiers, VisualTestContext};

fn redraw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear());
}

fn click(cx: &mut VisualTestContext, selector: &'static str) {
    let bounds = cx.debug_bounds(selector).expect("comparison control");
    cx.simulate_click(bounds.center(), Modifiers::none());
    redraw(cx);
}

pub(super) fn commit(git: &git2::Repository, parent: Option<git2::Oid>) -> git2::Oid {
    let tree = git
        .find_tree(git.treebuilder(None).unwrap().write().unwrap())
        .unwrap();
    let signature = git2::Signature::now("Test", "test@example.invalid").unwrap();
    let parent = parent.map(|id| git.find_commit(id).unwrap());
    git.commit(
        None,
        &signature,
        &signature,
        "fixture",
        &tree,
        &parent.iter().collect::<Vec<_>>(),
    )
    .unwrap()
}

#[gpui::test]
fn retry_repeats_initial_explicit_base_without_a_default(cx: &mut TestAppContext) {
    let (dir, workspace, tab, repository) = setup(cx);
    let git = git2::Repository::init(dir.path().join("repo")).unwrap();
    let head = commit(&git, None);
    for reference in ["refs/heads/topic", "refs/heads/base"] {
        git.reference(reference, head, true, "fixture").unwrap();
    }
    let (view, cx) = cx.add_window_view(|_, cx| ComparisonView::new(repository.clone(), cx));
    redraw(cx);
    repository.read_with(cx, |repository, cx| {
        assert!(
            repository
                .snapshot(cx)
                .data()
                .unwrap()
                .default_base
                .is_none()
        );
    });
    let (before, records) = workspace.read_with(cx, |workspace, _| {
        (
            workspace.state().clone(),
            workspace.viewing_history().records().to_vec(),
        )
    });
    click(cx, "choose-base");
    git.find_reference("refs/heads/base")
        .unwrap()
        .delete()
        .unwrap();
    click(cx, "base:refs/heads/base");
    view.read_with(cx, |view, cx| {
        assert!(view.review.error().is_some());
        assert_eq!(
            view.review_request.as_ref().unwrap().retry_label(),
            "Retry comparison"
        );
        assert_eq!(view.current_key(cx).unwrap().base, BaseSelection::Default);
        assert!(view.current_key(cx).unwrap().comparison.is_none());
    });
    workspace.read_with(cx, |workspace, _| {
        assert_eq!(workspace.state(), &before);
        assert_eq!(workspace.viewing_history().records(), records);
    });

    // A second failure must retain the request too, rather than returning to the chooser.
    click(cx, "retry-review");
    view.read_with(cx, |view, _| assert!(view.review.error().is_some()));
    workspace.read_with(cx, |workspace, _| {
        assert_eq!(workspace.state(), &before);
        assert_eq!(workspace.viewing_history().records(), records);
    });

    git.reference("refs/heads/base", head, true, "repair fixture")
        .unwrap();
    click(cx, "retry-review");
    view.read_with(cx, |view, cx| {
        assert!(view.review.data().is_some());
        assert!(view.review_request.is_none());
        let key = view.current_key(cx).unwrap();
        assert_eq!(key.base, BaseSelection::Explicit("refs/heads/base".into()));
        let comparison = key.comparison.unwrap();
        assert_eq!(comparison.base, RefName::from("refs/heads/base"));
        assert_eq!(comparison.commits.new.to_string(), head.to_string());
    });
    workspace.read_with(cx, |workspace, _| {
        assert_eq!(
            workspace.state().tab(tab).unwrap().visit_id(),
            before.tab(tab).unwrap().visit_id()
        );
        let after = workspace.viewing_history().records();
        assert_eq!(after.len(), records.len());
        assert_ne!(
            after, records,
            "successful resolution amends the pending viewing"
        );
    });
}

#[gpui::test]
fn retries_preserve_pinned_reload_base_change_and_latest_intents(cx: &mut TestAppContext) {
    let (dir, workspace, tab, repository) = setup(cx);
    let git = git2::Repository::init(dir.path().join("repo")).unwrap();
    let head = commit(&git, None);
    for reference in ["refs/heads/main", "refs/heads/topic"] {
        git.reference(reference, head, true, "fixture").unwrap();
    }
    let (view, cx) = cx.add_window_view(|_, cx| ComparisonView::new(repository, cx));
    view.update(cx, |view, cx| {
        view.load_review(Some("refs/heads/main".into()), false, cx)
    });
    redraw(cx);
    let newer = commit(&git, Some(head));
    git.reference("refs/heads/topic", newer, true, "advance fixture")
        .unwrap();

    for (override_base, latest, label, expected_base, expected_head) in [
        (
            None,
            false,
            "Retry pinned comparison",
            "refs/heads/main",
            head,
        ),
        (
            Some("refs/heads/other"),
            false,
            "Retry base change",
            "refs/heads/other",
            head,
        ),
        (None, true, "Retry load latest", "refs/heads/other", newer),
    ] {
        let (before, records) = workspace.read_with(cx, |workspace, _| {
            (
                workspace.state().clone(),
                workspace.viewing_history().records().to_vec(),
            )
        });
        view.update(cx, |view, cx| {
            view.load_review(override_base.map(RefName::from), latest, cx);
            view.review_task.take();
            let request = view.review_request.as_ref().unwrap().clone();
            view.finish_review(
                view.review_revision,
                request.key,
                request.base,
                Err("temporary read failure".into()),
                cx,
            );
            assert_eq!(view.review_request.as_ref().unwrap().retry_label(), label);
        });
        redraw(cx);
        workspace.read_with(cx, |workspace, _| {
            assert_eq!(workspace.state(), &before);
            assert_eq!(workspace.viewing_history().records(), records);
        });
        git.reference("refs/heads/other", head, true, "repair fixture")
            .unwrap();
        click(cx, "retry-review");
        view.read_with(cx, |view, cx| {
            assert!(view.review.data().is_some());
            assert!(view.review_request.is_none());
            let comparison = view.current_key(cx).unwrap().comparison.unwrap();
            assert_eq!(comparison.base, RefName::from(expected_base));
            assert_eq!(
                comparison.commits.new.to_string(),
                expected_head.to_string()
            );
        });
        workspace.read_with(cx, |workspace, _| {
            if override_base.is_none() && !latest {
                assert_eq!(workspace.state(), &before);
                assert_eq!(workspace.viewing_history().records(), records);
            } else {
                assert_ne!(
                    workspace.state().tab(tab).unwrap().visit_id(),
                    before.tab(tab).unwrap().visit_id()
                );
                assert_eq!(
                    workspace.viewing_history().records().len(),
                    records.len() + 1
                );
                assert_eq!(
                    &workspace.viewing_history().records()[..records.len()],
                    records
                );
            }
        });
    }
}

#[gpui::test]
fn retry_rejects_a_failed_request_after_navigation_before_observers_run(cx: &mut TestAppContext) {
    let (_dir, workspace, tab, repository) = setup(cx);
    let (view, cx) = cx.add_window_view(|_, cx| ComparisonView::new(repository, cx));
    view.update(cx, |view, cx| {
        view.load_review(Some("refs/heads/main".into()), false, cx);
        view.review_task.take();
        let request = view.review_request.as_ref().unwrap().clone();
        view.finish_review(
            view.review_revision,
            request.key,
            request.base,
            Err("temporary read failure".into()),
            cx,
        );
        let revision = view.review_revision;
        workspace.update(cx, |workspace, cx| {
            workspace
                .edit(
                    |state| {
                        state.go_back(tab)?;
                        state.go_forward(tab)
                    },
                    cx,
                )
                .unwrap();
        });
        view.retry_review(cx);
        assert_eq!(view.review_revision, revision);
        assert!(!view.review.is_fetching());
        view.synchronize(cx);
        assert!(view.review_request.is_none());
        assert!(view.review.is_pending());
        assert!(!view.review.is_fetching());
    });
}
