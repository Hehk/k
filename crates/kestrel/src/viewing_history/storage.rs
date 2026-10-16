use std::{
    fs::{self, OpenOptions},
    io::{Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

use thiserror::Error;

use super::{Record, ViewingHistory};
use crate::{
    state::{AppState, StateError},
    storage::Recovery,
};

#[cfg(test)]
mod tests;

// JSON Lines: a format/version header, then one application Record per line. A newline
// commits a record; a later value with the same ID updates it without changing visit order.
// Snapshot project IDs are reconciled by repository identity, never trusted across files.
const HEADER: &[u8] = b"{\"format\":\"kestrel-viewing-history\",\"version\":1}\n";

#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    State(#[from] StateError),
    #[error("unsupported history journal format or version")]
    Version,
    #[error("incomplete final journal line")]
    Interrupted,
    #[error("history journal changed outside its writer")]
    Changed,
    #[error("history must be loaded or safely recovered before appending")]
    NotLoaded,
    #[error("append failed ({write}); rollback failed ({rollback}); restart to recover")]
    Rollback {
        write: String,
        rollback: std::io::Error,
    },
}

pub struct Loaded {
    pub history: ViewingHistory,
    pub recovery: Option<Recovery>,
}

/// One process owns a configuration directory. The workspace serializes append attempts;
/// the last synced offset lets a failed append roll back without rewriting earlier history.
pub struct Store {
    path: PathBuf,
    loaded: bool,
    offset: u64,
    count: usize,
}

impl Store {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            loaded: false,
            offset: 0,
            count: 0,
        }
    }

    pub fn load(&mut self, state: &mut AppState) -> Result<Loaded, Error> {
        self.loaded = false;
        // TODO: Replay eagerly for now. Long-lived journals may eventually need compaction
        // of superseded values and streaming reads, not deletion of viewing records.
        let bytes = match fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                self.loaded = true;
                self.offset = 0;
                self.count = 0;
                return Ok(Loaded {
                    history: ViewingHistory::default(),
                    recovery: None,
                });
            }
            Err(error) => return Err(error.into()),
        };
        let mut records = Vec::new();
        let mut valid = 0;
        let replay = (|| -> Result<(), Error> {
            if bytes.is_empty() {
                return Err(Error::Interrupted);
            }
            for (index, line) in bytes.split_inclusive(|byte| *byte == b'\n').enumerate() {
                if !line.ends_with(b"\n") {
                    return Err(Error::Interrupted);
                }
                if index == 0 {
                    let header: serde_json::Value = serde_json::from_slice(line)?;
                    if header.get("format").and_then(|value| value.as_str())
                        != Some("kestrel-viewing-history")
                        || header.get("version").and_then(|value| value.as_u64()) != Some(1)
                    {
                        return Err(Error::Version);
                    }
                } else {
                    let record: Record = serde_json::from_slice(line)?;
                    let id = check(&record, records.len())?;
                    if id == records.len() {
                        records.push(record);
                    } else {
                        records[id] = record;
                    }
                }
                valid += line.len();
            }
            Ok(())
        })();
        // Publish the reconciled catalog only after replay and any recovery succeed.
        let mut candidate = state.clone();
        // TODO: Registration uses a linear project lookup; measure multi-repository startup
        // before adding an index. Reconcile final records, not every journal amendment.
        for record in &mut records {
            record.viewed.snapshot = record.viewed.snapshot_for(&mut candidate)?;
        }
        let recovery = if let Err(error) = replay {
            let mut backup = tempfile::Builder::new()
                .prefix("kestrel-history-recovery-")
                .suffix(".jsonl")
                .tempfile_in(self.directory())?;
            backup.write_all(&bytes)?;
            backup.as_file().sync_all()?;
            let (_, backup) = backup.keep().map_err(|error| error.error)?;
            self.sync_directory()?;
            // Preserve *all* original bytes before repairing the primary file to its valid
            // prefix. Never append after a partial line, or skip corruption in the middle.
            let mut repaired = tempfile::NamedTempFile::new_in(self.directory())?;
            repaired.write_all(&bytes[..valid])?;
            repaired.as_file().sync_all()?;
            repaired.persist(&self.path).map_err(|error| error.error)?;
            self.sync_directory()?;
            Some(Recovery {
                reason: error.to_string(),
                backup,
            })
        } else {
            None
        };
        self.offset = valid as u64;
        self.count = records.len();
        self.loaded = true;
        *state = candidate;
        Ok(Loaded {
            history: ViewingHistory {
                records,
                ..Default::default()
            },
            recovery,
        })
    }

    pub fn append<'a>(
        &mut self,
        records: impl IntoIterator<Item = &'a Record>,
    ) -> Result<(), Error> {
        self.append_with(records, |file, bytes| file.write_all(bytes))
    }

    fn append_with<'a>(
        &mut self,
        records: impl IntoIterator<Item = &'a Record>,
        write: impl FnOnce(&mut fs::File, &[u8]) -> std::io::Result<()>,
    ) -> Result<(), Error> {
        if !self.loaded {
            return Err(Error::NotLoaded);
        }
        let mut bytes = Vec::new();
        let mut count = self.count;
        for record in records {
            let id = check(record, count)?;
            if id == count {
                count += 1;
            }
            if bytes.is_empty() && self.offset == 0 {
                bytes.extend_from_slice(HEADER);
            }
            serde_json::to_writer(&mut bytes, record)?;
            bytes.push(b'\n');
        }
        if bytes.is_empty() {
            return Ok(());
        }
        fs::create_dir_all(self.directory())?;
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&self.path)?;
        if file.metadata()?.len() != self.offset {
            return Err(Error::Changed);
        }
        file.seek(SeekFrom::Start(self.offset))?;
        let result = (|| -> Result<(), std::io::Error> {
            write(&mut file, &bytes)?;
            file.sync_all()?;
            if self.offset == 0 {
                self.sync_directory()?;
            }
            Ok(())
        })();
        if let Err(write) = result {
            if let Err(rollback) = file.set_len(self.offset).and_then(|()| file.sync_all()) {
                self.loaded = false;
                return Err(Error::Rollback {
                    write: write.to_string(),
                    rollback,
                });
            }
            return Err(write.into());
        }
        self.offset += bytes.len() as u64;
        self.count = count;
        Ok(())
    }

    fn directory(&self) -> &Path {
        self.path
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or(Path::new("."))
    }

    fn sync_directory(&self) -> Result<(), std::io::Error> {
        #[cfg(unix)]
        fs::File::open(self.directory())?.sync_all()?;
        Ok(())
    }
}

fn check(record: &Record, count: usize) -> Result<usize, Error> {
    let id = usize::try_from(record.id.0).map_err(|_| StateError("history ID is too large"))?;
    if id > count {
        return Err(StateError("history IDs must not leave gaps").into());
    }
    let viewed = &record.viewed;
    if viewed.snapshot.project().is_none() {
        return Err(StateError("utility pages are not viewing records").into());
    }
    if !viewed.repository.directory.is_absolute() || !viewed.repository.common_dir.is_absolute() {
        return Err(StateError("history repository paths must be absolute").into());
    }
    viewed.snapshot.check()?;
    Ok(id)
}
