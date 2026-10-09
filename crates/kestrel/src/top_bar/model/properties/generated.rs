use super::*;
use hegel::{TestCase, generators as gs};

#[hegel::composite]
fn initial(tc: &TestCase) -> Initial {
    Initial {
        source: tc.draw(gs::integers()),
        first: tc.draw(gs::integers()),
        title: tc.draw(gs::text().max_size(24)),
        count: tc.draw(gs::integers::<usize>().min_value(1).max_value(16)),
    }
}

#[hegel::composite]
fn operation(tc: &TestCase) -> Op {
    match tc.draw(gs::integers::<u8>().max_value(9)) {
        0 => Op::Add,
        1 => Op::Select(tc.draw(gs::integers())),
        2 => Op::SelectIndex(tc.draw(gs::integers())),
        3 => Op::Next,
        4 => Op::Previous,
        5 => Op::Close(tc.draw(gs::integers())),
        6 => Op::CloseActive,
        7 => Op::Hover(tc.draw(gs::integers()), tc.draw(gs::booleans())),
        8 => Op::Background,
        _ => Op::Missing,
    }
}

#[hegel::composite]
fn sequence(tc: &TestCase) -> Sequence {
    Sequence {
        initial: tc.draw(initial()),
        operations: tc.draw(gs::vecs(operation()).max_size(128)),
    }
}

#[hegel::composite]
fn drag(tc: &TestCase) -> Drag {
    Drag {
        initial: tc.draw(initial()),
        start: tc.draw(gs::integers()),
        targets: tc.draw(gs::vecs(gs::integers()).max_size(128)),
        cancel: tc.draw(gs::booleans()),
        success: tc.draw(gs::booleans()),
        elapsed: tc.draw(gs::integers::<u16>().max_value(319)),
    }
}

#[hegel::test]
fn lifecycle(tc: TestCase) {
    check(Case::Lifecycle(tc.draw(sequence())));
}

#[hegel::test]
fn drag_lifecycle(tc: TestCase) {
    check(Case::Drag(tc.draw(drag())));
}

#[hegel::test]
fn detach_protocol(tc: TestCase) {
    check(Case::Detach(tc.draw(drag())));
}

#[hegel::test]
fn rejected_events(tc: TestCase) {
    check(Case::Rejected(tc.draw(drag())));
}

#[hegel::test]
fn animation(tc: TestCase) {
    check(Case::Animation(tc.draw(drag())));
}

#[hegel::test]
fn geometry(tc: TestCase) {
    check(Case::Geometry(GeometryCase {
        count: tc.draw(gs::integers::<usize>().min_value(1).max_value(16)),
        width: tc.draw(gs::integers::<u16>().max_value(4096)),
        x: tc.draw(gs::integers::<i16>().min_value(-1024).max_value(1024)),
        y: tc.draw(gs::integers::<i16>().min_value(-1024).max_value(1024)),
        scroll: tc.draw(gs::integers::<u16>().max_value(4096)),
        pointer_x: tc.draw(gs::integers::<i16>().min_value(-256).max_value(4096)),
        pointer_y: tc.draw(gs::integers::<i16>().min_value(-64).max_value(128)),
    }));
}
