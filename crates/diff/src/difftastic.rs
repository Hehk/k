use std::{
    fs,
    path::PathBuf,
    process::Command,
    sync::{Mutex, OnceLock, TryLockError},
    thread,
    time::{Duration, Instant},
};

use serde::Deserialize;

use super::{AlignedLine, Cancellation, Diff, Error, Input, Limits, SourceLine, process};

// Limit memory across all windows/tabs, not just within one engine instance.
static WORKER: Mutex<()> = Mutex::new(());
const VERSION: &str = "Difftastic 0.71.0";

pub(super) struct Executable {
    path: PathBuf,
    verified: OnceLock<()>,
}

impl Executable {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            verified: OnceLock::new(),
        }
    }

    pub fn compare(
        &self,
        input: Input<'_>,
        limits: Limits,
        cancellation: &Cancellation,
    ) -> Result<Diff, Error> {
        if !self.path.is_absolute() {
            return Err(Error::Unavailable);
        }
        let _worker = loop {
            cancellation.check()?;
            match WORKER.try_lock() {
                Ok(guard) => break guard,
                Err(TryLockError::Poisoned(error)) => break error.into_inner(),
                Err(TryLockError::WouldBlock) => thread::sleep(Duration::from_millis(5)),
            }
        };
        // Queueing behind another tab doesn't spend this request's execution
        // budget. Obsolete queued requests are still cancellable.
        let deadline = Instant::now() + limits.timeout;
        let directory = tempfile::Builder::new().prefix("kestrel-diff-").tempdir()?;
        let empty_path = directory.path().join("bin");
        fs::create_dir(&empty_path)?;
        let command = || {
            let mut command = Command::new(&self.path);
            command
                .current_dir(directory.path())
                .env_clear()
                // Difftastic invokes git check-attr internally. An explicitly
                // empty search path prevents checkout/global attributes and
                // external programs from affecting immutable blob comparison.
                .env("PATH", &empty_path)
                .env("DFT_UNSTABLE", "yes");
            command
        };
        if self.verified.get().is_none() {
            let output = process::run(command().arg("--version"), 4096, deadline, cancellation)?;
            if std::str::from_utf8(&output)
                .ok()
                .and_then(|text| text.lines().next())
                != Some(VERSION)
            {
                return Err(Error::Version);
            }
            let _ = self.verified.set(());
        }
        let old = directory.path().join("old");
        let new = directory.path().join("new");
        fs::write(&old, input.old)?;
        fs::write(&new, input.new)?;
        let bytes = process::run(
            command()
                .args([
                    "--display=json",
                    "--color=never",
                    "--strip-cr=off",
                    "--parse-error-limit=0",
                    "--graph-limit=100000",
                ])
                .arg(format!("--byte-limit={}", limits.source_bytes))
                // The language hint is a separate argument, never an input path.
                .arg("--")
                .arg(input.path)
                .arg(&old)
                .args(["0", "100644"])
                .arg(&new)
                .args(["0", "100644"]),
            limits.output_bytes,
            deadline,
            cancellation,
        )?;
        cancellation.check()?;
        decode(&bytes, input, limits)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    language: String,
    path: String,
    status: Status,
    #[serde(default)]
    aligned_lines: Vec<(Option<usize>, Option<usize>)>,
    #[serde(default)]
    chunks: Vec<Vec<Line>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum Status {
    Changed,
    Unchanged,
    Created,
    Deleted,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Line {
    lhs: Option<Side>,
    rhs: Option<Side>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Side {
    line_number: usize,
    changes: Vec<Change>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Change {
    start: usize,
    end: usize,
    content: String,
    highlight: Highlight,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Highlight {
    Delimiter,
    Normal,
    String,
    Type,
    Comment,
    Keyword,
    TreeSitterError,
}

fn source_lines(text: &str) -> Vec<SourceLine> {
    text.split_inclusive('\n')
        .enumerate()
        .map(|(index, text)| SourceLine {
            number: index as u32 + 1,
            text: text.to_owned(),
            changes: Vec::new(),
        })
        .collect()
}

pub(super) fn decode(bytes: &[u8], input: Input<'_>, limits: Limits) -> Result<Diff, Error> {
    let file: File = serde_json::from_slice(bytes).map_err(|_| Error::InvalidOutput)?;
    if file.path != input.path || file.language.is_empty() || file.language.len() > 256 {
        return Err(Error::InvalidOutput);
    }
    if file.language == "Text" || file.language == "Binary" {
        return Err(Error::Unsupported);
    }
    if let Some(reason) = file
        .language
        .strip_prefix("Text (")
        .and_then(|s| s.strip_suffix(')'))
    {
        return Err(Error::TextFallback(reason.to_owned()));
    }
    let mut old = source_lines(input.old);
    let mut new = source_lines(input.new);
    let lines = match file.status {
        Status::Unchanged => {
            if !file.aligned_lines.is_empty() || !file.chunks.is_empty() {
                return Err(Error::InvalidOutput);
            }
            Vec::new()
        }
        Status::Created | Status::Deleted => {
            if !file.aligned_lines.is_empty() || !file.chunks.is_empty() {
                return Err(Error::InvalidOutput);
            }
            let created = matches!(file.status, Status::Created);
            if (created && (!old.is_empty() || new.is_empty()))
                || (!created && (!new.is_empty() || old.is_empty()))
            {
                return Err(Error::InvalidOutput);
            }
            let source = if created { new } else { old };
            source
                .into_iter()
                .map(|mut line| {
                    let len = line.text.trim_end_matches(['\n', '\r']).len();
                    if len > 0 {
                        line.changes.push(0..len);
                    }
                    if created {
                        AlignedLine {
                            old: None,
                            new: Some(line),
                        }
                    } else {
                        AlignedLine {
                            old: Some(line),
                            new: None,
                        }
                    }
                })
                .collect()
        }
        Status::Changed => {
            let mut seen_old = vec![false; old.len()];
            let mut seen_new = vec![false; new.len()];
            for chunk in file.chunks {
                for line in chunk {
                    if line.lhs.is_none() && line.rhs.is_none() {
                        return Err(Error::InvalidOutput);
                    }
                    for (side, source, seen) in [
                        (line.lhs, &mut old, &mut seen_old),
                        (line.rhs, &mut new, &mut seen_new),
                    ] {
                        if let Some(side) = side {
                            let row = source
                                .get_mut(side.line_number)
                                .ok_or(Error::InvalidOutput)?;
                            if seen[side.line_number] {
                                return Err(Error::InvalidOutput);
                            }
                            seen[side.line_number] = true;
                            for change in side.changes {
                                let _kind = change.highlight;
                                let range = change.start..change.end;
                                let content = row.text.strip_suffix('\n').unwrap_or(&row.text);
                                if range.is_empty()
                                    || content.get(range.clone()) != Some(change.content.as_str())
                                {
                                    return Err(Error::InvalidOutput);
                                }
                                row.changes.push(range);
                            }
                            row.changes.sort_by_key(|range| range.start);
                            if row
                                .changes
                                .windows(2)
                                .any(|pair| pair[0].end > pair[1].start)
                            {
                                return Err(Error::InvalidOutput);
                            }
                        }
                    }
                }
            }
            if !old.iter().chain(&new).any(|line| !line.changes.is_empty()) {
                return Err(Error::InvalidOutput);
            }
            if file.aligned_lines.len() > limits.lines.saturating_add(2) {
                return Err(Error::Limit);
            }
            let mut old = old.into_iter();
            let mut new = new.into_iter();
            let (old_count, new_count) = (old.len(), new.len());
            let (mut old_next, mut new_next) = (0, 0);
            let mut lines = Vec::new();
            for (left, right) in file.aligned_lines {
                if left.is_none() && right.is_none() {
                    return Err(Error::InvalidOutput);
                }
                let old = aligned(left, &mut old_next, old_count, &mut old)?;
                let new = aligned(right, &mut new_next, new_count, &mut new)?;
                if old.is_some() || new.is_some() {
                    lines.push(AlignedLine { old, new });
                }
            }
            if old.len() != 0 || new.len() != 0 {
                return Err(Error::InvalidOutput);
            }
            lines
        }
    };
    if lines.len() > limits.lines {
        return Err(Error::Limit);
    }
    Ok(Diff {
        language: file.language,
        has_changes: !matches!(file.status, Status::Unchanged),
        lines,
    })
}

fn aligned(
    index: Option<usize>,
    next: &mut usize,
    count: usize,
    source: &mut impl Iterator<Item = SourceLine>,
) -> Result<Option<SourceLine>, Error> {
    let Some(index) = index else {
        return Ok(None);
    };
    if index != *next || index > count {
        return Err(Error::InvalidOutput);
    }
    *next += 1;
    // Difftastic appends a newline to nonempty unterminated input, and its
    // split('\n') alignment includes the final empty line. It is not source.
    if index == count {
        Ok(None)
    } else {
        source.next().map(Some).ok_or(Error::InvalidOutput)
    }
}
