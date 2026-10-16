use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use sha2::{Digest, Sha256};
use thiserror::Error;

use super::{Record, ViewingHistory};
use crate::{
    state::{AppState, StateError},
    storage::Recovery,
};

mod v1;

#[cfg(test)]
mod tests;

// Framing: magic[8], version:u32 LE, payload length:u64 LE, SHA-256[32], Postcard payload.
// File v2 serializes Vec<Record> directly, in append order. IDs must match vector positions.
// Stored project IDs are not trusted; repository identity replaces them on load.
// Postcard serializes owned fields, never Rust layout. Enums use declaration-order tags;
// integers/lengths use varints. Git paths/refs are bytes, object IDs are 40-byte hex strings,
// and local paths use the existing tagged Unix-bytes/Windows-u16 codec.
// Record and its nested application types form the v2 schema. Changing their serialized
// form requires a new version and fixture/migration. V1 decoding is isolated in v1.rs.
const MAGIC: &[u8; 8] = b"KSTRHIST";
const VERSION: u32 = 2;
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
    check(records)?;
    frame(records)
}

fn check(records: &[Record]) -> Result<(), Error> {
    for (index, record) in records.iter().enumerate() {
        if record.id.0 != index as u64 {
            return Err(StateError("history IDs must match append order").into());
        }
        let viewed = &record.viewed;
        if viewed.snapshot.project().is_none() {
            return Err(StateError("utility pages are not viewing records").into());
        }
        if !viewed.repository.directory.is_absolute() || !viewed.repository.common_dir.is_absolute()
        {
            return Err(StateError("history repository paths must be absolute").into());
        }
        viewed.snapshot.check()?;
    }
    Ok(())
}

fn frame(records: &[Record]) -> Result<Vec<u8>, Error> {
    let payload = postcard::to_stdvec(records)?;
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
    let version = if bytes[8..12] == VERSION.to_le_bytes() {
        VERSION
    } else if bytes[8..12] == 1u32.to_le_bytes() {
        1
    } else {
        return Err(Error::Version);
    };
    let payload = &bytes[HEADER_LEN..];
    if bytes[12..20] != (payload.len() as u64).to_le_bytes()
        || bytes[20..HEADER_LEN] != Sha256::digest(payload)[..]
    {
        return Err(Error::Integrity);
    }
    // A rejected record must not partly change the otherwise usable workspace catalog.
    let mut candidate = state.clone();
    let mut records = if version == 1 {
        v1::decode(payload, &mut candidate)?
    } else {
        let (records, rest) = postcard::take_from_bytes::<Vec<Record>>(payload)?;
        if !rest.is_empty() {
            return Err(Error::Integrity);
        }
        records
    };
    check(&records)?;
    // TODO: Registration repeats a linear project lookup per record. Measure many-repository
    // loads before introducing a catalog index; the sizing fixture uses just one repository.
    for record in &mut records {
        record.viewed.snapshot = record.viewed.snapshot_for(&mut candidate)?;
    }
    *state = candidate;
    Ok(ViewingHistory {
        records,
        ..Default::default()
    })
}
