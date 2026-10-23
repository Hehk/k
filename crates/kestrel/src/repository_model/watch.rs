use std::{
    path::Path,
    sync::{Arc, Mutex},
};

use async_channel::{Receiver, Sender};
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};

#[derive(Clone)]
pub(super) struct Invalidation {
    wake: Sender<()>,
    failure: Arc<Mutex<Option<String>>>,
}

impl Invalidation {
    pub fn send(&self, failure: Option<String>) {
        if let Some(failure) = failure {
            *self
                .failure
                .lock()
                .unwrap_or_else(|error| error.into_inner()) = Some(failure);
        }
        // One pending wake is enough. Keep errors separately so a full channel cannot lose them.
        let _ = self.wake.try_send(());
    }
}

pub(super) struct RefWatch {
    _native: Option<RecommendedWatcher>,
    pub events: Receiver<()>,
    invalidation: Invalidation,
}

impl RefWatch {
    pub fn new(common: &Path) -> Result<Self, String> {
        let mut watch = Self::channel();
        let invalidation = watch.invalidation.clone();
        let root = common.to_owned();
        let mut native =
            notify::recommended_watcher(move |event: notify::Result<Event>| match event {
                Ok(event) if affects_refs(&root, &event) => invalidation.send(None),
                Ok(_) => {}
                Err(error) => invalidation.send(Some(format!("Local ref watcher failed: {error}"))),
            })
            .map_err(|error| format!("Could not start local ref watcher: {error}"))?;
        native
            .watch(common, RecursiveMode::Recursive)
            .map_err(|error| format!("Could not watch local refs: {error}"))?;
        watch._native = Some(native);
        Ok(watch)
    }

    fn channel() -> Self {
        let (wake, events) = async_channel::bounded(1);
        Self {
            _native: None,
            events,
            invalidation: Invalidation {
                wake,
                failure: Arc::new(Mutex::new(None)),
            },
        }
    }

    pub fn failure(&self) -> Option<String> {
        self.invalidation
            .failure
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }
}

fn affects_refs(common: &Path, event: &Event) -> bool {
    if event.need_rescan() {
        return true;
    }
    if matches!(event.kind, EventKind::Access(_)) {
        return false;
    }
    if event.paths.is_empty() {
        return true;
    }

    let refs = common.join("refs");
    let packed_refs = common.join("packed-refs");
    event
        .paths
        .iter()
        .any(|path| path == common || path.starts_with(&refs) || path == &packed_refs)
}

#[cfg(test)]
mod tests;
