use super::*;
use crate::{
    git::RepositoryInfo,
    review::RefName,
    state::{Browsing, ReadingMode, Route},
};

#[test]
fn reopen_restores_exact_review_not_the_projects_newer_defaults() {
    let mut before = AppState::default();
    let directory = std::env::temp_dir().join("history-repository");
    let branch = RefName::from("refs/heads/review");
    let project = before
        .register_project(&RepositoryInfo {
            common_dir: directory.join(".git"),
            directory,
            current_branch: Some(branch.clone()),
        })
        .unwrap();
    let window = before.new_window().unwrap();
    let first = before.new_tab(window).unwrap();
    let closed = before.new_tab(window).unwrap();
    let last = before.new_tab(window).unwrap();
    before.open_project(project, closed).unwrap();
    before
        .set_browsing(
            closed,
            &branch,
            Browsing {
                mode: ReadingMode::HeadFile,
                first_visible_line: 83,
                ..Default::default()
            },
        )
        .unwrap();
    before.go_home(closed).unwrap();
    before.go_back(closed).unwrap();
    let route = before.tab(closed).unwrap().route.clone();
    let mut after = before.clone();
    after.close_tab(closed);
    let history = Closed::removed(&before, &after);
    assert_eq!(history.len(), 1);
    after.open_project(project, first).unwrap();
    after
        .select_branch(first, "refs/heads/other".into())
        .unwrap();
    history[0].restore(&mut after, Some(window)).unwrap();
    let tabs = &after.window(window).unwrap().tabs;
    assert_eq!(tabs.len(), 3);
    assert_eq!(tabs[0].id, first);
    assert_eq!(tabs[2].id, last);
    assert_ne!(tabs[1].id, closed);
    assert_eq!(tabs[1].route, route);
    let restored = tabs[1].id;
    let mut expected = before.tab(closed).unwrap().clone();
    expected.id = restored;
    assert_eq!(tabs[1], expected);
    assert_eq!(after.window(window).unwrap().active_tab, Some(restored));
    assert!(after.tab(closed).is_err());
    after.go_back(restored).unwrap();
    assert_eq!(after.tab(restored).unwrap().route, Route::Home);
    after.go_forward(restored).unwrap();
    assert_eq!(after.tab(restored).unwrap().route, route);
    after.go_forward(restored).unwrap();
    assert_eq!(after.tab(restored).unwrap().route, Route::Home);
    let encoded = serde_json::to_vec(&after).unwrap();
    assert_eq!(serde_json::from_slice::<AppState>(&encoded).unwrap(), after);
}

#[test]
fn window_recovery_restores_order_and_selection_in_a_new_window() {
    let mut before = AppState::default();
    let window = before.new_window().unwrap();
    let first = before.new_tab(window).unwrap();
    before.new_tab(window).unwrap();
    before.new_tab(window).unwrap();
    before.select_tab(first).unwrap();
    let directory = std::env::temp_dir().join("history-window");
    let project = before
        .register_project(&RepositoryInfo {
            common_dir: directory.join(".git"),
            directory,
            current_branch: None,
        })
        .unwrap();
    before.open_project(project, first).unwrap();
    before.go_home(first).unwrap();
    before.go_back(first).unwrap();
    let other = before.new_window().unwrap();
    before.new_tab(other).unwrap();
    let mut after = before.clone();
    after.close_window(window);
    let history = Closed::removed(&before, &after);
    assert_eq!(history.len(), 1);
    let restored = history[0].restore(&mut after, Some(other)).unwrap();
    assert_ne!(restored, other);
    assert_ne!(restored, window);
    let tabs = &after.window(restored).unwrap().tabs;
    assert_eq!(tabs.len(), 3);
    assert_eq!(after.window(restored).unwrap().active_tab, Some(tabs[0].id));
    assert_eq!(after.window(other).unwrap(), before.window(other).unwrap());
    for (old, new) in before.window(window).unwrap().tabs.iter().zip(tabs) {
        let mut expected = old.clone();
        assert_ne!(expected.id, new.id);
        expected.id = new.id;
        assert_eq!(&expected, new);
    }
}

#[test]
fn detachment_reordering_and_route_navigation_do_not_create_closed_history() {
    let mut before = AppState::default();
    let a = before.new_window().unwrap();
    let tab = before.new_tab(a).unwrap();
    let b = before.new_window().unwrap();
    before.new_tab(b).unwrap();
    let mut after = before.clone();
    after.move_tab(tab, b, 0).unwrap();
    assert!(Closed::removed(&before, &after).is_empty());
    let before = after.clone();
    after.move_tab(tab, b, 1).unwrap();
    assert!(Closed::removed(&before, &after).is_empty());
    after.go_home(tab).unwrap();
    assert!(Closed::removed(&before, &after).is_empty());
}

#[test]
fn a_closed_tab_can_reopen_without_a_window_and_clamps_its_original_position() {
    let mut state = AppState::default();
    let window = state.new_window().unwrap();
    let tab = state.new_tab(window).unwrap();
    let closed = Closed::Tab {
        index: 5,
        tab: state.tab(tab).unwrap().clone(),
    };
    state.close_window(window);
    let window = closed.restore(&mut state, None).unwrap();
    assert_eq!(state.window(window).unwrap().tabs.len(), 1);
    closed.restore(&mut state, Some(window)).unwrap();
    assert_eq!(state.window(window).unwrap().tabs.len(), 2);
}
