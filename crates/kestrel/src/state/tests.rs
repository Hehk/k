use super::*;
use crate::review::{Endpoints, ObjectId};

fn info(name: &str, current_branch: &str) -> RepositoryInfo {
    let directory = std::env::temp_dir().join(name);
    RepositoryInfo {
        common_dir: directory.join(".git"),
        directory,
        current_branch: Some(RefName::from(current_branch)),
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

fn assert_round_trip(state: &AppState) {
    let json = serde_json::to_vec(state).unwrap();
    let restored: AppState = serde_json::from_slice(&json).unwrap();
    assert_eq!(&restored, state);
}

#[test]
fn saved_state_keeps_the_existing_flat_json_shape() {
    let saved = serde_json::json!({
        "projects": [],
        "windows": [],
        "active_window": null,
        "next_id": 1,
    });
    let state: AppState = serde_json::from_value(saved.clone()).unwrap();
    assert_eq!(state, AppState::default());
    assert_eq!(serde_json::to_value(&state).unwrap(), saved);
}

#[test]
fn branches_keep_their_comparison_base_and_reading_position() {
    let mut state = AppState::default();
    let id = state
        .register_project(&info("one", "refs/heads/a"))
        .unwrap();
    state
        .set_comparison(
            id,
            BaseSelection::Explicit(RefName::from("refs/heads/main")),
            comparison("refs/heads/a"),
        )
        .unwrap();
    state
        .set_browsing(
            id,
            &RefName::from("refs/heads/a"),
            Browsing {
                file: Some(GitPath::from("src/file.rs")),
                directory: Some(GitPath::from("src")),
                mode: ReadingMode::HeadFile,
                first_visible_line: 82,
            },
        )
        .unwrap();
    let expected = state.project(id).unwrap().active_review().unwrap().clone();
    state
        .select_branch(id, RefName::from("refs/heads/b"))
        .unwrap();
    let b = state.project(id).unwrap().active_review().unwrap();
    assert!(b.comparison.is_none());
    assert_eq!(b.base, BaseSelection::Default);
    state
        .set_comparison(id, BaseSelection::Default, comparison("refs/heads/b"))
        .unwrap();
    state
        .select_branch(id, RefName::from("refs/heads/a"))
        .unwrap();
    assert_eq!(state.project(id).unwrap().active_review(), Some(&expected));
    assert_round_trip(&state);
}

#[test]
fn worktrees_share_one_project_and_do_not_replace_saved_selection() {
    let mut state = AppState::default();
    let main = info("repo", "refs/heads/a");
    let mut linked = main.clone();
    linked.directory = std::env::temp_dir().join("linked");
    linked.current_branch = Some(RefName::from("refs/heads/b"));
    let project = state.register_project(&main).unwrap();
    assert_eq!(state.register_project(&linked).unwrap(), project);
    assert_eq!(state.projects().len(), 1);
    assert_eq!(
        state.project(project).unwrap().selected_branch,
        main.current_branch
    );
    assert_round_trip(&state);
}

#[test]
fn opening_an_existing_project_focuses_its_window_without_duplicate_tabs() {
    let mut state = AppState::default();
    let id = state
        .register_project(&info("one", "refs/heads/a"))
        .unwrap();
    let first = state.new_window().unwrap();
    state.open_project(id, first).unwrap();
    let second = state.new_window().unwrap();
    assert_eq!(state.open_project(id, second).unwrap(), first);
    assert_eq!(state.active_window(), Some(first));
    assert_eq!(state.windows()[0].tabs, [id]);
    assert!(state.windows()[1].tabs.is_empty());
    assert_round_trip(&state);
}

#[test]
fn closing_and_reopening_retains_context_but_not_closed_tabs() {
    let mut state = AppState::default();
    let one = state
        .register_project(&info("one", "refs/heads/a"))
        .unwrap();
    let two = state
        .register_project(&info("two", "refs/heads/b"))
        .unwrap();
    let first = state.new_window().unwrap();
    state.open_project(one, first).unwrap();
    state.open_project(two, first).unwrap();
    let projects = state.projects().to_vec();
    state.close_project(two);
    assert_eq!(state.windows()[0].tabs, [one]);
    assert_eq!(state.windows()[0].active_tab, Some(one));
    assert_eq!(state.projects(), projects);
    state.close_window(first);
    assert!(state.windows().is_empty());
    assert_eq!(state.active_window(), None);
    assert_eq!(state.projects(), projects);
    let next = state.new_window().unwrap();
    state.open_project(two, next).unwrap();
    assert_eq!(state.projects(), projects);
    assert_eq!(state.windows()[0].tabs, [two]);
    assert_round_trip(&state);
}

#[test]
fn moving_and_reordering_projects_preserves_context_and_removes_only_empty_sources() {
    let mut state = AppState::default();
    let one = state
        .register_project(&info("one", "refs/heads/a"))
        .unwrap();
    let two = state
        .register_project(&info("two", "refs/heads/b"))
        .unwrap();
    let source = state.new_window().unwrap();
    state.open_project(one, source).unwrap();
    state.open_project(two, source).unwrap();
    state.move_project(two, source, 0).unwrap();
    assert_eq!(state.windows()[0].tabs, [two, one]);
    let empty = state.new_window().unwrap();
    let target = state.new_window().unwrap();
    let projects = state.projects().to_vec();
    state.move_project(one, target, 0).unwrap();
    state.move_project(two, target, 1).unwrap();
    assert_eq!(state.projects(), projects);
    assert!(!state.windows().iter().any(|window| window.id == source));
    assert!(state.windows().iter().any(|window| window.id == empty));
    assert_eq!(
        state
            .windows()
            .iter()
            .find(|window| window.id == target)
            .unwrap()
            .tabs,
        [one, two]
    );
    assert_eq!(state.active_window(), Some(target));
    assert_round_trip(&state);
}

#[test]
fn whole_model_round_trips_without_resolving_repositories() {
    let mut state = AppState::default();
    let id = state
        .register_project(&info("nonexistent-kestrel-repository", "refs/heads/a"))
        .unwrap();
    state
        .set_comparison(id, BaseSelection::Default, comparison("refs/heads/a"))
        .unwrap();
    state
        .set_browsing(
            id,
            &RefName::from("refs/heads/a"),
            Browsing {
                file: Some(GitPath(b"unusual\n\xff".to_vec())),
                ..Browsing::default()
            },
        )
        .unwrap();
    let window = state.new_window().unwrap();
    state.open_project(id, window).unwrap();
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
fn malformed_state_is_rejected_during_deserialization() {
    let mut state = AppState::default();
    let project = state
        .register_project(&info("one", "refs/heads/a"))
        .unwrap();
    let window = state.new_window().unwrap();
    state.open_project(project, window).unwrap();
    state
        .set_comparison(project, BaseSelection::Default, comparison("refs/heads/a"))
        .unwrap();
    let json = serde_json::to_value(&state).unwrap();
    let mut bad = json.clone();
    bad["windows"][0]["tabs"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::to_value(project).unwrap());
    assert!(serde_json::from_value::<AppState>(bad).is_err());
    let mut bad = json.clone();
    bad["next_id"] = serde_json::json!(1);
    assert!(serde_json::from_value::<AppState>(bad).is_err());
    let mut bad = json.clone();
    bad["projects"][0]["selected_branch"] =
        serde_json::to_value(RefName::from("refs/heads/missing")).unwrap();
    assert!(serde_json::from_value::<AppState>(bad).is_err());
    let mut bad = json.clone();
    bad["projects"][0]["reviews"][0]["base"] =
        serde_json::to_value(BaseSelection::Explicit(RefName::from("refs/heads/other"))).unwrap();
    assert!(serde_json::from_value::<AppState>(bad).is_err());
    let mut bad = json;
    bad["projects"][0]["reviews"][0]["browsing"]["file"] =
        serde_json::to_value(GitPath::from("../outside")).unwrap();
    assert!(serde_json::from_value::<AppState>(bad).is_err());
}

#[test]
fn rejected_transitions_leave_the_model_unchanged() {
    let mut state = AppState::default();
    let project = state
        .register_project(&info("one", "refs/heads/a"))
        .unwrap();
    let before = state.clone();
    assert!(state.move_project(project, WindowId(100), 0).is_err());
    assert!(
        state
            .select_branch(project, RefName::from("not-a-ref"))
            .is_err()
    );
    assert!(state.register_project(&info("bad", "not-a-ref")).is_err());
    assert!(
        state
            .set_comparison(
                project,
                BaseSelection::Explicit(RefName::from("refs/heads/other")),
                comparison("refs/heads/a")
            )
            .is_err()
    );
    assert!(
        state
            .set_comparison(
                project,
                BaseSelection::Default,
                comparison("refs/heads/unknown")
            )
            .is_err()
    );
    assert!(
        state
            .set_browsing(
                project,
                &RefName::from("refs/heads/a"),
                Browsing {
                    file: Some(GitPath::from("../outside")),
                    ..Browsing::default()
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
        let projects: Vec<_> = (0..3)
            .map(|i| {
                state
                    .register_project(&info(&format!("repo-{i}"), "refs/heads/a"))
                    .unwrap()
            })
            .collect();
        for step in 0..256 {
            random = random.wrapping_mul(6364136223846793005).wrapping_add(1);
            let project = projects[(random >> 32) as usize % projects.len()];
            if state.windows().is_empty() {
                state.new_window().unwrap();
            }
            let window = state.windows()[(random >> 16) as usize % state.windows().len()].id;
            match random % 7 {
                0 => {
                    state.new_window().unwrap();
                }
                1 => {
                    state.open_project(project, window).unwrap();
                }
                2 => state.close_project(project),
                3 => state.close_window(window),
                4 => state.move_project(project, window, step % 4).unwrap(),
                5 => {
                    let name = format!("refs/heads/branch-{}", step % 3);
                    state
                        .select_branch(project, RefName::from(name.as_str()))
                        .unwrap();
                    state
                        .set_comparison(project, BaseSelection::Default, comparison(&name))
                        .unwrap();
                }
                _ => {
                    let branch = state
                        .project(project)
                        .unwrap()
                        .selected_branch
                        .clone()
                        .unwrap();
                    state
                        .set_browsing(
                            project,
                            &branch,
                            Browsing {
                                file: Some(GitPath::from("file.rs")),
                                first_visible_line: step as u32,
                                ..Browsing::default()
                            },
                        )
                        .unwrap();
                }
            }
            let mut open = HashSet::new();
            for window in state.windows() {
                assert!(
                    window
                        .tabs
                        .iter()
                        .all(|tab| projects.contains(tab) && open.insert(*tab))
                );
                assert_eq!(window.active_tab.is_none(), window.tabs.is_empty());
                if let Some(active) = window.active_tab {
                    assert!(window.tabs.contains(&active));
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
