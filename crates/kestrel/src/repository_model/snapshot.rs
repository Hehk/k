use std::time::Duration;

use gpui::{Context, Task};
use kestrel::{git::Snapshot, query::Query, state::Project};

use super::{
    open,
    refresh::{Effect, Input, Scheduler},
    watch::RefWatch,
};

const EVENT_COALESCE_DELAY: Duration = Duration::from_millis(100);

pub struct RepositorySnapshot {
    project: Project,
    pub(super) query: Query<Snapshot>,
    pub(super) watch_error: Option<String>,
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
            watch_error: None,
            scheduler: Scheduler::default(),
            task: None,
            delay: None,
            monitor: None,
        };
        this.refresh(cx);
        this
    }

    pub fn start_watching(&mut self, cx: &mut Context<Self>) {
        if self.monitor.is_some() {
            return;
        }
        let common = self.project.common_dir.clone();
        let setup = cx
            .background_executor()
            .spawn(async move { RefWatch::new(&common) });
        self.monitor = Some(cx.spawn(async move |this, cx| {
            let watch = match setup.await {
                Ok(watch) => watch,
                Err(error) => {
                    let _ = this.update(cx, |this, cx| {
                        this.watch_error = Some(error);
                        cx.notify();
                    });
                    return;
                }
            };
            // Reconcile after attachment, even if the initial read finished before watching began.
            if this.update(cx, |this, cx| this.refresh(cx)).is_err() {
                return;
            }
            while watch.events.recv().await.is_ok() {
                let failure = watch.failure();
                if this
                    .update(cx, |this, cx| {
                        this.watch_error = failure;
                        this.dispatch(Input::Invalidated, cx);
                        cx.notify();
                    })
                    .is_err()
                {
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
