use diff::{Cancellation, Engine, Input};
use kestrel::{
    git::Repository,
    review::{FileChange, FileContent, PreviewLimits},
};

use super::{DiffRow, DiffTone, PreparedPatch, RowSource};

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
            diff::Limits {
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
            Err(diff::Error::Cancelled) => return Err("Diff cancelled".into()),
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

impl PreparedPatch {
    fn structural(
        diff: diff::Diff,
        old: &str,
        new: &str,
        limits: PreviewLimits,
    ) -> Result<Self, diff::Error> {
        let mut rows = structural::rows(&diff, old, new);
        if rows.len() > limits.patch_lines
            || rows.iter().map(|row| row.text.len()).sum::<usize>() > limits.patch_bytes
        {
            return Err(diff::Error::Limit);
        }
        // Adapt source alignment to the existing renderer and its copy buffer.
        // Display order may contain both revisions; metadata is never copied.
        let mut plain = String::new();
        let mut terminated = true;
        for row in &mut rows {
            row.source = if row.old.is_empty() && row.new.is_empty() {
                RowSource::Boundary(plain.len())
            } else {
                terminated = row.text.ends_with('\n');
                row.text = line_text(&row.text).to_owned().into();
                let start = plain.len();
                plain.push_str(&row.text);
                let source = RowSource::Code(start..plain.len());
                plain.push('\n');
                source
            };
        }
        if !terminated {
            plain.pop();
        }
        Ok(Self::Rows {
            rows: rows.into(),
            widest: Default::default(),
            text: plain.into(),
        })
    }
}

fn metadata(text: String, tone: DiffTone) -> DiffRow {
    DiffRow {
        old: "".into(),
        new: "".into(),
        text: text.into(),
        marker: "",
        tone,
        source: RowSource::Boundary(0),
    }
}

fn line_text(text: &str) -> &str {
    text.strip_suffix('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line))
        .unwrap_or(text)
}
