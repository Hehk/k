use super::*;
use crate::workspace::{LoadedWorkspace, Workspace};
use gpui::{Focusable, TestAppContext};
use kestrel::git::{Branch, Repository};
use kestrel::storage::Store;

#[gpui::test]
fn branch_view_searches_and_selects_without_a_comparison_view(cx: &mut TestAppContext) {
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
    git.reference("refs/heads/topic", tip, true, "fixture")
        .unwrap();
    git.set_head("refs/heads/main").unwrap();
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
    let repository = cx.new(|cx| RepositoryModel::new(workspace.clone(), window, tab, project, cx));
    cx.update(crate::text_input::bind_keys);
    let (view, cx) = cx.add_window_view(|_, cx| BranchView::new(repository, cx));
    cx.run_until_parked();
    let search = view.read_with(cx, |view, _| view.search.clone());
    cx.update(|window, cx| {
        window.focus(&search.read(cx).focus_handle(cx));
        window.draw(cx).clear();
    });
    cx.simulate_input("TOPIC");
    cx.run_until_parked();
    view.read_with(cx, |view, _| assert_eq!(view.visible.len(), 1));
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    let selected = workspace.read_with(cx, |workspace, _| {
        workspace.state().tab(tab).unwrap().route.clone()
    });
    let Route::Repository {
        page,
        review,
        branches,
        ..
    } = &selected
    else {
        panic!()
    };
    assert_eq!(*page, RepositoryPage::Comparison);
    assert_eq!(branches.query, "TOPIC");
    assert_eq!(review.selected_branch, Some("refs/heads/topic".into()));
    assert!(review.active_review().unwrap().comparison.is_none());

    view.update(cx, |view, cx| {
        view.select_branch("refs/heads/main".into(), cx);
        view.change_filters(BranchList::default(), cx);
    });
    workspace.read_with(cx, |workspace, _| {
        assert_eq!(workspace.state().tab(tab).unwrap().route, selected)
    });
}

#[test]
fn branch_sorting_is_deterministic_and_search_keeps_distinct_refs() {
    let tip = "1111111111111111111111111111111111111111".parse().unwrap();
    let branches = [
        ("refs/heads/zebra", 400, false),
        ("refs/remotes/origin/topic", 200, true),
        ("refs/heads/origin/topic", 200, false),
        ("refs/heads/Alpha", 300, false),
    ]
    .map(|(name, committed_at, remote)| Branch {
        name: name.into(),
        tip,
        committed_at,
        remote,
    });
    let snapshot = Snapshot {
        branches: branches.into(),
        default_base: None,
    };
    assert_eq!(
        ordered_branches(&snapshot, &BranchList::default()),
        [0, 3, 2, 1]
    );
    assert_eq!(
        ordered_branches(
            &snapshot,
            &BranchList {
                query: "TOPIC".into(),
                sort: BranchSort::Name,
                ..Default::default()
            }
        ),
        [2, 1]
    );
    assert_eq!(
        ordered_branches(
            &snapshot,
            &BranchList {
                query: String::new(),
                sort: BranchSort::Name,
                ..Default::default()
            }
        ),
        [3, 2, 1, 0]
    );
    assert!(
        ordered_branches(
            &snapshot,
            &BranchList {
                query: "missing".into(),
                ..Default::default()
            }
        )
        .is_empty()
    );
}
