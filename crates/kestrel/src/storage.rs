use std::{
    fs,
    io::{BufWriter, Write},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use tempfile::NamedTempFile;
use thiserror::Error;

use crate::state::AppState;

#[cfg(test)]
mod tests;

const VERSION: u32 = 1;

#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("unsupported state version")]
    Version,
    #[error("state must be loaded or safely recovered before saving")]
    NotLoaded,
    #[error("configuration directory is unavailable")]
    NoConfigDirectory,
}

#[derive(Debug)]
pub struct Recovery {
    pub reason: String,
    pub backup: PathBuf,
}

#[derive(Debug)]
pub struct Loaded {
    pub state: AppState,
    pub recovery: Option<Recovery>,
}

#[derive(Serialize)]
struct Snapshot<'a> {
    version: u32,
    state: &'a AppState,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Saved {
    version: u32,
    state: AppState,
}

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

    pub fn for_app() -> Result<Self, Error> {
        Ok(Self::new(state_path(
            std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from),
            dirs::home_dir(),
            dirs::config_dir(),
        )?))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(&mut self) -> Result<Loaded, Error> {
        self.loaded = false;
        let bytes = match fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                self.loaded = true;
                return Ok(Loaded {
                    state: AppState::default(),
                    recovery: None,
                });
            }
            Err(error) => return Err(error.into()),
        };
        let result = match decode(&bytes) {
            Ok(state) => Loaded {
                state,
                recovery: None,
            },
            Err(error) => {
                let mut backup = tempfile::Builder::new()
                    .prefix("kestrel-state-recovery-")
                    .suffix(".json")
                    .tempfile_in(self.directory())?;
                backup.write_all(&bytes)?;
                backup.as_file().sync_all()?;
                let (_, path) = backup.keep().map_err(|error| error.error)?;
                Loaded {
                    state: AppState::default(),
                    recovery: Some(Recovery {
                        reason: error.to_string(),
                        backup: path,
                    }),
                }
            }
        };
        self.loaded = true;
        Ok(result)
    }

    pub fn save(&mut self, state: &AppState) -> Result<(), Error> {
        if !self.loaded {
            return Err(Error::NotLoaded);
        }
        fs::create_dir_all(self.directory())?;
        let mut file = NamedTempFile::new_in(self.directory())?;
        {
            let mut writer = BufWriter::new(file.as_file_mut());
            serde_json::to_writer_pretty(
                &mut writer,
                &Snapshot {
                    version: VERSION,
                    state,
                },
            )?;
            writer.write_all(b"\n")?;
            writer.flush()?;
        }
        file.as_file().sync_all()?;
        file.persist(&self.path).map_err(|error| error.error)?;
        // Persist the rename too; opening a directory this way is Unix-specific.
        #[cfg(unix)]
        fs::File::open(self.directory())?.sync_all()?;
        Ok(())
    }

    fn directory(&self) -> &Path {
        self.path
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or(Path::new("."))
    }
}

fn state_path(
    config_home: Option<PathBuf>,
    home: Option<PathBuf>,
    native_config: Option<PathBuf>,
) -> Result<PathBuf, Error> {
    let directory = config_home
        .filter(|path| path.is_absolute())
        .or_else(|| home.map(|home| home.join(".config")))
        .or(native_config)
        .ok_or(Error::NoConfigDirectory)?;
    Ok(directory.join("kestrel").join("state.json"))
}

fn decode(bytes: &[u8]) -> Result<AppState, Error> {
    let value: serde_json::Value = serde_json::from_slice(bytes)?;
    if value.get("version").and_then(|version| version.as_u64()) != Some(u64::from(VERSION)) {
        return Err(Error::Version);
    }
    let saved: Saved = serde_json::from_value(value)?;
    if saved.version != VERSION {
        return Err(Error::Version);
    }
    Ok(saved.state)
}
