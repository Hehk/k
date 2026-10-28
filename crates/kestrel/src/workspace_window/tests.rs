use gpui::{AppContext, Modifiers, TestAppContext, VisualTestContext};
use kestrel::{
    review::RefName,
    state::{AppState, Route},
    storage::Store,
};

use super::*;

mod branches;
mod labels;
mod recent;
use crate::workspace::LoadedWorkspace;

fn shared(cx: &mut TestAppContext, path: PathBuf) -> Entity<Workspace> {
    let loaded = LoadedWorkspace::load(Ok(Store::new(path)));
    cx.new(|cx| Workspace::new(loaded, cx))
}

fn repository(path: &std::path::Path) -> RepositoryInfo {
    git2::Repository::init(path).unwrap();
    Repository::open(path).unwrap().info().unwrap()
}

fn open(root: &mut Kestrel, info: RepositoryInfo, cx: &mut Context<Kestrel>) {
    let target = root.open_target(cx).unwrap();
    root.open_repository(target, info, cx);
}

fn active(workspace: &Entity<Workspace>, index: usize, cx: &TestAppContext) -> TabId {
    workspace.read_with(cx, |workspace, _| {
        workspace.state().windows()[index].active_tab.unwrap()
    })
}

fn redraw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear());
    cx.run_until_parked();
}

#[gpui::test]
fn back_forward_buttons_and_shortcuts_navigate_only_the_selected_tab(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let workspace = shared(cx, dir.path().join("state.json"));
    cx.update(|cx| {
        cx.set_global(crate::workspace::Session(workspace.clone()));
        commands::install(cx);
    });
    let handle = cx.update(|cx| Kestrel::new_window(workspace.clone(), cx).unwrap());
    let tab = active(&workspace, 0, cx);
    let one = repository(&dir.path().join("one"));
    let two = repository(&dir.path().join("two"));
    handle
        .update(cx, |root, _, cx| open(root, one, cx))
        .unwrap();
    cx.run_until_parked();
    handle
        .update(cx, |root, _, cx| open(root, two, cx))
        .unwrap();
    cx.run_until_parked();
    handle
        .update(cx, |_, window, _| window.activate_window())
        .unwrap();
    let cx = &mut VisualTestContext::from_window(handle.into(), cx);
    redraw(cx);
    let back = cx.debug_bounds("navigate-back").unwrap().center();
    let forward = cx.debug_bounds("navigate-forward").unwrap().center();
    cx.simulate_click(back, Modifiers::none());
    redraw(cx);
    handle
        .read_with(cx, |root, cx| {
            assert_eq!(root.top_bar.read(cx).active_tab().unwrap().title, "one")
        })
        .unwrap();
    cx.simulate_keystrokes("cmd-[");
    redraw(cx);
    handle
        .read_with(cx, |root, cx| {
            assert_eq!(root.top_bar.read(cx).active_tab().unwrap().title, "Home")
        })
        .unwrap();
    let before = workspace.read_with(cx, |workspace, _| workspace.state().clone());
    cx.simulate_click(back, Modifiers::none());
    cx.simulate_keystrokes("cmd-[");
    redraw(cx);
    assert_eq!(
        workspace.read_with(cx, |workspace, _| workspace.state().clone()),
        before
    );
    cx.simulate_click(forward, Modifiers::none());
    cx.simulate_keystrokes("cmd-]");
    redraw(cx);
    handle
        .read_with(cx, |root, cx| {
            assert_eq!(root.top_bar.read(cx).active_tab().unwrap().title, "two")
        })
        .unwrap();
    let saved = workspace.read_with(cx, |workspace, _| {
        workspace.state().tab(tab).unwrap().clone()
    });
    cx.simulate_keystrokes("cmd-t cmd-[ cmd-]");
    redraw(cx);
    workspace.read_with(cx, |workspace, _| {
        assert_eq!(workspace.state().tab(tab).unwrap(), &saved);
        let window = &workspace.state().windows()[0];
        let current = workspace.state().tab(window.active_tab.unwrap()).unwrap();
        assert_ne!(current.id, tab);
        assert_eq!(current.route, Route::Home);
        assert!(!current.can_go_back());
        assert!(!current.can_go_forward());
    });
    cx.simulate_keystrokes("cmd-shift-[");
    redraw(cx);
    handle
        .read_with(cx, |root, cx| {
            assert_eq!(root.top_bar.read(cx).active_tab().unwrap().id, tab.as_u64())
        })
        .unwrap();
    assert_eq!(
        workspace.read_with(cx, |workspace, _| workspace
            .state()
            .tab(tab)
            .unwrap()
            .clone()),
        saved
    );
}

#[gpui::test]
fn navigation_commands_are_disabled_during_repository_opening(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let workspace = shared(cx, dir.path().join("state.json"));
    cx.update(|cx| {
        cx.set_global(crate::workspace::Session(workspace.clone()));
        commands::install(cx);
    });
    let handle = cx.update(|cx| Kestrel::new_window(workspace.clone(), cx).unwrap());
    let info = repository(&dir.path().join("repo"));
    handle
        .update(cx, |root, _, cx| open(root, info, cx))
        .unwrap();
    let tab = active(&workspace, 0, cx);
    workspace.update(cx, |workspace, cx| {
        workspace
            .edit(
                |state| {
                    state.go_home(tab)?;
                    state.go_back(tab)
                },
                cx,
            )
            .unwrap()
    });
    cx.run_until_parked();
    let before = workspace.read_with(cx, |workspace, _| workspace.state().clone());
    let target = handle
        .update(cx, |root, window, cx| {
            window.activate_window();
            assert!(root.navigation_allowed(Command::Back, cx));
            assert!(root.navigation_allowed(Command::Forward, cx));
            let target = root.open_target(cx).unwrap();
            root.opening = Some(target.clone());
            root.command(Command::Back, window, cx);
            root.command(Command::Forward, window, cx);
            cx.notify();
            target
        })
        .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        for state in commands::menu_state(cx)
            .iter()
            .filter(|state| state.menu == "Go")
        {
            assert!(!state.enabled);
        }
        cx.dispatch_action(&Command::Back);
        cx.dispatch_action(&Command::Forward);
    });
    cx.run_until_parked();
    let cx = &mut VisualTestContext::from_window(handle.into(), cx);
    redraw(cx);
    for id in ["navigate-back", "navigate-forward"] {
        let point = cx.debug_bounds(id).unwrap().center();
        cx.simulate_click(point, Modifiers::none());
    }
    cx.simulate_keystrokes("cmd-[ cmd-]");
    redraw(cx);
    assert_eq!(
        workspace.read_with(cx, |workspace, _| workspace.state().clone()),
        before
    );
    handle
        .update(cx, |root, _, cx| root.finish_open(target, Ok(None), cx))
        .unwrap();
    redraw(cx);
    cx.simulate_keystrokes("cmd-[");
    redraw(cx);
    assert_eq!(
        workspace.read_with(cx, |workspace, _| workspace
            .state()
            .tab(tab)
            .unwrap()
            .route
            .clone()),
        Route::Home
    );
}

#[gpui::test]
fn plus_and_cmd_t_open_home_without_a_repository_picker(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let workspace = shared(cx, dir.path().join("state.json"));
    let id = workspace.update(cx, |workspace, cx| {
        workspace
            .edit(
                |state| {
                    let window = state.new_window()?;
                    state.new_tab(window)?;
                    Ok(window)
                },
                cx,
            )
            .unwrap()
    });
    cx.update(crate::top_bar::bind_keys);
    let (root, cx) =
        cx.add_window_view(|window, cx| Kestrel::new(workspace.clone(), id, window, cx));
    cx.simulate_keystrokes("cmd-t");
    cx.update(|window, cx| {
        window.draw(cx).clear();
    });
    cx.run_until_parked();
    let add = cx.debug_bounds("new-tab").unwrap().center();
    cx.simulate_click(add, Modifiers::none());
    cx.run_until_parked();
    workspace.read_with(cx, |workspace, _| {
        let window = workspace.state().window(id).unwrap();
        assert_eq!(window.tabs.len(), 3);
        assert!(window.tabs.iter().all(|tab| tab.route == Route::Home));
        assert!(workspace.state().projects().is_empty());
    });
    root.read_with(cx, |root, cx| {
        assert_eq!(root.top_bar.read(cx).active_tab().unwrap().title, "Home");
        assert!(root.opening.is_none());
        assert!(root.selection.is_none());
    });
    cx.simulate_keystrokes("cmd-w");
    cx.run_until_parked();
    assert_eq!(
        workspace.read_with(cx, |workspace, _| workspace
            .state()
            .window(id)
            .unwrap()
            .tabs
            .len()),
        2
    );
}

#[gpui::test]
fn same_repository_in_multiple_windows_has_independent_navigation_and_labels(
    cx: &mut TestAppContext,
) {
    let dir = tempfile::tempdir().unwrap();
    let info = repository(&dir.path().join("repo"));
    let workspace = shared(cx, dir.path().join("state.json"));
    let first = cx.update(|cx| Kestrel::new_window(workspace.clone(), cx).unwrap());
    first
        .update(cx, |root, _, cx| open(root, info.clone(), cx))
        .unwrap();
    cx.run_until_parked();
    let a = active(&workspace, 0, cx);
    let second = cx.update(|cx| Kestrel::new_window(workspace.clone(), cx).unwrap());
    second
        .update(cx, |root, _, cx| open(root, info, cx))
        .unwrap();
    cx.run_until_parked();
    let b = active(&workspace, 1, cx);
    assert_ne!(a, b);
    workspace.update(cx, |workspace, cx| {
        workspace
            .edit(
                |state| {
                    state.select_branch(a, "refs/heads/review-a".into())?;
                    state.select_branch(b, "refs/heads/review-b".into())
                },
                cx,
            )
            .unwrap()
    });
    cx.run_until_parked();
    workspace.read_with(cx, |workspace, _| {
        assert_eq!(workspace.state().projects().len(), 1);
        assert_eq!(
            workspace.state().tab(a).unwrap().route.project(),
            workspace.state().tab(b).unwrap().route.project()
        );
        assert_eq!(workspace.state().windows()[0].active_tab, Some(a));
        assert_eq!(workspace.state().windows()[1].active_tab, Some(b));
    });
    first
        .read_with(cx, |root, cx| {
            assert_eq!(
                root.top_bar.read(cx).active_tab().unwrap().title,
                "repo - review-a"
            )
        })
        .unwrap();
    second
        .read_with(cx, |root, cx| {
            assert_eq!(
                root.top_bar.read(cx).active_tab().unwrap().title,
                "repo - review-b"
            )
        })
        .unwrap();
    workspace.update(cx, |workspace, cx| {
        workspace.edit(|state| state.go_home(a), cx).unwrap()
    });
    cx.run_until_parked();
    first
        .read_with(cx, |root, cx| {
            assert_eq!(root.top_bar.read(cx).active_tab().unwrap().title, "Home")
        })
        .unwrap();
    second
        .read_with(cx, |root, cx| {
            assert_eq!(
                root.top_bar.read(cx).active_tab().unwrap().title,
                "repo - review-b"
            )
        })
        .unwrap();
}

#[gpui::test]
fn opening_a_repository_replaces_the_current_route_not_the_tab(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let workspace = shared(cx, dir.path().join("state.json"));
    let handle = cx.update(|cx| Kestrel::new_window(workspace.clone(), cx).unwrap());
    let first = active(&workspace, 0, cx);
    let info = repository(&dir.path().join("one"));
    handle
        .update(cx, |root, _, cx| open(root, info.clone(), cx))
        .unwrap();
    handle
        .update(cx, |root, window, cx| {
            root.top_bar
                .update(cx, |bar, cx| bar.dispatch(Msg::Add, window, cx))
        })
        .unwrap();
    cx.run_until_parked();
    let second = active(&workspace, 0, cx);
    handle
        .update(cx, |root, _, cx| open(root, info, cx))
        .unwrap();
    cx.run_until_parked();
    workspace.read_with(cx, |workspace, _| {
        assert_eq!(workspace.state().windows()[0].tabs.len(), 2);
        assert_eq!(
            workspace.state().tab(first).unwrap().route.project(),
            workspace.state().tab(second).unwrap().route.project()
        );
    });
    let info = repository(&dir.path().join("two"));
    handle
        .update(cx, |root, _, cx| open(root, info, cx))
        .unwrap();
    cx.run_until_parked();
    workspace.read_with(cx, |workspace, _| {
        assert_eq!(workspace.state().windows()[0].tabs.len(), 2);
        assert_eq!(workspace.state().windows()[0].active_tab, Some(second));
        assert_ne!(
            workspace.state().tab(first).unwrap().route.project(),
            workspace.state().tab(second).unwrap().route.project()
        );
    });
}

#[gpui::test]
fn tab_selection_reorder_close_and_detach_update_shared_routes(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let workspace = shared(cx, dir.path().join("state.json"));
    let handle = cx.update(|cx| Kestrel::new_window(workspace.clone(), cx).unwrap());
    let info = repository(&dir.path().join("repo"));
    let mut tabs = Vec::new();
    for i in 0..3 {
        if i > 0 {
            handle
                .update(cx, |root, window, cx| {
                    root.top_bar
                        .update(cx, |bar, cx| bar.dispatch(Msg::Add, window, cx))
                })
                .unwrap();
            cx.run_until_parked();
        }
        handle
            .update(cx, |root, _, cx| open(root, info.clone(), cx))
            .unwrap();
        cx.run_until_parked();
        tabs.push(active(&workspace, 0, cx));
    }
    let [a, b, c] = tabs[..] else { panic!() };
    handle
        .update(cx, |root, window, cx| {
            root.top_bar.update(cx, |bar, cx| {
                bar.dispatch(Msg::Select(a.as_u64()), window, cx)
            })
        })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(active(&workspace, 0, cx), a);
    handle
        .update(cx, |root, window, cx| {
            let bar = root.top_bar.clone();
            root.handle_window_command(
                &bar,
                &WindowCommand::LayoutChanged {
                    tabs: vec![c.as_u64(), a.as_u64(), b.as_u64()],
                    active: Some(a.as_u64()),
                },
                window,
                cx,
            );
        })
        .unwrap();
    cx.run_until_parked();
    handle
        .update(cx, |root, _, cx| root.detach(a.as_u64(), cx).unwrap())
        .unwrap();
    cx.run_until_parked();
    workspace.read_with(cx, |workspace, _| {
        assert_eq!(workspace.state().windows().len(), 2);
        assert_eq!(
            workspace.state().windows()[0]
                .tabs
                .iter()
                .map(|tab| tab.id)
                .collect::<Vec<_>>(),
            [c, b]
        );
        assert_eq!(workspace.state().windows()[1].tabs[0].id, a);
    });
    handle
        .update(cx, |root, window, cx| {
            root.top_bar.update(cx, |bar, cx| {
                bar.dispatch(Msg::Close(b.as_u64()), window, cx)
            })
        })
        .unwrap();
    cx.run_until_parked();
    workspace.read_with(cx, |workspace, _| {
        assert_eq!(workspace.state().windows()[0].tabs.len(), 1);
        assert_eq!(workspace.state().windows()[0].tabs[0].id, c);
        assert_eq!(workspace.state().windows()[1].tabs[0].id, a);
        assert_eq!(workspace.state().projects().len(), 1);
    });
}

#[gpui::test]
fn native_close_removes_tabs_and_rejects_late_open_results(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let workspace = shared(cx, dir.path().join("state.json"));
    let handle = cx.update(|cx| Kestrel::new_window(workspace.clone(), cx).unwrap());
    let info = repository(&dir.path().join("repo"));
    handle
        .update(cx, |root, _, cx| open(root, info, cx))
        .unwrap();
    cx.run_until_parked();
    let root = handle.root(cx).unwrap();
    let target = root.read_with(cx, |root, cx| root.open_target(cx).unwrap());
    handle
        .update(cx, |_, window, _| window.remove_window())
        .unwrap();
    cx.run_until_parked();
    let late = repository(&dir.path().join("late"));
    root.update(cx, |root, cx| root.open_repository(target, late, cx));
    workspace.read_with(cx, |workspace, _| {
        assert!(workspace.state().windows().is_empty());
        assert_eq!(workspace.state().projects().len(), 1);
    });
}

fn detach_last(cx: &mut TestAppContext, repository_route: bool) {
    let dir = tempfile::tempdir().unwrap();
    let workspace = shared(cx, dir.path().join("state.json"));
    let handle = cx.update(|cx| Kestrel::new_window(workspace.clone(), cx).unwrap());
    if repository_route {
        let info = repository(&dir.path().join("repo"));
        handle
            .update(cx, |root, _, cx| open(root, info, cx))
            .unwrap();
    }
    cx.run_until_parked();
    let before = workspace.read_with(cx, |workspace, _| {
        workspace.state().windows()[0].tabs[0].clone()
    });
    handle
        .update(cx, |root, window, cx| {
            let token = crate::top_bar::model::DragToken {
                source: root.top_bar.entity_id().as_u64(),
                serial: 1,
            };
            root.top_bar.update(cx, |bar, cx| {
                bar.dispatch(
                    Msg::DragStarted {
                        tab: before.id.as_u64(),
                        token,
                    },
                    window,
                    cx,
                );
                bar.dispatch(
                    Msg::Released {
                        token,
                        outside: true,
                    },
                    window,
                    cx,
                );
            });
        })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(cx.windows().len(), 1);
    assert!(cx.windows()[0] != handle.into());
    workspace.read_with(cx, |workspace, _| {
        assert_eq!(workspace.state().windows().len(), 1);
        assert_eq!(workspace.state().windows()[0].tabs, [before]);
    });
}

#[gpui::test]
fn last_repository_tab_detaches_without_recreating_its_route(cx: &mut TestAppContext) {
    detach_last(cx, true);
}

#[gpui::test]
fn last_home_tab_detaches_without_needing_a_repository(cx: &mut TestAppContext) {
    detach_last(cx, false);
}

#[gpui::test]
fn restoration_keeps_home_duplicate_routes_and_missing_repositories(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.json");
    let mut state = AppState::default();
    let available = state
        .register_project(&repository(&dir.path().join("available")))
        .unwrap();
    let missing = state
        .register_project(&repository(&dir.path().join("missing")))
        .unwrap();
    let first = state.new_window().unwrap();
    let home = state.new_tab(first).unwrap();
    let a = state.new_tab(first).unwrap();
    let b = state.new_tab(first).unwrap();
    state.open_project(available, a).unwrap();
    state.open_project(available, b).unwrap();
    state
        .select_branch(a, "refs/heads/review-a".into())
        .unwrap();
    state
        .select_branch(b, "refs/heads/review-b".into())
        .unwrap();
    let second = state.new_window().unwrap();
    let c = state.new_tab(second).unwrap();
    state.open_project(missing, c).unwrap();
    let mut store = Store::new(path.clone());
    store.load().unwrap();
    store.save(&state).unwrap();
    std::fs::remove_dir_all(dir.path().join("missing")).unwrap();
    let workspace = shared(cx, path);
    let first = cx.update(|cx| Kestrel::open(workspace.clone(), first, cx).unwrap());
    let second = cx.update(|cx| Kestrel::open(workspace.clone(), second, cx).unwrap());
    cx.run_until_parked();
    assert_eq!(
        workspace.read_with(cx, |workspace, _| workspace.state().clone()),
        state
    );
    first
        .read_with(cx, |root, cx| {
            assert!(root.repository.data().is_some());
            assert!(!root.repository.is_fetching());
            assert_eq!(
                root.top_bar.read(cx).active_tab().unwrap().title,
                "available - review-b"
            );
        })
        .unwrap();
    second
        .read_with(cx, |root, _| {
            assert!(root.repository.error().is_some());
            assert!(root.repository.data().is_none());
            assert!(!root.repository.is_fetching());
        })
        .unwrap();

    first
        .update(cx, |root, _, cx| {
            let cached = root.repository.data().unwrap().clone();
            root.workspace.update(cx, |workspace, cx| {
                workspace.edit(|state| state.select_tab(home), cx).unwrap();
            });
            root.load_project(cx);
            assert!(root.repository.is_pending());
            assert!(!root.repository.is_fetching());
            assert!(root.repository.data().is_none());

            root.workspace.update(cx, |workspace, cx| {
                workspace.edit(|state| state.select_tab(b), cx).unwrap();
            });
            root.load_project(cx);
            assert_eq!(root.repository.data(), Some(&cached));
            assert!(!root.repository.is_fetching());
        })
        .unwrap();

    repository(&dir.path().join("missing"));
    second
        .update(cx, |root, _, cx| {
            root.selection = None;
            root.load_project(cx);
            assert!(root.repository.is_loading());
            assert!(root.repository.data().is_none());
            assert!(root.repository.error().is_none());
        })
        .unwrap();
    cx.run_until_parked();
    second
        .read_with(cx, |root, _| {
            assert!(root.repository.data().is_some());
            assert!(root.repository.error().is_none());
            assert!(!root.repository.is_fetching());
        })
        .unwrap();
}

#[gpui::test]
fn linked_worktrees_share_a_project_but_can_open_in_separate_tabs(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("repo");
    let repo = git2::Repository::init(&path).unwrap();
    let tree = repo.treebuilder(None).unwrap().write().unwrap();
    let signature = git2::Signature::now("Test", "test@example.com").unwrap();
    repo.commit(
        Some("HEAD"),
        &signature,
        &signature,
        "initial",
        &repo.find_tree(tree).unwrap(),
        &[],
    )
    .unwrap();
    repo.worktree("linked", &dir.path().join("linked"), None)
        .unwrap();
    let workspace = shared(cx, dir.path().join("state.json"));
    let handle = cx.update(|cx| Kestrel::new_window(workspace.clone(), cx).unwrap());
    let info = Repository::open(path).unwrap().info().unwrap();
    handle
        .update(cx, |root, _, cx| open(root, info, cx))
        .unwrap();
    cx.run_until_parked();
    let before = workspace.read_with(cx, |workspace, _| {
        workspace.state().windows()[0].tabs[0].clone()
    });
    handle
        .update(cx, |root, window, cx| {
            root.top_bar
                .update(cx, |bar, cx| bar.dispatch(Msg::Add, window, cx))
        })
        .unwrap();
    cx.run_until_parked();
    let linked = Repository::open(dir.path().join("linked"))
        .unwrap()
        .info()
        .unwrap();
    handle
        .update(cx, |root, _, cx| open(root, linked, cx))
        .unwrap();
    cx.run_until_parked();
    workspace.read_with(cx, |workspace, _| {
        assert_eq!(workspace.state().projects().len(), 1);
        let tabs = &workspace.state().windows()[0].tabs;
        assert_eq!(tabs.len(), 2);
        assert_eq!(tabs[0], before);
        assert_ne!(tabs[0].id, tabs[1].id);
        assert_eq!(tabs[0].route.project(), tabs[1].route.project());
    });
}

#[gpui::test]
fn clean_quit_preserves_home_and_independent_pinned_contexts(cx: &mut TestAppContext) {
    use kestrel::{
        review::{Comparison, Endpoints},
        state::{BaseSelection, Browsing},
    };
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.json");
    let workspace = shared(cx, path.clone());
    let info = repository(&dir.path().join("repo"));
    for _ in 0..2 {
        let handle = cx.update(|cx| Kestrel::new_window(workspace.clone(), cx).unwrap());
        handle
            .update(cx, |root, _, cx| open(root, info.clone(), cx))
            .unwrap();
        cx.run_until_parked();
    }
    workspace.update(cx, |workspace, cx| {
        workspace
            .edit(
                |state| {
                    for i in 0..2 {
                        let tab = state.windows()[i].active_tab.unwrap();
                        let branch = RefName::from("refs/heads/review");
                        state.select_branch(tab, branch.clone())?;
                        state.set_comparison(
                            tab,
                            BaseSelection::Default,
                            Comparison {
                                branch: branch.clone(),
                                base: "refs/heads/main".into(),
                                base_tip: "1111111111111111111111111111111111111111"
                                    .parse()
                                    .unwrap(),
                                commits: Endpoints {
                                    old: "2222222222222222222222222222222222222222"
                                        .parse()
                                        .unwrap(),
                                    new: format!("{:040x}", i + 3).parse().unwrap(),
                                },
                            },
                        )?;
                        state.set_browsing(
                            tab,
                            &branch,
                            Browsing {
                                first_visible_line: (i * 75) as u32,
                                ..Default::default()
                            },
                        )?;
                    }
                    let first = state.windows()[0].tabs[0].id;
                    state.go_home(first)?;
                    state.go_back(first)?;
                    state.new_tab(state.windows()[0].id)?;
                    Ok(())
                },
                cx,
            )
            .unwrap()
    });
    let before = workspace.read_with(cx, |workspace, _| workspace.state().clone());
    cx.quit();
    assert!(cx.windows().is_empty());
    assert_eq!(Store::new(path).load().unwrap().state, before);
    assert_eq!(
        workspace.read_with(cx, |workspace, _| workspace.state().clone()),
        before
    );
}

#[gpui::test]
fn delayed_open_navigates_its_origin_tab_without_stealing_selection(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let workspace = shared(cx, dir.path().join("state.json"));
    let handle = cx.update(|cx| Kestrel::new_window(workspace.clone(), cx).unwrap());
    let target = handle
        .update(cx, |root, _, cx| {
            let target = root.open_target(cx).unwrap();
            root.opening = Some(target.clone());
            target
        })
        .unwrap();
    handle
        .update(cx, |root, window, cx| {
            root.top_bar
                .update(cx, |bar, cx| bar.dispatch(Msg::Add, window, cx))
        })
        .unwrap();
    cx.run_until_parked();
    let selected = active(&workspace, 0, cx);
    let info = repository(&dir.path().join("repo"));
    handle
        .update(cx, |root, _, cx| {
            root.finish_open(target.clone(), Ok(Some(info)), cx)
        })
        .unwrap();
    cx.run_until_parked();
    workspace.read_with(cx, |workspace, _| {
        assert_eq!(workspace.state().windows()[0].active_tab, Some(selected));
        assert_eq!(workspace.state().tab(selected).unwrap().route, Route::Home);
        assert!(
            workspace
                .state()
                .tab(target.tab)
                .unwrap()
                .route
                .project()
                .is_some()
        );
    });
}

#[gpui::test]
fn closing_the_origin_tab_discards_its_pending_open(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let workspace = shared(cx, dir.path().join("state.json"));
    let handle = cx.update(|cx| Kestrel::new_window(workspace.clone(), cx).unwrap());
    let target = handle
        .update(cx, |root, window, cx| {
            let target = root.open_target(cx).unwrap();
            root.opening = Some(target.clone());
            root.top_bar
                .update(cx, |bar, cx| bar.dispatch(Msg::Add, window, cx));
            target
        })
        .unwrap();
    cx.run_until_parked();
    workspace.update(cx, |workspace, cx| {
        workspace
            .edit(
                |state| {
                    state.close_tab(target.tab);
                    Ok(())
                },
                cx,
            )
            .unwrap()
    });
    cx.run_until_parked();
    let late = repository(&dir.path().join("late"));
    handle
        .update(cx, |root, _, cx| {
            root.finish_open(target, Ok(Some(late)), cx)
        })
        .unwrap();
    cx.run_until_parked();
    workspace.read_with(cx, |workspace, _| {
        assert_eq!(workspace.state().windows()[0].tabs.len(), 1);
        assert_eq!(workspace.state().windows()[0].tabs[0].route, Route::Home);
        assert!(workspace.state().projects().is_empty());
    });
}

#[gpui::test]
fn closing_the_last_home_tab_closes_only_its_window(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let workspace = shared(cx, dir.path().join("state.json"));
    let first = cx.update(|cx| Kestrel::new_window(workspace.clone(), cx).unwrap());
    let second = cx.update(|cx| Kestrel::new_window(workspace.clone(), cx).unwrap());
    first
        .update(cx, |root, window, cx| {
            root.top_bar
                .update(cx, |bar, cx| bar.dispatch(Msg::CloseActive, window, cx))
        })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(cx.windows().len(), 1);
    assert!(cx.windows()[0] == second.into());
    workspace.read_with(cx, |workspace, _| {
        assert_eq!(workspace.state().windows().len(), 1);
        assert_eq!(workspace.state().windows()[0].tabs[0].route, Route::Home);
    });
}

#[gpui::test]
fn pending_open_tracks_activation_not_view_edits_or_route_equality(cx: &mut TestAppContext) {
    for traverse in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let workspace = shared(cx, dir.path().join("state.json"));
        let handle = cx.update(|cx| Kestrel::new_window(workspace.clone(), cx).unwrap());
        let known = repository(&dir.path().join("known"));
        let late = repository(&dir.path().join("late"));
        handle
            .update(cx, |root, _, cx| open(root, known, cx))
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |root, _, cx| {
                let target = root.open_target(cx).unwrap();
                root.opening = Some(target.clone());
                workspace.update(cx, |workspace, cx| {
                    workspace
                        .edit(
                            |state| {
                                state.set_branch_list(
                                    target.tab,
                                    kestrel::state::BranchList {
                                        query: "updated filter".into(),
                                        ..Default::default()
                                    },
                                )?;
                                if traverse {
                                    state.go_home(target.tab)?;
                                    state.go_back(target.tab)?;
                                }
                                Ok(())
                            },
                            cx,
                        )
                        .unwrap();
                });
                let before = workspace.read(cx).state().clone();
                // Deliver before observers can cancel the request or rebuild the view.
                root.finish_open(target, Ok(Some(late)), cx);
                let after = workspace.read(cx).state();
                if traverse {
                    assert_eq!(after, &before);
                } else {
                    assert_eq!(after.projects().len(), 2);
                    assert_eq!(
                        after.windows()[0].tabs[0].route.project(),
                        Some(after.projects()[1].id)
                    );
                }
            })
            .unwrap();
    }
}

#[gpui::test]
fn navigating_away_and_back_cancels_a_pending_open(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let workspace = shared(cx, dir.path().join("state.json"));
    let handle = cx.update(|cx| Kestrel::new_window(workspace.clone(), cx).unwrap());
    let known = repository(&dir.path().join("known"));
    let project = workspace.update(cx, |workspace, cx| {
        workspace
            .edit(|state| state.register_project(&known), cx)
            .unwrap()
    });
    let target = handle
        .update(cx, |root, _, cx| {
            let target = root.open_target(cx).unwrap();
            root.opening = Some(target.clone());
            root.open_saved(project, cx);
            target
        })
        .unwrap();
    cx.run_until_parked();
    workspace.update(cx, |workspace, cx| {
        workspace
            .edit(|state| state.go_home(target.tab), cx)
            .unwrap()
    });
    cx.run_until_parked();
    let late = repository(&dir.path().join("late"));
    handle
        .update(cx, |root, _, cx| {
            root.finish_open(target, Ok(Some(late)), cx)
        })
        .unwrap();
    cx.run_until_parked();
    workspace.read_with(cx, |workspace, _| {
        assert_eq!(workspace.state().windows()[0].tabs[0].route, Route::Home);
        assert_eq!(workspace.state().projects().len(), 1);
    });
}
