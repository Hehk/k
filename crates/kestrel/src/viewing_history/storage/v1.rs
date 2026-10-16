use serde::Deserialize;

use super::Error;
use crate::{
    review::{Comparison, RefName},
    state::{AppState, BaseSelection, Destination, Snapshot, ViewState},
    viewing_history::{Labels, Record, RepositoryIdentity, Viewed, ViewingId},
};

// These types describe the former v1 payload, not the format used for new saves.
#[derive(Deserialize)]
struct SavedRecord {
    timestamp_ms: i64,
    repository: RepositoryIdentity,
    destination: SavedDestination,
    view: ViewState,
    labels: Labels,
}

#[derive(Deserialize)]
enum SavedDestination {
    Branches,
    Comparison {
        branch: RefName,
        base: BaseSelection,
        comparison: Option<Comparison>,
    },
}

pub(super) fn decode(payload: &[u8], state: &mut AppState) -> Result<Vec<Record>, Error> {
    let (saved, rest) = postcard::take_from_bytes::<Vec<SavedRecord>>(payload)?;
    if !rest.is_empty() {
        return Err(Error::Integrity);
    }
    saved
        .into_iter()
        .enumerate()
        .map(|(index, saved)| {
            let project = saved.repository.register(state)?;
            let destination = match saved.destination {
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
            Ok(Record {
                id: ViewingId(index as u64),
                timestamp_ms: saved.timestamp_ms,
                viewed: Viewed {
                    repository: saved.repository,
                    snapshot: Snapshot {
                        destination,
                        view: saved.view,
                    },
                    labels: saved.labels,
                },
            })
        })
        .collect()
}
