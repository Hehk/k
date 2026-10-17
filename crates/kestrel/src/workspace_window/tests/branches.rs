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
        self.commit_files(reference, parent, time, &[("file.rs", text)])
    }

    fn commit_files(
        &self,
        reference: &str,
        parent: Option<git2::Oid>,
        time: i64,
        files: &[(&str, &str)],
    ) -> git2::Oid {
        let mut index = git2::Index::new().unwrap();
        for (path, text) in files {
            index
                .add(&git2::IndexEntry {
                    ctime: git2::IndexTime::new(0, 0),
                    mtime: git2::IndexTime::new(0, 0),
                    dev: 0,
                    ino: 0,
                    mode: 0o100644,
                    uid: 0,
                    gid: 0,
                    file_size: 0,
                    id: self.git.blob(text.as_bytes()).unwrap(),
                    flags: 0,
                    flags_extended: 0,
                    path: path.as_bytes().to_vec(),
                })
                .unwrap();
        }
        let tree = self
            .git
            .find_tree(index.write_tree_to(&self.git).unwrap())
            .unwrap();
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
fn restoring_windows_keeps_the_last_used_review_for_new_tabs(cx: &mut TestAppContext) {
    use kestrel::{
        state::{AppState, BaseSelection, Browsing},
        storage::Store,
    };

    let fixture = Fixture::new();
    let main = fixture.git.refname_to_id("refs/heads/main").unwrap();
    fixture.commit_files(
        "refs/heads/topic",
        Some(main),
        400,
        &[("file.rs", "topic\n"), ("second.rs", "second\n")],
    );
    let repository = Repository::open(fixture.git.workdir().unwrap()).unwrap();
    let pinned = repository
        .compare(
            &"refs/heads/topic".into(),
            &"refs/remotes/origin/main".into(),
        )
        .unwrap();
    let other = repository
        .change_base(&pinned, &"refs/heads/Alpha".into())
        .unwrap();
    let mut state = AppState::default();
    let project = state.register_project(&repository.info().unwrap()).unwrap();
    let first = state.new_window().unwrap();
    let a = state.new_tab(first).unwrap();
    state.open_project(project, a).unwrap();
    state.select_branch(a, pinned.branch.clone()).unwrap();
    state
        .set_comparison(a, BaseSelection::Default, pinned.clone())
        .unwrap();
    state
        .set_browsing(
            a,
            &pinned.branch,
            Browsing {
                file: Some("file.rs".into()),
                ..Default::default()
            },
        )
        .unwrap();
    let second = state.new_window().unwrap();
    let b = state.new_tab(second).unwrap();
    state.open_project(project, b).unwrap();
    state.select_branch(b, other.branch.clone()).unwrap();
    state
        .set_comparison(
            b,
            BaseSelection::Explicit(other.base.clone()),
            other.clone(),
        )
        .unwrap();
    state
        .set_browsing(
            b,
            &other.branch,
            Browsing {
                file: Some("second.rs".into()),
                ..Default::default()
            },
        )
        .unwrap();
    state.new_tab(second).unwrap();
    let path = fixture.dir.path().join("restored.json");
    let mut store = Store::new(path.clone());
    store.load().unwrap();
    store.save(&state).unwrap();
    let workspace = shared(cx, path);
    let handle = cx.update(|cx| {
        cx.set_global(crate::workspace::Session(workspace.clone()));
        commands::install(cx);
        let handle = Kestrel::open(workspace.clone(), first, cx).unwrap();
        Kestrel::open(workspace.clone(), second, cx).unwrap();
        handle
    });
    cx.run_until_parked();
    let visual = &mut VisualTestContext::from_window(handle.into(), cx);
    redraw(visual);
    assert!(visual.debug_bounds("diff-line-1").is_some());
    workspace.update(cx, |workspace, cx| {
        assert_eq!(
            workspace.state().project(project).unwrap(),
            state.project(project).unwrap()
        );
        for tab in [a, b] {
            assert_eq!(workspace.state().tab(tab).unwrap(), state.tab(tab).unwrap());
        }
        workspace
            .edit(
                |state| {
                    let tab = state.new_tab(second)?;
                    state.open_project(project, tab)?;
                    assert_eq!(state.tab(tab)?.route.review(), state.tab(b)?.route.review());
                    Ok(())
                },
                cx,
            )
            .unwrap();
    });
}

#[gpui::test]
fn nested_filenames_stay_visible_and_identical_names_select_distinct_files(
    cx: &mut TestAppContext,
) {
    const FIRST: &str = "crates/kestrel/src/deeply/nested/first_module/mod.rs";
    const SECOND: &str = "crates/kestrel/src/deeply/nested/second_module/mod.rs";
    let fixture = Fixture::new();
    let main = fixture.git.refname_to_id("refs/heads/main").unwrap();
    let head = fixture.commit_files(
        "refs/heads/topic",
        Some(main),
        400,
        &[
            ("file.rs", "before\n"),
            (FIRST, "first module\n"),
            (SECOND, "second module\n"),
            ("z.rs", "short name\n"),
        ],
    );
    let (workspace, handle) = fixture.start(cx);
    let tab = active(&workspace, 0, cx);
    let cx = &mut VisualTestContext::from_window(handle.into(), cx);
    cx.simulate_resize(gpui::size(px(800.), px(600.)));
    click(cx, "branch:refs/heads/topic");
    for (row, name, directory, path) in [
        ("changed-file-0", "file-name-0", "file-directory-0", FIRST),
        ("changed-file-1", "file-name-1", "file-directory-1", SECOND),
    ] {
        let row_bounds = cx.debug_bounds(row).unwrap();
        assert_eq!(row_bounds.size.width, px(250.));
        let name_bounds = cx.debug_bounds(name).unwrap();
        let directory_bounds = cx.debug_bounds(directory).unwrap();
        assert!(name_bounds.size.width > px(40.));
        assert!(name_bounds.size.height > px(0.));
        assert!(name_bounds.left() >= row_bounds.left());
        assert!(name_bounds.right() <= row_bounds.right());
        assert!(name_bounds.bottom() <= directory_bounds.top());
        assert!(directory_bounds.bottom() <= row_bounds.bottom());
        click(cx, name);
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
            assert_eq!(review.browsing.file, Some(path.into()));
            assert_eq!(
                review.comparison.as_ref().unwrap().commits.new.to_string(),
                head.to_string()
            );
        });
    }
    let short = cx.debug_bounds("changed-file-2").unwrap();
    let long = cx.debug_bounds("changed-file-0").unwrap();
    assert_eq!(short.left(), long.left());
    assert_eq!(short.right(), long.right());
    cx.simulate_click(
        gpui::point(short.right() - px(2.), short.center().y),
        Modifiers::none(),
    );
    redraw(cx);
    assert_eq!(
        cx.debug_bounds("changed-file-2").unwrap().size.width,
        px(250.)
    );
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
        assert_eq!(review.browsing.file, Some("z.rs".into()));
    });
    let bounds = cx.debug_bounds("changed-file-0").unwrap();
    cx.simulate_mouse_move(bounds.center(), None, Modifiers::none());
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(1));
    redraw(cx);
    assert!(
        cx.debug_bounds("tooltip-crates/kestrel/src/deeply/nested/first_module/mod.rs")
            .is_some()
    );
}

#[gpui::test]
fn diff_selection_copies_only_source_and_resets_on_file_changes(cx: &mut TestAppContext) {
    let fixture = Fixture::new();
    let main = fixture.git.refname_to_id("refs/heads/main").unwrap();
    let source = "-keep\t a\u{301}😀\n\n++value\n";
    fixture.commit_files(
        "refs/heads/topic",
        Some(main),
        400,
        &[("file.rs", source), ("other.rs", "other\n")],
    );
    let index = std::fs::read(fixture.git.path().join("index")).unwrap();
    let (workspace, handle) = fixture.start(cx);
    let cx = &mut VisualTestContext::from_window(handle.into(), cx);
    click(cx, "branch:refs/heads/topic");
    let state = workspace.read_with(cx, |workspace, _| workspace.state().clone());
    let first = cx.debug_bounds("diff-line-2").unwrap();
    let last = cx.debug_bounds("diff-line-4").unwrap();
    let start = gpui::point(first.left() + px(1.), first.center().y);
    let end = gpui::point(last.right() - px(1.), last.center().y);
    let selectable = cx.debug_bounds("selectable-diff").unwrap();
    assert!(selectable.contains(&start) && selectable.contains(&end));
    for (from, to) in [(start, end), (end, start)] {
        cx.simulate_mouse_down(from, gpui::MouseButton::Left, Modifiers::none());
        redraw(cx);
        cx.simulate_mouse_move(to, gpui::MouseButton::Left, Modifiers::none());
        redraw(cx);
        cx.simulate_mouse_up(to, gpui::MouseButton::Left, Modifiers::none());
        cx.simulate_keystrokes("cmd-c");
        cx.update(|_, cx| {
            assert_eq!(
                cx.read_from_clipboard().unwrap().text().unwrap(),
                source.trim_end_matches('\n')
            )
        });
    }
    cx.simulate_click(start, Modifiers::none());
    cx.simulate_click(
        end,
        Modifiers {
            shift: true,
            ..Modifiers::none()
        },
    );
    cx.simulate_keystrokes("cmd-c");
    cx.update(|_, cx| {
        assert_eq!(
            cx.read_from_clipboard().unwrap().text().unwrap(),
            source.trim_end_matches('\n')
        )
    });
    cx.simulate_keystrokes("cmd-a cmd-c");
    cx.update(|_, cx| {
        assert_eq!(
            cx.read_from_clipboard().unwrap().text().unwrap(),
            format!("before\n{source}")
        )
    });
    assert_eq!(
        workspace.read_with(cx, |workspace, _| workspace.state().clone()),
        state
    );
    assert_eq!(
        std::fs::read(fixture.git.path().join("index")).unwrap(),
        index
    );
    cx.simulate_keystrokes("escape");
    cx.update(|_, cx| cx.write_to_clipboard(gpui::ClipboardItem::new_string("unchanged".into())));
    cx.simulate_keystrokes("cmd-c");
    cx.update(|_, cx| {
        assert_eq!(
            cx.read_from_clipboard().unwrap().text().unwrap(),
            "unchanged"
        )
    });
    cx.simulate_keystrokes("cmd-a");
    click(cx, "changed-file-1");
    click(cx, "diff-line-1");
    cx.simulate_keystrokes("cmd-c");
    cx.update(|_, cx| {
        assert_eq!(
            cx.read_from_clipboard().unwrap().text().unwrap(),
            "unchanged"
        )
    });
    cx.simulate_keystrokes("cmd-a cmd-c");
    cx.update(|_, cx| assert_eq!(cx.read_from_clipboard().unwrap().text().unwrap(), "other\n"));
    cx.simulate_keystrokes("cmd-[");
    redraw(cx);
    click(cx, "branch-search");
    cx.simulate_input("topic");
    cx.simulate_keystrokes("cmd-a cmd-c");
    cx.update(|_, cx| assert_eq!(cx.read_from_clipboard().unwrap().text().unwrap(), "topic"));
}

#[gpui::test]
fn large_diffs_keep_gutters_fixed_and_selection_virtualized(cx: &mut TestAppContext) {
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
    click(cx, "diff-line-2");
    cx.simulate_keystrokes("cmd-a cmd-c");
    cx.update(|_, cx| {
        assert_eq!(
            cx.read_from_clipboard().unwrap().text().unwrap(),
            format!("before\n{text}")
        )
    });
    let row = cx.debug_bounds("diff-line-2").unwrap();
    let position = gpui::point(row.left() + px(1.), row.center().y);
    cx.simulate_mouse_down(position, gpui::MouseButton::Left, Modifiers::none());
    redraw(cx);
    cx.simulate_event(gpui::ScrollWheelEvent {
        position,
        delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-2200.))),
        modifiers: Modifiers::none(),
        touch_phase: gpui::TouchPhase::Moved,
    });
    redraw(cx);
    cx.simulate_mouse_up(position, gpui::MouseButton::Left, Modifiers::none());
    cx.simulate_keystrokes("cmd-c");
    cx.update(|_, cx| {
        assert_eq!(
            cx.read_from_clipboard().unwrap().text().unwrap(),
            "added line\n".repeat(100)
        )
    });
    assert!(cx.debug_bounds("diff-line-2500").is_none());
    let gutter = cx.debug_bounds("diff-gutter-102").unwrap();
    assert_eq!(gutter.size.width, px(124.));
    let source = cx.debug_bounds("diff-source-102").unwrap();
    cx.simulate_event(gpui::ScrollWheelEvent {
        position,
        delta: gpui::ScrollDelta::Pixels(gpui::point(px(-500.), px(0.))),
        modifiers: Modifiers::none(),
        touch_phase: gpui::TouchPhase::Moved,
    });
    redraw(cx);
    assert_eq!(cx.debug_bounds("diff-gutter-102").unwrap(), gutter);
    assert_eq!(
        cx.debug_bounds("diff-source-102").unwrap().left(),
        source.left() - px(500.)
    );
    assert_eq!(
        gutter.top(),
        cx.debug_bounds("diff-line-102").unwrap().top()
    );
    let next = gpui::point(position.x, position.y + px(22.));
    cx.simulate_mouse_down(position, gpui::MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_move(next, gpui::MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_up(next, gpui::MouseButton::Left, Modifiers::none());
    cx.simulate_keystrokes("cmd-c");
    cx.update(|_, cx| {
        assert_eq!(
            cx.read_from_clipboard().unwrap().text().unwrap(),
            "added line\n"
        )
    });
    let body = gpui::point(gutter.right() + px(1.), position.y);
    let next = gpui::point(body.x, body.y + px(22.));
    cx.simulate_mouse_down(body, gpui::MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_move(next, gpui::MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_up(next, gpui::MouseButton::Left, Modifiers::none());
    cx.simulate_keystrokes("cmd-c");
    cx.update(|_, cx| {
        assert_eq!(
            cx.read_from_clipboard().unwrap().text().unwrap(),
            "\nadded line"
        )
    });
    cx.simulate_event(gpui::ScrollWheelEvent {
        position: body,
        delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-11.))),
        modifiers: Modifiers::none(),
        touch_phase: gpui::TouchPhase::Moved,
    });
    redraw(cx);
    let shifted = cx.debug_bounds("diff-gutter-102").unwrap();
    assert_eq!(shifted.left(), gutter.left());
    assert_eq!(shifted.top(), gutter.top() - px(11.));
    assert_eq!(
        shifted.top(),
        cx.debug_bounds("diff-line-102").unwrap().top()
    );
    cx.simulate_resize(gpui::size(px(800.), px(600.)));
    redraw(cx);
    let resized = cx.debug_bounds("diff-gutter-102").unwrap();
    assert_eq!(resized.left(), gutter.left());
    assert_eq!(
        resized.top(),
        cx.debug_bounds("diff-line-102").unwrap().top()
    );
    assert!(cx.debug_bounds("diff-line-2500").is_none());
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
