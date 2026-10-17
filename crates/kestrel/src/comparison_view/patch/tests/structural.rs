use super::*;

fn render(old: &str, new: &str) -> Arc<[DiffRow]> {
    let diff = Engine::bundled()
        .compare(
            Input {
                old,
                new,
                path: "example.rs",
            },
            structural_diff::Limits::default(),
            &Cancellation::default(),
        )
        .unwrap();
    let expected = diff
        .lines
        .iter()
        .flat_map(|line| {
            [
                (line.old.as_ref(), DiffTone::Deleted),
                (line.new.as_ref(), DiffTone::Added),
            ]
            .into_iter()
            .flat_map(|(side, tone)| {
                side.into_iter().flat_map(move |line| {
                    line.changes
                        .iter()
                        .map(move |range| (tone, line.number, line.text[range.clone()].to_owned()))
                })
            })
        })
        .collect::<Vec<_>>();
    let PreparedPatch::Rows { rows, .. } =
        PreparedPatch::structural(diff, old, new, PreviewLimits::default()).unwrap()
    else {
        panic!()
    };
    let mut actual = Vec::new();
    for (source, old_side) in [(old, true), (new, false)] {
        let source_lines = source.split_inclusive('\n').collect::<Vec<_>>();
        let mut previous = 0;
        for row in rows.iter() {
            let number = if old_side { &row.old } else { &row.new };
            if number.is_empty() {
                continue;
            }
            let number: u32 = number.parse().unwrap();
            assert!(number > previous, "source lines duplicated or reordered");
            previous = number;
            let source = line_text(source_lines[number as usize - 1]);
            if old_side && !row.new.is_empty() {
                assert_eq!(
                    source.trim_start_matches([' ', '\t']),
                    row.text.trim_start_matches([' ', '\t'])
                );
            } else {
                assert_eq!(source, row.text.as_ref());
            }
            for (range, tone) in &row.highlights {
                actual.push((*tone, number, row.text[range.clone()].to_owned()));
            }
        }
    }
    for tone in [DiffTone::Deleted, DiffTone::Added] {
        assert_eq!(
            actual
                .iter()
                .filter(|(side, _, _)| *side == tone)
                .collect::<Vec<_>>(),
            expected
                .iter()
                .filter(|(side, _, _)| *side == tone)
                .collect::<Vec<_>>()
        );
    }
    rows
}

#[test]
fn adjacent_edits_read_as_a_before_block_then_an_after_block() {
    let old = "fn main() {\n    let first = alpha(1);\n    let second = beta(2);\n    consume(first, second);\n}\n";
    let new = "fn main() {\n    let first = alpha(10);\n    let second = beta(20);\n    consume(first, second);\n}\n";
    let rows = render(old, new);
    let changed = rows
        .iter()
        .filter(|row| !row.highlights.is_empty())
        .map(|row| (row.old.as_ref(), row.new.as_ref(), row.marker))
        .collect::<Vec<_>>();
    assert_eq!(
        changed,
        [
            ("2", "", "−"),
            ("3", "", "−"),
            ("", "2", "+"),
            ("", "3", "+")
        ]
    );
    assert_eq!(rows[0].text.as_ref(), "Change 1 · Before 1–5 · After 1–5");
}

#[test]
fn wrapping_and_unwrapping_keep_surviving_code_once_and_never_invent_deletions() {
    let old = "fn review(enabled: bool) {\n    show();\n}\n";
    let new = "fn review(enabled: bool) {\n    if enabled {\n        show();\n    }\n}\n";
    let rows = render(old, new);
    assert!(
        !rows
            .iter()
            .any(|row| !row.old.is_empty() && row.new.is_empty())
    );
    assert_eq!(
        rows.iter()
            .filter(|row| !row.new.is_empty())
            .map(|row| row.text.as_ref())
            .collect::<Vec<_>>()
            .join("\n"),
        new.trim_end_matches('\n')
    );
    let rows = render(new, old);
    assert_eq!(
        rows.iter()
            .filter(|row| row.text.contains("show()"))
            .count(),
        1
    );
    assert!(
        rows.iter()
            .any(|row| row.marker == "−" && row.text.contains("if enabled"))
    );
}

#[test]
fn reflowed_expressions_are_not_cut_at_the_context_boundary() {
    let old = "fn main() {\n    foo(1, 2, 3, 4, 5, 6, 7, 8);\n}\n";
    let new = "fn main() {\n    foo(10,\n        2,\n        3,\n        4,\n        5,\n        6,\n        7,\n        8,\n    );\n}\n";
    let rows = render(old, new);
    assert_eq!(
        rows.iter()
            .filter(|row| !row.new.is_empty())
            .map(|row| row.text.as_ref())
            .collect::<Vec<_>>()
            .join("\n"),
        new.trim_end_matches('\n')
    );
    assert_eq!(
        rows.iter().filter(|row| row.tone == DiffTone::Hunk).count(),
        1
    );
}

#[test]
fn separated_changes_have_source_ranges_and_explicit_omissions() {
    let old = format!(
        "fn main() {{\n{}}}\n",
        (1..=30)
            .map(|n| format!("    call_{n}(1);\n"))
            .collect::<String>()
    );
    let new = old
        .replace("call_5(1)", "call_5(2)")
        .replace("call_25(1)", "call_25(2)");
    let rows = render(&old, &new);
    let headings = rows
        .iter()
        .filter(|row| row.tone == DiffTone::Hunk)
        .map(|row| row.text.as_ref())
        .collect::<Vec<_>>();
    assert_eq!(
        headings,
        [
            "Change 1 · Before 3–9 · After 3–9",
            "Change 2 · Before 23–29 · After 23–29"
        ]
    );
    let gaps = rows
        .iter()
        .filter(|row| row.text.ends_with("omitted"))
        .map(|row| row.text.as_ref())
        .collect::<Vec<_>>();
    assert_eq!(
        gaps,
        [
            "… Before 1–2 · After 1–2 omitted",
            "… Before 10–22 · After 10–22 omitted",
            "… Before 30–32 · After 30–32 omitted"
        ]
    );
}

#[test]
fn unicode_multiline_and_one_sided_sources_keep_exact_highlight_identity() {
    for (old, new) in [
        (
            "fn main() { let café = \"🦀\"; foo(1); }",
            "fn main() { let café = \"🦀\"; foo(2); }",
        ),
        (
            "fn main() {\r\n\tfoo(1);\r\n}\r\n",
            "fn main() {\r\n\tfoo(2);\r\n}\r\n",
        ),
        (
            "fn main() {\n    println!(\"old\\nvalue\");\n}\n",
            "fn main() {\n    println!(\"new\\nvalue\");\n}\n",
        ),
        ("", "fn first() {}\n\nfn second() {}\n"),
        ("fn first() {}\n\nfn second() {}\n", ""),
    ] {
        let rows = render(old, new);
        if old.is_empty() || new.is_empty() {
            let marker = if old.is_empty() { "+" } else { "−" };
            assert!(
                rows.iter()
                    .filter(|row| row.is_source())
                    .all(|row| row.marker == marker)
            );
        }
    }
}

#[test]
fn text_fallback_keeps_source_separate_from_markers_and_only_strips_line_endings() {
    let patch = FilePatch::Hunks(vec![kestrel::review::Hunk {
        header: "@@ -0,0 +1,3 @@\n".into(),
        lines: ["+literal\n", "crlf\r\n", "lone\r"]
            .into_iter()
            .enumerate()
            .map(|(i, text)| DiffLine::Added {
                new_line: i as u32 + 1,
                text: text.into(),
            })
            .collect(),
    }]);
    let PreparedPatch::Rows { rows, .. } = PreparedPatch::new(patch) else {
        panic!()
    };
    assert_eq!(rows[1].text.as_ref(), "+literal");
    assert_eq!(rows[2].text.as_ref(), "crlf");
    assert_eq!(rows[3].text.as_ref(), "lone\r");
    assert!(rows[1..].iter().all(|row| row.marker == "+"));
}
