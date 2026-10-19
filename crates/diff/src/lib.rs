//! Source comparison independent of Git and GPUI. The backend owns executable
//! discovery, transport, and schema validation; callers only see source lines
//! and changed UTF-8 byte ranges. A future in-process backend uses this boundary.

#![forbid(unsafe_code)]

use std::{
    ops::Range,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use thiserror::Error;

mod difftastic;
mod process;

#[cfg(test)]
mod tests;

pub struct Engine {
    backend: difftastic::Executable,
}

impl Engine {
    pub fn bundled() -> Self {
        let path = std::env::var_os("KESTREL_DIFFTASTIC")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::current_exe()
                    .ok()
                    .map(|path| path.with_file_name("difft"))
                    .filter(|path| path.is_file())
            })
            .or_else(|| option_env!("KESTREL_DIFFTASTIC").map(PathBuf::from))
            .unwrap_or_default();
        Self::new(path)
    }

    /// Use a trusted, single-process helper. Supervision covers only the child
    /// we launch; it must not leave descendants holding its output pipes open.
    pub fn new(executable: PathBuf) -> Self {
        Self {
            backend: difftastic::Executable::new(executable),
        }
    }

    pub fn compare(
        &self,
        input: Input<'_>,
        limits: Limits,
        cancellation: &Cancellation,
    ) -> Result<Diff, Error> {
        cancellation.check()?;
        for source in [input.old, input.new] {
            if source.len() > limits.source_bytes
                || source.split_inclusive('\n').count() > limits.lines
            {
                return Err(Error::Limit);
            }
            if source.contains('\0') {
                return Err(Error::Unsupported);
            }
        }
        self.backend.compare(input, limits, cancellation)
    }
}

#[derive(Clone, Copy)]
pub struct Input<'a> {
    pub old: &'a str,
    pub new: &'a str,
    /// Language hint only, never a destination on disk or a source identity.
    pub path: &'a str,
}

#[derive(Clone, Copy)]
pub struct Limits {
    pub source_bytes: usize,
    pub output_bytes: usize,
    pub lines: usize,
    pub timeout: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            source_bytes: 1024 * 1024,
            output_bytes: 2 * 1024 * 1024,
            lines: 20_000,
            timeout: Duration::from_secs(1),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diff {
    pub language: String,
    pub has_changes: bool,
    pub lines: Vec<AlignedLine>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AlignedLine {
    pub old: Option<SourceLine>,
    pub new: Option<SourceLine>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceLine {
    /// One-based original source line, not a display-row index.
    pub number: u32,
    /// Original source including its line ending, if present.
    pub text: String,
    /// Non-overlapping UTF-8 byte ranges within this line, excluding its LF terminator.
    pub changes: Vec<Range<usize>>,
}

#[derive(Clone, Default)]
pub struct Cancellation(Arc<AtomicBool>);

impl Cancellation {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    pub fn check(&self) -> Result<(), Error> {
        if self.0.load(Ordering::Relaxed) {
            Err(Error::Cancelled)
        } else {
            Ok(())
        }
    }
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("Difftastic is unavailable; install the pinned helper")]
    Unavailable,
    #[error("Difftastic version is incompatible; expected 0.71.0")]
    Version,
    #[error("No structural parser is available for this content")]
    Unsupported,
    #[error("Difftastic used text comparison: {0}")]
    TextFallback(String),
    #[error("Structural diff exceeded the preview limit")]
    Limit,
    #[error("Structural diff timed out")]
    Timeout,
    #[error("Structural diff was cancelled")]
    Cancelled,
    #[error("Difftastic failed")]
    Failed,
    #[error("Difftastic returned invalid source coordinates or output")]
    InvalidOutput,
    #[error("Could not prepare structural comparison: {0}")]
    Io(#[from] std::io::Error),
}
