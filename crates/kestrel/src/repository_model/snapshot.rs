use gpui::{Context, Task};
use kestrel::{git::Snapshot, query::Query, state::Project};

use super::open;

pub struct RepositorySnapshot {
    project: Project,
    pub(super) query: Query<Snapshot>,
    task: Option<Task<()>>,
    revision: u64,
}

impl RepositorySnapshot {
    pub fn new(project: Project, cx: &mut Context<Self>) -> Self {
        let mut this = Self {
            project,
            query: Query::new(),
            task: None,
            revision: 0,
        };
        this.refresh(cx);
        this
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.revision += 1;
        let revision = self.revision;
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
                if this.revision != revision {
                    return;
                }
                this.query.complete(result);
                cx.notify();
            });
        }));
        cx.notify();
    }
}
