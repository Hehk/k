use gpui::{App, Context, Entity, Subscription};
use kestrel::{
    git::{Branch, Repository, Snapshot},
    query::Query,
    state::{Project, Route, TabId, WindowId},
};

use crate::workspace::Workspace;

mod refresh;
mod snapshot;
mod watch;
mod watch_lifecycle;
pub use snapshot::RepositorySnapshot;

pub struct RepositoryModel {
    workspace: Entity<Workspace>,
    window: WindowId,
    tab: TabId,
    project: Project,
    snapshot: Entity<RepositorySnapshot>,
    _subscription: Subscription,
}

impl RepositoryModel {
    pub fn new(
        workspace: Entity<Workspace>,
        window: WindowId,
        tab: TabId,
        project: Project,
        cx: &mut Context<Self>,
    ) -> Self {
        let snapshot = workspace.update(cx, |workspace, cx| {
            workspace.repository_snapshot(&project, cx)
        });
        let subscription = cx.observe(&snapshot, |_, _, cx| cx.notify());
        Self {
            workspace,
            window,
            tab,
            project,
            snapshot,
            _subscription: subscription,
        }
    }

    pub fn workspace(&self) -> Entity<Workspace> {
        self.workspace.clone()
    }

    pub fn tab(&self) -> TabId {
        self.tab
    }

    pub fn project<'a>(&'a self, cx: &'a App) -> &'a Project {
        self.workspace
            .read(cx)
            .state()
            .project(self.project.id)
            .ok()
            .filter(|project| project.common_dir == self.project.common_dir)
            .unwrap_or(&self.project)
    }

    pub fn snapshot<'a>(&self, cx: &'a App) -> &'a Query<Snapshot> {
        self.snapshot.read(cx).query()
    }

    pub fn watch_error<'a>(&self, cx: &'a App) -> Option<&'a str> {
        self.snapshot.read(cx).watch_error()
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
        self.snapshot
            .update(cx, |snapshot, cx| snapshot.refresh(cx));
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
