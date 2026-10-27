use super::*;
use crate::{
    git::RepositoryInfo,
    review::Endpoints,
    state::{
        BaseSelection, BranchList, Browsing, ComparisonChange, Navigation, StateError, WindowId,
    },
};

struct Harness {
    state: AppState,
    history: ViewingHistory,
    clock: i64,
    window: WindowId,
    a: TabId,
    b: TabId,
    project: ProjectId,
    other: ProjectId,
}

impl Harness {
    fn new() -> Self {
        let mut state = AppState::default();
        let mut register = |name| {
            let directory = std::env::temp_dir().join(name);
            state
                .register_project(&RepositoryInfo {
                    common_dir: directory.join(".git"),
                    directory,
                    current_branch: None,
                })
                .unwrap()
        };
        let project = register("viewing-first");
        let other = register("viewing-second");
        let window = state.new_window().unwrap();
        let a = state.new_tab(window).unwrap();
        let b = state.new_tab(window).unwrap();
        state.select_tab(a).unwrap();
        Self {
            state,
            history: ViewingHistory::default(),
            clock: 0,
            window,
            a,
            b,
            project,
            other,
        }
    }

    fn edit<R>(
        &mut self,
        change: impl FnOnce(&mut AppState) -> Result<R, StateError>,
    ) -> Result<R, StateError> {
        let mut candidate = self.state.clone();
        let (result, changes) =
            candidate.collect_view_changes(Foreground::SelectedWindow, change)?;
        self.history.apply(changes, || {
            self.clock += 1;
            self.clock
        });
        self.history.retain_live_bindings(&candidate);
        self.state = candidate;
        Ok(result)
    }

    fn nav(&mut self, tab: TabId, action: Navigation) {
        self.edit(|state| state.navigate(tab, action)).unwrap();
    }

    fn resolve(&mut self, tab: TabId, change: ComparisonChange, head: u8) {
        self.nav(
            tab,
            Navigation::ComparisonLoaded {
                activation: self.state.tab(tab).unwrap().activation(),
                change,
                base: BaseSelection::Default,
                comparison: comparison(head),
            },
        );
    }

    fn browse(&mut self, tab: TabId, line: u32) {
        self.nav(
            tab,
            Navigation::SetBrowsing {
                branch: "refs/heads/topic".into(),
                browsing: Browsing {
                    file: Some("saved.rs".into()),
                    first_visible_line: line,
                    ..Default::default()
                },
            },
        );
    }

    fn comparison(&mut self, tab: TabId) {
        self.nav(tab, Navigation::OpenProject(self.project));
        self.nav(tab, Navigation::SelectBranch("refs/heads/topic".into()));
        self.resolve(tab, ComparisonChange::Resolve, 2);
    }
}

fn comparison(head: u8) -> Comparison {
    let old = "1111111111111111111111111111111111111111".parse().unwrap();
    Comparison {
        branch: "refs/heads/topic".into(),
        base: "refs/heads/main".into(),
        base_tip: old,
        commits: Endpoints {
            old,
            new: format!("{head:040x}").parse().unwrap(),
        },
    }
}

#[test]
fn a1_a4_a9_c1_c2_navigation_records_once_and_resolution_updates_the_event() {
    let mut h = Harness::new();
    h.comparison(h.a);
    assert_eq!(h.history.records().len(), 2);
    let resolved = h.history.records()[1].clone();
    assert!(matches!(
        resolved.viewed.snapshot.destination,
        Destination::Comparison {
            comparison: Some(_),
            ..
        }
    ));
    assert_eq!(resolved.viewed.labels.repository, "viewing-first");
    assert_eq!(resolved.viewed.labels.branch.as_deref(), Some("topic"));
    assert_eq!(resolved.viewed.labels.base.as_deref(), Some("main"));
    h.resolve(h.a, ComparisonChange::Resolve, 2);
    h.resolve(h.a, ComparisonChange::LoadLatest, 2);
    h.browse(h.a, 75);
    assert_eq!(h.clock, 2);
    assert_eq!(h.history.records()[1].timestamp_ms, resolved.timestamp_ms);
    assert_eq!(
        h.history.records()[1].viewed.labels.file.as_deref(),
        Some("saved.rs")
    );
    h.resolve(h.a, ComparisonChange::LoadLatest, 3);
    h.nav(h.a, Navigation::Back);
    assert_eq!(h.history.records().len(), 4);
    assert!(
        matches!(&h.history.records()[3].viewed.snapshot.destination,
        Destination::Comparison { comparison: Some(pinned), .. } if *pinned == comparison(2))
    );
    h.nav(h.a, Navigation::Forward);
    assert_eq!(h.history.records().len(), 5);
    h.nav(h.a, Navigation::Home);
    assert_eq!(h.history.records().len(), 5);
    h.nav(h.a, Navigation::Back);
    assert_eq!(h.history.records().len(), 6);
    assert_eq!(h.clock, 6);
}

#[test]
fn a6_a7_base_policy_updates_the_event_but_a_different_requested_base_creates_one() {
    let mut h = Harness::new();
    h.comparison(h.a);
    let activation = h.state.tab(h.a).unwrap().activation();
    h.nav(
        h.a,
        Navigation::ComparisonLoaded {
            activation,
            change: ComparisonChange::ChangeBase,
            base: BaseSelection::Explicit("refs/heads/main".into()),
            comparison: comparison(2),
        },
    );
    assert_eq!(h.history.records().len(), 2);
    assert_eq!(h.clock, 2);
    let old = h.history.records()[1].clone();
    let mut other = comparison(2);
    other.base = "refs/heads/other".into();
    h.nav(
        h.a,
        Navigation::ComparisonLoaded {
            activation,
            change: ComparisonChange::ChangeBase,
            base: BaseSelection::Explicit(other.base.clone()),
            comparison: other,
        },
    );
    assert_eq!(h.history.records().len(), 3);
    assert_eq!(h.history.records()[1], old);
    h.nav(h.a, Navigation::Back);
    assert_eq!(h.history.records().len(), 4);
    assert_eq!(h.history.records()[3].viewed, old.viewed);
}

#[test]
fn c4_switches_create_events_not_steps_and_only_the_latest_event_is_updated() {
    let mut h = Harness::new();
    h.comparison(h.a);
    h.browse(h.a, 10);
    let original = h.history.records()[1].clone();
    let visit = h.state.tab(h.a).unwrap().visit_id();
    h.nav(h.b, Navigation::SelectTab);
    h.nav(h.a, Navigation::SelectTab);
    assert_eq!(h.history.records().len(), 3);
    assert_eq!(h.state.tab(h.a).unwrap().visit_id(), visit);
    h.nav(h.a, Navigation::SelectTab);
    let window = h.window;
    h.edit(|state| state.focus_window(window)).unwrap();
    assert_eq!(h.history.records().len(), 3);
    h.browse(h.a, 40);
    assert_eq!(h.history.records()[1], original);
    assert_eq!(
        h.history.records()[2]
            .viewed
            .snapshot
            .view
            .browsing
            .first_visible_line,
        40
    );
    assert_eq!(h.clock, 3);
}

#[test]
fn c5_background_loading_waits_for_first_selection_and_does_not_rewrite_the_old_event() {
    let mut h = Harness::new();
    h.comparison(h.b);
    assert!(h.history.records().is_empty());
    h.nav(h.b, Navigation::SelectTab);
    assert_eq!(h.history.records().len(), 1);
    let first = h.history.records()[0].clone();
    h.nav(h.a, Navigation::SelectTab);
    h.nav(h.b, Navigation::OpenProject(h.other));
    h.nav(
        h.b,
        Navigation::SetBranchList(BranchList {
            query: "filter".into(),
            ..Default::default()
        }),
    );
    assert_eq!(h.history.records(), std::slice::from_ref(&first));
    h.nav(h.b, Navigation::SelectTab);
    assert_eq!(h.history.records().len(), 2);
    assert_eq!(h.history.records()[0], first);
    assert_eq!(
        h.history.records()[1].viewed.snapshot.view.branches.query,
        "filter"
    );
}

#[test]
fn c6_focus_only_finishes_a_pending_background_view_never_duplicates_seen_pages() {
    let mut h = Harness::new();
    let second = h
        .edit(|state| {
            let window = state.new_window()?;
            state.new_tab(window)?;
            Ok(window)
        })
        .unwrap();
    h.comparison(h.a);
    assert!(h.history.records().is_empty());
    let first = h.window;
    h.edit(|state| state.focus_window(first)).unwrap();
    assert_eq!(h.history.records().len(), 1);
    h.edit(|state| state.focus_window(second)).unwrap();
    h.edit(|state| state.focus_window(first)).unwrap();
    assert_eq!(h.history.records().len(), 1);
}

#[test]
fn clicking_the_selected_tab_of_a_background_window_has_the_same_first_view_policy_as_focus() {
    let mut h = Harness::new();
    let background = h
        .edit(|state| {
            let window = state.new_window()?;
            state.new_tab(window)?;
            Ok(window)
        })
        .unwrap();
    h.comparison(h.a);
    assert!(h.history.records().is_empty());
    h.nav(h.a, Navigation::SelectTab);
    assert_eq!(h.history.records().len(), 1);
    h.edit(|state| state.focus_window(background)).unwrap();
    h.nav(h.a, Navigation::SelectTab);
    assert_eq!(h.history.records().len(), 1);
}

#[test]
fn closure_is_not_a_viewing_even_when_it_reveals_a_pending_background_tab() {
    let mut h = Harness::new();
    h.comparison(h.b);
    assert!(h.history.records().is_empty());
    let tab = h.a;
    h.edit(|state| {
        state.close_tab(tab);
        Ok(())
    })
    .unwrap();
    assert!(h.history.records().is_empty());
    let window = h.window;
    h.nav(h.b, Navigation::SelectTab);
    assert!(h.history.records().is_empty());
    h.edit(|state| state.new_tab(window)).unwrap();
    h.nav(h.b, Navigation::SelectTab);
    assert_eq!(h.history.records().len(), 1);
    h.nav(h.b, Navigation::Home);
    h.nav(h.b, Navigation::ReopenTab);
    assert_eq!(h.history.records().len(), 1);
}

#[test]
fn a_selected_tab_is_not_foreground_while_the_application_is_hidden() {
    let mut h = Harness::new();
    let (tab, project, window) = (h.a, h.project, h.window);
    let (_, changes) = h
        .state
        .collect_view_changes(Foreground::Hidden, |state| state.open_project(project, tab))
        .unwrap();
    h.history.apply(changes, || {
        panic!("background load has no viewing timestamp")
    });
    assert!(h.history.records().is_empty());
    let (_, changes) = h
        .state
        .collect_view_changes(Foreground::Hidden, |state| state.focus_window(window))
        .unwrap();
    h.history.apply(changes, || {
        panic!("persisted selection is not native focus")
    });
    let (_, changes) = h
        .state
        .collect_view_changes(Foreground::Window(window), |state| {
            state.focus_window(window)
        })
        .unwrap();
    h.history.apply(changes, || 42);
    assert_eq!(h.history.records().len(), 1);
    assert_eq!(h.history.records()[0].timestamp_ms, 42);
}

#[test]
fn startup_resolution_focus_and_view_updates_do_not_synthesize_a_viewing() {
    let mut h = Harness::new();
    h.state.open_project(h.project, h.a).unwrap();
    h.state
        .select_branch(h.a, "refs/heads/topic".into())
        .unwrap();
    h.state = serde_json::from_slice(&serde_json::to_vec(&h.state).unwrap()).unwrap();
    h.resolve(h.a, ComparisonChange::Resolve, 2);
    h.browse(h.a, 30);
    let window = h.window;
    h.edit(|state| state.focus_window(window)).unwrap();
    h.nav(h.a, Navigation::SelectTab);
    assert!(h.history.records().is_empty());
    h.nav(h.b, Navigation::SelectTab);
    h.nav(h.a, Navigation::SelectTab);
    assert_eq!(h.history.records().len(), 1);
    assert_eq!(
        h.history.records()[0]
            .viewed
            .snapshot
            .view
            .browsing
            .first_visible_line,
        30
    );
}

#[test]
fn workspace_clones_and_serialization_do_not_inherit_in_flight_viewing_outputs() {
    let mut h = Harness::new();
    let (tab, project) = (h.a, h.project);
    let (snapshot, changes) = h
        .state
        .collect_view_changes(Foreground::SelectedWindow, |state| {
            state.open_project(project, tab)?;
            let snapshot = state.clone();
            assert_eq!(&snapshot, state);
            let restored: AppState =
                serde_json::from_slice(&serde_json::to_vec(state).unwrap()).unwrap();
            assert_eq!(snapshot, restored);
            let mut independent = snapshot.clone();
            let (_, copied) =
                independent.collect_view_changes(Foreground::SelectedWindow, |_| Ok(()))?;
            assert!(copied.is_empty());
            Ok(snapshot)
        })
        .unwrap();
    assert_eq!(snapshot, h.state);
    assert_eq!(changes.len(), 1);
    let (_, drained) = h
        .state
        .collect_view_changes(Foreground::SelectedWindow, |_| Ok(()))
        .unwrap();
    assert!(drained.is_empty());
}

#[test]
fn rejected_transactions_and_stale_completions_leave_log_and_clock_unchanged() {
    let mut h = Harness::new();
    let before = h.state.clone();
    let (tab, project) = (h.a, h.project);
    assert!(
        h.edit(|state| {
            state.open_project(project, tab)?;
            Err::<(), _>(StateError("cancel transaction"))
        })
        .is_err()
    );
    assert_eq!(h.state, before);
    assert_eq!(h.clock, 0);
    assert!(
        h.edit(|state| {
            *state = AppState::default();
            Ok(())
        })
        .is_err()
    );
    assert_eq!(h.state, before);
    assert_eq!(h.clock, 0);
    h.comparison(tab);
    let activation = h.state.tab(tab).unwrap().activation();
    h.nav(tab, Navigation::Back);
    h.nav(tab, Navigation::Forward);
    let before = h.history.records().to_vec();
    let time = h.clock;
    h.nav(
        tab,
        Navigation::ComparisonLoaded {
            activation,
            change: ComparisonChange::LoadLatest,
            base: BaseSelection::Default,
            comparison: comparison(3),
        },
    );
    assert_eq!(h.history.records(), before);
    assert_eq!(h.clock, time);
}

#[test]
fn a11_explicit_old_snapshot_pushes_even_for_equal_pins_and_survives_source_closure() {
    let mut h = Harness::new();
    h.comparison(h.a);
    h.browse(h.a, 10);
    let original = h.history.records()[1].clone();
    h.browse(h.a, 40);
    let prior = h.state.tab(h.a).unwrap().snapshot().unwrap();
    h.nav(
        h.a,
        Navigation::OpenSnapshot(original.viewed.snapshot.clone()),
    );
    let reopened = h.state.tab(h.a).unwrap().snapshot().unwrap();
    assert_ne!(reopened.id, prior.id);
    assert_eq!(reopened.snapshot, original.viewed.snapshot);
    h.nav(h.a, Navigation::Back);
    assert_eq!(h.state.tab(h.a).unwrap().snapshot().unwrap(), prior);
    let (window, target) = (h.window, h.b);
    h.edit(|state| {
        state.close_window(window);
        Ok(())
    })
    .unwrap();
    assert!(h.state.tab(target).is_err());
    let snapshot = original.viewed.snapshot.clone();
    let tab = h
        .edit(|state| {
            let window = state.new_window()?;
            let tab = state.new_tab(window)?;
            state.navigate(tab, Navigation::OpenSnapshot(snapshot))?;
            Ok(tab)
        })
        .unwrap();
    assert_eq!(
        h.state.tab(tab).unwrap().snapshot().unwrap().snapshot,
        original.viewed.snapshot
    );
    assert_eq!(h.history.records().last().unwrap().viewed, original.viewed);
}

#[test]
fn c9_c10_moves_closures_and_forward_truncation_do_not_delete_records() {
    let mut h = Harness::new();
    h.nav(h.a, Navigation::OpenProject(h.project));
    h.nav(h.a, Navigation::OpenProject(h.other));
    h.nav(h.a, Navigation::Back);
    h.nav(h.a, Navigation::SelectBranch("refs/heads/topic".into()));
    assert!(!h.state.tab(h.a).unwrap().can_go_forward());
    let records = h.history.records().to_vec();
    let tab = h.a;
    h.edit(|state| {
        let window = state.new_window()?;
        state.move_tab(tab, window, 0)?;
        state.close_tab(tab);
        Ok(())
    })
    .unwrap();
    assert_eq!(h.history.records(), records);
    assert!(!h.history.bindings.contains_key(&tab));
}

#[test]
fn recent_is_lazy_deduplicates_before_limit_and_ignores_reading_state() {
    let mut h = Harness::new();
    h.comparison(h.a);
    h.browse(h.a, 10);
    h.nav(h.b, Navigation::SelectTab);
    h.nav(h.a, Navigation::SelectTab);
    h.browse(h.a, 20);
    assert_eq!(h.history.records().len(), 3);
    let recent_ids = h
        .history
        .recent(RECENT_LIMIT)
        .map(|record| record.id)
        .collect::<Vec<_>>();
    assert_eq!(recent_ids, [ViewingId(2), ViewingId(0)]);
    h.resolve(h.a, ComparisonChange::LoadLatest, 3);
    assert_eq!(h.history.recent(RECENT_LIMIT).count(), 3);
    let records = h.history.records();
    let input = [&records[2], &records[1], &records[0], &records[3]];
    let examined = std::cell::Cell::new(0);
    let projected = recent(
        input
            .into_iter()
            .inspect(|_| examined.set(examined.get() + 1)),
        2,
    );
    assert_eq!(examined.get(), 0);
    assert_eq!(
        projected.map(|record| record.id).collect::<Vec<_>>(),
        [ViewingId(2), ViewingId(0)]
    );
    assert_eq!(examined.get(), 3);
    assert_eq!(
        recent(
            input
                .into_iter()
                .inspect(|_| panic!("zero limit reads nothing")),
            0
        )
        .count(),
        0
    );
}

#[test]
fn recent_keys_keep_requested_refs_distinct_but_ignore_resolved_base_selection_policy() {
    let mut h = Harness::new();
    h.comparison(h.a);
    let original = h.history.records()[1].clone();
    let mut explicit = original.clone();
    let Destination::Comparison { base, .. } = &mut explicit.viewed.snapshot.destination else {
        panic!()
    };
    *base = BaseSelection::Explicit("refs/heads/main".into());
    assert_eq!(recent([&explicit, &original].into_iter(), 10).count(), 1);
    let mut other_ref = original.clone();
    let Destination::Comparison {
        branch, comparison, ..
    } = &mut other_ref.viewed.snapshot.destination
    else {
        panic!()
    };
    *branch = "refs/remotes/origin/topic".into();
    comparison.as_mut().unwrap().branch = branch.clone();
    assert_eq!(recent([&other_ref, &original].into_iter(), 10).count(), 2);
    let mut pending_default = original;
    let Destination::Comparison { comparison, .. } =
        &mut pending_default.viewed.snapshot.destination
    else {
        panic!()
    };
    *comparison = None;
    let mut pending_explicit = explicit;
    let Destination::Comparison { comparison, .. } =
        &mut pending_explicit.viewed.snapshot.destination
    else {
        panic!()
    };
    *comparison = None;
    assert_eq!(
        recent([&pending_explicit, &pending_default].into_iter(), 10).count(),
        2
    );
}

#[test]
fn append_order_not_wall_clock_order_drives_recents_and_record_ids() {
    let mut h = Harness::new();
    h.nav(h.a, Navigation::OpenProject(h.project));
    h.clock = -100;
    h.nav(h.a, Navigation::OpenProject(h.other));
    assert_eq!(h.history.records()[1].timestamp_ms, -99);
    assert_eq!(h.history.recent(1).next().unwrap().id, ViewingId(1));
}
