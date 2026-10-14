use super::*;
use crate::{
    git::RepositoryInfo,
    review::Endpoints,
    state::{AppState, ComparisonChange, Navigation, ReadingMode, TabId},
};

fn comparison(head: u8, base: &str) -> Comparison {
    let old = "1111111111111111111111111111111111111111".parse().unwrap();
    let new = format!("{head:040x}").parse().unwrap();
    Comparison {
        branch: "refs/heads/topic".into(),
        base: base.into(),
        base_tip: old,
        commits: Endpoints { old, new },
    }
}

fn setup() -> (AppState, TabId, ProjectId) {
    let mut state = AppState::default();
    let directory = std::env::temp_dir().join("visit-snapshots");
    let project = state
        .register_project(&RepositoryInfo {
            common_dir: directory.join(".git"),
            directory,
            current_branch: None,
        })
        .unwrap();
    let window = state.new_window().unwrap();
    let tab = state.new_tab(window).unwrap();
    state.open_project(project, tab).unwrap();
    state.select_branch(tab, "refs/heads/topic".into()).unwrap();
    (state, tab, project)
}

fn complete(state: &mut AppState, tab: TabId, change: ComparisonChange, comparison: Comparison) {
    state
        .navigate(
            tab,
            Navigation::ComparisonLoaded {
                activation: state.tab(tab).unwrap().activation(),
                change,
                base: BaseSelection::Explicit(comparison.base.clone()),
                comparison,
            },
        )
        .unwrap();
}

fn browsing(state: &mut AppState, tab: TabId, file: &str, line: u32) {
    state
        .set_browsing(
            tab,
            &"refs/heads/topic".into(),
            Browsing {
                file: Some(file.into()),
                mode: ReadingMode::Diff,
                first_visible_line: line,
                ..Default::default()
            },
        )
        .unwrap();
}

#[test]
fn resolution_updates_a_visit_but_new_comparisons_preserve_the_old_visit() {
    let (mut state, tab, _) = setup();
    let pending = state.tab(tab).unwrap().visit_id();
    let main = comparison(2, "refs/heads/main");
    complete(&mut state, tab, ComparisonChange::Resolve, main.clone());
    browsing(&mut state, tab, "one.rs", 75);
    let first = state.tab(tab).unwrap().snapshot().unwrap();
    assert_eq!(first.id, pending);
    let other = comparison(2, "refs/heads/other");
    complete(&mut state, tab, ComparisonChange::ChangeBase, other.clone());
    browsing(&mut state, tab, "two.rs", 12);
    let second = state.tab(tab).unwrap().snapshot().unwrap();
    assert_ne!(first.id, second.id);
    let latest = comparison(3, "refs/heads/other");
    complete(&mut state, tab, ComparisonChange::LoadLatest, latest);
    let third = state.tab(tab).unwrap().snapshot().unwrap();
    state.go_back(tab).unwrap();
    assert_eq!(state.tab(tab).unwrap().snapshot().unwrap(), second);
    state.go_back(tab).unwrap();
    assert_eq!(state.tab(tab).unwrap().snapshot().unwrap(), first);
    state.go_forward(tab).unwrap();
    state.go_forward(tab).unwrap();
    assert_eq!(state.tab(tab).unwrap().snapshot().unwrap(), third);
}

#[test]
fn same_comparison_and_view_changes_do_not_discard_forward() {
    let (mut state, tab, _) = setup();
    let first = comparison(2, "refs/heads/main");
    complete(&mut state, tab, ComparisonChange::Resolve, first.clone());
    complete(
        &mut state,
        tab,
        ComparisonChange::LoadLatest,
        comparison(3, "refs/heads/main"),
    );
    state.go_back(tab).unwrap();
    let before = state.tab(tab).unwrap().clone();
    complete(&mut state, tab, ComparisonChange::LoadLatest, first);
    assert_eq!(state.tab(tab).unwrap(), &before);
    browsing(&mut state, tab, "other.rs", 33);
    let after = state.tab(tab).unwrap();
    assert_eq!(after.visit_id(), before.visit_id());
    assert_eq!(after.activation(), before.activation());
    assert_eq!(after.back, before.back);
    assert_eq!(after.forward, before.forward);
}

#[test]
fn making_the_resolved_default_base_explicit_does_not_push() {
    let (mut state, tab, _) = setup();
    let pinned = comparison(2, "refs/heads/main");
    state
        .set_comparison(tab, BaseSelection::Default, pinned.clone())
        .unwrap();
    state.go_home(tab).unwrap();
    state.go_back(tab).unwrap();
    let before = state.tab(tab).unwrap().clone();
    complete(&mut state, tab, ComparisonChange::ChangeBase, pinned);
    let after = state.tab(tab).unwrap();
    assert_eq!(after.visit_id(), before.visit_id());
    assert_eq!(after.activation(), before.activation());
    assert_eq!(after.back, before.back);
    assert_eq!(after.forward, before.forward);
}

#[test]
fn visit_identity_survives_traversal_but_activation_does_not() {
    let (mut state, tab, _) = setup();
    let before = state.tab(tab).unwrap().snapshot().unwrap();
    let activation = state.tab(tab).unwrap().activation();
    state.go_back(tab).unwrap();
    state.go_forward(tab).unwrap();
    assert_eq!(state.tab(tab).unwrap().snapshot().unwrap(), before);
    assert_eq!(state.tab(tab).unwrap().activation(), activation + 2);
    state.go_back(tab).unwrap();
    state.select_branch(tab, "refs/heads/other".into()).unwrap();
    assert_ne!(state.tab(tab).unwrap().visit_id(), before.id);
    assert!(!state.tab(tab).unwrap().can_go_forward());
    let saved = serde_json::to_vec(&state).unwrap();
    assert_eq!(serde_json::from_slice::<AppState>(&saved).unwrap(), state);
}

#[test]
fn stale_completion_is_ignored_at_the_policy_boundary() {
    let (mut state, tab, _) = setup();
    let activation = state.tab(tab).unwrap().activation();
    state.go_back(tab).unwrap();
    state.go_forward(tab).unwrap();
    let before = state.clone();
    state
        .navigate(
            tab,
            Navigation::ComparisonLoaded {
                activation,
                change: ComparisonChange::Resolve,
                base: BaseSelection::Default,
                comparison: comparison(2, "refs/heads/main"),
            },
        )
        .unwrap();
    assert_eq!(state, before);
}

#[test]
fn snapshots_do_not_copy_the_repositorys_other_branch_reviews() {
    let (mut state, tab, _) = setup();
    state
        .select_branch(tab, "refs/heads/unrelated".into())
        .unwrap();
    state.select_branch(tab, "refs/heads/topic".into()).unwrap();
    complete(
        &mut state,
        tab,
        ComparisonChange::Resolve,
        comparison(2, "refs/heads/main"),
    );
    let snapshot = state.tab(tab).unwrap().snapshot().unwrap();
    let saved = serde_json::to_string(&snapshot).unwrap();
    assert!(!saved.contains("unrelated"));
    assert!(!saved.contains("reviews"));
    state.go_home(tab).unwrap();
    assert_eq!(state.tab(tab).unwrap().back.last().unwrap(), &snapshot);
    state.go_back(tab).unwrap();
    assert_eq!(state.tab(tab).unwrap().snapshot().unwrap(), snapshot);
    assert!(
        state
            .tab(tab)
            .unwrap()
            .route
            .review()
            .unwrap()
            .reviews
            .iter()
            .any(|review| review.branch == RefName::from("refs/heads/unrelated"))
    );
}

#[test]
fn first_base_choice_is_resolution_and_resolve_cannot_advance_existing_pins() {
    let (mut state, tab, _) = setup();
    let before = state.tab(tab).unwrap().clone();
    complete(
        &mut state,
        tab,
        ComparisonChange::ChangeBase,
        comparison(2, "refs/heads/main"),
    );
    assert_eq!(state.tab(tab).unwrap().visit_id(), before.visit_id());
    assert_eq!(state.tab(tab).unwrap().back, before.back);
    let before = state.clone();
    assert!(
        state
            .navigate(
                tab,
                Navigation::ComparisonLoaded {
                    activation: state.tab(tab).unwrap().activation(),
                    change: ComparisonChange::Resolve,
                    base: BaseSelection::Default,
                    comparison: comparison(3, "refs/heads/main"),
                }
            )
            .is_err()
    );
    assert_eq!(state, before);
}

#[test]
fn legacy_route_stacks_migrate_without_losing_views_or_branch_memories() {
    let (mut state, tab, project) = setup();
    complete(
        &mut state,
        tab,
        ComparisonChange::Resolve,
        comparison(2, "refs/heads/main"),
    );
    browsing(&mut state, tab, "old.rs", 81);
    state
        .select_branch(tab, "refs/heads/unrelated".into())
        .unwrap();
    state.select_branch(tab, "refs/heads/topic".into()).unwrap();
    let expected = state.tab(tab).unwrap().snapshot().unwrap().snapshot;
    let current = state.tab(tab).unwrap().route.clone();
    let list = Route::Repository {
        project,
        page: RepositoryPage::Branches,
        review: current.review().unwrap().clone(),
        branches: BranchList {
            query: "topic".into(),
            ..Default::default()
        },
    };
    let mut json = serde_json::to_value(&state).unwrap();
    json["windows"][0]["tabs"][0] = serde_json::json!({
        "id": tab, "route": current, "back": [Route::Home, list], "forward": []
    });
    let mut restored: AppState = serde_json::from_value(json).unwrap();
    assert_eq!(
        restored.tab(tab).unwrap().snapshot().unwrap().snapshot,
        expected
    );
    restored.go_back(tab).unwrap();
    let Route::Repository {
        branches, review, ..
    } = &restored.tab(tab).unwrap().route
    else {
        panic!()
    };
    assert_eq!(branches.query, "topic");
    assert!(
        review
            .reviews
            .iter()
            .any(|review| review.branch == RefName::from("refs/heads/unrelated"))
    );
    restored.go_forward(tab).unwrap();
    assert_eq!(
        restored.tab(tab).unwrap().snapshot().unwrap().snapshot,
        expected
    );
    let encoded = serde_json::to_vec(&restored).unwrap();
    assert_eq!(
        serde_json::from_slice::<AppState>(&encoded).unwrap(),
        restored
    );
}

#[test]
fn legacy_migration_retains_branch_memories_absent_from_other_routes() {
    let (mut state, tab, project) = setup();
    state.select_branch(tab, "refs/heads/other".into()).unwrap();
    state.select_branch(tab, "refs/heads/topic".into()).unwrap();
    let remembered = state.tab(tab).unwrap().route.clone();
    let empty = Route::Repository {
        project,
        review: ReviewContext::default(),
        page: RepositoryPage::Branches,
        branches: BranchList::default(),
    };
    let mut json = serde_json::to_value(&state).unwrap();
    json["windows"][0]["tabs"][0] = serde_json::json!({
        "id": tab, "route": "Home", "back": [remembered], "forward": [empty]
    });
    let mut restored: AppState = serde_json::from_value(json).unwrap();
    restored.go_back(tab).unwrap();
    assert_eq!(
        restored
            .tab(tab)
            .unwrap()
            .route
            .review()
            .unwrap()
            .reviews
            .len(),
        2
    );
}

#[test]
fn malformed_visit_ids_and_historical_pins_are_rejected() {
    let (mut state, tab, _) = setup();
    complete(
        &mut state,
        tab,
        ComparisonChange::Resolve,
        comparison(2, "refs/heads/main"),
    );
    state.go_home(tab).unwrap();
    let json = serde_json::to_value(&state).unwrap();
    let mut duplicate = json.clone();
    duplicate["windows"][0]["tabs"][0]["back"][0]["id"] =
        duplicate["windows"][0]["tabs"][0]["visit_id"].clone();
    assert!(serde_json::from_value::<AppState>(duplicate).is_err());
    let mut invalid = json.clone();
    invalid["windows"][0]["tabs"][0]["next_visit"] = serde_json::json!(0);
    assert!(serde_json::from_value::<AppState>(invalid).is_err());
    let mut invalid = json;
    let entries = invalid["windows"][0]["tabs"][0]["back"]
        .as_array_mut()
        .unwrap();
    entries.last_mut().unwrap()["snapshot"]["destination"]["Comparison"]["comparison"]["branch"] =
        serde_json::json!(b"refs/heads/wrong".to_vec());
    assert!(serde_json::from_value::<AppState>(invalid).is_err());
}
