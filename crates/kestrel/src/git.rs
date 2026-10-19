use std::path::{Path, PathBuf};

use git2::{DiffFindOptions, DiffOptions, ErrorCode, ObjectType, RepositoryOpenFlags};
use thiserror::Error;

use crate::review::{
    Comparison, DiffLine, DiffSide, Endpoints, FileChange, FileContent, FileEntry, FilePatch,
    Freshness, GitPath, Hunk, ObjectId, PreviewLimits, PreviewUnavailable, RefName, RefState,
};

#[cfg(test)]
mod tests;

#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    Git(#[from] git2::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("expected a full local or remote-tracking branch ref")]
    InvalidRef,
    #[error("branch is unavailable: {}", .0.label())]
    MissingRef(RefName),
    #[error("no common ancestor is available locally")]
    NoMergeBase,
    #[error("comparison has {0} merge bases; this is not supported yet")]
    MultipleMergeBases(usize),
    #[error("invalid repository-relative path")]
    InvalidPath,
    #[error("path is unavailable at this revision: {0:?}")]
    MissingPath(GitPath),
    #[error("unsupported tree delta: {0:?}")]
    UnsupportedDelta(git2::Delta),
    #[error("diff entry is missing its path")]
    MissingDiffPath,
    #[error("invalid diff line")]
    InvalidDiffLine,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepositoryInfo {
    pub directory: PathBuf,
    pub common_dir: PathBuf,
    pub current_branch: Option<RefName>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Branch {
    pub name: RefName,
    pub tip: ObjectId,
    pub committed_at: i64,
    pub remote: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub branches: Vec<Branch>,
    pub default_base: Option<RefName>,
}

pub struct Repository(git2::Repository);

impl Repository {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, Error> {
        git2::Repository::open_ext(
            path,
            RepositoryOpenFlags::empty(),
            &[] as &[&std::ffi::OsStr],
        )
        .map(Self)
        .map_err(Into::into)
    }

    pub fn info(&self) -> Result<RepositoryInfo, Error> {
        let current_branch = match self.0.head() {
            Ok(head) if head.is_branch() => Some(RefName(head.name_bytes().to_vec())),
            Ok(_) => None,
            Err(error) if matches!(error.code(), ErrorCode::UnbornBranch | ErrorCode::NotFound) => {
                None
            }
            Err(error) => return Err(error.into()),
        };
        Ok(RepositoryInfo {
            directory: self.0.workdir().unwrap_or(self.0.path()).canonicalize()?,
            common_dir: self.0.commondir().canonicalize()?,
            current_branch,
        })
    }

    pub fn snapshot(&self) -> Result<Snapshot, Error> {
        let mut branches = Vec::new();
        let mut defaults = Vec::new();
        for reference in self.0.references()? {
            let reference = reference?;
            let name = RefName(reference.name_bytes().to_vec());
            if !name.is_branch() {
                continue;
            }
            if let Some(target) = reference.symbolic_target_bytes() {
                if name.0.starts_with(b"refs/remotes/") && name.0.ends_with(b"/HEAD") {
                    defaults.push(RefName(target.to_vec()));
                }
                continue;
            }
            let commit = reference.peel_to_commit()?;
            branches.push(Branch {
                tip: ObjectId(commit.id()),
                committed_at: commit.time().seconds(),
                remote: name.0.starts_with(b"refs/remotes/"),
                name,
            });
        }
        branches.sort_by(|left, right| left.name.0.cmp(&right.name.0));
        defaults.sort_by(|left, right| left.0.cmp(&right.0));
        defaults.dedup();
        let default_base = match defaults.as_slice() {
            [name] if branches.iter().any(|branch| branch.name == *name) => Some(name.clone()),
            _ => None,
        };
        Ok(Snapshot {
            branches,
            default_base,
        })
    }

    pub fn resolve(&self, name: &RefName) -> Result<ObjectId, Error> {
        if !name.is_branch() {
            return Err(Error::InvalidRef);
        }
        for reference in self.0.references()? {
            let reference = reference?;
            if reference.name_bytes() == name.0 {
                return Ok(ObjectId(reference.peel_to_commit()?.id()));
            }
        }
        Err(Error::MissingRef(name.clone()))
    }

    pub fn compare(&self, branch: &RefName, base: &RefName) -> Result<Comparison, Error> {
        self.compare_at(branch, self.resolve(branch)?, base)
    }

    pub fn change_base(
        &self,
        comparison: &Comparison,
        base: &RefName,
    ) -> Result<Comparison, Error> {
        self.compare_at(&comparison.branch, comparison.commits.new, base)
    }

    pub fn load_latest(&self, comparison: &Comparison) -> Result<Comparison, Error> {
        self.compare(&comparison.branch, &comparison.base)
    }

    fn compare_at(
        &self,
        branch: &RefName,
        head: ObjectId,
        base: &RefName,
    ) -> Result<Comparison, Error> {
        self.0.find_commit(head.0)?;
        let base_tip = self.resolve(base)?;
        let bases = match self.0.merge_bases(base_tip.0, head.0) {
            Ok(bases) => bases,
            Err(error) if error.code() == ErrorCode::NotFound => return Err(Error::NoMergeBase),
            Err(error) => return Err(error.into()),
        };
        let old = match bases.len() {
            0 => return Err(Error::NoMergeBase),
            1 => ObjectId(bases[0]),
            count => return Err(Error::MultipleMergeBases(count)),
        };
        Ok(Comparison {
            branch: branch.clone(),
            base: base.clone(),
            base_tip,
            commits: Endpoints { old, new: head },
        })
    }

    pub fn freshness(&self, comparison: &Comparison) -> Result<Freshness, Error> {
        let state = |name| match self.resolve(name) {
            Ok(id) => Ok(RefState::At(id)),
            Err(Error::MissingRef(_)) => Ok(RefState::Missing),
            Err(error) => Err(error),
        };
        Ok(Freshness {
            head: state(&comparison.branch)?,
            base: state(&comparison.base)?,
        })
    }

    pub fn changes(&self, commits: &Endpoints) -> Result<Vec<FileChange>, Error> {
        let old = self.0.find_commit(commits.old.0)?.tree()?;
        let new = self.0.find_commit(commits.new.0)?.tree()?;
        let mut options = DiffOptions::new();
        options.include_typechange(true).skip_binary_check(true);
        let mut diff = self
            .0
            .diff_tree_to_tree(Some(&old), Some(&new), Some(&mut options))?;
        diff.find_similar(Some(
            DiffFindOptions::new().renames(true).rename_limit(1000),
        ))?;
        diff.deltas()
            .map(|delta| {
                let entry = |file: git2::DiffFile<'_>| -> Result<FileEntry, Error> {
                    Ok(FileEntry {
                        path: GitPath(file.path_bytes().ok_or(Error::MissingDiffPath)?.to_vec()),
                        id: ObjectId(file.id()),
                        mode: file.mode().into(),
                    })
                };
                Ok(match delta.status() {
                    git2::Delta::Added => FileChange::Added {
                        new: entry(delta.new_file())?,
                    },
                    git2::Delta::Deleted => FileChange::Deleted {
                        old: entry(delta.old_file())?,
                    },
                    git2::Delta::Modified => FileChange::Modified {
                        old: entry(delta.old_file())?,
                        new: entry(delta.new_file())?,
                    },
                    git2::Delta::Renamed => FileChange::Renamed {
                        old: entry(delta.old_file())?,
                        new: entry(delta.new_file())?,
                    },
                    git2::Delta::Typechange => FileChange::TypeChanged {
                        old: entry(delta.old_file())?,
                        new: entry(delta.new_file())?,
                    },
                    other => return Err(Error::UnsupportedDelta(other)),
                })
            })
            .collect()
    }

    pub fn list_tree(
        &self,
        commit: ObjectId,
        directory: Option<&GitPath>,
    ) -> Result<Vec<FileEntry>, Error> {
        let tree = if let Some(path) = directory {
            self.0.find_tree(self.entry(commit, path)?.id.0)?
        } else {
            self.0.find_commit(commit.0)?.tree()?
        };
        Ok(tree
            .iter()
            .map(|entry| {
                let mut path = Vec::new();
                if let Some(directory) = directory {
                    path.extend_from_slice(&directory.0);
                    path.push(b'/');
                }
                path.extend_from_slice(entry.name_bytes());
                FileEntry {
                    path: GitPath(path),
                    id: ObjectId(entry.id()),
                    mode: entry.filemode() as u32,
                }
            })
            .collect())
    }

    pub fn entry(&self, commit: ObjectId, path: &GitPath) -> Result<FileEntry, Error> {
        if !path.is_valid() {
            return Err(Error::InvalidPath);
        }
        let mut tree = self.0.find_commit(commit.0)?.tree()?;
        let mut parts = path.0.split(|byte| *byte == b'/').peekable();
        while let Some(part) = parts.next() {
            let entry = tree
                .get_name_bytes(part)
                .ok_or_else(|| Error::MissingPath(path.clone()))?
                .to_owned();
            if parts.peek().is_none() {
                return Ok(FileEntry {
                    path: path.clone(),
                    id: ObjectId(entry.id()),
                    mode: entry.filemode() as u32,
                });
            }
            tree = self.0.find_tree(entry.id())?;
        }
        Err(Error::InvalidPath)
    }

    pub fn content(&self, entry: &FileEntry, limits: PreviewLimits) -> Result<FileContent, Error> {
        let unavailable = |reason| Ok(FileContent::Unavailable(reason));
        if entry.mode == 0o160000 {
            return unavailable(PreviewUnavailable::Submodule);
        }
        if !matches!(entry.mode, 0o100644 | 0o100755 | 0o120000) {
            return unavailable(PreviewUnavailable::NotAFile);
        }
        let (size, kind) = self.0.odb()?.read_header(entry.id.0)?;
        if kind != ObjectType::Blob {
            return unavailable(PreviewUnavailable::NotAFile);
        }
        if size > limits.blob_bytes {
            return unavailable(PreviewUnavailable::TooLarge);
        }
        let blob = self.0.find_blob(entry.id.0)?;
        if blob.content().contains(&0) {
            return unavailable(PreviewUnavailable::Binary);
        }
        match std::str::from_utf8(blob.content()) {
            Ok(text) => Ok(FileContent::Text(text.to_owned())),
            Err(_) => unavailable(PreviewUnavailable::Encoding),
        }
    }

    pub fn contents(
        &self,
        change: &FileChange,
        limits: PreviewLimits,
    ) -> Result<(FileContent, FileContent), Error> {
        let text = |entry: Option<&FileEntry>| match entry {
            Some(entry) => self.content(entry, limits),
            None => Ok(FileContent::Text(String::new())),
        };
        Ok((text(change.old_file())?, text(change.new_file())?))
    }

    pub fn patch(&self, change: &FileChange, limits: PreviewLimits) -> Result<FilePatch, Error> {
        match self.contents(change, limits)? {
            (FileContent::Text(old), FileContent::Text(new)) => {
                Self::text_patch(&old, &new, limits)
            }
            (FileContent::Unavailable(reason), _) | (_, FileContent::Unavailable(reason)) => {
                Ok(FilePatch::Unavailable(reason))
            }
        }
    }

    pub fn text_patch(old: &str, new: &str, limits: PreviewLimits) -> Result<FilePatch, Error> {
        if old.len() > limits.blob_bytes || new.len() > limits.blob_bytes {
            return Ok(FilePatch::Unavailable(PreviewUnavailable::TooLarge));
        }
        // Buffer diffs avoid reading checkout attributes or executing configured drivers.
        let patch = git2::Patch::from_buffers(
            old.as_bytes(),
            None,
            new.as_bytes(),
            None,
            Some(DiffOptions::new().context_lines(3)),
        )?;
        let (context, added, deleted) = patch.line_stats()?;
        if patch.size(true, true, true) > limits.patch_bytes
            || context.saturating_add(added).saturating_add(deleted) > limits.patch_lines
        {
            return Ok(FilePatch::Unavailable(PreviewUnavailable::TooLarge));
        }
        let mut hunks = Vec::new();
        for index in 0..patch.num_hunks() {
            let (hunk, count) = patch.hunk(index)?;
            let mut lines = Vec::new();
            for line in 0..count {
                let line = patch.line_in_hunk(index, line)?;
                let text = || String::from_utf8_lossy(line.content()).into_owned();
                lines.push(match line.origin_value() {
                    git2::DiffLineType::Context => DiffLine::Context {
                        old_line: line.old_lineno().ok_or(Error::InvalidDiffLine)?,
                        new_line: line.new_lineno().ok_or(Error::InvalidDiffLine)?,
                        text: text(),
                    },
                    git2::DiffLineType::Addition => DiffLine::Added {
                        new_line: line.new_lineno().ok_or(Error::InvalidDiffLine)?,
                        text: text(),
                    },
                    git2::DiffLineType::Deletion => DiffLine::Deleted {
                        old_line: line.old_lineno().ok_or(Error::InvalidDiffLine)?,
                        text: text(),
                    },
                    git2::DiffLineType::ContextEOFNL => DiffLine::NoNewline(DiffSide::Both),
                    git2::DiffLineType::AddEOFNL => DiffLine::NoNewline(DiffSide::Old),
                    git2::DiffLineType::DeleteEOFNL => DiffLine::NoNewline(DiffSide::New),
                    _ => return Err(Error::InvalidDiffLine),
                });
            }
            hunks.push(Hunk {
                header: String::from_utf8_lossy(hunk.header()).into_owned(),
                lines,
            });
        }
        Ok(FilePatch::Hunks(hunks))
    }
}
