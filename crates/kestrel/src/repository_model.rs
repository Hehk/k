use gpui::{App, Context, Entity, Task};
use kestrel::{
    git::{Branch, Repository, Snapshot},
    query::Query,
    state::{Project, Route, TabId, WindowId},
};

use crate::workspace::Workspace;

pub struct RepositoryModel {
    workspace: Entity<Workspace>,
    window: WindowId,
    tab: TabId,
    project: Project,
    snapshot: Query<Snapshot>,
    task: Option<Task<()>>,
    revision: u64,
}

impl RepositoryModel {
    pub fn new(
        workspace: Entity<Workspace>,
        window: WindowId,
        tab: TabId,
        project: Project,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut this = Self {
            workspace,
            window,
            tab,
            project,
            snapshot: Query::new(),
            task: None,
            revision: 0,
        };
        this.refresh(cx);
        this
    }

    pub fn workspace(&self) -> Entity<Workspace> {
        self.workspace.clone()
    }

    pub fn tab(&self) -> TabId {
        self.tab
    }

    pub fn project(&self) -> &Project {
        &self.project
    }

    pub fn snapshot(&self) -> &Query<Snapshot> {
        &self.snapshot
    }

    pub fn route<'a>(&self, cx: &'a App) -> Option<&'a Route> {
        let state = self.workspace.read(cx).state();
        let window = state.window(self.window).ok()?;
        window
            .tabs
            .iter()
            .find(|tab| tab.id == self.tab)
            .map(|tab| &tab.route)
            .filter(|route| route.project() == Some(self.project.id))
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.revision += 1;
        let revision = self.revision;
        let project = self.project.clone();
        self.snapshot.start();
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
                if this.revision != revision || this.route(cx).is_none() {
                    return;
                }
                this.snapshot.complete(result);
                cx.notify();
            });
        }));
        cx.notify();
    }
}

pub fn open(project: &Project) -> Result<Repository, String> {
    let repository = Repository::open(&project.directory).map_err(|error| error.to_string())?;
    if repository
        .info()
        .map_err(|error| error.to_string())?
        .common_dir
        != project.common_dir
    {
        return Err("This path now belongs to a different repository.".into());
    }
    Ok(repository)
}

#[cfg(test)]
mod tests;

pub fn branch_label(branch: &Branch) -> String {
    format!(
        "{} · {}",
        branch.name.label(),
        if branch.remote { "Remote" } else { "Local" }
    )
}
