use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ObjectId(pub(crate) git2::Oid);

impl FromStr for ObjectId {
    type Err = git2::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.len() != 40 {
            return Err(git2::Error::from_str("expected a full SHA-1 object ID"));
        }
        git2::Oid::from_str(value).map(Self)
    }
}

impl TryFrom<String> for ObjectId {
    type Error = git2::Error;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.parse()
    }
}

impl From<ObjectId> for String {
    fn from(value: ObjectId) -> Self {
        value.to_string()
    }
}

impl fmt::Display for ObjectId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RefName(pub Vec<u8>);

impl RefName {
    pub fn is_branch(&self) -> bool {
        [b"refs/heads/".as_slice(), b"refs/remotes/".as_slice()]
            .iter()
            .any(|prefix| self.0.starts_with(prefix) && self.0.len() > prefix.len())
            && !self.0.contains(&0)
    }

    pub fn label(&self) -> String {
        let name = self
            .0
            .strip_prefix(b"refs/heads/")
            .or_else(|| self.0.strip_prefix(b"refs/remotes/"))
            .unwrap_or(&self.0);
        String::from_utf8_lossy(name).into_owned()
    }
}

impl From<&str> for RefName {
    fn from(value: &str) -> Self {
        Self(value.as_bytes().to_vec())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct GitPath(pub Vec<u8>);

impl GitPath {
    pub fn is_valid(&self) -> bool {
        !self.0.contains(&0)
            && self
                .0
                .split(|byte| *byte == b'/')
                .all(|part| !part.is_empty() && part != b"." && part != b"..")
    }

    pub fn label(&self) -> String {
        String::from_utf8_lossy(&self.0).into_owned()
    }
}

impl From<&str> for GitPath {
    fn from(value: &str) -> Self {
        Self(value.as_bytes().to_vec())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Endpoints {
    pub old: ObjectId,
    pub new: ObjectId,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Comparison {
    pub branch: RefName,
    pub base: RefName,
    pub base_tip: ObjectId,
    pub commits: Endpoints,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefState {
    At(ObjectId),
    Missing,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Freshness {
    pub head: RefState,
    pub base: RefState,
}

impl Freshness {
    pub fn matches(&self, comparison: &Comparison) -> bool {
        self.head == RefState::At(comparison.commits.new)
            && self.base == RefState::At(comparison.base_tip)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileEntry {
    pub path: GitPath,
    pub id: ObjectId,
    pub mode: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileChange {
    Added { new: FileEntry },
    Deleted { old: FileEntry },
    Modified { old: FileEntry, new: FileEntry },
    Renamed { old: FileEntry, new: FileEntry },
    TypeChanged { old: FileEntry, new: FileEntry },
}

impl FileChange {
    pub fn old_file(&self) -> Option<&FileEntry> {
        match self {
            Self::Added { .. } => None,
            Self::Deleted { old }
            | Self::Modified { old, .. }
            | Self::Renamed { old, .. }
            | Self::TypeChanged { old, .. } => Some(old),
        }
    }

    pub fn new_file(&self) -> Option<&FileEntry> {
        match self {
            Self::Deleted { .. } => None,
            Self::Added { new }
            | Self::Modified { new, .. }
            | Self::Renamed { new, .. }
            | Self::TypeChanged { new, .. } => Some(new),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreviewUnavailable {
    Binary,
    Encoding,
    TooLarge,
    Submodule,
    NotAFile,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileContent {
    Text(String),
    Unavailable(PreviewUnavailable),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiffSide {
    Old,
    New,
    Both,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiffLine {
    Context {
        old_line: u32,
        new_line: u32,
        text: String,
    },
    Added {
        new_line: u32,
        text: String,
    },
    Deleted {
        old_line: u32,
        text: String,
    },
    NoNewline(DiffSide),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hunk {
    pub header: String,
    pub lines: Vec<DiffLine>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FilePatch {
    Hunks(Vec<Hunk>),
    Unavailable(PreviewUnavailable),
}

#[derive(Clone, Copy, Debug)]
pub struct PreviewLimits {
    pub blob_bytes: usize,
    pub patch_bytes: usize,
    pub patch_lines: usize,
}

impl Default for PreviewLimits {
    fn default() -> Self {
        Self {
            blob_bytes: 1024 * 1024,
            patch_bytes: 2 * 1024 * 1024,
            patch_lines: 20_000,
        }
    }
}
