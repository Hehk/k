use std::{ops::Range, sync::Arc};

use kestrel::{
    git::Repository,
    review::{DiffLine, FileChange, FileContent, FilePatch, PreviewLimits, PreviewUnavailable},
    structural_diff::{self, Cancellation, Engine, Input},
};

use crate::{appearance::DiffTone, diff_selection::RowSource};

mod structural;

#[cfg(test)]
mod tests;

pub(super) struct LoadedPatch {
    pub content: PreparedPatch,
    pub status: String,
}

impl LoadedPatch {
    pub fn load(
        repository: &Repository,
        change: &FileChange,
        engine: &Engine,
        cancellation: &Cancellation,
    ) -> Result<Self, String> {
        let limits = PreviewLimits::default();
        let (old, new) = match repository
            .contents(change, limits)
            .map_err(|e| e.to_string())?
        {
            (FileContent::Text(old), FileContent::Text(new)) => (old, new),
            (FileContent::Unavailable(reason), _) | (_, FileContent::Unavailable(reason)) => {
                return Ok(Self {
                    content: PreparedPatch::Unavailable(reason),
                    status: "Preview unavailable".into(),
                });
            }
        };
        cancellation.check().map_err(|e| e.to_string())?;
        let hint = super::path(change).label();
        let result = engine.compare(
            Input {
                old: &old,
                new: &new,
                path: &hint,
            },
            structural_diff::Limits {
                source_bytes: limits.blob_bytes,
                output_bytes: limits.patch_bytes,
                lines: limits.patch_lines,
                ..Default::default()
            },
            cancellation,
        );
        let fallback = match result {
            Ok(diff) => {
                let status = format!("Difftastic · {} · Structural", diff.language);
                match PreparedPatch::structural(diff, &old, &new, limits) {
                    Ok(content) => return Ok(Self { content, status }),
                    Err(error) => error.to_string(),
                }
            }
            Err(structural_diff::Error::Cancelled) => return Err("Diff cancelled".into()),
            Err(error) => error.to_string(),
        };
        cancellation.check().map_err(|e| e.to_string())?;
        let content = Repository::text_patch(&old, &new, limits)
            .map(PreparedPatch::new)
            .map_err(|e| e.to_string())?;
        Ok(Self {
            content,
            status: format!("Text fallback · {fallback}"),
        })
    }
}

pub(super) struct DiffRow {
    pub old: gpui::SharedString,
    pub new: gpui::SharedString,
    pub text: gpui::SharedString,
    pub marker: &'static str,
    pub tone: DiffTone,
    pub highlights: Vec<(Range<usize>, DiffTone)>,
    pub source: RowSource,
}

impl DiffRow {
    pub fn is_source(&self) -> bool {
        !self.old.is_empty() || !self.new.is_empty()
    }
}

pub(super) enum PreparedPatch {
    Unavailable(PreviewUnavailable),
    Rows {
        rows: Arc<[DiffRow]>,
        widest: Option<usize>,
        text: gpui::SharedString,
    },
}

impl PreparedPatch {
    fn rows(mut rows: Vec<DiffRow>) -> Self {
        // Selection follows the displayed reading order, not either source file.
        // Assign offsets after grouping; headings and omissions are boundaries,
        // never clipboard text.
        let mut plain = String::new();
        let mut terminated = true;
        for row in &mut rows {
            row.source = if row.is_source() {
                terminated = row.text.ends_with('\n');
                row.text = line_text(&row.text).to_owned().into();
                let start = plain.len();
                plain.push_str(&row.text);
                let source = RowSource::Code(start..plain.len());
                // Keep old/new rows separate even when either lacks a newline.
                plain.push('\n');
                source
            } else {
                RowSource::Boundary(plain.len())
            };
        }
        if !terminated {
            plain.pop();
        }
        let widest = rows
            .iter()
            .enumerate()
            .filter(|(_, row)| row.is_source())
            .max_by_key(|(_, row)| row.text.len())
            .map(|(index, _)| index);
        Self::Rows {
            rows: rows.into(),
            widest,
            text: plain.into(),
        }
    }

    pub fn new(patch: FilePatch) -> Self {
        let hunks = match patch {
            FilePatch::Unavailable(reason) => return Self::Unavailable(reason),
            FilePatch::Hunks(hunks) => hunks,
        };
        let mut rows = Vec::new();
        for hunk in hunks {
            rows.push(metadata(hunk.header.trim_end().to_owned(), DiffTone::Hunk));
            for line in hunk.lines {
                let (old, new, text, tone, marker) = match line {
                    DiffLine::Context {
                        old_line,
                        new_line,
                        text,
                    } => (
                        old_line.to_string(),
                        new_line.to_string(),
                        text,
                        DiffTone::Context,
                        "",
                    ),
                    DiffLine::Added { new_line, text } => (
                        String::new(),
                        new_line.to_string(),
                        text,
                        DiffTone::Added,
                        "+",
                    ),
                    DiffLine::Deleted { old_line, text } => (
                        old_line.to_string(),
                        String::new(),
                        text,
                        DiffTone::Deleted,
                        "−",
                    ),
                    DiffLine::NoNewline(side) => (
                        String::new(),
                        String::new(),
                        format!(
                            "No final newline · {}",
                            match side {
                                kestrel::review::DiffSide::Old => "Before",
                                kestrel::review::DiffSide::New => "After",
                                kestrel::review::DiffSide::Both => "Both",
                            }
                        ),
                        DiffTone::Metadata,
                        "",
                    ),
                };
                rows.push(DiffRow {
                    old: old.into(),
                    new: new.into(),
                    text: text.into(),
                    marker,
                    tone,
                    highlights: Vec::new(),
                    source: RowSource::Boundary(0),
                });
            }
        }
        Self::rows(rows)
    }

    pub fn structural(
        diff: structural_diff::Diff,
        old: &str,
        new: &str,
        limits: PreviewLimits,
    ) -> Result<Self, structural_diff::Error> {
        let rows = structural::rows(&diff, old, new);
        if rows.len() > limits.patch_lines
            || rows.iter().map(|row| row.text.len()).sum::<usize>() > limits.patch_bytes
        {
            return Err(structural_diff::Error::Limit);
        }
        Ok(Self::rows(rows))
    }
}

fn metadata(text: String, tone: DiffTone) -> DiffRow {
    DiffRow {
        old: "".into(),
        new: "".into(),
        text: text.into(),
        marker: "",
        tone,
        highlights: Vec::new(),
        source: RowSource::Boundary(0),
    }
}

fn line_text(text: &str) -> &str {
    text.strip_suffix('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line))
        .unwrap_or(text)
}
