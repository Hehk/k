use std::ops::Range;

use diff::{AlignedLine, Diff, SourceLine};

use super::{DiffRow, DiffTone, RowSource, line_text, metadata};

const CONTEXT_LINES: usize = 3;

pub(super) fn rows(diff: &Diff, old: &str, new: &str) -> Vec<DiffRow> {
    let mut rows = Vec::new();
    if !diff.has_changes {
        rows.push(metadata(
            "No structural changes.".into(),
            DiffTone::Metadata,
        ));
    } else {
        let whole_file = old.is_empty() || new.is_empty();
        let mut end = 0;
        for (index, section) in sections(&diff.lines, whole_file).into_iter().enumerate() {
            omitted(&mut rows, &diff.lines[end..section.start]);
            let lines = &diff.lines[section.clone()];
            rows.push(metadata(
                format!("Change {} · {}", index + 1, location(lines)),
                DiffTone::Hunk,
            ));
            append_section(&mut rows, lines, whole_file);
            end = section.end;
        }
        omitted(&mut rows, &diff.lines[end..]);
    }
    for (side, text) in [("Before", old), ("After", new)] {
        if !text.is_empty() && !text.ends_with('\n') {
            rows.push(metadata(
                format!("No final newline · {side}"),
                DiffTone::Metadata,
            ));
        }
    }
    if old.ends_with('\n') != new.ends_with('\n') && !old.is_empty() && !new.is_empty() {
        rows.push(metadata(
            "Final newline changed.".into(),
            DiffTone::Metadata,
        ));
    }
    rows
}

fn shared(line: &AlignedLine) -> bool {
    match (&line.old, &line.new) {
        (Some(old), Some(new)) => {
            old.changes.is_empty()
                && new.changes.is_empty()
                && old.text.trim_start_matches([' ', '\t'])
                    == new.text.trim_start_matches([' ', '\t'])
        }
        _ => false,
    }
}

fn sections(lines: &[AlignedLine], whole_file: bool) -> Vec<Range<usize>> {
    let mut sections: Vec<Range<usize>> = Vec::new();
    let mut index = 0;
    while index < lines.len() {
        if shared(&lines[index]) {
            index += 1;
            continue;
        }
        let start = index;
        while index < lines.len() && !shared(&lines[index]) {
            index += 1;
        }
        if !whole_file
            && !lines[start..index].iter().any(|line| {
                line.old
                    .iter()
                    .chain(&line.new)
                    .any(|line| !line.changes.is_empty())
            })
        {
            continue;
        }
        let mut range =
            start.saturating_sub(CONTEXT_LINES)..(index + CONTEXT_LINES).min(lines.len());
        // A wrapped expression can span many aligned rows. Context must not
        // split a before/after run, even when only one token in it changed.
        while range.start > 0 && !shared(&lines[range.start]) && !shared(&lines[range.start - 1]) {
            range.start -= 1;
        }
        while range.end < lines.len()
            && !shared(&lines[range.end - 1])
            && !shared(&lines[range.end])
        {
            range.end += 1;
        }
        if let Some(last) = sections.last_mut().filter(|last| last.end >= range.start) {
            last.end = last.end.max(range.end);
        } else {
            sections.push(range);
        }
    }
    sections
}

fn append_section(rows: &mut Vec<DiffRow>, lines: &[AlignedLine], whole_file: bool) {
    let mut index = 0;
    while index < lines.len() {
        if shared(&lines[index]) {
            let line = &lines[index];
            let old = line.old.as_ref().expect("shared lines have both sides");
            let new = line.new.as_ref().expect("shared lines have both sides");
            let mut row = source_row(new, false, false);
            row.old = old.number.to_string().into();
            rows.push(row);
            index += 1;
        } else {
            let start = index;
            while index < lines.len() && !shared(&lines[index]) {
                index += 1;
            }
            let run = &lines[start..index];
            // Read a replacement as one before block followed by one after
            // block, not old/new/old/new. Span-free old lines contain surviving
            // syntax; render that syntax in its new layout instead of doubling
            // reflowed lines or displaying unchanged delimiters as deletions.
            for old in run.iter().filter_map(|line| line.old.as_ref()) {
                if whole_file || !old.changes.is_empty() {
                    rows.push(source_row(old, true, whole_file));
                }
            }
            for new in run.iter().filter_map(|line| line.new.as_ref()) {
                rows.push(source_row(new, false, whole_file));
            }
        }
    }
}

fn location(lines: &[AlignedLine]) -> String {
    fn extent(mut lines: impl Iterator<Item = u32>) -> String {
        let Some(first) = lines.next() else {
            return "—".into();
        };
        match lines.last() {
            Some(last) if last != first => format!("{first}–{last}"),
            _ => first.to_string(),
        }
    }
    let old = extent(
        lines
            .iter()
            .filter_map(|line| line.old.as_ref().map(|line| line.number)),
    );
    let new = extent(
        lines
            .iter()
            .filter_map(|line| line.new.as_ref().map(|line| line.number)),
    );
    format!("Before {old} · After {new}")
}

fn omitted(rows: &mut Vec<DiffRow>, lines: &[AlignedLine]) {
    if !lines.is_empty() {
        rows.push(metadata(
            format!("… {} omitted", location(lines)),
            DiffTone::Metadata,
        ));
    }
}

fn source_row(line: &SourceLine, old: bool, whole_file: bool) -> DiffRow {
    let text = line_text(&line.text);
    let changed = whole_file || !line.changes.is_empty();
    let tone = if old {
        DiffTone::Deleted
    } else {
        DiffTone::Added
    };
    DiffRow {
        old: if old {
            line.number.to_string().into()
        } else {
            "".into()
        },
        new: if old {
            "".into()
        } else {
            line.number.to_string().into()
        },
        text: line.text.clone().into(),
        source: RowSource::Boundary(0),
        marker: if !changed {
            ""
        } else if old {
            "−"
        } else {
            "+"
        },
        tone: if changed { tone } else { DiffTone::Context },
        highlights: line
            .changes
            .iter()
            .filter_map(|range| {
                let end = range.end.min(text.len());
                (range.start < end).then_some((range.start..end, tone))
            })
            .collect(),
    }
}
