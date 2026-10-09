use super::*;
use proptest::prelude::*;

fn initial() -> impl Strategy<Value = Initial> {
    (
        any::<u64>(),
        any::<u32>(),
        prop::collection::vec(any::<char>(), 0..=24),
        1usize..=16,
    )
        .prop_map(|(source, first, title, count)| Initial {
            source,
            first,
            title: title.into_iter().collect(),
            count,
        })
}

fn operation() -> impl Strategy<Value = Op> {
    prop_oneof![
        Just(Op::Add),
        any::<u8>().prop_map(Op::Select),
        any::<u8>().prop_map(Op::SelectIndex),
        Just(Op::Next),
        Just(Op::Previous),
        any::<u8>().prop_map(Op::Close),
        Just(Op::CloseActive),
        (any::<u8>(), any::<bool>()).prop_map(|(rank, value)| Op::Hover(rank, value)),
        Just(Op::Background),
        Just(Op::Missing),
    ]
}

fn sequence() -> impl Strategy<Value = Sequence> {
    (initial(), prop::collection::vec(operation(), 0..=128)).prop_map(|(initial, operations)| {
        Sequence {
            initial,
            operations,
        }
    })
}

fn drag() -> impl Strategy<Value = Drag> {
    (
        initial(),
        any::<u8>(),
        prop::collection::vec(any::<u8>(), 0..=128),
        any::<bool>(),
        any::<bool>(),
        0u16..=319,
    )
        .prop_map(|(initial, start, targets, cancel, success, elapsed)| Drag {
            initial,
            start,
            targets,
            cancel,
            success,
            elapsed,
        })
}

fn geometry_case() -> impl Strategy<Value = GeometryCase> {
    (
        1usize..=16,
        0u16..=4096,
        -1024i16..=1024,
        -1024i16..=1024,
        0u16..=4096,
        -256i16..=4096,
        -64i16..=128,
    )
        .prop_map(
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

proptest! {
    #[test]
    fn lifecycle(case in sequence()) { check(Case::Lifecycle(case)); }

    #[test]
    fn drag_lifecycle(case in drag()) { check(Case::Drag(case)); }

    #[test]
    fn detach_protocol(case in drag()) { check(Case::Detach(case)); }

    #[test]
    fn rejected_events(case in drag()) { check(Case::Rejected(case)); }

    #[test]
    fn animation(case in drag()) { check(Case::Animation(case)); }

    #[test]
    fn geometry(case in geometry_case()) { check(Case::Geometry(case)); }
}
