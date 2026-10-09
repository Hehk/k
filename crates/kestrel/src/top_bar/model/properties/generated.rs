use super::*;
use bolero::Driver;
use bolero::generator::{TypeGenerator, ValueGenerator, produce};

fn initial() -> impl ValueGenerator<Output = Initial> {
    (
        produce::<u64>(),
        produce::<u32>(),
        produce::<Vec<char>>().with().len(0usize..=24),
        1usize..=16,
    )
        .map_gen(|(source, first, title, count)| Initial {
            source,
            first,
            title: title.into_iter().collect(),
            count,
        })
}

impl TypeGenerator for Op {
    fn generate<D: Driver>(driver: &mut D) -> Option<Self> {
        (0u8..=9, produce::<u8>(), produce::<bool>())
            .map_gen(|(kind, rank, value)| match kind {
                0 => Self::Add,
                1 => Self::Select(rank),
                2 => Self::SelectIndex(rank),
                3 => Self::Next,
                4 => Self::Previous,
                5 => Self::Close(rank),
                6 => Self::CloseActive,
                7 => Self::Hover(rank, value),
                8 => Self::Background,
                _ => Self::Missing,
            })
            .generate(driver)
    }
}

fn sequence() -> impl ValueGenerator<Output = Sequence> {
    (initial(), produce::<Vec<Op>>().with().len(0usize..=128)).map_gen(|(initial, operations)| {
        Sequence {
            initial,
            operations,
        }
    })
}

fn drag() -> impl ValueGenerator<Output = Drag> {
    (
        initial(),
        produce::<u8>(),
        produce::<Vec<u8>>().with().len(0usize..=128),
        produce::<bool>(),
        produce::<bool>(),
        0u16..=319,
    )
        .map_gen(|(initial, start, targets, cancel, success, elapsed)| Drag {
            initial,
            start,
            targets,
            cancel,
            success,
            elapsed,
        })
}

fn geometry_case() -> impl ValueGenerator<Output = GeometryCase> {
    (
        1usize..=16,
        0u16..=4096,
        -1024i16..=1024,
        -1024i16..=1024,
        0u16..=4096,
        -256i16..=4096,
        -64i16..=128,
    )
        .map_gen(
            |(count, width, x, y, scroll, pointer_x, pointer_y)| GeometryCase {
                count,
                width,
                x,
                y,
                scroll,
                pointer_x,
                pointer_y,
            },
        )
}

fn cases() -> usize {
    std::env::var("KESTREL_PBT_CASES")
        .map(|n| n.parse().unwrap())
        .unwrap_or(256)
}

#[test]
fn lifecycle() {
    bolero::check!()
        .with_generator(sequence())
        .with_iterations(cases())
        .for_each(|case| check(Case::Lifecycle(case.clone())));
}

#[test]
fn drag_lifecycle() {
    bolero::check!()
        .with_generator(drag())
        .with_iterations(cases())
        .for_each(|case| check(Case::Drag(case.clone())));
}

#[test]
fn detach_protocol() {
    bolero::check!()
        .with_generator(drag())
        .with_iterations(cases())
        .for_each(|case| check(Case::Detach(case.clone())));
}

#[test]
fn rejected_events() {
    bolero::check!()
        .with_generator(drag())
        .with_iterations(cases())
        .for_each(|case| check(Case::Rejected(case.clone())));
}

#[test]
fn animation() {
    bolero::check!()
        .with_generator(drag())
        .with_iterations(cases())
        .for_each(|case| check(Case::Animation(case.clone())));
}

#[test]
fn geometry() {
    bolero::check!()
        .with_generator(geometry_case())
        .with_iterations(cases())
        .for_each(|case| check(Case::Geometry(case.clone())));
}
