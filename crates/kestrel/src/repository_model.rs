use gpui::{App, Context, Entity, Task};
use kestrel::{
    git::{Branch, Repository, Snapshot},
    state::{Project, Route, TabId, WindowId},
};

use crate::workspace::Workspace;

pub enum Load<T> {
    Loading,
    Ready(T),
    Failed(String),
}

pub struct RepositoryModel {
    workspace: Entity<Workspace>,
    window: WindowId,
    tab: TabId,
    project: Project,
    snapshot: Load<Snapshot>,
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
            snapshot: Load::Loading,
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

    pub fn snapshot(&self) -> &Load<Snapshot> {
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
        self.snapshot = Load::Loading;
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
                this.snapshot = match result {
                    Ok(snapshot) => Load::Ready(snapshot),
                    Err(error) => Load::Failed(error),
                };
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

pub fn branch_label(branch: &Branch) -> String {
    format!(
        "{} · {}",
        branch.name.label(),
        if branch.remote { "Remote" } else { "Local" }
    )
}
