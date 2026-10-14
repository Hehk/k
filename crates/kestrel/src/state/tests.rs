use super::*;
use crate::review::{Endpoints, ObjectId};

fn info(name: &str, branch: &str) -> RepositoryInfo {
    let directory = std::env::temp_dir().join(name);
    RepositoryInfo {
        common_dir: directory.join(".git"),
        directory,
        current_branch: Some(RefName::from(branch)),
    }
}

fn comparison(branch: &str) -> Comparison {
    let id: ObjectId = "1234567890123456789012345678901234567890".parse().unwrap();
    Comparison {
        branch: RefName::from(branch),
        base: RefName::from("refs/heads/main"),
        base_tip: id,
        commits: Endpoints { old: id, new: id },
    }
}

fn context(state: &AppState, tab: TabId) -> &ReviewContext {
    state.tab(tab).unwrap().route.review().unwrap()
}

fn open(state: &mut AppState, project: ProjectId, window: WindowId) -> TabId {
    let tab = state.new_tab(window).unwrap();
    state.open_project(project, tab).unwrap();
    tab
}

fn assert_round_trip(state: &AppState) {
    let json = serde_json::to_vec(state).unwrap();
    let restored: AppState = serde_json::from_slice(&json).unwrap();
    assert_eq!(&restored, state);
}

#[test]
fn new_tabs_are_home_routes_with_no_repository_side_effects() {
    let mut state = AppState::default();
    let window = state.new_window().unwrap();
    let a = state.new_tab(window).unwrap();
    let b = state.new_tab(window).unwrap();
    assert_ne!(a, b);
    assert_eq!(state.tab(a).unwrap().route, Route::Home);
    assert_eq!(state.tab(b).unwrap().route, Route::Home);
    assert_eq!(state.window(window).unwrap().active_tab, Some(b));
    assert!(state.projects().is_empty());
    assert_round_trip(&state);
}

#[test]
fn history_moves_both_ways_and_new_destinations_discard_the_forward_stack() {
    let mut state = AppState::default();
    let a = state.register_project(&info("a", "refs/heads/a")).unwrap();
    let b = state.register_project(&info("b", "refs/heads/b")).unwrap();
    let c = state.register_project(&info("c", "refs/heads/c")).unwrap();
    let window = state.new_window().unwrap();
    let tab = state.new_tab(window).unwrap();
    let start = state.clone();
    state.go_back(tab).unwrap();
    state.go_forward(tab).unwrap();
    assert_eq!(state, start);
    state.open_project(a, tab).unwrap();
    state.open_project(b, tab).unwrap();
    state.go_back(tab).unwrap();
    assert_eq!(state.tab(tab).unwrap().route.project(), Some(a));
    let before = state.clone();
    state.open_project(a, tab).unwrap();
    assert_eq!(state, before);
    state.go_back(tab).unwrap();
    assert_eq!(state.tab(tab).unwrap().route, Route::Home);
    assert!(!state.tab(tab).unwrap().can_go_back());
    assert!(state.tab(tab).unwrap().can_go_forward());
    let before = state.clone();
    state.go_home(tab).unwrap();
    state.go_back(tab).unwrap();
    assert_eq!(state, before);
    state.go_forward(tab).unwrap();
    state.go_forward(tab).unwrap();
    assert_eq!(state.tab(tab).unwrap().route.project(), Some(b));
    let before = state.clone();
    state.go_forward(tab).unwrap();
    assert_eq!(state, before);
    state.go_back(tab).unwrap();
    state.open_project(c, tab).unwrap();
    assert!(!state.tab(tab).unwrap().can_go_forward());
    assert_eq!(state.tab(tab).unwrap().back.len(), 2);
    state.go_back(tab).unwrap();
    assert_eq!(state.tab(tab).unwrap().route.project(), Some(a));
    assert_round_trip(&state);
}

#[test]
fn history_preserves_exact_reviews_independently_and_moves_with_the_tab() {
    let mut state = AppState::default();
    let a = state.register_project(&info("a", "refs/heads/a")).unwrap();
    let b = state.register_project(&info("b", "refs/heads/b")).unwrap();
    let window = state.new_window().unwrap();
    let tab = open(&mut state, a, window);
    state
        .set_comparison(tab, BaseSelection::Default, comparison("refs/heads/a"))
        .unwrap();
    state
        .set_browsing(
            tab,
            &"refs/heads/a".into(),
            Browsing {
                file: Some("saved.rs".into()),
                mode: ReadingMode::HeadFile,
                first_visible_line: 80,
                ..Default::default()
            },
        )
        .unwrap();
    let expected = state.tab(tab).unwrap().route.clone();
    state.open_project(b, tab).unwrap();
    let sibling = open(&mut state, a, window);
    state
        .select_branch(sibling, "refs/heads/new-selection".into())
        .unwrap();
    let sibling_before = state.tab(sibling).unwrap().clone();
    state.go_back(tab).unwrap();
    assert_eq!(state.tab(tab).unwrap().route, expected);
    assert_eq!(state.tab(sibling).unwrap(), &sibling_before);
    assert_eq!(state.window(window).unwrap().active_tab, Some(sibling));
    let saved = state.tab(tab).unwrap().clone();
    let target = state.new_window().unwrap();
    state.move_tab(tab, target, 0).unwrap();
    assert_eq!(state.tab(tab).unwrap(), &saved);
    assert_round_trip(&state);
    let mut restored: AppState =
        serde_json::from_slice(&serde_json::to_vec(&state).unwrap()).unwrap();
    restored.go_forward(tab).unwrap();
    assert_eq!(restored.tab(tab).unwrap().route.project(), Some(b));
    restored.go_back(tab).unwrap();
    assert_eq!(restored.tab(tab).unwrap().route, expected);
}

#[test]
fn inserting_a_tab_validates_its_entire_history_before_mutating_state() {
    for forward in [false, true] {
        let mut source = AppState::default();
        let project = source.register_project(&info("a", "refs/heads/a")).unwrap();
        let window = source.new_window().unwrap();
        let tab = open(&mut source, project, window);
        source.go_home(tab).unwrap();
        if forward {
            source.go_back(tab).unwrap();
            source.go_back(tab).unwrap();
        }
        let tab = source.tab(tab).unwrap().clone();
        assert_eq!(tab.route, Route::Home);
        let mut target = AppState::default();
        let window = target.new_window().unwrap();
        let before = target.clone();
        assert!(target.insert_tab(window, 0, tab).is_err());
        assert_eq!(target, before);
    }
}

#[test]
fn branch_lists_restore_filters_and_remember_pins_without_changing_other_tabs() {
    let mut state = AppState::default();
    let project = state
        .register_project(&info("repo", "refs/heads/a"))
        .unwrap();
    let window = state.new_window().unwrap();
    let tab = open(&mut state, project, window);
    let other = open(&mut state, project, window);
    let other_before = state.tab(other).unwrap().clone();
    let filters = BranchList {
        query: "topic".into(),
        sort: BranchSort::Name,
    };
    state.set_branch_list(tab, filters.clone()).unwrap();
    state.select_branch(tab, "refs/heads/topic".into()).unwrap();
    let pinned = comparison("refs/heads/topic");
    state
        .set_comparison(tab, BaseSelection::Default, pinned.clone())
        .unwrap();
    state
        .set_browsing(
            tab,
            &"refs/heads/topic".into(),
            Browsing {
                file: Some("saved.rs".into()),
                first_visible_line: 75,
                ..Default::default()
            },
        )
        .unwrap();
    state.go_back(tab).unwrap();
    let Route::Repository {
        page,
        branches,
        review,
        ..
    } = &state.tab(tab).unwrap().route
    else {
        panic!()
    };
    assert_eq!(*page, RepositoryPage::Branches);
    assert_eq!(branches, &filters);
    assert_eq!(
        review.active_review().unwrap().comparison,
        Some(pinned.clone())
    );
    state.go_forward(tab).unwrap();
    assert_eq!(
        context(&state, tab)
            .active_review()
            .unwrap()
            .browsing
            .first_visible_line,
        75
    );
    state.go_back(tab).unwrap();
    state.select_branch(tab, "refs/heads/topic".into()).unwrap();
    assert_eq!(
        context(&state, tab).active_review().unwrap().comparison,
        Some(pinned)
    );
    assert_eq!(state.tab(other).unwrap(), &other_before);
    let target = state.new_window().unwrap();
    let saved = state.tab(tab).unwrap().clone();
    state.move_tab(tab, target, 0).unwrap();
    assert_eq!(state.tab(tab).unwrap(), &saved);
    assert_round_trip(&state);
}

#[test]
fn comparison_pages_require_a_selected_branch_when_loading() {
    let mut state = AppState::default();
    let project = state
        .register_project(&RepositoryInfo {
            current_branch: None,
            ..info("empty", "refs/heads/main")
        })
        .unwrap();
    let window = state.new_window().unwrap();
    open(&mut state, project, window);
    let mut json = serde_json::to_value(state).unwrap();
    json["windows"][0]["tabs"][0]["route"]["Repository"]["page"] = serde_json::json!("Comparison");
    assert!(serde_json::from_value::<AppState>(json).is_err());
}

#[test]
fn saved_tabs_without_history_start_with_empty_stacks() {
    let mut state = AppState::default();
    let project = state.register_project(&info("a", "refs/heads/a")).unwrap();
    let window = state.new_window().unwrap();
    let tab = open(&mut state, project, window);
    let mut json = serde_json::to_value(&state).unwrap();
    let saved = json["windows"][0]["tabs"][0].as_object_mut().unwrap();
    saved.remove("back");
    saved.remove("forward");
    let restored: AppState = serde_json::from_value(json).unwrap();
    assert_eq!(
        restored.tab(tab).unwrap().route,
        state.tab(tab).unwrap().route
    );
    assert!(!restored.tab(tab).unwrap().can_go_back());
    assert!(!restored.tab(tab).unwrap().can_go_forward());
}

#[test]
fn deserialization_checks_history_routes_as_well_as_the_current_route() {
    let mut state = AppState::default();
    let project = state.register_project(&info("a", "refs/heads/a")).unwrap();
    let window = state.new_window().unwrap();
    let tab = open(&mut state, project, window);
    let route = serde_json::to_value(&state.tab(tab).unwrap().route).unwrap();
    for stack in ["back", "forward"] {
        for bad_path in [false, true] {
            let mut route = route.clone();
            if bad_path {
                route["Repository"]["review"]["reviews"][0]["browsing"]["file"] =
                    serde_json::to_value(GitPath::from("../outside")).unwrap();
            } else {
                route["Repository"]["project"] = serde_json::json!(999);
            }
            let mut json = serde_json::to_value(&state).unwrap();
            json["windows"][0]["tabs"][0][stack] = serde_json::json!([route]);
            assert!(serde_json::from_value::<AppState>(json).is_err());
        }
    }
}

#[test]
fn focusing_a_window_changes_only_focus_and_rejects_closed_windows() {
    let mut state = AppState::default();
    let first = state.new_window().unwrap();
    let second = state.new_window().unwrap();
    state.focus_window(first).unwrap();
    assert_eq!(state.active_window(), Some(first));
    state.close_window(second);
    let before = state.clone();
    assert!(state.focus_window(second).is_err());
    assert_eq!(state, before);
    assert_round_trip(&state);
}

#[test]
fn saved_state_keeps_the_existing_flat_json_shape() {
    let saved =
        serde_json::json!({ "projects": [], "windows": [], "active_window": null, "next_id": 1 });
    let state: AppState = serde_json::from_value(saved.clone()).unwrap();
    assert_eq!(state, AppState::default());
    assert_eq!(serde_json::to_value(&state).unwrap(), saved);
}

#[test]
fn branches_keep_their_comparison_base_and_reading_position() {
    let mut state = AppState::default();
    let project = state
        .register_project(&info("one", "refs/heads/a"))
        .unwrap();
    let window = state.new_window().unwrap();
    let tab = open(&mut state, project, window);
    state
        .set_comparison(
            tab,
            BaseSelection::Explicit(RefName::from("refs/heads/main")),
            comparison("refs/heads/a"),
        )
        .unwrap();
    state
        .set_browsing(
            tab,
            &RefName::from("refs/heads/a"),
            Browsing {
                file: Some("src/file.rs".into()),
                directory: Some("src".into()),
                mode: ReadingMode::HeadFile,
                first_visible_line: 82,
            },
        )
        .unwrap();
    let expected = context(&state, tab).active_review().unwrap().clone();
    state
        .select_branch(tab, RefName::from("refs/heads/b"))
        .unwrap();
    assert!(
        context(&state, tab)
            .active_review()
            .unwrap()
            .comparison
            .is_none()
    );
    state
        .set_comparison(tab, BaseSelection::Default, comparison("refs/heads/b"))
        .unwrap();
    state
        .select_branch(tab, RefName::from("refs/heads/a"))
        .unwrap();
    assert_eq!(context(&state, tab).active_review(), Some(&expected));
    assert_round_trip(&state);
}

#[test]
fn duplicate_repository_routes_have_independent_branches_comparisons_and_browsing() {
    let mut state = AppState::default();
    let project = state
        .register_project(&info("repo", "refs/heads/a"))
        .unwrap();
    let window = state.new_window().unwrap();
    let a = open(&mut state, project, window);
    let b = open(&mut state, project, window);
    let other = state.new_window().unwrap();
    let c = open(&mut state, project, other);
    let before_b = state.tab(b).unwrap().clone();
    let before_c = state.tab(c).unwrap().clone();
    state
        .set_comparison(a, BaseSelection::Default, comparison("refs/heads/a"))
        .unwrap();
    state
        .set_browsing(
            a,
            &RefName::from("refs/heads/a"),
            Browsing {
                file: Some("a.rs".into()),
                first_visible_line: 42,
                ..Default::default()
            },
        )
        .unwrap();
    state
        .select_branch(a, RefName::from("refs/heads/b"))
        .unwrap();
    assert_eq!(state.tab(b).unwrap(), &before_b);
    assert_eq!(state.tab(c).unwrap(), &before_c);
    state.go_home(a).unwrap();
    assert_eq!(state.tab(a).unwrap().route, Route::Home);
    assert_eq!(state.tab(b).unwrap(), &before_b);
    assert_eq!(state.projects().len(), 1);
    assert_round_trip(&state);
}

#[test]
fn opening_a_repository_navigates_only_the_target_tab() {
    let mut state = AppState::default();
    let a = state.register_project(&info("a", "refs/heads/a")).unwrap();
    let b = state.register_project(&info("b", "refs/heads/b")).unwrap();
    let first = state.new_window().unwrap();
    let tab = open(&mut state, a, first);
    let other = state.new_window().unwrap();
    let other_tab = open(&mut state, a, other);
    let before = state.window(other).unwrap().clone();
    state.open_project(b, tab).unwrap();
    assert_eq!(state.window(first).unwrap().tabs.len(), 1);
    assert_eq!(state.tab(tab).unwrap().route.project(), Some(b));
    assert_eq!(state.tab(other_tab).unwrap().route.project(), Some(a));
    assert_eq!(state.window(other).unwrap(), &before);
    assert_eq!(state.active_window(), Some(other));
    assert_round_trip(&state);
}

#[test]
fn worktrees_share_metadata_not_tab_identity_or_live_navigation() {
    let mut state = AppState::default();
    let main = info("repo", "refs/heads/a");
    let mut linked = main.clone();
    linked.directory = std::env::temp_dir().join("linked");
    linked.current_branch = Some(RefName::from("refs/heads/b"));
    let project = state.register_project(&main).unwrap();
    let window = state.new_window().unwrap();
    let a = open(&mut state, project, window);
    let same_project = state.register_project(&linked).unwrap();
    let b = open(&mut state, same_project, window);
    assert_eq!(same_project, project);
    assert_ne!(a, b);
    assert_eq!(state.projects().len(), 1);
    assert_eq!(context(&state, a).selected_branch, main.current_branch);
    assert_round_trip(&state);
}

#[test]
fn closing_a_tab_retains_a_reopening_snapshot_without_closing_other_routes() {
    let mut state = AppState::default();
    let project = state
        .register_project(&info("repo", "refs/heads/a"))
        .unwrap();
    let window = state.new_window().unwrap();
    let a = open(&mut state, project, window);
    let b = open(&mut state, project, window);
    let before_b = state.tab(b).unwrap().clone();
    state
        .select_branch(a, RefName::from("refs/heads/review"))
        .unwrap();
    let saved = context(&state, a).clone();
    state.close_tab(a);
    assert_eq!(state.tab(b).unwrap(), &before_b);
    let reopened = open(&mut state, project, window);
    assert_ne!(a, reopened);
    assert_eq!(context(&state, reopened), &saved);
    state.close_window(window);
    assert!(state.windows().is_empty());
    assert_eq!(state.project(project).unwrap().last_review, saved);
    assert_round_trip(&state);
}

#[test]
fn moving_tabs_preserves_route_ids_and_context_and_only_removes_empty_sources() {
    let mut state = AppState::default();
    let project = state
        .register_project(&info("repo", "refs/heads/a"))
        .unwrap();
    let source = state.new_window().unwrap();
    let a = open(&mut state, project, source);
    let b = open(&mut state, project, source);
    state
        .select_branch(a, RefName::from("refs/heads/different"))
        .unwrap();
    let saved = state.tab(a).unwrap().clone();
    state.move_tab(b, source, 0).unwrap();
    assert_eq!(
        state
            .window(source)
            .unwrap()
            .tabs
            .iter()
            .map(|tab| tab.id)
            .collect::<Vec<_>>(),
        [b, a]
    );
    let empty = state.new_window().unwrap();
    let target = state.new_window().unwrap();
    state.move_tab(a, target, 0).unwrap();
    state.move_tab(b, target, 1).unwrap();
    assert_eq!(state.tab(a).unwrap(), &saved);
    assert!(state.window(source).is_err());
    assert!(state.window(empty).is_ok());
    state.close_tab(a);
    assert!(state.window(empty).is_ok());
    assert_eq!(state.active_window(), Some(target));
    assert_round_trip(&state);
}

#[test]
fn whole_model_round_trips_without_resolving_repositories() {
    let mut state = AppState::default();
    let project = state
        .register_project(&info("nonexistent-repository", "refs/heads/a"))
        .unwrap();
    let window = state.new_window().unwrap();
    let tab = open(&mut state, project, window);
    state
        .set_comparison(tab, BaseSelection::Default, comparison("refs/heads/a"))
        .unwrap();
    state
        .set_browsing(
            tab,
            &RefName::from("refs/heads/a"),
            Browsing {
                file: Some(GitPath(b"unusual\n\xff".to_vec())),
                ..Browsing::default()
            },
        )
        .unwrap();
    state.new_tab(window).unwrap();
    assert_round_trip(&state);
}

#[cfg(unix)]
#[test]
fn non_utf8_local_paths_round_trip_losslessly() {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};
    let mut state = AppState::default();
    let mut repository = info("one", "refs/heads/a");
    repository
        .directory
        .push(OsString::from_vec(b"raw\xff".to_vec()));
    repository.common_dir = repository.directory.join(".git");
    state.register_project(&repository).unwrap();
    assert_round_trip(&state);
}

#[test]
fn malformed_routes_and_contexts_are_rejected_during_deserialization() {
    let mut state = AppState::default();
    let project = state
        .register_project(&info("one", "refs/heads/a"))
        .unwrap();
    let window = state.new_window().unwrap();
    let tab = open(&mut state, project, window);
    state
        .set_comparison(tab, BaseSelection::Default, comparison("refs/heads/a"))
        .unwrap();
    let json = serde_json::to_value(&state).unwrap();
    let mut bad = json.clone();
    bad["windows"][0]["tabs"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::to_value(state.tab(tab).unwrap()).unwrap());
    assert!(serde_json::from_value::<AppState>(bad).is_err());
    for (pointer, value) in [
        ("/next_id", serde_json::json!(1)),
        ("/windows/0/active_tab", serde_json::json!(999)),
        (
            "/windows/0/tabs/0/route/Repository/project",
            serde_json::json!(999),
        ),
        (
            "/windows/0/tabs/0/route/Repository/review/selected_branch",
            serde_json::to_value(RefName::from("refs/heads/missing")).unwrap(),
        ),
        (
            "/windows/0/tabs/0/route/Repository/review/reviews/0/base",
            serde_json::to_value(BaseSelection::Explicit("refs/heads/other".into())).unwrap(),
        ),
        (
            "/windows/0/tabs/0/route/Repository/review/reviews/0/browsing/file",
            serde_json::to_value(GitPath::from("../outside")).unwrap(),
        ),
        (
            "/projects/0/selected_branch",
            serde_json::to_value(RefName::from("refs/heads/missing")).unwrap(),
        ),
    ] {
        let mut bad = json.clone();
        *bad.pointer_mut(pointer).unwrap() = value;
        assert!(
            serde_json::from_value::<AppState>(bad).is_err(),
            "{pointer}"
        );
    }
}

#[test]
fn rejected_transitions_leave_the_model_unchanged() {
    let mut state = AppState::default();
    let project = state
        .register_project(&info("one", "refs/heads/a"))
        .unwrap();
    let window = state.new_window().unwrap();
    let tab = open(&mut state, project, window);
    let home = state.new_tab(window).unwrap();
    let before = state.clone();
    assert!(state.new_tab(WindowId(100)).is_err());
    assert!(state.open_project(project, TabId(100)).is_err());
    assert!(state.open_project(ProjectId(100), home).is_err());
    assert!(state.go_home(TabId(100)).is_err());
    assert!(state.go_back(TabId(100)).is_err());
    assert!(state.go_forward(TabId(100)).is_err());
    assert!(state.select_tab(TabId(100)).is_err());
    assert!(state.move_tab(tab, WindowId(100), 0).is_err());
    assert!(state.select_branch(home, "refs/heads/a".into()).is_err());
    assert!(state.select_branch(tab, "not-a-ref".into()).is_err());
    assert!(state.register_project(&info("bad", "not-a-ref")).is_err());
    assert!(
        state
            .set_comparison(
                tab,
                BaseSelection::Explicit("refs/heads/other".into()),
                comparison("refs/heads/a")
            )
            .is_err()
    );
    assert!(
        state
            .set_comparison(
                tab,
                BaseSelection::Default,
                comparison("refs/heads/unknown")
            )
            .is_err()
    );
    assert!(
        state
            .set_browsing(
                tab,
                &"refs/heads/a".into(),
                Browsing {
                    file: Some("../outside".into()),
                    ..Default::default()
                }
            )
            .is_err()
    );
    assert_eq!(state, before);
    assert_round_trip(&state);
}

#[test]
fn generated_transitions_preserve_selections_identities_and_serializability() {
    for seed in 0..16_u64 {
        let mut random = seed;
        let mut state = AppState::default();
        let projects = (0..3)
            .map(|i| {
                state
                    .register_project(&info(&format!("repo-{i}"), "refs/heads/a"))
                    .unwrap()
            })
            .collect::<Vec<_>>();
        for step in 0..256 {
            random = random.wrapping_mul(6364136223846793005).wrapping_add(1);
            if state.windows().is_empty() {
                state.new_window().unwrap();
            }
            let window = state.windows()[(random >> 16) as usize % state.windows().len()].id;
            if state.window(window).unwrap().tabs.is_empty() {
                state.new_tab(window).unwrap();
            }
            let tab = state.window(window).unwrap().tabs[0].id;
            let project = projects[(random >> 32) as usize % projects.len()];
            match random % 11 {
                0 => {
                    state.new_window().unwrap();
                }
                1 => {
                    state.new_tab(window).unwrap();
                }
                2 => {
                    state.open_project(project, tab).unwrap();
                }
                3 => state.close_tab(tab),
                4 => state.close_window(window),
                5 => {
                    let target = state.windows().last().unwrap().id;
                    state.move_tab(tab, target, step % 4).unwrap();
                }
                6 => {
                    state.go_home(tab).unwrap();
                }
                7 => {
                    state.select_tab(tab).unwrap();
                }
                8 => state.go_back(tab).unwrap(),
                9 => state.go_forward(tab).unwrap(),
                _ => {
                    state.open_project(project, tab).unwrap();
                    let name = format!("refs/heads/branch-{}", step % 3);
                    state
                        .select_branch(tab, RefName::from(name.as_str()))
                        .unwrap();
                    state
                        .set_comparison(tab, BaseSelection::Default, comparison(&name))
                        .unwrap();
                    state
                        .set_browsing(
                            tab,
                            &RefName::from(name.as_str()),
                            Browsing {
                                file: Some("file.rs".into()),
                                first_visible_line: step as u32,
                                ..Default::default()
                            },
                        )
                        .unwrap();
                }
            }
            let mut open = HashSet::new();
            for window in state.windows() {
                for tab in &window.tabs {
                    assert!(open.insert(tab.id));
                    if let Some(project) = tab.route.project() {
                        assert!(projects.contains(&project));
                    }
                }
                assert_eq!(window.active_tab.is_none(), window.tabs.is_empty());
                if let Some(active) = window.active_tab {
                    assert!(window.tabs.iter().any(|tab| tab.id == active));
                }
            }
            assert_round_trip(&state);
        }
    }
}

#[test]
fn abbreviated_object_ids_cannot_enter_saved_comparisons() {
    assert!(serde_json::from_str::<ObjectId>("\"1234\"").is_err());
    assert!(
        serde_json::from_str::<ObjectId>("\"zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz\"").is_err()
    );
}
