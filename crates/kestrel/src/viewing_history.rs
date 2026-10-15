use std::collections::{HashMap, HashSet};

use crate::{
    review::{Comparison, RefName},
    state::{AppState, BaseSelection, Destination, ProjectId, Snapshot, TabId, VisitId, WindowId},
};

#[cfg(test)]
mod tests;

pub const RECENT_LIMIT: usize = 10;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ViewingId(pub u64);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Labels {
    pub repository: String,
    pub directory: String,
    pub branch: Option<String>,
    pub base: Option<String>,
    pub file: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Viewed {
    pub snapshot: Snapshot,
    pub labels: Labels,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    pub id: ViewingId,
    /// Unix milliseconds. Append order determines recency even if the clock moves backward.
    pub timestamp_ms: i64,
    pub viewed: Viewed,
}

/// One tab activation can produce several viewing records when the user switches tabs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Source {
    pub(crate) tab: TabId,
    pub(crate) visit: VisitId,
    pub(crate) activation: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Foreground {
    /// Headless models follow their selected window; native callers supply actual visibility.
    SelectedWindow,
    Window(WindowId),
    Hidden,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Exposure {
    Foreground,
    Background,
}

/// Transaction-local decisions emitted by the navigation policy, never by rendering.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ViewChange {
    Enter {
        source: Source,
        viewed: Option<Viewed>,
        exposure: Exposure,
    },
    Update {
        source: Source,
        viewed: Option<Viewed>,
    },
    ShowPending {
        source: Source,
        viewed: Option<Viewed>,
    },
}

struct Binding {
    source: Source,
    // None is an explicit background destination waiting for its first viewing.
    event: Option<ViewingId>,
}

/// In-memory viewing records, separate from workspace snapshots and closed-container recovery.
#[derive(Default)]
pub struct ViewingHistory {
    records: Vec<Record>,
    bindings: HashMap<TabId, Binding>,
}

impl ViewingHistory {
    pub fn records(&self) -> &[Record] {
        &self.records
    }

    pub fn get(&self, id: ViewingId) -> Option<&Record> {
        self.records
            .get(usize::try_from(id.0).ok()?)
            .filter(|record| record.id == id)
    }

    pub fn recent(&self, limit: usize) -> impl Iterator<Item = &Record> {
        recent(self.records.iter().rev(), limit)
    }

    pub fn apply(
        &mut self,
        changes: impl IntoIterator<Item = ViewChange>,
        mut clock: impl FnMut() -> i64,
    ) -> bool {
        let mut changed = false;
        for change in changes {
            match change {
                ViewChange::Enter {
                    source,
                    viewed,
                    exposure,
                } => match viewed {
                    None => {
                        self.bindings.remove(&source.tab);
                    }
                    Some(viewed) => match exposure {
                        Exposure::Foreground => {
                            self.record(source, viewed, clock());
                            changed = true;
                        }
                        Exposure::Background => {
                            self.bindings.insert(
                                source.tab,
                                Binding {
                                    source,
                                    event: None,
                                },
                            );
                        }
                    },
                },
                ViewChange::Update { source, viewed } => {
                    if let Some(binding) = self.bindings.get(&source.tab)
                        && binding.source == source
                        && let Some(id) = binding.event
                        && let Some(viewed) = viewed
                    {
                        // Only the latest viewing of this activation changes, never older repeats.
                        let record = &mut self.records[id.0 as usize];
                        if record.viewed != viewed {
                            record.viewed = viewed;
                            changed = true;
                        }
                    }
                }
                ViewChange::ShowPending { source, viewed } => {
                    if let Some(binding) = self.bindings.get(&source.tab)
                        && binding.source == source
                        && binding.event.is_none()
                        && let Some(viewed) = viewed
                    {
                        self.record(source, viewed, clock());
                        changed = true;
                    }
                }
            }
        }
        changed
    }

    /// Forget closed tabs' runtime bindings without deleting any viewing records.
    pub fn retain_live_bindings(&mut self, state: &AppState) {
        self.bindings.retain(|tab, _| state.tab(*tab).is_ok());
    }

    fn record(&mut self, source: Source, viewed: Viewed, timestamp_ms: i64) {
        // Append-only IDs also address the vector; deletion/expiry is deliberately not supported.
        let id = ViewingId(self.records.len() as u64);
        self.records.push(Record {
            id,
            timestamp_ms,
            viewed,
        });
        self.bindings.insert(
            source.tab,
            Binding {
                source,
                event: Some(id),
            },
        );
    }
}

// This key belongs to the projection, not the stored record schema.
#[derive(PartialEq, Eq, Hash)]
enum RecentKey {
    Branches(ProjectId),
    Comparison {
        project: ProjectId,
        branch: RefName,
        base: Option<RefName>,
        pinned: Option<Comparison>,
    },
}

fn recent<'a>(
    newest_first: impl Iterator<Item = &'a Record>,
    limit: usize,
) -> impl Iterator<Item = &'a Record> {
    let mut seen = HashSet::new();
    // TODO: Measure duplicate-heavy scans: fewer than `limit` destinations scans the whole log.
    // Keep this projection lazy before considering a separate index or cache.
    newest_first
        .filter(move |record| {
            let key = match &record.viewed.snapshot.destination {
                Destination::Home => return false,
                Destination::Branches(project) => RecentKey::Branches(*project),
                Destination::Comparison {
                    project,
                    branch,
                    base,
                    comparison,
                } => RecentKey::Comparison {
                    project: *project,
                    branch: branch.clone(),
                    base: comparison
                        .as_ref()
                        .map(|comparison| comparison.base.clone())
                        .or_else(|| match base {
                            BaseSelection::Default => None,
                            BaseSelection::Explicit(base) => Some(base.clone()),
                        }),
                    pinned: comparison.clone(),
                },
            };
            seen.insert(key)
        })
        .take(limit)
}
