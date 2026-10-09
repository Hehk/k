mod contracts;
mod generated;

use super::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Initial {
    source: u64,
    first: u32,
    title: String,
    count: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
enum Op {
    Add,
    Select(u8),
    SelectIndex(u8),
    Next,
    Previous,
    Close(u8),
    CloseActive,
    Hover(u8, bool),
    Background,
    Missing,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Sequence {
    initial: Initial,
    operations: Vec<Op>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Drag {
    initial: Initial,
    start: u8,
    targets: Vec<u8>,
    cancel: bool,
    success: bool,
    elapsed: u16,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct GeometryCase {
    count: usize,
    width: u16,
    x: i16,
    y: i16,
    scroll: u16,
    pointer_x: i16,
    pointer_y: i16,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
enum Case {
    Lifecycle(Sequence),
    Drag(Drag),
    Detach(Drag),
    Rejected(Drag),
    Animation(Drag),
    Geometry(GeometryCase),
}

#[derive(Default, Serialize)]
struct Stats {
    steps: usize,
    accepted: usize,
    reorders: usize,
    detach_success: usize,
    detach_failure: usize,
    closes: usize,
}

struct Trace {
    model: Model,
    messages: Vec<(Msg, Vec<Command>)>,
    stats: Stats,
}

impl Trace {
    fn new(initial: &Initial) -> Self {
        assert!((1..=16).contains(&initial.count));
        let first = u64::from(initial.first);
        let mut trace = Self {
            model: Model::with_tab(
                initial.source,
                Tab {
                    id: first,
                    title: initial.title.clone(),
                },
                first + 1,
            ),
            messages: vec![],
            stats: Stats::default(),
        };
        trace.send(Msg::LayoutChanged(Geometry {
            width: SIDE_WIDTH * 2.
                + PILL_PADDING * 2.
                + initial.count as f32 * 100.
                + (initial.count - 1) as f32 * TAB_GAP,
            ..Geometry::default()
        }));
        for _ in 1..initial.count {
            trace.send(Msg::Add);
        }
        trace.settle();
        trace.stats = Stats::default();
        trace
    }

    fn send(&mut self, message: Msg) -> Vec<Command> {
        // Record before updating so a panic inside the reducer still has its input.
        self.messages.push((message.clone(), vec![]));
        let before = self.model.clone();
        let commands = self.model.update(message.clone());
        let changed = before != self.model;
        self.stats.steps += 1;
        self.stats.accepted += usize::from(changed || !commands.is_empty());
        self.stats.reorders += usize::from(matches!(message, Msg::DragMoved { .. }) && changed);
        self.stats.detach_success +=
            usize::from(matches!(message, Msg::DetachCompleted { success: true, .. }) && changed);
        self.stats.detach_failure +=
            usize::from(matches!(message, Msg::DetachCompleted { success: false, .. }) && changed);
        self.stats.closes +=
            usize::from(before.phase != Phase::Closing && self.model.phase == Phase::Closing);
        self.messages.last_mut().unwrap().1 = commands.clone();
        self.invariants();
        commands
    }

    fn order(&self) -> Vec<TabId> {
        self.model.tabs.iter().map(|tab| tab.id).collect()
    }

    fn settle(&mut self) {
        if let Some(animation) = &self.model.animation {
            self.send(Msg::Frame {
                generation: animation.generation,
                elapsed: ANIMATION_DURATION,
            });
        }
    }

    fn ignored(&mut self, message: Msg) {
        let before = self.model.clone();
        assert!(self.send(message).is_empty());
        assert_eq!(self.model, before);
    }

    fn invariants(&self) {
        let model = &self.model;
        let view = model.view();
        let ids = self.order();
        let unique: std::collections::BTreeSet<_> = ids.iter().collect();
        assert_eq!(unique.len(), ids.len());
        assert_eq!(ids.is_empty(), model.phase == Phase::Closing);
        assert_eq!(model.active_tab().map(|tab| tab.id), model.active);
        assert_eq!(model.active.is_none(), ids.is_empty());
        assert!(ids.iter().all(|id| *id < model.next_tab_id));
        assert_eq!(view.interactive, model.phase == Phase::Idle);
        assert_eq!(view.draggable, view.interactive && ids.len() > 1);
        for (index, tab) in view.tabs.iter().enumerate() {
            assert_eq!(model.tab_index(tab.tab.id), Some(index));
            assert_eq!(tab.active, Some(tab.tab.id) == model.active);
            assert_eq!(
                tab.close_visible,
                view.interactive
                    && ids.len() > 1
                    && (tab.active || model.hovered == Some(tab.tab.id))
            );
            assert!(tab.offset.is_finite());
            assert!((0.0..=1.0).contains(&tab.opacity));
        }
        let hidden: Vec<_> = view
            .tabs
            .iter()
            .filter(|tab| tab.hidden)
            .map(|tab| tab.tab.id)
            .collect();
        match model.phase {
            Phase::Dragging { tab, .. } | Phase::Detaching { tab, .. } => {
                assert_eq!(hidden, vec![tab]);
                assert_eq!(model.active, Some(tab));
            }
            _ => assert!(hidden.is_empty()),
        }
    }
}

fn check(case: Case) {
    let initial = match &case {
        Case::Lifecycle(case) => case.initial.clone(),
        Case::Drag(case) | Case::Detach(case) | Case::Rejected(case) | Case::Animation(case) => {
            case.initial.clone()
        }
        Case::Geometry(case) => Initial {
            source: 0,
            first: 0,
            title: String::new(),
            count: case.count,
        },
    };
    let mut trace = Trace::new(&initial);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match &case {
        Case::Lifecycle(case) => contracts::lifecycle(&mut trace, case),
        Case::Drag(case) => contracts::drag(&mut trace, case),
        Case::Detach(case) => contracts::detach(&mut trace, case),
        Case::Rejected(case) => contracts::rejected(&mut trace, case),
        Case::Animation(case) => contracts::animation(&mut trace, case),
        Case::Geometry(case) => contracts::geometry(&mut trace, case),
    }));
    if let Err(error) = result {
        if let Some(path) = std::env::var_os("KESTREL_PBT_FIRST_FAILURE") {
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path)
            {
                Ok(file) => serde_json::to_writer_pretty(file, &case).unwrap(),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => panic!("cannot record original failure: {error}"),
            }
        }
        let reason = error
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| error.downcast_ref::<&str>().copied())
            .unwrap_or("non-string panic");
        panic!(
            "{reason}\nREPLAY_JSON={}\nTRACE={:#?}\nMODEL={:#?}",
            serde_json::to_string(&case).unwrap(),
            trace.messages,
            trace.model
        );
    }
    if std::env::var_os("KESTREL_PBT_STATS").is_some() {
        let property = match case {
            Case::Lifecycle(_) => "lifecycle",
            Case::Drag(_) => "drag",
            Case::Detach(_) => "detach",
            Case::Rejected(_) => "rejected",
            Case::Animation(_) => "animation",
            Case::Geometry(_) => "geometry",
        };
        eprintln!(
            "PBT_STATS={}",
            serde_json::json!({"property": property, "stats": trace.stats})
        );
    }
}

#[test]
fn replay() {
    if let Ok(path) = std::env::var("KESTREL_PBT_REPLAY") {
        let json = std::fs::read_to_string(path).unwrap();
        check(serde_json::from_str(&json).unwrap());
    }
}

#[test]
fn mutation_witnesses() {
    for json in [
        include_str!("witnesses/neighbor.json"),
        include_str!("witnesses/token.json"),
        include_str!("witnesses/detach.json"),
        include_str!("witnesses/boundary.json"),
    ] {
        check(serde_json::from_str(json).unwrap());
    }
}

#[test]
fn contract_examples() {
    let initial = Initial {
        source: 7,
        first: 42,
        title: "Review 🦅".into(),
        count: 3,
    };
    check(Case::Lifecycle(Sequence {
        initial: initial.clone(),
        operations: vec![
            Op::Select(0),
            Op::Next,
            Op::Previous,
            Op::Close(0),
            Op::Add,
            Op::Hover(0, true),
            Op::Hover(1, false),
            Op::SelectIndex(99),
            Op::Missing,
            Op::Background,
            Op::CloseActive,
        ],
    }));
    let drag = Drag {
        initial,
        start: 0,
        targets: vec![2, 0, 1],
        cancel: true,
        success: false,
        elapsed: 80,
    };
    for case in [
        Case::Drag(drag.clone()),
        Case::Detach(drag.clone()),
        Case::Rejected(drag.clone()),
        Case::Animation(drag),
    ] {
        check(case);
    }
    for width in [0, 176, 180, 400, 1100] {
        check(Case::Geometry(GeometryCase {
            count: 3,
            width,
            x: -10,
            y: 20,
            scroll: 102,
            pointer_x: 100,
            pointer_y: 20,
        }));
    }
}
