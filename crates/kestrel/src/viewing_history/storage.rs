use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use super::{Labels, Record, RepositoryIdentity, Viewed, ViewingHistory, ViewingId};
use crate::{
    review::{Comparison, RefName},
    state::{AppState, BaseSelection, Destination, Snapshot, StateError, ViewState},
    storage::Recovery,
};

#[cfg(test)]
mod tests;

// File v1: magic[8], version:u32 LE, payload length:u64 LE, SHA-256[32], Postcard payload.
// The payload is Vec<SavedRecord> in append order; its index is the stable ViewingId.
// Postcard serializes owned fields, never Rust layout. Enums use declaration-order tags;
// integers/lengths use varints. Git paths/refs are bytes, object IDs are 40-byte hex strings,
// and local paths use the existing tagged Unix-bytes/Windows-u16 codec.
// SavedRecord, SavedDestination, Labels, ViewState, BaseSelection and Comparison form the
// v1 schema. Changing their serialized form requires a new version and fixture/migration.
const MAGIC: &[u8; 8] = b"KSTRHIST";
const VERSION: u32 = 1;
const HEADER_LEN: usize = 52;

#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Encoding(#[from] postcard::Error),
    #[error(transparent)]
    State(#[from] StateError),
    #[error("unsupported history version")]
    Version,
    #[error("invalid history framing or checksum")]
    Integrity,
    #[error("history must be loaded or safely recovered before saving")]
    NotLoaded,
}

#[derive(Serialize, Deserialize)]
struct SavedRecord {
    timestamp_ms: i64,
    repository: RepositoryIdentity,
    destination: SavedDestination,
    view: ViewState,
    labels: Labels,
}

#[derive(Serialize, Deserialize)]
enum SavedDestination {
    Branches,
    Comparison {
        branch: RefName,
        base: BaseSelection,
        comparison: Option<Comparison>,
    },
}

impl SavedRecord {
    fn capture(record: &Record) -> Result<Self, Error> {
        let Viewed {
            repository,
            snapshot,
            labels,
        } = &record.viewed;
        snapshot.check()?;
        if !repository.directory.is_absolute() || !repository.common_dir.is_absolute() {
            return Err(StateError("history repository paths must be absolute").into());
        }
        let destination = match &snapshot.destination {
            Destination::Home => {
                return Err(StateError("utility pages are not viewing records").into());
            }
            Destination::Branches(_) => SavedDestination::Branches,
            Destination::Comparison {
                branch,
                base,
                comparison,
                ..
            } => SavedDestination::Comparison {
                branch: branch.clone(),
                base: base.clone(),
                comparison: comparison.clone(),
            },
        };
        Ok(Self {
            timestamp_ms: record.timestamp_ms,
            repository: repository.clone(),
            destination,
            view: snapshot.view.clone(),
            labels: labels.clone(),
        })
    }

    fn restore(self, id: usize, state: &mut AppState) -> Result<Record, Error> {
        // Paths, not IDs from a separately saved workspace, identify the repository. No Git IO:
        // missing repositories and missing objects remain reopenable, unavailable destinations.
        let project = self.repository.register(state)?;
        let destination = match self.destination {
            SavedDestination::Branches => Destination::Branches(project),
            SavedDestination::Comparison {
                branch,
                base,
                comparison,
            } => Destination::Comparison {
                project,
                branch,
                base,
                comparison,
            },
        };
        let snapshot = Snapshot {
            destination,
            view: self.view,
        };
        snapshot.check()?;
        Ok(Record {
            id: ViewingId(id as u64),
            timestamp_ms: self.timestamp_ms,
            viewed: Viewed {
                repository: self.repository,
                snapshot,
                labels: self.labels,
            },
        })
    }
}

pub struct Loaded {
    pub history: ViewingHistory,
    pub recovery: Option<Recovery>,
}

/// Like workspace storage, this assumes one process owns a configuration directory.
/// The application's revision fences are not cross-process locks.
pub struct Store {
    path: PathBuf,
    loaded: bool,
}

impl Store {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            loaded: false,
        }
    }

    pub fn load(&mut self, state: &mut AppState) -> Result<Loaded, Error> {
        self.loaded = false;
        // TODO: Measure startup memory/latency before replacing this eager whole-log load.
        let bytes = match fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                self.loaded = true;
                return Ok(Loaded {
                    history: ViewingHistory::default(),
                    recovery: None,
                });
            }
            Err(error) => return Err(error.into()),
        };
        let result = match decode(&bytes, state) {
            Ok(history) => Loaded {
                history,
                recovery: None,
            },
            Err(error) => {
                // Do not enable writes unless the original bytes have a durable recovery copy.
                let mut backup = tempfile::Builder::new()
                    .prefix("kestrel-history-recovery-")
                    .suffix(".bin")
                    .tempfile_in(self.directory())?;
                backup.write_all(&bytes)?;
                backup.as_file().sync_all()?;
                let (_, backup) = backup.keep().map_err(|error| error.error)?;
                self.sync_directory()?;
                Loaded {
                    history: ViewingHistory::default(),
                    recovery: Some(Recovery {
                        reason: error.to_string(),
                        backup,
                    }),
                }
            }
        };
        self.loaded = true;
        Ok(result)
    }

    pub fn save(&mut self, records: &[Record]) -> Result<(), Error> {
        if !self.loaded {
            return Err(Error::NotLoaded);
        }
        // TODO: Whole-log encoding and replacement cost O(history size) per save. Measure before
        // introducing incremental writes; no retention cap or silent pruning substitutes for it.
        let bytes = encode(records)?;
        fs::create_dir_all(self.directory())?;
        let mut file = tempfile::NamedTempFile::new_in(self.directory())?;
        file.write_all(&bytes)?;
        file.as_file().sync_all()?;
        file.persist(&self.path).map_err(|error| error.error)?;
        self.sync_directory()?;
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

fn encode(records: &[Record]) -> Result<Vec<u8>, Error> {
    let saved = records
        .iter()
        .enumerate()
        .map(|(index, record)| {
            if record.id.0 != index as u64 {
                return Err(StateError("history IDs must match append order").into());
            }
            SavedRecord::capture(record)
        })
        .collect::<Result<Vec<_>, Error>>()?;
    frame(&saved)
}

fn frame(saved: &[SavedRecord]) -> Result<Vec<u8>, Error> {
    let payload = postcard::to_stdvec(saved)?;
    let mut bytes = Vec::with_capacity(HEADER_LEN + payload.len());
    bytes.extend_from_slice(MAGIC);
    bytes.extend_from_slice(&VERSION.to_le_bytes());
    bytes.extend_from_slice(&(payload.len() as u64).to_le_bytes());
    bytes.extend_from_slice(&Sha256::digest(&payload));
    bytes.extend_from_slice(&payload);
    Ok(bytes)
}

fn decode(bytes: &[u8], state: &mut AppState) -> Result<ViewingHistory, Error> {
    if bytes.len() < HEADER_LEN || &bytes[..8] != MAGIC {
        return Err(Error::Integrity);
    }
    if bytes[8..12] != VERSION.to_le_bytes() {
        return Err(Error::Version);
    }
    let payload = &bytes[HEADER_LEN..];
    if bytes[12..20] != (payload.len() as u64).to_le_bytes()
        || bytes[20..HEADER_LEN] != Sha256::digest(payload)[..]
    {
        return Err(Error::Integrity);
    }
    let (saved, rest) = postcard::take_from_bytes::<Vec<SavedRecord>>(payload)?;
    if !rest.is_empty() {
        return Err(Error::Integrity);
    }
    // A rejected record must not partly change the otherwise usable workspace catalog.
    let mut candidate = state.clone();
    // TODO: Registration repeats a linear project lookup per record. Measure many-repository
    // loads before introducing a catalog index; the sizing fixture uses just one repository.
    let records = saved
        .into_iter()
        .enumerate()
        .map(|(index, record)| record.restore(index, &mut candidate))
        .collect::<Result<Vec<_>, _>>()?;
    *state = candidate;
    Ok(ViewingHistory {
        records,
        ..Default::default()
    })
}
