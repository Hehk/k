use super::*;
use crate::{
    git::RepositoryInfo,
    review::{Comparison, Endpoints},
    state::{BaseSelection, BranchList, Browsing, ComparisonChange, Destination, ProjectId, Route},
};

struct Harness {
    machine: Model,
    context: Context,
    tab: TabId,
    window: WindowId,
    project: ProjectId,
}

impl Harness {
    fn new(persistence: Persistence) -> Self {
        let mut state = AppState::default();
        let directory = std::env::temp_dir().join("navigation-machine");
        let project = state
            .register_project(&RepositoryInfo {
                common_dir: directory.join(".git"),
                directory,
                current_branch: None,
            })
            .unwrap();
        let window = state.new_window().unwrap();
        let tab = state.new_tab(window).unwrap();
        Self {
            machine: Model::new(state, ViewingHistory::default(), persistence),
            context: Context {
                foreground: Foreground::SelectedWindow,
                timestamp_ms: 100,
            },
            tab,
            window,
            project,
        }
    }

    fn persistent() -> Self {
        Self::new(Persistence {
            workspace: true,
            history: true,
        })
    }

    fn event(&mut self, event: Event) -> Vec<Effect> {
        self.machine.transition(event, self.context).unwrap()
    }

    fn nav(&mut self, action: Navigation) -> Vec<Effect> {
        self.event(Event::Navigate {
            tab: self.tab,
            action,
        })
    }

    fn open(&mut self) -> Vec<Effect> {
        self.nav(Navigation::OpenProject(self.project))
    }

    fn capture(&mut self) -> SaveBatch {
        let id = scheduled(&self.event(Event::SaveRequested));
        let mut effects = self.event(Event::SaveDue(id));
        let Effect::Save(batch) = effects.remove(0) else {
            panic!("expected a save")
        };
        assert!(effects.is_empty());
        *batch
    }

    fn query(&mut self, value: &str) -> Vec<Effect> {
        self.nav(Navigation::SetBranchList(BranchList {
            query: value.into(),
            ..Default::default()
        }))
    }

    fn comparison(&mut self) {
        self.open();
        self.nav(Navigation::SelectBranch("refs/heads/topic".into()));
        let activation = self.machine.state().tab(self.tab).unwrap().activation();
        self.nav(Navigation::ComparisonLoaded {
            activation,
            change: ComparisonChange::Resolve,
            base: BaseSelection::Default,
            comparison: comparison(2),
        });
    }
}

fn scheduled(effects: &[Effect]) -> SaveId {
    effects
        .iter()
        .find_map(|effect| match effect {
            Effect::ScheduleSave(id) => Some(*id),
            _ => None,
        })
        .expect("expected a debounce effect")
}

fn success(batch: &SaveBatch) -> Event {
    Event::Saved(SaveResult {
        id: batch.id,
        workspace: batch.workspace.as_ref().map(|_| Ok(())),
        history: batch.history.as_ref().map(|_| Ok(())),
    })
}

fn comparison(head: u8) -> Comparison {
    let old = format!("{:040x}", 1).parse().unwrap();
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
fn edits_debounce_before_capture_and_obsolete_timers_cannot_start_writes() {
    let mut h = Harness::persistent();
    let first = scheduled(&h.open());
    let second = scheduled(&h.query("latest"));
    assert_ne!(first, second);
    assert!(h.event(Event::SaveDue(first)).is_empty());
    let mut effects = h.event(Event::SaveDue(second));
    let Effect::Save(batch) = effects.remove(0) else {
        panic!("expected save")
    };
    let records = batch.history.as_ref().unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].1.viewed.snapshot.view.branches.query, "latest");
    assert!(h.event(Event::SaveDue(second)).is_empty());
    assert!(h.event(Event::SaveRequested).is_empty());
    assert!(matches!(
        h.event(success(&batch)).as_slice(),
        [Effect::Changed]
    ));
    assert!(h.machine.history().pending().is_empty());
    assert!(h.event(success(&batch)).is_empty());
}

#[test]
fn failures_are_atomic_including_log_bindings_closed_history_and_save_schedule() {
    let mut h = Harness::persistent();
    let id = scheduled(&h.open());
    let before = h.machine.state().clone();
    let records = h.machine.history().pending();
    let revision = h.machine.revision();
    let result = h.machine.edit(h.context, |state| {
        state.navigate(h.tab, Navigation::Home)?;
        state.close_tab(h.tab);
        Err::<(), _>(StateError("rejected transaction"))
    });
    assert!(result.is_err());
    assert_eq!(h.machine.state(), &before);
    assert_eq!(h.machine.history().pending(), records);
    assert_eq!(h.machine.revision(), revision);
    assert_eq!(h.machine.closed_count(), 0);
    assert!(matches!(
        h.event(Event::SaveDue(id)).as_slice(),
        [Effect::Save(_)]
    ));
    h.query("still bound");
    assert_eq!(
        h.machine.history().records()[0]
            .viewed
            .snapshot
            .view
            .branches
            .query,
        "still bound"
    );
}

#[test]
fn noops_have_no_effects_but_history_only_visits_do() {
    let mut h = Harness::persistent();
    h.open();
    let state = h.machine.state().clone();
    let revision = h.machine.revision();
    assert!(h.open().is_empty());
    assert!(h.nav(Navigation::SelectTab).is_empty());
    assert_eq!(h.machine.revision(), revision);
    assert_eq!(h.nav(Navigation::ReopenTab).len(), 2);
    assert_eq!(h.machine.state(), &state);
    assert_eq!(h.machine.revision(), revision + 1);
    assert_eq!(h.machine.history().records().len(), 2);
}

#[test]
fn background_navigation_records_only_its_first_foreground_exposure() {
    let mut h = Harness::persistent();
    h.context.foreground = Foreground::Hidden;
    h.open();
    h.query("background reading");
    assert!(h.machine.history().records().is_empty());
    h.context.foreground = Foreground::Window(h.window);
    h.context.timestamp_ms = 200;
    let effects = h.event(Event::FocusWindow(h.window));
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, Effect::Changed))
    );
    let record = &h.machine.history().records()[0];
    assert_eq!(record.timestamp_ms, 200);
    assert_eq!(
        record.viewed.snapshot.view.branches.query,
        "background reading"
    );
    assert!(h.event(Event::FocusWindow(h.window)).is_empty());
    assert_eq!(h.machine.history().records().len(), 1);
}

#[test]
fn back_stale_resolution_and_save_completion_can_interleave_without_losing_changes() {
    for order in [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ] {
        let mut h = Harness::persistent();
        h.comparison();
        h.nav(Navigation::SetBrowsing {
            branch: "refs/heads/topic".into(),
            browsing: Browsing {
                first_visible_line: 30,
                ..Default::default()
            },
        });
        let old_activation = h.machine.state().tab(h.tab).unwrap().activation();
        h.nav(Navigation::ComparisonLoaded {
            activation: old_activation,
            change: ComparisonChange::LoadLatest,
            base: BaseSelection::Default,
            comparison: comparison(3),
        });
        let batch = h.capture();
        for step in order {
            match step {
                0 => {
                    h.nav(Navigation::Back);
                }
                1 => {
                    let before = h.machine.state().clone();
                    let dirty = h.machine.history().pending();
                    let result = h.machine.transition(
                        Event::Navigate {
                            tab: h.tab,
                            action: Navigation::ComparisonLoaded {
                                activation: old_activation,
                                change: ComparisonChange::Resolve,
                                base: BaseSelection::Default,
                                comparison: comparison(2),
                            },
                        },
                        h.context,
                    );
                    assert!(result.unwrap().is_empty());
                    assert_eq!(h.machine.state(), &before);
                    assert_eq!(h.machine.history().pending(), dirty);
                }
                2 => {
                    h.event(success(&batch));
                }
                _ => unreachable!(),
            }
        }
        let snapshot = h
            .machine
            .state()
            .tab(h.tab)
            .unwrap()
            .snapshot()
            .unwrap()
            .snapshot;
        assert!(matches!(snapshot.destination,
            Destination::Comparison { comparison: Some(ref pinned), .. } if *pinned == comparison(2)));
        assert_eq!(snapshot.view.browsing.first_visible_line, 30);
        assert_eq!(h.machine.history().pending().len(), 1);
        let final_batch = h.capture();
        h.event(success(&final_batch));
        assert!(h.machine.history().pending().is_empty());
        h.nav(Navigation::Forward);
        let snapshot = h
            .machine
            .state()
            .tab(h.tab)
            .unwrap()
            .snapshot()
            .unwrap()
            .snapshot;
        assert!(matches!(snapshot.destination,
            Destination::Comparison { comparison: Some(ref pinned), .. } if *pinned == comparison(3)));
    }
}

#[test]
fn amendments_and_new_ids_during_io_survive_an_old_generation_ack() {
    let mut h = Harness::persistent();
    h.open();
    let batch = h.capture();
    assert!(matches!(h.query("newer").as_slice(), [Effect::Changed]));
    h.nav(Navigation::ReopenTab);
    let effects = h.event(success(&batch));
    let next = scheduled(&effects);
    assert_eq!(h.machine.history().pending().len(), 2);
    assert!(h.event(success(&batch)).is_empty());
    let mut effects = h.event(Event::SaveDue(next));
    let Effect::Save(batch) = effects.remove(0) else {
        panic!("expected save")
    };
    let records = batch.history.as_ref().unwrap();
    assert_eq!(
        records
            .iter()
            .map(|(_, record)| record.id.0)
            .collect::<Vec<_>>(),
        [0, 1]
    );
    assert_eq!(records[0].1.viewed.snapshot.view.branches.query, "newer");
    h.event(success(&batch));
    assert!(h.machine.history().pending().is_empty());
}

#[test]
fn independent_write_failures_preserve_dirty_records_without_a_retry_loop() {
    for history_fails in [false, true] {
        let mut h = Harness::persistent();
        h.open();
        let batch = h.capture();
        let result = SaveResult {
            id: batch.id,
            workspace: Some(if history_fails {
                Ok(())
            } else {
                Err("unavailable".into())
            }),
            history: Some(if history_fails {
                Err("unavailable".into())
            } else {
                Ok(())
            }),
        };
        assert!(matches!(
            h.event(Event::Saved(result)).as_slice(),
            [Effect::Changed]
        ));
        assert_eq!(
            h.machine.history().pending().len(),
            usize::from(history_fails)
        );
        assert!(h.machine.save_error().unwrap().contains(if history_fails {
            "Viewing history"
        } else {
            "Workspace"
        }));
        let retry = h.capture();
        h.event(success(&retry));
        assert!(h.machine.history().pending().is_empty());
        assert!(h.machine.save_error().is_none());
    }
}

#[test]
fn quit_flushes_latest_values_and_ignores_old_completions_and_timers() {
    let mut h = Harness::persistent();
    let timer = scheduled(&h.open());
    let old = h.capture();
    h.query("final");
    let mut effects = h.event(Event::Quit);
    let Effect::Flush(final_batch) = effects.remove(0) else {
        panic!("expected flush")
    };
    assert_eq!(
        final_batch.history.as_ref().unwrap()[0]
            .1
            .viewed
            .snapshot
            .view
            .branches
            .query,
        "final"
    );
    assert!(h.machine.stopped());
    assert!(h.event(Event::SaveDue(timer)).is_empty());
    assert!(h.event(success(&old)).is_empty());
    assert!(!h.machine.history().pending().is_empty());
    assert!(h.event(Event::SaveRequested).is_empty());
    assert!(h.event(Event::Quit).is_empty());
    assert!(
        h.machine
            .edit(h.context, |state| state.new_window())
            .is_err()
    );
    assert!(
        h.machine
            .transition(
                Event::Navigate {
                    tab: h.tab,
                    action: Navigation::Home
                },
                h.context
            )
            .is_err()
    );
    let state = h.machine.state().clone();
    assert!(h.event(Event::Replace(AppState::default())).is_empty());
    assert_eq!(h.machine.state(), &state);
    h.event(success(&final_batch));
    assert!(h.machine.history().pending().is_empty());
    assert!(h.event(success(&old)).is_empty());
}

#[test]
fn generated_navigation_and_write_sequences_converge_without_files_or_timers() {
    for seed in 1..=32_u64 {
        let mut h = Harness::persistent();
        let mut random = seed;
        let mut timer = None;
        let mut writing: Option<SaveBatch> = None;
        let mut disk = Vec::<Record>::new();
        let persist = |disk: &mut Vec<Record>, batch: &SaveBatch| {
            for (_, record) in batch.history.as_ref().unwrap() {
                let id = record.id.0 as usize;
                assert!(id <= disk.len(), "incremental writes cannot leave ID gaps");
                if id == disk.len() {
                    disk.push(record.clone());
                } else {
                    disk[id] = record.clone();
                }
            }
        };
        for step in 0..200 {
            random = random.wrapping_mul(6364136223846793005).wrapping_add(1);
            h.context.timestamp_ms = step;
            let event = match (random >> 32) % 10 {
                0 => Event::Navigate {
                    tab: h.tab,
                    action: Navigation::OpenProject(h.project),
                },
                1 => Event::Navigate {
                    tab: h.tab,
                    action: Navigation::Home,
                },
                2 => Event::Navigate {
                    tab: h.tab,
                    action: Navigation::Back,
                },
                3 => Event::Navigate {
                    tab: h.tab,
                    action: Navigation::Forward,
                },
                4 => Event::Navigate {
                    tab: h.tab,
                    action: Navigation::ReopenTab,
                },
                5 => Event::Navigate {
                    tab: h.tab,
                    action: Navigation::SetBranchList(BranchList {
                        first_visible_row: step as u32,
                        ..Default::default()
                    }),
                },
                6 => match timer.take() {
                    Some(id) => Event::SaveDue(id),
                    None => Event::SaveRequested,
                },
                outcome @ (7 | 8) => match writing.take() {
                    Some(batch) if outcome == 7 => {
                        persist(&mut disk, &batch);
                        success(&batch)
                    }
                    Some(batch) => Event::Saved(SaveResult {
                        id: batch.id,
                        workspace: Some(Ok(())),
                        history: Some(Err("injected failure".into())),
                    }),
                    None => Event::SaveRequested,
                },
                9 => Event::SaveRequested,
                _ => unreachable!(),
            };
            let before = h.machine.state().clone();
            let pending = h.machine.history().pending();
            match h.machine.transition(event, h.context) {
                Ok(effects) => {
                    for effect in effects {
                        match effect {
                            Effect::ScheduleSave(id) => {
                                assert!(writing.is_none());
                                timer = Some(id);
                            }
                            Effect::Save(batch) => {
                                assert!(writing.is_none(), "only one batch can be in flight");
                                writing = Some(*batch);
                            }
                            Effect::Changed => {}
                            Effect::Flush(_) => panic!("not quitting yet"),
                        }
                    }
                }
                Err(_) => {
                    assert_eq!(h.machine.state(), &before);
                    assert_eq!(h.machine.history().pending(), pending);
                }
            }
            let dirty = h.machine.history().pending();
            for (index, record) in h.machine.history().records().iter().enumerate() {
                assert_eq!(record.id.0, index as u64);
                if disk.get(index) != Some(record) {
                    assert!(
                        dirty.iter().any(|(_, pending)| pending.id == record.id),
                        "a value not on disk must remain dirty (seed {seed}, step {step})"
                    );
                }
            }
        }
        let mut effects = h.event(Event::Quit);
        let Effect::Flush(batch) = effects.remove(0) else {
            panic!("expected flush")
        };
        // Alternate between a worker already running and one fenced before it starts.
        if seed % 2 == 0
            && let Some(old) = writing
        {
            persist(&mut disk, &old);
            assert!(h.event(success(&old)).is_empty());
        }
        persist(&mut disk, &batch);
        h.event(success(&batch));
        assert_eq!(disk, h.machine.history().records());
        assert!(h.machine.history().pending().is_empty());
    }
}

#[test]
fn disabled_storage_does_not_disable_navigation_or_create_save_effects() {
    let mut h = Harness::new(Persistence::default());
    assert!(matches!(h.open().as_slice(), [Effect::Changed]));
    assert_eq!(h.machine.history().records().len(), 1);
    assert!(h.event(Event::SaveRequested).is_empty());
    let mut effects = h.event(Event::Quit);
    let Effect::Flush(batch) = effects.remove(0) else {
        panic!("expected flush")
    };
    assert!(batch.workspace.is_none());
    assert!(batch.history.is_none());
}

#[test]
fn append_order_not_clock_order_drives_recents_and_opening_preserves_new_tab_behavior() {
    let mut h = Harness::persistent();
    h.comparison();
    h.context.timestamp_ms = -100;
    h.nav(Navigation::Back);
    let record = h.machine.history().recent(1).next().unwrap().clone();
    assert_eq!(record.timestamp_ms, -100);
    assert!(
        !h.event(Event::OpenView {
            id: record.id,
            target: OpenViewIn::NewTab(h.window)
        })
        .is_empty()
    );
    let new_tab = h
        .machine
        .state()
        .window(h.window)
        .unwrap()
        .tabs
        .last()
        .unwrap()
        .id;
    assert_eq!(
        h.machine.state().window(h.window).unwrap().active_tab,
        Some(new_tab)
    );
    assert_ne!(new_tab, h.tab);
    h.event(Event::Navigate {
        tab: new_tab,
        action: Navigation::Back,
    });
    assert_eq!(h.machine.state().tab(new_tab).unwrap().route, Route::Home);
}
