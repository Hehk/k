use super::*;
use crate::{
    git::RepositoryInfo,
    review::{Endpoints, ObjectId},
    state::{ReadingMode, WindowId},
};

fn setup() -> (AppState, TabId, WindowId, ProjectId, ProjectId) {
    let mut state = AppState::default();
    let mut project = |name: &str| {
        let directory = std::env::temp_dir().join(name);
        state
            .register_project(&RepositoryInfo {
                common_dir: directory.join(".git"),
                directory,
                current_branch: Some("refs/heads/review".into()),
            })
            .unwrap()
    };
    let a = project("navigation-a");
    let b = project("navigation-b");
    let window = state.new_window().unwrap();
    let tab = state.new_tab(window).unwrap();
    state.navigate(tab, Navigation::OpenProject(a)).unwrap();
    state.navigate(tab, Navigation::OpenProject(b)).unwrap();
    state.navigate(tab, Navigation::Back).unwrap();
    let other = state.new_window().unwrap();
    state.new_tab(other).unwrap();
    (state, tab, window, a, b)
}

fn comparison() -> Comparison {
    let old: ObjectId = "1111111111111111111111111111111111111111".parse().unwrap();
    let new: ObjectId = "2222222222222222222222222222222222222222".parse().unwrap();
    Comparison {
        branch: "refs/heads/review".into(),
        base: "refs/heads/main".into(),
        base_tip: old,
        commits: Endpoints { old, new },
    }
}

#[test]
fn policy_matrix_preserves_the_committed_baseline() {
    let (before, tab, window, a, b) = setup();
    // IDs refer to the action checklist. Comparison completion still updates
    // the current visit; explicit base/latest boundaries change in the next PR.
    let cases = [
        ("A12 Home", Navigation::Home, None, 2, 0, false),
        (
            "A2 repository",
            Navigation::OpenProject(b),
            Some(b),
            2,
            0,
            false,
        ),
        (
            "A10 same repository",
            Navigation::OpenProject(a),
            Some(a),
            1,
            1,
            false,
        ),
        ("C1 Back", Navigation::Back, None, 0, 2, false),
        ("C1 Forward", Navigation::Forward, Some(b), 2, 0, false),
        ("C4 select tab", Navigation::SelectTab, Some(a), 1, 1, true),
        (
            "A5 branch",
            Navigation::SelectBranch("refs/heads/other".into()),
            Some(a),
            2,
            0,
            false,
        ),
        (
            "A6 comparison (baseline)",
            Navigation::SetComparison {
                base: BaseSelection::Default,
                comparison: comparison(),
            },
            Some(a),
            1,
            1,
            false,
        ),
        (
            "B1 branch filter",
            Navigation::SetBranchList(BranchList {
                query: "topic".into(),
                ..Default::default()
            }),
            Some(a),
            1,
            1,
            false,
        ),
        (
            "B5 browsing",
            Navigation::SetBrowsing {
                branch: "refs/heads/review".into(),
                browsing: Browsing {
                    file: Some("src/lib.rs".into()),
                    first_visible_line: 42,
                    ..Default::default()
                },
            },
            Some(a),
            1,
            1,
            false,
        ),
    ];
    for (name, action, project, back, forward, selects) in cases {
        let mut state = before.clone();
        state.navigate(tab, action).unwrap();
        let current = state.tab(tab).unwrap();
        assert_eq!(current.route.project(), project, "{name}");
        assert_eq!(current.back.len(), back, "{name}");
        assert_eq!(current.forward.len(), forward, "{name}");
        assert_eq!(
            state.active_window(),
            if selects {
                Some(window)
            } else {
                before.active_window()
            },
            "{name}"
        );
        for other in before.windows().iter().flat_map(|window| &window.tabs) {
            if other.id != tab {
                assert_eq!(state.tab(other.id).unwrap(), other, "{name}");
            }
        }
    }
}

#[test]
fn no_ops_preserve_forward_history_and_saved_state() {
    let (mut state, tab, _, a, _) = setup();
    state.navigate(tab, Navigation::SelectTab).unwrap();
    for action in [Navigation::OpenProject(a), Navigation::SelectTab] {
        let before = state.clone();
        state.navigate(tab, action).unwrap();
        assert_eq!(state, before);
    }
    state.navigate(tab, Navigation::Back).unwrap();
    for action in [Navigation::Home, Navigation::Back] {
        let before = state.clone();
        state.navigate(tab, action).unwrap();
        assert_eq!(state, before);
    }
    state.navigate(tab, Navigation::Forward).unwrap();
    state.navigate(tab, Navigation::Forward).unwrap();
    let before = state.clone();
    state.navigate(tab, Navigation::Forward).unwrap();
    assert_eq!(state, before);
}

#[test]
fn review_updates_remember_context_without_touching_either_stack() {
    let (mut state, tab, _, a, _) = setup();
    let before = state.tab(tab).unwrap().clone();
    let browsing = Browsing {
        file: Some("src/lib.rs".into()),
        mode: ReadingMode::HeadFile,
        first_visible_line: 42,
        ..Default::default()
    };
    state
        .navigate(
            tab,
            Navigation::SetComparison {
                base: BaseSelection::Default,
                comparison: comparison(),
            },
        )
        .unwrap();
    state
        .navigate(
            tab,
            Navigation::SetBrowsing {
                branch: "refs/heads/review".into(),
                browsing: browsing.clone(),
            },
        )
        .unwrap();
    let current = state.tab(tab).unwrap();
    let context = current.route.review().unwrap();
    assert_eq!(
        context.active_review().unwrap().comparison,
        Some(comparison())
    );
    assert_eq!(context.active_review().unwrap().browsing, browsing);
    assert_eq!(&state.project(a).unwrap().last_review, context);
    assert_eq!(current.back, before.back);
    assert_eq!(current.forward, before.forward);
    let saved = serde_json::to_vec(&state).unwrap();
    assert_eq!(serde_json::from_slice::<AppState>(&saved).unwrap(), state);
}

#[test]
fn rejected_navigation_does_not_mutate_state() {
    let (before, tab, _, _, _) = setup();
    let cases = [
        Navigation::OpenProject(ProjectId(u64::MAX)),
        Navigation::SelectBranch("not-a-branch".into()),
        Navigation::SetComparison {
            base: BaseSelection::Explicit("refs/heads/other".into()),
            comparison: comparison(),
        },
        Navigation::SetComparison {
            base: BaseSelection::Default,
            comparison: Comparison {
                branch: "refs/heads/unknown".into(),
                ..comparison()
            },
        },
        Navigation::SetComparison {
            base: BaseSelection::Default,
            comparison: Comparison {
                base: "refs/tags/base".into(),
                ..comparison()
            },
        },
        Navigation::SetBrowsing {
            branch: "refs/heads/review".into(),
            browsing: Browsing {
                file: Some("../outside".into()),
                ..Default::default()
            },
        },
        Navigation::SetBrowsing {
            branch: "refs/heads/unknown".into(),
            browsing: Browsing::default(),
        },
    ];
    for action in cases {
        let mut state = before.clone();
        assert!(state.navigate(tab, action).is_err());
        assert_eq!(state, before);
    }
    for action in [
        Navigation::Home,
        Navigation::Back,
        Navigation::Forward,
        Navigation::SelectTab,
    ] {
        let mut state = before.clone();
        assert!(state.navigate(TabId(u64::MAX), action).is_err());
        assert_eq!(state, before);
    }
}
