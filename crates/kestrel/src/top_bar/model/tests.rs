use super::*;

struct Harness {
    model: Model,
    commands: Vec<Command>,
}

impl Harness {
    fn new(count: usize) -> Self {
        let mut model = Model::new(7);
        model.update(Msg::LayoutChanged(Geometry {
            width: 1100.,
            ..Geometry::default()
        }));
        for _ in 1..count {
            model.update(Msg::Add);
        }
        if let Some(animation) = &model.animation {
            model.update(Msg::Frame {
                generation: animation.generation,
                elapsed: ANIMATION_DURATION,
            });
        }
        Self {
            model,
            commands: vec![],
        }
    }

    fn send(&mut self, message: Msg) -> Vec<Command> {
        let commands = self.model.update(message);
        self.commands.extend(commands.clone());
        self.assert_invariants();
        commands
    }

    fn start(&mut self, tab: TabId) -> DragToken {
        let token = self.model.next_drag();
        let commands = self.send(Msg::DragStarted { tab, token });
        assert_eq!(commands[0], Command::FocusDrag);
        token
    }

    fn move_to(&mut self, token: DragToken, index: usize, fraction: f32) -> Vec<Command> {
        let layout = self.model.view().layout;
        self.send(Msg::DragMoved {
            token,
            x: self.model.geometry.x
                + SIDE_WIDTH
                + layout.slot_x(index)
                + layout.scroll
                + layout.tab_width * fraction,
            y: self.model.geometry.y + TITLEBAR_HEIGHT / 2.,
        })
    }

    fn order(&self) -> Vec<TabId> {
        self.model.tabs.iter().map(|tab| tab.id).collect()
    }

    fn assert_invariants(&self) {
        let mut ids = self.order();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), self.model.tabs.len());
        assert_eq!(
            self.model.tabs.is_empty(),
            self.model.phase == Phase::Closing
        );
        assert_eq!(
            self.model.active_tab().is_none(),
            self.model.phase == Phase::Closing
        );
        let hidden: Vec<_> = self
            .model
            .view()
            .tabs
            .iter()
            .filter(|tab| tab.hidden)
            .map(|tab| tab.tab.id)
            .collect();
        match self.model.phase {
            Phase::Dragging { tab, .. } | Phase::Detaching { tab, .. } => {
                assert_eq!(hidden, vec![tab]);
                assert_eq!(self.model.active, Some(tab));
            }
            _ => assert!(hidden.is_empty()),
        }
    }
}

fn close(a: f32, b: f32) {
    assert!((a - b).abs() < 0.01, "{a} != {b}");
}

#[test]
fn initial_selection_and_transferred_ids() {
    let h = Harness::new(1);
    assert_eq!(h.model.active_tab().unwrap().title, "Project 1");
    assert_eq!(h.model.phase, Phase::Idle);
    assert!(!h.model.view().draggable);
    assert_eq!(h.model.view().layout.slot_at(100., 24.), None);

    let mut model = Model::with_tab(
        10,
        Tab {
            id: 42,
            title: "Review".into(),
        },
        2,
    );
    assert_eq!(model.update(Msg::Add)[0], Command::Reveal(43));
    assert_eq!(model.active, Some(43));
}

#[test]
fn add_and_select_emit_ordered_commands() {
    let mut h = Harness::new(1);
    let commands = h.send(Msg::Add);
    assert_eq!(
        commands,
        vec![
            Command::Reveal(2),
            Command::RequestFrame {
                generation: 1,
                elapsed: Duration::ZERO
            }
        ]
    );
    assert_eq!(h.order(), vec![1, 2]);
    assert_eq!(h.model.active, Some(2));
    assert!(h.model.view().draggable);
    assert_eq!(h.send(Msg::Select(2)), vec![Command::Reveal(2)]);
    assert!(h.send(Msg::Select(999)).is_empty());
    assert_eq!(h.send(Msg::Select(1))[0], Command::Reveal(1));
    assert_eq!(h.model.active, Some(1));
}

#[test]
fn titlebar_press_only_drags_window_while_idle() {
    let mut h = Harness::new(2);
    assert_eq!(
        h.send(Msg::BackgroundPressed),
        vec![Command::BeginWindowDrag]
    );
    h.start(1);
    assert!(h.send(Msg::BackgroundPressed).is_empty());
}

#[test]
fn drag_start_selects_and_hides_active_or_inactive_source_immediately() {
    for source in [1, 3] {
        let mut h = Harness::new(3);
        h.start(source);
        assert_eq!(h.model.active, Some(source));
        assert!(!h.model.view().interactive);
        assert!(
            h.model
                .view()
                .tabs
                .iter()
                .find(|tab| tab.tab.id == source)
                .unwrap()
                .hidden
        );
        assert!(h.send(Msg::Add).is_empty());
        assert!(h.send(Msg::Select(2)).is_empty());
    }
}

#[test]
fn any_part_of_slot_reorders_in_both_directions() {
    for fraction in [0.0001, 0.2, 0.8, 0.9999] {
        let mut h = Harness::new(3);
        let token = h.start(1);
        assert_eq!(h.move_to(token, 1, fraction).len(), 1);
        assert_eq!(h.order(), vec![2, 1, 3]);
        assert_eq!(h.model.active, Some(1));
        h.move_to(token, 0, fraction);
        assert_eq!(h.order(), vec![1, 2, 3]);
    }
}

#[test]
fn stationary_pointer_does_not_oscillate_even_during_animation() {
    let mut h = Harness::new(3);
    let token = h.start(1);
    h.move_to(token, 1, 0.01);
    for _ in 0..20 {
        assert!(h.move_to(token, 1, 0.01).is_empty());
    }
    assert_eq!(h.order(), vec![2, 1, 3]);
    h.move_to(token, 2, 0.99);
    assert_eq!(h.order(), vec![2, 3, 1]);
    h.move_to(token, 0, 0.01);
    assert_eq!(h.order(), vec![1, 2, 3]);
}

#[test]
fn can_jump_directly_across_multiple_slots() {
    let mut h = Harness::new(5);
    let token = h.start(1);
    h.move_to(token, 4, 0.01);
    assert_eq!(h.order(), vec![2, 3, 4, 5, 1]);
    h.move_to(token, 0, 0.99);
    assert_eq!(h.order(), vec![1, 2, 3, 4, 5]);
}

#[test]
fn geometry_excludes_padding_gaps_and_content_area() {
    let h = Harness::new(3);
    let layout = h.model.view().layout;
    let left = SIDE_WIDTH + PILL_PADDING;
    let top = (TITLEBAR_HEIGHT - TAB_HEIGHT) / 2.;
    let center = TITLEBAR_HEIGHT / 2.;
    assert_eq!(layout.slot_at(left, top), Some(0));
    assert_eq!(layout.slot_at(left - 0.01, center), None);
    assert_eq!(layout.slot_at(left + layout.tab_width, center), None);
    assert_eq!(layout.slot_at(left + 1., top - 0.01), None);
    assert_eq!(layout.slot_at(left + 1., top + TAB_HEIGHT), None);
    assert_eq!(layout.slot_at(left + 1., 200.), None);
}

#[test]
fn tabs_fill_available_width_then_scroll_at_minimum_width() {
    let mut h = Harness::new(3);
    let layout = h.model.view().layout;
    close(layout.pill_width, 924.);
    close(
        layout.tab_width * 3. + TAB_GAP * 2. + PILL_PADDING * 2.,
        layout.pill_width,
    );
    h.send(Msg::LayoutChanged(Geometry {
        x: 10.,
        y: 20.,
        width: 400.,
        scroll: -102.,
    }));
    let layout = h.model.view().layout;
    close(layout.tab_width, 100.);
    // Scroll is clamped to the actual content extent.
    close(layout.scroll, -84.);
    assert_eq!(layout.slot_at(10. + SIDE_WIDTH + 2., 44.), Some(0));
    assert_eq!(layout.slot_at(10. + SIDE_WIDTH + 21., 44.), Some(1));
    assert_eq!(layout.slot_at(10. + SIDE_WIDTH + 224., 44.), None);
    assert_eq!(layout.slot_at(10. + SIDE_WIDTH - 1., 44.), None);
}

#[test]
fn resize_and_scroll_during_drag_use_updated_logical_slots() {
    let mut h = Harness::new(10);
    let token = h.start(1);
    h.send(Msg::LayoutChanged(Geometry {
        width: 600.,
        scroll: -204.,
        ..Geometry::default()
    }));
    assert!(h.move_to(token, 3, 0.1).len() == 1);
    assert_eq!(h.model.tab_index(1), Some(3));
    h.send(Msg::LayoutChanged(Geometry {
        width: 1200.,
        scroll: 0.,
        ..Geometry::default()
    }));
    h.move_to(token, 0, 0.1);
    assert_eq!(h.model.tab_index(1), Some(0));
}

#[test]
fn zero_width_geometry_has_no_targets() {
    let mut h = Harness::new(2);
    h.send(Msg::LayoutChanged(Geometry::default()));
    assert_eq!(h.model.view().layout.slot_at(90., 24.), None);
    assert!(h.model.view().layout.tab_width.is_finite());
}

#[test]
fn animation_has_deterministic_start_midpoint_and_end() {
    let mut h = Harness::new(3);
    let token = h.start(1);
    h.move_to(token, 1, 0.01);
    let layout = h.model.view().layout;
    let pitch = layout.tab_width + TAB_GAP;
    close(h.model.view().tabs[0].offset, pitch);
    close(h.model.view().tabs[1].offset, -pitch);
    let generation = h.model.animation.as_ref().unwrap().generation;
    h.send(Msg::Frame {
        generation,
        elapsed: Duration::from_millis(80),
    });
    close(h.model.view().tabs[0].offset, pitch / 2.);
    assert_eq!(h.model.view().layout, layout);
    h.send(Msg::Frame {
        generation,
        elapsed: ANIMATION_DURATION,
    });
    assert!(h.model.animation.is_none());
    assert!(h.model.view().tabs.iter().all(|tab| tab.offset == 0.));
}

#[test]
fn focus_animation_is_sampled_without_sleeping() {
    let mut h = Harness::new(2);
    h.send(Msg::Select(1));
    close(h.model.view().tabs[0].opacity, 0.88);
    close(h.model.view().tabs[1].opacity, 1.);
    let generation = h.model.animation.as_ref().unwrap().generation;
    h.send(Msg::Frame {
        generation,
        elapsed: Duration::from_millis(80),
    });
    close(h.model.view().tabs[0].opacity, 0.94);
    h.send(Msg::Frame {
        generation,
        elapsed: ANIMATION_DURATION,
    });
    close(h.model.view().tabs[0].opacity, 1.);
    close(h.model.view().tabs[1].opacity, 0.88);
}

#[test]
fn interrupted_animation_retargets_from_current_visual_positions() {
    let mut h = Harness::new(3);
    let token = h.start(1);
    h.move_to(token, 1, 0.1);
    let generation = h.model.animation.as_ref().unwrap().generation;
    h.send(Msg::Frame {
        generation,
        elapsed: Duration::from_millis(64),
    });
    let before = h.model.samples();
    h.move_to(token, 2, 0.1);
    let after = h.model.samples();
    for sample in before {
        let current = after
            .iter()
            .find(|current| current.id == sample.id)
            .unwrap();
        close(current.x, sample.x);
        close(current.opacity, sample.opacity);
    }
    let snapshot = h.model.clone();
    assert!(
        h.send(Msg::Frame {
            generation,
            elapsed: ANIMATION_DURATION
        })
        .is_empty()
    );
    assert_eq!(h.model, snapshot);
}

#[test]
fn duplicate_and_backwards_ticks_are_ignored() {
    let mut h = Harness::new(2);
    h.send(Msg::Select(1));
    let generation = h.model.animation.as_ref().unwrap().generation;
    h.send(Msg::Frame {
        generation,
        elapsed: Duration::from_millis(80),
    });
    let snapshot = h.model.clone();
    for elapsed in [80, 40] {
        assert!(
            h.send(Msg::Frame {
                generation,
                elapsed: Duration::from_millis(elapsed)
            })
            .is_empty()
        );
        assert_eq!(h.model, snapshot);
    }
}

#[test]
fn release_and_cancellation_keep_live_order_and_restore_visibility() {
    for cancel in [false, true] {
        let mut h = Harness::new(3);
        let token = h.start(1);
        h.move_to(token, 2, 0.5);
        let commands = h.send(if cancel {
            Msg::Cancelled { token }
        } else {
            Msg::Released {
                token,
                outside: false,
            }
        });
        assert_eq!(
            commands,
            if cancel {
                vec![Command::StopDrag, Command::RestoreFocus]
            } else {
                vec![Command::RestoreFocus]
            }
        );
        assert_eq!(h.order(), vec![2, 3, 1]);
        assert_eq!(h.model.phase, Phase::Idle);
        assert_eq!(h.model.active, Some(1));
        assert!(
            h.send(Msg::Released {
                token,
                outside: true
            })
            .is_empty()
        );
        assert!(h.move_to(token, 0, 0.1).is_empty());
    }
}

#[test]
fn release_over_content_does_not_detach() {
    let mut h = Harness::new(2);
    let token = h.start(1);
    h.send(Msg::DragMoved {
        token,
        x: 500.,
        y: 500.,
    });
    assert_eq!(
        h.send(Msg::Released {
            token,
            outside: false
        }),
        vec![Command::RestoreFocus]
    );
    assert_eq!(h.order(), vec![1, 2]);
}

#[test]
fn detach_command_retains_source_until_correlated_success() {
    let mut h = Harness::new(3);
    let token = h.start(2);
    let tab = h.model.tabs[1].clone();
    assert_eq!(
        h.send(Msg::Released {
            token,
            outside: true
        }),
        vec![
            Command::RestoreFocus,
            Command::Window(WindowCommand::OpenDetached {
                request: token,
                tab,
                next_tab_id: 4,
            })
        ]
    );
    assert_eq!(h.order(), vec![1, 2, 3]);
    assert_eq!(
        h.model.phase,
        Phase::Detaching {
            tab: 2,
            request: token
        }
    );
    assert!(
        h.send(Msg::Released {
            token,
            outside: true
        })
        .is_empty()
    );
    assert!(h.send(Msg::Add).is_empty());
    assert!(h.send(Msg::Select(1)).is_empty());
    let stale = DragToken {
        serial: token.serial + 1,
        ..token
    };
    assert!(
        h.send(Msg::DetachCompleted {
            request: stale,
            success: true
        })
        .is_empty()
    );
    let commands = h.send(Msg::DetachCompleted {
        request: token,
        success: true,
    });
    assert_eq!(commands[0], Command::Reveal(3));
    assert_eq!(h.order(), vec![1, 3]);
    assert_eq!(h.model.active, Some(3));
    assert!(
        h.send(Msg::DetachCompleted {
            request: token,
            success: true
        })
        .is_empty()
    );
}

#[test]
fn detaching_last_position_selects_previous_neighbor() {
    let mut h = Harness::new(3);
    let token = h.start(3);
    h.send(Msg::Released {
        token,
        outside: true,
    });
    h.send(Msg::DetachCompleted {
        request: token,
        success: true,
    });
    assert_eq!(h.model.active, Some(2));
}

#[test]
fn failed_detach_restores_source_without_losing_order_or_selection() {
    let mut h = Harness::new(3);
    let token = h.start(1);
    h.move_to(token, 1, 0.1);
    h.send(Msg::Released {
        token,
        outside: true,
    });
    assert!(
        h.send(Msg::DetachCompleted {
            request: token,
            success: false
        })
        .is_empty()
    );
    assert_eq!(h.model.phase, Phase::Idle);
    assert_eq!(h.order(), vec![2, 1, 3]);
    assert_eq!(h.model.active, Some(1));
}

#[test]
fn last_tab_only_closes_after_success_and_closing_projection_is_safe() {
    let mut h = Harness::new(1);
    let token = h.start(1);
    h.send(Msg::Released {
        token,
        outside: true,
    });
    assert_eq!(
        h.send(Msg::DetachCompleted {
            request: token,
            success: true
        }),
        vec![Command::Window(WindowCommand::Close)]
    );
    assert_eq!(h.model.phase, Phase::Closing);
    assert!(h.model.view().tabs.is_empty());
    assert!(h.model.active_tab().is_none());
    assert!(h.send(Msg::Add).is_empty());
    assert!(
        h.send(Msg::DetachCompleted {
            request: token,
            success: true
        })
        .is_empty()
    );
}

#[test]
fn foreign_and_stale_drag_events_cannot_change_state() {
    let mut h = Harness::new(3);
    let stale = h.start(1);
    h.send(Msg::Cancelled { token: stale });
    assert!(
        h.send(Msg::DragStarted {
            tab: 1,
            token: stale
        })
        .is_empty()
    );
    let token = h.start(2);
    let foreign = DragToken {
        source: 99,
        ..token
    };
    let snapshot = h.model.clone();
    for invalid in [stale, foreign] {
        assert!(h.move_to(invalid, 0, 0.1).is_empty());
        assert!(
            h.send(Msg::Released {
                token: invalid,
                outside: false
            })
            .is_empty()
        );
        assert!(h.send(Msg::Cancelled { token: invalid }).is_empty());
    }
    assert!(
        h.send(Msg::DragStarted {
            tab: 3,
            token: h.model.next_drag()
        })
        .is_empty()
    );
    assert_eq!(h.model, snapshot);
}

#[test]
fn resize_reveals_selection_but_does_not_scroll_during_drag() {
    let mut h = Harness::new(3);
    let geometry = Geometry {
        width: 400.,
        ..Geometry::default()
    };
    assert_eq!(
        h.send(Msg::LayoutChanged(geometry)),
        vec![Command::Reveal(3)]
    );
    assert!(h.send(Msg::LayoutChanged(geometry)).is_empty());
    h.start(1);
    let commands = h.send(Msg::LayoutChanged(Geometry {
        width: 500.,
        ..geometry
    }));
    assert!(
        commands
            .iter()
            .all(|command| !matches!(command, Command::Reveal(_)))
    );
}

#[test]
fn late_success_from_a_failed_request_cannot_complete_a_new_detach() {
    let mut h = Harness::new(2);
    let first = h.start(1);
    h.send(Msg::Released {
        token: first,
        outside: true,
    });
    h.send(Msg::DetachCompleted {
        request: first,
        success: false,
    });
    let second = h.start(1);
    h.send(Msg::Released {
        token: second,
        outside: true,
    });
    assert!(
        h.send(Msg::DetachCompleted {
            request: first,
            success: true
        })
        .is_empty()
    );
    assert_eq!(
        h.model.phase,
        Phase::Detaching {
            tab: 1,
            request: second
        }
    );
    assert_eq!(h.order(), vec![1, 2]);
    h.send(Msg::DetachCompleted {
        request: second,
        success: true,
    });
    assert_eq!(h.order(), vec![2]);
    assert_eq!(
        h.commands
            .iter()
            .filter(|command| matches!(
                command,
                Command::Window(WindowCommand::OpenDetached { .. })
            ))
            .count(),
        2
    );
}

#[test]
fn single_tab_never_shows_close_button_even_on_hover() {
    let mut h = Harness::new(1);
    assert!(!h.model.view().tabs[0].close_visible);
    h.send(Msg::Hovered {
        tab: 1,
        hovered: true,
    });
    assert!(!h.model.view().tabs[0].close_visible);
    h.send(Msg::Add);
    assert!(h.model.view().tabs.iter().all(|tab| tab.close_visible));
    h.send(Msg::Close(2));
    assert!(!h.model.view().tabs[0].close_visible);
}

#[test]
fn close_visibility_tracks_selection_and_hover_without_changing_selection() {
    let mut h = Harness::new(3);
    let visible = |h: &Harness| {
        h.model
            .view()
            .tabs
            .iter()
            .filter(|tab| tab.close_visible)
            .map(|tab| tab.tab.id)
            .collect::<Vec<_>>()
    };
    assert_eq!(visible(&h), vec![3]);
    h.send(Msg::Hovered {
        tab: 1,
        hovered: true,
    });
    assert_eq!(visible(&h), vec![1, 3]);
    assert_eq!(h.model.active, Some(3));
    h.send(Msg::Hovered {
        tab: 2,
        hovered: true,
    });
    h.send(Msg::Hovered {
        tab: 1,
        hovered: false,
    });
    assert_eq!(visible(&h), vec![2, 3]);
    h.send(Msg::Hovered {
        tab: 2,
        hovered: false,
    });
    assert_eq!(visible(&h), vec![3]);
    h.start(1);
    assert!(visible(&h).is_empty());
}

#[test]
fn closing_inactive_tab_preserves_selection_and_clears_hover() {
    let mut h = Harness::new(3);
    h.send(Msg::Hovered {
        tab: 1,
        hovered: true,
    });
    assert_eq!(h.send(Msg::Close(1))[0], Command::Reveal(3));
    assert_eq!(h.order(), vec![2, 3]);
    assert_eq!(h.model.active, Some(3));
    assert_eq!(h.model.hovered, None);
    assert!(h.send(Msg::Close(1)).is_empty());
    assert!(h.send(Msg::Close(999)).is_empty());
    h.send(Msg::Add);
    assert_eq!(h.order(), vec![2, 3, 4]);
}

#[test]
fn closing_active_tab_selects_next_or_previous_neighbor() {
    for (source, neighbor) in [(1, 2), (2, 3), (3, 2)] {
        let mut h = Harness::new(3);
        h.send(Msg::Select(source));
        assert_eq!(h.send(Msg::Close(source))[0], Command::Reveal(neighbor));
        assert_eq!(h.model.active, Some(neighbor));
        assert_eq!(h.order().len(), 2);
    }
}

#[test]
fn closing_last_tab_closes_the_window_once() {
    let mut h = Harness::new(1);
    assert_eq!(
        h.send(Msg::Close(1)),
        vec![Command::Window(WindowCommand::Close)]
    );
    assert!(h.model.view().tabs.is_empty());
    assert!(h.model.active_tab().is_none());
    assert!(h.send(Msg::Close(1)).is_empty());
}

#[test]
fn close_is_ignored_during_drag_and_pending_detach() {
    let mut h = Harness::new(3);
    let token = h.start(1);
    assert!(h.send(Msg::Close(1)).is_empty());
    assert!(h.send(Msg::Close(2)).is_empty());
    h.send(Msg::Released {
        token,
        outside: true,
    });
    assert!(h.send(Msg::Close(1)).is_empty());
    assert!(h.send(Msg::Close(2)).is_empty());
    assert_eq!(h.order(), vec![1, 2, 3]);
}

#[test]
fn numbered_selection_uses_current_order_and_ignores_missing_slots() {
    let mut h = Harness::new(3);
    let token = h.start(1);
    h.move_to(token, 2, 0.5);
    h.send(Msg::Released {
        token,
        outside: false,
    });
    assert_eq!(h.send(Msg::SelectIndex(0))[0], Command::Reveal(2));
    assert_eq!(h.model.active, Some(2));
    assert!(h.send(Msg::SelectIndex(99)).is_empty());
    assert_eq!(h.model.active, Some(2));
}

#[test]
fn relative_selection_wraps_and_handles_one_tab() {
    let mut h = Harness::new(3);
    assert_eq!(h.send(Msg::SelectNext)[0], Command::Reveal(1));
    assert_eq!(h.send(Msg::SelectPrevious)[0], Command::Reveal(3));
    let mut h = Harness::new(1);
    assert_eq!(h.send(Msg::SelectNext), vec![Command::Reveal(1)]);
    assert_eq!(h.send(Msg::SelectPrevious), vec![Command::Reveal(1)]);
    assert_eq!(
        h.send(Msg::CloseActive),
        vec![Command::Window(WindowCommand::Close)]
    );
    assert!(h.send(Msg::SelectNext).is_empty());
    assert!(h.send(Msg::SelectPrevious).is_empty());
}

#[test]
fn tab_shortcuts_are_ignored_while_dragging_or_detaching() {
    let mut h = Harness::new(3);
    let token = h.start(1);
    for outside in [false, true] {
        if outside {
            h.send(Msg::Released { token, outside });
        }
        let snapshot = h.model.clone();
        for message in [
            Msg::Add,
            Msg::CloseActive,
            Msg::SelectNext,
            Msg::SelectPrevious,
            Msg::SelectIndex(2),
        ] {
            assert!(h.send(message).is_empty());
        }
        assert_eq!(h.model, snapshot);
    }
}

#[test]
fn generated_message_sequences_preserve_invariants() {
    let mut seed = 17_u64;
    let mut h = Harness::new(4);
    for _ in 0..5000 {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        if h.model.phase == Phase::Closing {
            h = Harness::new(4);
        }
        let id = (seed >> 8) % 8 + 1;
        let token = h.model.drag_token().unwrap_or(h.model.next_drag());
        let message = match seed % 11 {
            0 => Msg::Add,
            1 => Msg::Select(id),
            2 => Msg::DragStarted { tab: id, token },
            3 => Msg::DragMoved {
                token,
                x: ((seed >> 16) % 1200) as f32,
                y: 24.,
            },
            4 => Msg::Released {
                token,
                outside: seed & 16 != 0,
            },
            5 => Msg::Cancelled { token },
            6 => Msg::DetachCompleted {
                request: match h.model.phase {
                    Phase::Detaching { request, .. } => request,
                    _ => token,
                },
                success: seed & 32 != 0,
            },
            7 => Msg::Frame {
                generation: h.model.generation,
                elapsed: Duration::from_millis((seed >> 16) % 200),
            },
            8 => Msg::Close(id),
            9 => Msg::Hovered {
                tab: id,
                hovered: seed & 16 != 0,
            },
            _ => Msg::LayoutChanged(Geometry {
                width: ((seed >> 16) % 1500) as f32,
                scroll: -(((seed >> 32) % 400) as f32),
                ..Geometry::default()
            }),
        };
        h.send(message);
    }
}
