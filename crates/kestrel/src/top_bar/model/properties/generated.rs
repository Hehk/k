use super::*;
use quickcheck::{Arbitrary, Gen, QuickCheck, Testable};

fn bounded(g: &mut Gen, maximum: usize) -> usize {
    usize::arbitrary(g) % (maximum + 1)
}

impl Arbitrary for Initial {
    fn arbitrary(g: &mut Gen) -> Self {
        Self {
            source: u64::arbitrary(g),
            first: u32::arbitrary(g),
            title: (0..bounded(g, 24)).map(|_| char::arbitrary(g)).collect(),
            count: bounded(g, 15) + 1,
        }
    }

    fn shrink(&self) -> Box<dyn Iterator<Item = Self>> {
        Box::new(
            (self.source, self.first, self.title.clone(), self.count)
                .shrink()
                .filter(|(_, _, _, count)| *count >= 1)
                .map(|(source, first, title, count)| Self {
                    source,
                    first,
                    title,
                    count,
                }),
        )
    }
}

impl Arbitrary for Op {
    fn arbitrary(g: &mut Gen) -> Self {
        match bounded(g, 9) {
            0 => Self::Add,
            1 => Self::Select(u8::arbitrary(g)),
            2 => Self::SelectIndex(u8::arbitrary(g)),
            3 => Self::Next,
            4 => Self::Previous,
            5 => Self::Close(u8::arbitrary(g)),
            6 => Self::CloseActive,
            7 => Self::Hover(u8::arbitrary(g), bool::arbitrary(g)),
            8 => Self::Background,
            _ => Self::Missing,
        }
    }

    fn shrink(&self) -> Box<dyn Iterator<Item = Self>> {
        match *self {
            Self::Select(rank) => Box::new(rank.shrink().map(Self::Select)),
            Self::SelectIndex(rank) => Box::new(rank.shrink().map(Self::SelectIndex)),
            Self::Close(rank) => Box::new(rank.shrink().map(Self::Close)),
            Self::Hover(rank, value) => Box::new(
                (rank, value)
                    .shrink()
                    .map(|(rank, value)| Self::Hover(rank, value)),
            ),
            _ => quickcheck::empty_shrinker(),
        }
    }
}

impl Arbitrary for Sequence {
    fn arbitrary(g: &mut Gen) -> Self {
        Self {
            initial: Initial::arbitrary(g),
            operations: (0..bounded(g, 128)).map(|_| Op::arbitrary(g)).collect(),
        }
    }

    fn shrink(&self) -> Box<dyn Iterator<Item = Self>> {
        Box::new(
            (self.initial.clone(), self.operations.clone())
                .shrink()
                .map(|(initial, operations)| Self {
                    initial,
                    operations,
                }),
        )
    }
}

impl Arbitrary for Drag {
    fn arbitrary(g: &mut Gen) -> Self {
        Self {
            initial: Initial::arbitrary(g),
            start: u8::arbitrary(g),
            targets: (0..bounded(g, 128)).map(|_| u8::arbitrary(g)).collect(),
            cancel: bool::arbitrary(g),
            success: bool::arbitrary(g),
            elapsed: bounded(g, 319) as u16,
        }
    }

    fn shrink(&self) -> Box<dyn Iterator<Item = Self>> {
        Box::new(
            (
                self.initial.clone(),
                self.start,
                self.targets.clone(),
                self.cancel,
                self.success,
                self.elapsed,
            )
                .shrink()
                .map(|(initial, start, targets, cancel, success, elapsed)| Self {
                    initial,
                    start,
                    targets,
                    cancel,
                    success,
                    elapsed,
                }),
        )
    }
}

impl Arbitrary for GeometryCase {
    fn arbitrary(g: &mut Gen) -> Self {
        Self {
            count: bounded(g, 15) + 1,
            width: bounded(g, 4096) as u16,
            x: bounded(g, 2048) as i16 - 1024,
            y: bounded(g, 2048) as i16 - 1024,
            scroll: bounded(g, 4096) as u16,
            pointer_x: bounded(g, 4352) as i16 - 256,
            pointer_y: bounded(g, 192) as i16 - 64,
        }
    }

    fn shrink(&self) -> Box<dyn Iterator<Item = Self>> {
        Box::new(
            (
                self.count,
                self.width,
                self.x,
                self.y,
                self.scroll,
                self.pointer_x,
                self.pointer_y,
            )
                .shrink()
                .filter(|(count, ..)| *count >= 1)
                .map(|(count, width, x, y, scroll, pointer_x, pointer_y)| Self {
                    count,
                    width,
                    x,
                    y,
                    scroll,
                    pointer_x,
                    pointer_y,
                }),
        )
    }
}

fn run(property: impl Testable) {
    let cases = std::env::var("QUICKCHECK_TESTS")
        .map(|n| n.parse().unwrap())
        .unwrap_or(256);
    let mut runner = QuickCheck::new()
        .tests(cases)
        .max_tests(cases)
        .min_tests_passed(cases);
    if let Ok(seed) = std::env::var("KESTREL_PBT_SEED") {
        runner = runner.rng(Gen::from_size_and_seed(128, seed.parse().unwrap()));
    }
    runner.quickcheck(property);
}

#[test]
fn lifecycle() {
    fn property(case: Sequence) -> bool {
        check(Case::Lifecycle(case));
        true
    }
    run(property as fn(Sequence) -> bool);
}

#[test]
fn drag_lifecycle() {
    fn property(case: Drag) -> bool {
        check(Case::Drag(case));
        true
    }
    run(property as fn(Drag) -> bool);
}

#[test]
fn detach_protocol() {
    fn property(case: Drag) -> bool {
        check(Case::Detach(case));
        true
    }
    run(property as fn(Drag) -> bool);
}

#[test]
fn rejected_events() {
    fn property(case: Drag) -> bool {
        check(Case::Rejected(case));
        true
    }
    run(property as fn(Drag) -> bool);
}

#[test]
fn animation() {
    fn property(case: Drag) -> bool {
        check(Case::Animation(case));
        true
    }
    run(property as fn(Drag) -> bool);
}

#[test]
fn geometry() {
    fn property(case: GeometryCase) -> bool {
        check(Case::Geometry(case));
        true
    }
    run(property as fn(GeometryCase) -> bool);
}
