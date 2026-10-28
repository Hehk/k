use std::time::Duration;

use gpui::{Context, Task};
use kestrel::{git::Snapshot, query::Query, state::Project};

use super::{
    open,
    refresh::{Effect, Input, Scheduler},
    watch::RefWatch,
    watch_lifecycle::{self, Lifecycle},
};

const EVENT_COALESCE_DELAY: Duration = Duration::from_millis(100);

pub struct RepositorySnapshot {
    project: Project,
    pub(super) query: Query<Snapshot>,
    watcher: Lifecycle,
    scheduler: Scheduler,
    task: Option<Task<()>>,
    delay: Option<Task<()>>,
    monitor: Option<Task<()>>,
}

impl RepositorySnapshot {
    pub fn new(project: Project, cx: &mut Context<Self>) -> Self {
        let mut this = Self {
            project,
            query: Query::new(),
            watcher: Lifecycle::default(),
            scheduler: Scheduler::default(),
            task: None,
            delay: None,
            monitor: None,
        };
        this.refresh(cx);
        this
    }

    pub fn query(&self) -> &Query<Snapshot> {
        &self.query
    }

    pub(super) fn watch_error(&self) -> Option<&str> {
        self.watcher.error()
    }

    pub fn set_watching(&mut self, watching: bool, cx: &mut Context<Self>) {
        let input = if watching {
            watch_lifecycle::Input::Requested
        } else {
            watch_lifecycle::Input::Released
        };
        self.dispatch_watch(input, cx);
    }

    fn dispatch_watch(&mut self, input: watch_lifecycle::Input, cx: &mut Context<Self>) {
        let effects = self.watcher.update(input);
        if effects.is_empty() {
            return;
        }
        for effect in effects {
            match effect {
                watch_lifecycle::Effect::Start { generation } => self.attach_watch(generation, cx),
                watch_lifecycle::Effect::Stop => {
                    self.monitor.take();
                }
                watch_lifecycle::Effect::RefreshRefs => self.refresh(cx),
            }
        }
        cx.notify();
    }

    fn attach_watch(&mut self, generation: u64, cx: &mut Context<Self>) {
        let common = self.project.common_dir.clone();
        let setup = cx
            .background_executor()
            .spawn(async move { RefWatch::new(&common) });
        self.monitor = Some(cx.spawn(async move |this, cx| {
            let watch = match setup.await {
                Ok(watch) => watch,
                Err(error) => {
                    let _ = this.update(cx, |this, cx| {
                        this.dispatch_watch(
                            watch_lifecycle::Input::Failed { generation, error },
                            cx,
                        );
                    });
                    return;
                }
            };
            let attached = this.update(cx, |this, cx| {
                this.dispatch_watch(watch_lifecycle::Input::Started { generation }, cx);
                this.watcher.is_watching(generation)
            });
            if !matches!(attached, Ok(true)) {
                return;
            }
            while watch.events.recv().await.is_ok() {
                let failure = watch.failure();
                let active = this.update(cx, |this, cx| {
                    if !this.watcher.is_watching(generation) {
                        return false;
                    }
                    match failure {
                        Some(error) => this.dispatch_watch(
                            watch_lifecycle::Input::Failed { generation, error },
                            cx,
                        ),
                        None => this.dispatch(Input::Invalidated, cx),
                    }
                    this.watcher.is_watching(generation)
                });
                if !matches!(active, Ok(true)) {
                    break;
                }
            }
        }));
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.dispatch(Input::RefreshRequested, cx);
    }

    fn dispatch(&mut self, input: Input, cx: &mut Context<Self>) {
        match self.scheduler.update(input) {
            Some(Effect::ScheduleDelay(timer)) => {
                self.delay = Some(cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(EVENT_COALESCE_DELAY).await;
                    let _ =
                        this.update(cx, |this, cx| this.dispatch(Input::DelayElapsed(timer), cx));
                }));
            }
            Some(Effect::ReadRefs) => {
                self.delay.take();
                self.read(cx);
            }
            None => {}
        }
    }

    fn read(&mut self, cx: &mut Context<Self>) {
        let project = self.project.clone();
        self.query.start();
        self.task = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    open(&project)?
                        .snapshot()
                        .map_err(|error| error.to_string())
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.query.complete(result);
                this.dispatch(Input::ReadFinished, cx);
                cx.notify();
            });
        }));
        cx.notify();
    }
}

#[cfg(test)]
mod tests;
