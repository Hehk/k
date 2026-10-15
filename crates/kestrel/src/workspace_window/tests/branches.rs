use super::*;
use kestrel::state::{BranchSort, RepositoryPage};

struct Fixture {
    dir: tempfile::TempDir,
    git: git2::Repository,
    topic: git2::Oid,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let git = git2::Repository::init(dir.path().join("repo")).unwrap();
        let mut this = Self {
            dir,
            git,
            topic: git2::Oid::zero(),
        };
        let main = this.commit("refs/heads/main", None, 100, "before\n");
        this.topic = this.commit("refs/heads/topic", Some(main), 200, "topic\n");
        this.commit("refs/heads/Alpha", Some(main), 300, "alpha\n");
        this.git
            .reference("refs/remotes/origin/main", main, true, "fixture")
            .unwrap();
        this.git
            .reference("refs/remotes/origin/topic", this.topic, true, "fixture")
            .unwrap();
        this.git
            .reference_symbolic(
                "refs/remotes/origin/HEAD",
                "refs/remotes/origin/main",
                true,
                "fixture",
            )
            .unwrap();
        this.git.set_head("refs/heads/main").unwrap();
        std::fs::write(
            this.git.workdir().unwrap().join("file.rs"),
            "dirty checkout\n",
        )
        .unwrap();
        let mut index = this.git.index().unwrap();
        index.add_path(std::path::Path::new("file.rs")).unwrap();
        index.write().unwrap();
        this
    }

    fn commit(
        &self,
        reference: &str,
        parent: Option<git2::Oid>,
        time: i64,
        text: &str,
    ) -> git2::Oid {
        let mut tree = self.git.treebuilder(None).unwrap();
        tree.insert("file.rs", self.git.blob(text.as_bytes()).unwrap(), 0o100644)
            .unwrap();
        let tree = self.git.find_tree(tree.write().unwrap()).unwrap();
        let parent = parent.map(|id| self.git.find_commit(id).unwrap());
        let signature =
            git2::Signature::new("Test", "test@example.com", &git2::Time::new(time, 0)).unwrap();
        let id = self
            .git
            .commit(
                None,
                &signature,
                &signature,
                "fixture",
                &tree,
                &parent.iter().collect::<Vec<_>>(),
            )
            .unwrap();
        self.git.reference(reference, id, true, "fixture").unwrap();
        id
    }

    fn start(&self, cx: &mut TestAppContext) -> (Entity<Workspace>, gpui::WindowHandle<Kestrel>) {
        let workspace = shared(cx, self.dir.path().join("state.json"));
        cx.update(|cx| {
            cx.set_global(crate::workspace::Session(workspace.clone()));
            commands::install(cx);
        });
        let handle = cx.update(|cx| Kestrel::new_window(workspace.clone(), cx).unwrap());
        let info = Repository::open(self.git.workdir().unwrap())
            .unwrap()
            .info()
            .unwrap();
        handle
            .update(cx, |root, window, cx| {
                window.activate_window();
                open(root, info, cx);
            })
            .unwrap();
        cx.run_until_parked();
        (workspace, handle)
    }
}

fn click(cx: &mut VisualTestContext, selector: &'static str) {
    redraw(cx);
    let bounds = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("missing {selector}"));
    assert!(
        bounds.size.width > px(0.) && bounds.size.height > px(0.),
        "{selector}"
    );
    cx.simulate_click(bounds.center(), Modifiers::none());
    redraw(cx);
}

#[gpui::test]
fn large_diffs_stay_virtualized_across_redraws(cx: &mut TestAppContext) {
    let fixture = Fixture::new();
    let main = fixture.git.refname_to_id("refs/heads/main").unwrap();
    let mut text = "added line\n".repeat(5000);
    text.push_str(&"wide line ".repeat(200));
    fixture.commit("refs/heads/topic", Some(main), 400, &text);
    let (_, handle) = fixture.start(cx);
    let cx = &mut VisualTestContext::from_window(handle.into(), cx);
    click(cx, "branch:refs/heads/topic");
    for _ in 0..5 {
        redraw(cx);
        assert!(cx.debug_bounds("diff-line-1").unwrap().size.height > px(0.));
        assert!(cx.debug_bounds("diff-line-2500").is_none());
    }
}

#[gpui::test]
fn appearance_preferences_preserve_the_loaded_review_and_open_base_picker(cx: &mut TestAppContext) {
    let fixture = Fixture::new();
    let (workspace, handle) = fixture.start(cx);
    let cx = &mut VisualTestContext::from_window(handle.into(), cx);
    click(cx, "branch-search");
    cx.simulate_input("topic");
    click(cx, "branch:refs/heads/topic");
    click(cx, "choose-base");
    assert!(cx.debug_bounds("diff-line-1").is_some());
    assert!(cx.debug_bounds("base:refs/heads/main").is_some());

    let before = workspace.read_with(cx, |workspace, _| workspace.state().clone());
    let view = handle
        .read_with(cx, |root, _| root.repository_view.clone().unwrap())
        .unwrap();
    for preferences in [
        Preferences {
            reduce_transparency: true,
            ..Preferences::default()
        },
        Preferences {
            increase_contrast: true,
            ..Preferences::default()
        },
        Preferences {
            accent: gpui::rgb(0xff0000).into(),
            ..Preferences::default()
        },
        Preferences::default(),
    ] {
        cx.update(|_, cx| cx.set_global(preferences));
        redraw(cx);
        assert_eq!(
            workspace.read_with(cx, |workspace, _| workspace.state().clone()),
            before,
        );
        assert_eq!(
            handle
                .read_with(cx, |root, _| root.repository_view.clone().unwrap())
                .unwrap(),
            view,
        );
        assert!(cx.debug_bounds("diff-line-1").is_some());
        assert!(cx.debug_bounds("base:refs/heads/main").is_some());
    }
}

#[gpui::test]
fn branch_list_search_sort_and_comparison_navigation_use_the_real_ui(cx: &mut TestAppContext) {
    let fixture = Fixture::new();
    let index_before = std::fs::read(fixture.git.path().join("index")).unwrap();
    let (workspace, handle) = fixture.start(cx);
    let tab = active(&workspace, 0, cx);
    let cx = &mut VisualTestContext::from_window(handle.into(), cx);
    redraw(cx);
    assert!(cx.debug_bounds("branch:refs/remotes/origin/HEAD").is_none());
    let alpha = cx.debug_bounds("branch:refs/heads/Alpha").unwrap();
    let main = cx.debug_bounds("branch:refs/heads/main").unwrap();
    assert!(alpha.top() < main.top());
    assert!(cx.debug_bounds("branch:refs/heads/topic").unwrap().top() < main.top());
    click(cx, "branch-sort");
    click(cx, "sort-name");
    assert!(
        cx.debug_bounds("branch:refs/heads/main").unwrap().top()
            < cx.debug_bounds("branch:refs/heads/topic").unwrap().top()
    );
    click(cx, "branch-search");
    cx.simulate_input("TOPIC");
    redraw(cx);
    workspace.read_with(cx, |workspace, _| {
        let Route::Repository { branches, .. } = &workspace.state().tab(tab).unwrap().route else {
            panic!()
        };
        assert_eq!(branches.query, "TOPIC");
        assert_eq!(branches.sort, BranchSort::Name);
    });
    assert!(
        cx.debug_bounds("branch:refs/remotes/origin/topic")
            .is_some()
    );
    click(cx, "branch:refs/heads/topic");
    workspace.read_with(cx, |workspace, _| {
        let route = &workspace.state().tab(tab).unwrap().route;
        let Route::Repository { page, review, .. } = route else {
            panic!()
        };
        assert_eq!(*page, RepositoryPage::Comparison);
        let comparison = review.active_review().unwrap().comparison.as_ref().unwrap();
        assert_eq!(
            comparison.commits.new.to_string(),
            fixture.topic.to_string()
        );
        assert_eq!(comparison.base, RefName::from("refs/remotes/origin/main"));
    });
    assert!(cx.debug_bounds("choose-base").is_some());
    assert!(cx.debug_bounds("diff-line-1").unwrap().size.height > px(0.));
    cx.simulate_keystrokes("cmd-[");
    redraw(cx);
    workspace.read_with(cx, |workspace, _| {
        let Route::Repository {
            page,
            branches,
            review,
            ..
        } = &workspace.state().tab(tab).unwrap().route
        else {
            panic!()
        };
        assert_eq!(*page, RepositoryPage::Branches);
        assert_eq!(branches.query, "TOPIC");
        assert_eq!(branches.sort, BranchSort::Name);
        assert!(review.active_review().unwrap().comparison.is_some());
    });
    click(cx, "branch-search");
    cx.simulate_keystrokes("enter");
    redraw(cx);
    workspace.read_with(cx, |workspace, _| {
        assert_eq!(
            workspace
                .state()
                .tab(tab)
                .unwrap()
                .route
                .review()
                .unwrap()
                .selected_branch,
            Some(RefName::from("refs/remotes/origin/topic"))
        )
    });
    cx.simulate_keystrokes("cmd-[");
    redraw(cx);
    let before = workspace.read_with(cx, |workspace, _| workspace.state().clone());
    cx.quit();
    assert_eq!(
        Store::new(fixture.dir.path().join("state.json"))
            .load()
            .unwrap()
            .state,
        before
    );
    assert_eq!(
        std::fs::read(fixture.git.path().join("index")).unwrap(),
        index_before
    );
    assert_eq!(
        std::fs::read_to_string(fixture.git.workdir().unwrap().join("file.rs")).unwrap(),
        "dirty checkout\n"
    );
    assert_eq!(
        fixture.git.head().unwrap().target().unwrap(),
        fixture.git.refname_to_id("refs/heads/main").unwrap()
    );
}

#[gpui::test]
fn list_preferences_are_independent_and_tab_shortcuts_work_from_search(cx: &mut TestAppContext) {
    let fixture = Fixture::new();
    let (workspace, handle) = fixture.start(cx);
    let first = active(&workspace, 0, cx);
    let cx = &mut VisualTestContext::from_window(handle.into(), cx);
    click(cx, "branch-sort");
    click(cx, "sort-name");
    click(cx, "branch-search");
    cx.simulate_input("Alpha");
    cx.simulate_keystrokes("cmd-t");
    redraw(cx);
    cx.simulate_keystrokes("cmd-t");
    redraw(cx);
    let third = workspace.read_with(cx, |workspace, _| {
        let window = &workspace.state().windows()[0];
        assert_eq!(window.tabs.len(), 3);
        window.active_tab.unwrap()
    });
    let info = Repository::open(fixture.git.workdir().unwrap())
        .unwrap()
        .info()
        .unwrap();
    handle
        .update(cx, |root, _, cx| open(root, info, cx))
        .unwrap();
    redraw(cx);
    click(cx, "branch-search");
    cx.simulate_input("topic");
    redraw(cx);
    workspace.read_with(cx, |workspace, _| {
        let Route::Repository { branches, .. } = &workspace.state().tab(first).unwrap().route
        else {
            panic!()
        };
        assert_eq!(branches.query, "Alpha");
        assert_eq!(branches.sort, BranchSort::Name);
        let Route::Repository { branches, .. } = &workspace.state().tab(third).unwrap().route
        else {
            panic!()
        };
        assert_eq!(branches.query, "topic");
        assert_eq!(branches.sort, BranchSort::Newest);
    });
    fixture
        .git
        .reference("refs/heads/Alpha-new", fixture.topic, true, "fixture")
        .unwrap();
    cx.simulate_keystrokes("cmd-1");
    redraw(cx);
    assert!(cx.debug_bounds("branch:refs/heads/Alpha-new").is_none());
    click(cx, "refresh-branches");
    assert!(cx.debug_bounds("branch:refs/heads/Alpha-new").is_some());
    cx.simulate_keystrokes("cmd-3");
    redraw(cx);
    let saved = workspace.read_with(cx, |workspace, _| {
        workspace.state().tab(third).unwrap().clone()
    });
    cx.update(|window, cx| window.dispatch_action(Box::new(Command::DetachTab), cx));
    cx.run_until_parked();
    workspace.read_with(cx, |workspace, _| {
        assert_eq!(workspace.state().windows().len(), 2);
        assert_eq!(workspace.state().tab(third).unwrap(), &saved);
    });
}

#[gpui::test]
fn refresh_preserves_pins_and_only_load_latest_advances_them(cx: &mut TestAppContext) {
    let fixture = Fixture::new();
    let (workspace, handle) = fixture.start(cx);
    let tab = active(&workspace, 0, cx);
    let cx = &mut VisualTestContext::from_window(handle.into(), cx);
    click(cx, "branch:refs/heads/topic");
    let latest = fixture.commit("refs/heads/topic", Some(fixture.topic), 400, "new topic\n");
    click(cx, "refresh-review-refs");
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
                .as_ref()
                .unwrap()
                .commits
                .new
                .to_string(),
            fixture.topic.to_string()
        )
    });
    click(cx, "choose-base");
    click(cx, "base:refs/heads/Alpha");
    workspace.read_with(cx, |workspace, _| {
        let review = workspace
            .state()
            .tab(tab)
            .unwrap()
            .route
            .review()
            .unwrap()
            .active_review()
            .unwrap();
        assert_eq!(
            review.comparison.as_ref().unwrap().commits.new.to_string(),
            fixture.topic.to_string()
        );
        assert_eq!(
            review.base,
            kestrel::state::BaseSelection::Explicit("refs/heads/Alpha".into())
        );
    });
    click(cx, "latest-review");
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
                .as_ref()
                .unwrap()
                .commits
                .new
                .to_string(),
            latest.to_string()
        )
    });
    cx.simulate_keystrokes("cmd-[");
    redraw(cx);
    click(cx, "branch:refs/heads/topic");
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
                .as_ref()
                .unwrap()
                .commits
                .new
                .to_string(),
            latest.to_string()
        )
    });
}

#[gpui::test]
fn restoring_a_comparison_rebuilds_the_saved_commits_not_newer_refs(cx: &mut TestAppContext) {
    let fixture = Fixture::new();
    let repository = Repository::open(fixture.git.workdir().unwrap()).unwrap();
    let comparison = repository
        .compare(
            &"refs/heads/topic".into(),
            &"refs/remotes/origin/main".into(),
        )
        .unwrap();
    let mut saved = AppState::default();
    let project = saved.register_project(&repository.info().unwrap()).unwrap();
    let window = saved.new_window().unwrap();
    let tab = saved.new_tab(window).unwrap();
    saved.open_project(project, tab).unwrap();
    saved.select_branch(tab, "refs/heads/topic".into()).unwrap();
    saved
        .set_comparison(tab, kestrel::state::BaseSelection::Default, comparison)
        .unwrap();
    saved
        .set_browsing(
            tab,
            &"refs/heads/topic".into(),
            kestrel::state::Browsing {
                file: Some("file.rs".into()),
                ..Default::default()
            },
        )
        .unwrap();
    let path = fixture.dir.path().join("state.json");
    let mut store = Store::new(path.clone());
    store.load().unwrap();
    store.save(&saved).unwrap();
    let latest = fixture.commit("refs/heads/topic", Some(fixture.topic), 400, "new topic\n");
    let workspace = shared(cx, path);
    cx.update(|cx| {
        cx.set_global(crate::workspace::Session(workspace.clone()));
        commands::install(cx);
    });
    let handle = cx.update(|cx| Kestrel::open(workspace.clone(), window, cx).unwrap());
    handle
        .update(cx, |_, window, _| window.activate_window())
        .unwrap();
    let cx = &mut VisualTestContext::from_window(handle.into(), cx);
    redraw(cx);
    assert_eq!(
        workspace.read_with(cx, |workspace, _| workspace.state().clone()),
        saved
    );
    assert!(cx.debug_bounds("diff-line-1").is_some());
    click(cx, "latest-review");
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
                .as_ref()
                .unwrap()
                .commits
                .new
                .to_string(),
            latest.to_string()
        )
    });
}

#[gpui::test]
fn a_repository_without_a_default_requires_an_explicit_base(cx: &mut TestAppContext) {
    let fixture = Fixture::new();
    fixture
        .git
        .find_reference("refs/remotes/origin/HEAD")
        .unwrap()
        .delete()
        .unwrap();
    let (workspace, handle) = fixture.start(cx);
    let tab = active(&workspace, 0, cx);
    let cx = &mut VisualTestContext::from_window(handle.into(), cx);
    click(cx, "branch:refs/heads/topic");
    workspace.read_with(cx, |workspace, _| {
        assert!(
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
                .is_none()
        )
    });
    click(cx, "choose-base");
    click(cx, "base:refs/heads/main");
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
                .as_ref()
                .unwrap()
                .base,
            RefName::from("refs/heads/main")
        )
    });
}
