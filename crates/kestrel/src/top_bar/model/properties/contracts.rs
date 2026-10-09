use super::*;

fn effects(commands: Vec<Command>) -> Vec<Command> {
    commands
        .into_iter()
        .filter(|command| !matches!(command, Command::RequestFrame { .. }))
        .collect()
}

pub(super) fn lifecycle(trace: &mut Trace, case: &Sequence) {
    let first = u64::from(case.initial.first);
    let mut ids: Vec<_> = (first..first + case.initial.count as u64).collect();
    let mut selected = *ids.last().unwrap();
    let mut next = first + case.initial.count as u64;
    let mut hovered = None;
    for operation in &case.operations {
        if ids.is_empty() {
            break;
        }
        let mut expected = vec![];
        let message = match *operation {
            Op::Add => {
                selected = next;
                ids.push(next);
                next += 1;
                expected.push(Command::Reveal(selected));
                Msg::Add
            }
            Op::Select(rank) => {
                selected = ids[usize::from(rank) % ids.len()];
                expected.push(Command::Reveal(selected));
                Msg::Select(selected)
            }
            Op::SelectIndex(index) => {
                if let Some(id) = ids.get(usize::from(index)) {
                    selected = *id;
                    expected.push(Command::Reveal(selected));
                }
                Msg::SelectIndex(usize::from(index))
            }
            Op::Next | Op::Previous => {
                let at = ids.iter().position(|id| *id == selected).unwrap();
                let mut ring = ids.clone();
                ring.rotate_left(at);
                selected = if matches!(operation, Op::Next) {
                    ring[1 % ring.len()]
                } else {
                    *ring.last().unwrap()
                };
                expected.push(Command::Reveal(selected));
                if matches!(operation, Op::Next) {
                    Msg::SelectNext
                } else {
                    Msg::SelectPrevious
                }
            }
            Op::Close(_) | Op::CloseActive => {
                let id = match *operation {
                    Op::Close(rank) => ids[usize::from(rank) % ids.len()],
                    _ => selected,
                };
                let index = ids.iter().position(|item| *item == id).unwrap();
                let neighbor = ids
                    .get(index + 1)
                    .copied()
                    .or_else(|| index.checked_sub(1).map(|i| ids[i]));
                ids.retain(|item| *item != id);
                if hovered == Some(id) {
                    hovered = None;
                }
                if ids.is_empty() {
                    expected.push(Command::Window(WindowCommand::Close));
                } else {
                    if selected == id {
                        selected = neighbor.unwrap();
                    }
                    expected.push(Command::Reveal(selected));
                }
                if matches!(operation, Op::CloseActive) {
                    Msg::CloseActive
                } else {
                    Msg::Close(id)
                }
            }
            Op::Hover(rank, value) => {
                let id = ids[usize::from(rank) % ids.len()];
                if value {
                    hovered = Some(id);
                } else if hovered == Some(id) {
                    hovered = None;
                }
                Msg::Hovered {
                    tab: id,
                    hovered: value,
                }
            }
            Op::Background => {
                expected.push(Command::BeginWindowDrag);
                Msg::BackgroundPressed
            }
            Op::Missing => Msg::Select(u64::MAX),
        };
        assert_eq!(effects(trace.send(message)), expected);
        assert_eq!(trace.order(), ids);
        assert_eq!(trace.model.active, (!ids.is_empty()).then_some(selected));
        assert_eq!(trace.model.next_tab_id, next);
        assert_eq!(trace.model.hovered, hovered);
    }
}

fn start(trace: &mut Trace, case: &Drag) -> (TabId, DragToken) {
    let tab = u64::from(case.initial.first) + u64::from(case.start) % case.initial.count as u64;
    let token = DragToken {
        source: case.initial.source,
        serial: 1,
    };
    assert_eq!(trace.model.next_drag(), token);
    assert_eq!(
        effects(trace.send(Msg::DragStarted { tab, token })),
        vec![Command::FocusDrag]
    );
    assert_eq!(trace.model.drag_token(), Some(token));
    assert_eq!(trace.model.active, Some(tab));
    assert_eq!(trace.model.next_drag(), DragToken { serial: 2, ..token });
    (tab, token)
}

fn point(index: usize) -> (f32, f32) {
    // The fixture constructs 100px slots with 2px gaps, independent of Layout.
    (
        SIDE_WIDTH + PILL_PADDING + index as f32 * 102. + 50.,
        TITLEBAR_HEIGHT / 2.,
    )
}

fn reorder(trace: &mut Trace, case: &Drag) -> (TabId, DragToken) {
    let (tab, token) = start(trace, case);
    let first = u64::from(case.initial.first);
    let mut expected: Vec<_> = (first..first + case.initial.count as u64).collect();
    for rank in &case.targets {
        let index = usize::from(*rank) % expected.len();
        let (x, y) = point(index);
        trace.send(Msg::DragMoved { token, x, y });
        if expected.len() > 1 {
            expected.retain(|id| *id != tab);
            expected.insert(index, tab);
        }
        assert_eq!(trace.order(), expected);
        assert_eq!(trace.model.active, Some(tab));
        trace.ignored(Msg::DragMoved { token, x, y });
    }
    (tab, token)
}

pub(super) fn drag(trace: &mut Trace, case: &Drag) {
    let (_, token) = reorder(trace, case);
    let order = trace.order();
    let selected = trace.model.active;
    let (message, expected) = if case.cancel {
        (
            Msg::Cancelled { token },
            vec![Command::StopDrag, Command::RestoreFocus],
        )
    } else {
        (
            Msg::Released {
                token,
                outside: false,
            },
            vec![Command::RestoreFocus],
        )
    };
    assert_eq!(trace.send(message), expected);
    assert_eq!(trace.model.phase, Phase::Idle);
    assert_eq!(trace.order(), order);
    assert_eq!(trace.model.active, selected);
    trace.ignored(Msg::Released {
        token,
        outside: true,
    });
    trace.ignored(Msg::Cancelled { token });
    trace.ignored(Msg::DragMoved {
        token,
        x: 90.,
        y: 20.,
    });
}

pub(super) fn detach(trace: &mut Trace, case: &Drag) {
    let (tab, token) = reorder(trace, case);
    let order = trace.order();
    let source = trace.model.active_tab().unwrap().clone();
    let next = trace.model.next_tab_id;
    assert_eq!(
        trace.send(Msg::Released {
            token,
            outside: true
        }),
        vec![
            Command::RestoreFocus,
            Command::Window(WindowCommand::OpenDetached {
                request: token,
                tab: source,
                next_tab_id: next
            }),
        ]
    );
    assert_eq!(trace.order(), order);
    assert_eq!(
        trace.model.phase,
        Phase::Detaching {
            tab,
            request: token
        }
    );
    trace.ignored(Msg::Released {
        token,
        outside: true,
    });
    trace.ignored(Msg::DetachCompleted {
        request: DragToken {
            source: token.source ^ 1,
            ..token
        },
        success: true,
    });
    trace.ignored(Msg::DetachCompleted {
        request: DragToken { serial: 0, ..token },
        success: true,
    });
    let commands = effects(trace.send(Msg::DetachCompleted {
        request: token,
        success: case.success,
    }));
    if case.success {
        let remaining: Vec<_> = order.iter().copied().filter(|id| *id != tab).collect();
        assert_eq!(trace.order(), remaining);
        if remaining.is_empty() {
            assert_eq!(commands, vec![Command::Window(WindowCommand::Close)]);
        } else {
            let index = order.iter().position(|id| *id == tab).unwrap();
            let selected = order
                .get(index + 1)
                .copied()
                .unwrap_or_else(|| order[index - 1]);
            assert_eq!(trace.model.active, Some(selected));
            assert_eq!(commands, vec![Command::Reveal(selected)]);
            assert_eq!(trace.model.phase, Phase::Idle);
        }
    } else {
        assert!(commands.is_empty());
        assert_eq!(trace.order(), order);
        assert_eq!(trace.model.active, Some(tab));
        assert_eq!(trace.model.phase, Phase::Idle);
    }
    trace.ignored(Msg::DetachCompleted {
        request: token,
        success: !case.success,
    });
    trace.ignored(Msg::DetachCompleted {
        request: token,
        success: case.success,
    });
    if trace.model.phase == Phase::Idle {
        let current = trace.model.next_drag();
        let tab = trace.model.active.unwrap();
        trace.send(Msg::DragStarted {
            tab,
            token: current,
        });
        trace.send(Msg::Released {
            token: current,
            outside: true,
        });
        trace.ignored(Msg::DetachCompleted {
            request: token,
            success: true,
        });
        trace.send(Msg::DetachCompleted {
            request: current,
            success: false,
        });
    }
}

fn busy(trace: &mut Trace, tab: TabId) {
    for message in [
        Msg::Add,
        Msg::Select(tab),
        Msg::SelectIndex(0),
        Msg::SelectNext,
        Msg::SelectPrevious,
        Msg::Close(tab),
        Msg::CloseActive,
        Msg::BackgroundPressed,
        Msg::Hovered { tab, hovered: true },
        Msg::DragStarted {
            tab,
            token: trace.model.next_drag(),
        },
    ] {
        trace.ignored(message);
    }
}

pub(super) fn rejected(trace: &mut Trace, case: &Drag) {
    let tab = u64::from(case.initial.first);
    let next = trace.model.next_drag();
    for message in [
        Msg::Select(u64::MAX),
        Msg::Close(u64::MAX),
        Msg::SelectIndex(usize::MAX),
        Msg::Hovered {
            tab: u64::MAX,
            hovered: true,
        },
        Msg::DragStarted {
            tab: u64::MAX,
            token: next,
        },
        Msg::DragStarted {
            tab,
            token: DragToken {
                source: next.source ^ 1,
                ..next
            },
        },
    ] {
        trace.ignored(message);
    }
    let (tab, token) = start(trace, case);
    busy(trace, tab);
    for invalid in [
        DragToken { serial: 0, ..token },
        DragToken {
            source: token.source ^ 1,
            ..token
        },
    ] {
        let (x, y) = point((usize::from(case.start) + 1) % case.initial.count);
        for message in [
            Msg::DragMoved {
                token: invalid,
                x,
                y,
            },
            Msg::Released {
                token: invalid,
                outside: true,
            },
            Msg::Cancelled { token: invalid },
            Msg::DetachCompleted {
                request: invalid,
                success: true,
            },
        ] {
            trace.ignored(message);
        }
    }
    trace.send(Msg::Released {
        token,
        outside: true,
    });
    busy(trace, tab);
    trace.ignored(Msg::Cancelled { token });
    trace.send(Msg::DetachCompleted {
        request: token,
        success: false,
    });
    trace.ignored(Msg::DragStarted { tab, token });
    while trace.model.phase != Phase::Closing {
        trace.send(Msg::CloseActive);
    }
    busy(trace, tab);
    trace.ignored(Msg::LayoutChanged(Geometry {
        width: 1000.,
        ..Geometry::default()
    }));
    trace.ignored(Msg::Frame {
        generation: trace.model.generation,
        elapsed: ANIMATION_DURATION,
    });
    trace.ignored(Msg::Released {
        token,
        outside: true,
    });
}

pub(super) fn animation(trace: &mut Trace, case: &Drag) {
    let (tab, token) = reorder(trace, case);
    let generation = trace.model.animation.as_ref().unwrap().generation;
    let layout = trace.model.view().layout;
    let order = trace.order();
    let elapsed = Duration::from_millis(u64::from(case.elapsed % 159) + 1);
    assert_eq!(
        trace.send(Msg::Frame {
            generation,
            elapsed
        }),
        vec![Command::RequestFrame {
            generation,
            elapsed
        }]
    );
    assert_eq!(trace.order(), order);
    assert_eq!(trace.model.active, Some(tab));
    assert_eq!(trace.model.view().layout, layout);
    trace.ignored(Msg::Frame {
        generation,
        elapsed,
    });
    trace.ignored(Msg::Frame {
        generation,
        elapsed: Duration::ZERO,
    });
    trace.ignored(Msg::Frame {
        generation: generation - 1,
        elapsed: ANIMATION_DURATION,
    });
    let before = trace.model.samples();
    let resized = Geometry {
        width: trace.model.geometry().width + f32::from(case.elapsed) + 1.,
        ..trace.model.geometry()
    };
    let commands = trace.send(Msg::LayoutChanged(resized));
    assert!(
        commands
            .iter()
            .all(|command| !matches!(command, Command::Reveal(_)))
    );
    assert_eq!(trace.order(), order);
    for (old, new) in before.iter().zip(trace.model.samples()) {
        assert_eq!(old.id, new.id);
        // At these bounded pixel magnitudes, 0.001px covers f32 arithmetic error.
        assert!((old.x - new.x).abs() < 0.001);
        assert_eq!(old.opacity, new.opacity);
    }
    trace.ignored(Msg::Frame {
        generation,
        elapsed: ANIMATION_DURATION,
    });
    trace.settle();
    assert!(trace.model.animation.is_none());
    for projected in trace.model.view().tabs {
        assert_eq!(projected.offset, 0.);
        assert_eq!(projected.opacity, if projected.active { 1. } else { 0.88 });
    }
    trace.send(Msg::Cancelled { token });
    assert_eq!(trace.model.active, Some(tab));
}

pub(super) fn geometry(trace: &mut Trace, case: &GeometryCase) {
    let geometry = Geometry {
        x: f32::from(case.x),
        y: f32::from(case.y),
        width: f32::from(case.width),
        scroll: -f32::from(case.scroll),
    };
    trace.send(Msg::LayoutChanged(geometry));
    let layout = trace.model.view().layout;
    assert!(layout.tab_width >= 100. && layout.tab_width.is_finite());
    assert!(layout.pill_width >= 0. && layout.scroll <= 0.);
    // Independent rectangle scan in f64, rather than the production quotient/floor hit test.
    let pill = (f64::from(case.width) - 176.).max(0.);
    let width = ((pill - 4. - (case.count - 1) as f64 * 2.) / case.count as f64).max(100.);
    let content = width * case.count as f64 + (case.count - 1) as f64 * 2. + 4.;
    let scroll = (-f64::from(case.scroll)).max(-(content - pill).max(0.));
    assert!((f64::from(layout.tab_width) - width).abs() < 0.001);
    assert!((f64::from(layout.scroll) - scroll).abs() < 0.001);
    let expected = |x: f64, y: f64| {
        if case.count < 2 || !(7.0..33.0).contains(&y) || x < 90. || x >= 88. + pill - 2. {
            return None;
        }
        (0..case.count).find(|index| {
            let left = 90. + scroll + *index as f64 * (width + 2.);
            x >= left && x < left + width
        })
    };
    for (x, y) in [
        (f64::from(case.pointer_x), f64::from(case.pointer_y)),
        (90., 7.),
        (90., 33.),
        (89., 20.),
        (88. + pill - 2., 20.),
    ] {
        let actual = layout.slot_at(geometry.x + x as f32, geometry.y + y as f32);
        assert_eq!(
            actual,
            expected(x, y),
            "local pointer ({x}, {y}), layout {layout:?}"
        );
    }
    // Exact boundaries and translation on an integer grid avoid inventing f32 equality laws.
    let fixed = Geometry {
        width: 180. + case.count as f32 * 100. + (case.count - 1) as f32 * 2.,
        scroll: 0.,
        ..geometry
    };
    trace.send(Msg::LayoutChanged(fixed));
    let translated = trace.model.view().layout;
    let origin = Layout::new(
        Geometry {
            x: 0.,
            y: 0.,
            ..fixed
        },
        case.count,
    );
    for index in 0..case.count {
        let left = 90. + index as f32 * 102.;
        for (x, hit) in [
            (left, Some(index)),
            (left + 99., Some(index)),
            (left + 100., None),
            (left - 1., None),
        ] {
            let expected = if case.count < 2 { None } else { hit };
            assert_eq!(origin.slot_at(x, 20.), expected);
            assert_eq!(
                translated.slot_at(geometry.x + x, geometry.y + 20.),
                expected
            );
        }
    }
}
