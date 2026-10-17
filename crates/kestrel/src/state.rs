use std::{collections::HashSet, path::PathBuf};

use serde::{Deserialize, Deserializer, Serialize, de::Error};
use thiserror::Error;

use crate::{
    git::RepositoryInfo,
    review::{Comparison, GitPath, RefName},
};

#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ProjectId(u64);

impl ProjectId {
    pub fn as_u64(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct WindowId(u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TabId(u64);

impl TabId {
    pub fn as_u64(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BaseSelection {
    Default,
    Explicit(RefName),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReadingMode {
    #[default]
    Diff,
    HeadFile,
    BaseFile,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Browsing {
    pub file: Option<GitPath>,
    pub directory: Option<GitPath>,
    #[serde(default)]
    pub mode: ReadingMode,
    pub first_visible_line: u32,
}

impl Browsing {
    fn check_paths(&self) -> Result<(), StateError> {
        if self
            .file
            .iter()
            .chain(self.directory.iter())
            .any(|path| !path.is_valid())
        {
            return Err(StateError("invalid browsing path"));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BranchReview {
    pub branch: RefName,
    pub base: BaseSelection,
    pub comparison: Option<Comparison>,
    pub browsing: Browsing,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Project {
    pub id: ProjectId,
    #[serde(with = "local_path")]
    pub directory: PathBuf,
    #[serde(with = "local_path")]
    pub common_dir: PathBuf,
    #[serde(flatten)]
    pub last_review: ReviewContext,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewContext {
    pub selected_branch: Option<RefName>,
    pub reviews: Vec<BranchReview>,
}

impl ReviewContext {
    fn select_branch(&mut self, branch: RefName) -> Result<(), StateError> {
        if !branch.is_branch() {
            return Err(StateError("invalid branch ref"));
        }
        if !self.reviews.iter().any(|review| review.branch == branch) {
            self.reviews.push(BranchReview {
                branch: branch.clone(),
                base: BaseSelection::Default,
                comparison: None,
                browsing: Browsing::default(),
            });
        }
        self.selected_branch = Some(branch);
        Ok(())
    }

    fn check_loaded(&self) -> Result<(), StateError> {
        let mut branches = HashSet::new();
        for review in &self.reviews {
            if !review.branch.is_branch() || !branches.insert(&review.branch) {
                return Err(StateError("invalid or duplicate branch review"));
            }
            if let BaseSelection::Explicit(base) = &review.base
                && !base.is_branch()
            {
                return Err(StateError("invalid base ref"));
            }
            if let Some(comparison) = &review.comparison {
                if comparison.branch != review.branch || !comparison.base.is_branch() {
                    return Err(StateError("comparison does not match its branch review"));
                }
                if let BaseSelection::Explicit(base) = &review.base
                    && *base != comparison.base
                {
                    return Err(StateError("comparison does not match its selected base"));
                }
            }
            review.browsing.check_paths()?;
        }
        if let Some(branch) = &self.selected_branch
            && !branches.contains(branch)
        {
            return Err(StateError("selected branch has no review context"));
        }
        Ok(())
    }

    pub fn active_review(&self) -> Option<&BranchReview> {
        self.reviews
            .iter()
            .find(|review| Some(&review.branch) == self.selected_branch.as_ref())
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum BranchSort {
    #[default]
    Newest,
    Name,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BranchList {
    pub query: String,
    pub sort: BranchSort,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum RepositoryPage {
    #[default]
    Branches,
    Comparison,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Route {
    Home,
    Repository {
        project: ProjectId,
        review: ReviewContext,
        #[serde(default)]
        page: RepositoryPage,
        #[serde(default)]
        branches: BranchList,
    },
}

impl Route {
    pub fn project(&self) -> Option<ProjectId> {
        match self {
            Self::Home => None,
            Self::Repository { project, .. } => Some(*project),
        }
    }

    pub fn review(&self) -> Option<&ReviewContext> {
        match self {
            Self::Home => None,
            Self::Repository { review, .. } => Some(review),
        }
    }
}

fn resume_branch_list(previous: &Route, next: &mut Route) {
    if let Route::Repository {
        project: from,
        review,
        ..
    } = previous
        && let Route::Repository {
            project: to,
            review: remembered,
            page: RepositoryPage::Branches,
            ..
        } = next
        && from == to
    {
        *remembered = review.clone();
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tab {
    pub id: TabId,
    pub route: Route,
    #[serde(default)]
    back: Vec<Route>,
    #[serde(default)]
    forward: Vec<Route>,
}

impl Tab {
    pub fn can_go_back(&self) -> bool {
        !self.back.is_empty()
    }

    pub fn can_go_forward(&self) -> bool {
        !self.forward.is_empty()
    }

    fn routes(&self) -> impl Iterator<Item = &Route> {
        std::iter::once(&self.route)
            .chain(&self.back)
            .chain(&self.forward)
    }

    fn visit(&mut self, route: Route) {
        if self.route != route {
            self.back.push(std::mem::replace(&mut self.route, route));
            self.forward.clear();
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceWindow {
    pub id: WindowId,
    pub tabs: Vec<Tab>,
    pub active_tab: Option<TabId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AppState {
    projects: Vec<Project>,
    windows: Vec<WorkspaceWindow>,
    active_window: Option<WindowId>,
    next_id: u64,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            projects: Vec::new(),
            windows: Vec::new(),
            active_window: None,
            next_id: 1,
        }
    }
}

impl<'de> Deserialize<'de> for AppState {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Fields {
            projects: Vec<Project>,
            windows: Vec<WorkspaceWindow>,
            active_window: Option<WindowId>,
            next_id: u64,
        }

        let fields = Fields::deserialize(deserializer)?;
        let state = Self {
            projects: fields.projects,
            windows: fields.windows,
            active_window: fields.active_window,
            next_id: fields.next_id,
        };
        state.check_loaded().map_err(D::Error::custom)?;
        Ok(state)
    }
}

#[derive(Debug, Error)]
#[error("invalid application state: {0}")]
pub struct StateError(pub &'static str);

impl AppState {
    pub fn projects(&self) -> &[Project] {
        &self.projects
    }

    pub fn windows(&self) -> &[WorkspaceWindow] {
        &self.windows
    }

    pub fn active_window(&self) -> Option<WindowId> {
        self.active_window
    }

    pub fn window(&self, id: WindowId) -> Result<&WorkspaceWindow, StateError> {
        self.windows
            .iter()
            .find(|window| window.id == id)
            .ok_or(StateError("unknown window"))
    }

    pub fn focus_window(&mut self, id: WindowId) -> Result<(), StateError> {
        self.window(id)?;
        self.active_window = Some(id);
        Ok(())
    }

    pub fn project(&self, id: ProjectId) -> Result<&Project, StateError> {
        self.projects
            .iter()
            .find(|project| project.id == id)
            .ok_or(StateError("unknown project"))
    }

    pub fn tab(&self, id: TabId) -> Result<&Tab, StateError> {
        self.windows
            .iter()
            .flat_map(|window| &window.tabs)
            .find(|tab| tab.id == id)
            .ok_or(StateError("unknown tab"))
    }

    fn tab_mut(&mut self, id: TabId) -> Result<&mut Tab, StateError> {
        self.windows
            .iter_mut()
            .flat_map(|window| &mut window.tabs)
            .find(|tab| tab.id == id)
            .ok_or(StateError("unknown tab"))
    }

    fn allocate(&mut self) -> Result<u64, StateError> {
        let id = self.next_id;
        self.next_id = id.checked_add(1).ok_or(StateError("IDs exhausted"))?;
        Ok(id)
    }

    pub fn register_project(&mut self, info: &RepositoryInfo) -> Result<ProjectId, StateError> {
        if let Some(project) = self
            .projects
            .iter()
            .find(|project| project.common_dir == info.common_dir)
        {
            return Ok(project.id);
        }
        if !info.directory.is_absolute() || !info.common_dir.is_absolute() {
            return Err(StateError("repository paths must be absolute"));
        }
        if info
            .current_branch
            .as_ref()
            .is_some_and(|branch| !branch.is_branch())
        {
            return Err(StateError("invalid branch ref"));
        }
        let mut project = Project {
            id: ProjectId(self.allocate()?),
            directory: info.directory.clone(),
            common_dir: info.common_dir.clone(),
            last_review: ReviewContext::default(),
        };
        if let Some(branch) = &info.current_branch {
            project.last_review.select_branch(branch.clone())?;
        }
        let id = project.id;
        self.projects.push(project);
        Ok(id)
    }

    pub fn select_branch(&mut self, tab: TabId, branch: RefName) -> Result<(), StateError> {
        let mut route = self.tab(tab)?.route.clone();
        let Route::Repository { review, page, .. } = &mut route else {
            return Err(StateError("tab is not a repository route"));
        };
        review.select_branch(branch)?;
        *page = RepositoryPage::Comparison;
        self.tab_mut(tab)?.visit(route);
        self.remember_tab(tab);
        Ok(())
    }

    pub fn set_branch_list(&mut self, tab: TabId, list: BranchList) -> Result<(), StateError> {
        let Route::Repository { branches, .. } = &mut self.tab_mut(tab)?.route else {
            return Err(StateError("tab is not a repository route"));
        };
        *branches = list;
        Ok(())
    }

    pub fn set_comparison(
        &mut self,
        tab: TabId,
        base: BaseSelection,
        comparison: Comparison,
    ) -> Result<(), StateError> {
        if !comparison.base.is_branch() {
            return Err(StateError("invalid base ref"));
        }
        if let BaseSelection::Explicit(base) = &base
            && *base != comparison.base
        {
            return Err(StateError("comparison does not match its selected base"));
        }
        self.edit_review(tab, |context| {
            let review = context
                .reviews
                .iter_mut()
                .find(|review| review.branch == comparison.branch)
                .ok_or(StateError("unknown branch review"))?;
            review.base = base;
            review.comparison = Some(comparison);
            Ok(())
        })
    }

    pub fn set_browsing(
        &mut self,
        tab: TabId,
        branch: &RefName,
        browsing: Browsing,
    ) -> Result<(), StateError> {
        browsing.check_paths()?;
        self.edit_review(tab, |context| {
            let review = context
                .reviews
                .iter_mut()
                .find(|review| review.branch == *branch)
                .ok_or(StateError("unknown branch review"))?;
            review.browsing = browsing;
            Ok(())
        })
    }

    fn edit_review(
        &mut self,
        tab: TabId,
        change: impl FnOnce(&mut ReviewContext) -> Result<(), StateError>,
    ) -> Result<(), StateError> {
        let Route::Repository { review, .. } = &mut self.tab_mut(tab)?.route else {
            return Err(StateError("tab is not a repository route"));
        };
        let previous = review.clone();
        change(review)?;
        // Loading an unchanged pinned review is not a new user visit.
        if *review != previous {
            self.remember_tab(tab);
        }
        Ok(())
    }

    fn remember_tab(&mut self, tab: TabId) {
        if let Route::Repository {
            project, review, ..
        } = &self
            .tab(tab)
            .expect("tab was checked before remembering its review")
            .route
        {
            let (project, review) = (*project, review.clone());
            self.projects
                .iter_mut()
                .find(|item| item.id == project)
                .expect("repository routes reference registered projects")
                .last_review = review;
        }
    }

    pub fn new_window(&mut self) -> Result<WindowId, StateError> {
        let id = WindowId(self.allocate()?);
        self.windows.push(WorkspaceWindow {
            id,
            tabs: Vec::new(),
            active_tab: None,
        });
        self.active_window = Some(id);
        Ok(id)
    }

    pub fn new_tab(&mut self, window: WindowId) -> Result<TabId, StateError> {
        let index = self
            .windows
            .iter()
            .position(|item| item.id == window)
            .ok_or(StateError("unknown window"))?;
        let id = TabId(self.allocate()?);
        let window = &mut self.windows[index];
        window.tabs.push(Tab {
            id,
            route: Route::Home,
            back: Vec::new(),
            forward: Vec::new(),
        });
        window.active_tab = Some(id);
        self.active_window = Some(window.id);
        Ok(id)
    }

    pub fn insert_tab(
        &mut self,
        window: WindowId,
        index: usize,
        mut tab: Tab,
    ) -> Result<TabId, StateError> {
        self.window(window)?;
        for route in tab.routes() {
            if let Route::Repository {
                project,
                review,
                page,
                ..
            } = route
            {
                self.project(*project)?;
                review.check_loaded()?;
                if *page == RepositoryPage::Comparison && review.active_review().is_none() {
                    return Err(StateError("comparison page has no branch"));
                }
            }
        }
        let id = TabId(self.allocate()?);
        tab.id = id;
        let window = self
            .windows
            .iter_mut()
            .find(|item| item.id == window)
            .expect("destination window was validated");
        window.tabs.insert(index.min(window.tabs.len()), tab);
        window.active_tab = Some(id);
        self.active_window = Some(window.id);
        Ok(id)
    }

    pub fn select_tab(&mut self, tab: TabId) -> Result<(), StateError> {
        let window = self
            .windows
            .iter_mut()
            .find(|window| window.tabs.iter().any(|item| item.id == tab))
            .ok_or(StateError("unknown tab"))?;
        window.active_tab = Some(tab);
        self.active_window = Some(window.id);
        Ok(())
    }

    pub fn go_home(&mut self, tab: TabId) -> Result<(), StateError> {
        self.tab(tab)?;
        self.remember_tab(tab);
        self.tab_mut(tab)?.visit(Route::Home);
        Ok(())
    }

    pub fn open_project(&mut self, project: ProjectId, tab: TabId) -> Result<(), StateError> {
        let remembered = &self.project(project)?.last_review;
        let route = match &self.tab(tab)?.route {
            Route::Repository {
                project: current,
                page: RepositoryPage::Branches,
                ..
            } if *current == project => return Ok(()),
            Route::Repository {
                project: current,
                review,
                branches,
                ..
            } if *current == project => Route::Repository {
                project,
                review: review.clone(),
                page: RepositoryPage::Branches,
                branches: branches.clone(),
            },
            _ => Route::Repository {
                project,
                review: remembered.clone(),
                page: RepositoryPage::Branches,
                branches: BranchList::default(),
            },
        };
        self.remember_tab(tab);
        self.tab_mut(tab)?.visit(route);
        Ok(())
    }

    pub fn go_back(&mut self, id: TabId) -> Result<(), StateError> {
        let tab = self.tab_mut(id)?;
        let Some(route) = tab.back.pop() else {
            return Ok(());
        };
        let mut route = route;
        resume_branch_list(&tab.route, &mut route);
        tab.forward.push(std::mem::replace(&mut tab.route, route));
        self.remember_tab(id);
        Ok(())
    }

    pub fn go_forward(&mut self, id: TabId) -> Result<(), StateError> {
        let tab = self.tab_mut(id)?;
        let Some(route) = tab.forward.pop() else {
            return Ok(());
        };
        let mut route = route;
        resume_branch_list(&tab.route, &mut route);
        tab.back.push(std::mem::replace(&mut tab.route, route));
        self.remember_tab(id);
        Ok(())
    }

    pub fn close_tab(&mut self, id: TabId) {
        if let Some(source) = self
            .windows
            .iter()
            .position(|window| window.tabs.iter().any(|tab| tab.id == id))
        {
            self.remember_tab(id);
            self.take_tab(id);
            if self.windows[source].tabs.is_empty() {
                self.windows.remove(source);
            }
            self.repair_active_window();
        }
    }

    pub fn close_window(&mut self, id: WindowId) {
        if let Ok(window) = self.window(id) {
            let active = window.active_tab;
            let tabs = window.tabs.iter().map(|tab| tab.id).collect::<Vec<_>>();
            for tab in tabs {
                self.remember_tab(tab);
            }
            if let Some(active) = active {
                self.remember_tab(active);
            }
        }
        self.windows.retain(|window| window.id != id);
        self.repair_active_window();
    }

    pub fn move_tab(
        &mut self,
        tab: TabId,
        target: WindowId,
        index: usize,
    ) -> Result<(), StateError> {
        let source = self
            .windows
            .iter()
            .find(|window| window.tabs.iter().any(|item| item.id == tab))
            .ok_or(StateError("unknown tab"))?
            .id;
        self.window(target)?;
        let tab = self.take_tab(tab);
        let window = self
            .windows
            .iter_mut()
            .find(|window| window.id == target)
            .expect("removing a tab preserves the validated destination window");
        window.active_tab = Some(tab.id);
        window.tabs.insert(index.min(window.tabs.len()), tab);
        self.windows
            .retain(|window| window.id != source || source == target || !window.tabs.is_empty());
        self.active_window = Some(target);
        Ok(())
    }

    fn take_tab(&mut self, id: TabId) -> Tab {
        let window = self
            .windows
            .iter_mut()
            .find(|window| window.tabs.iter().any(|tab| tab.id == id))
            .expect("tab was checked before removal");
        let index = window
            .tabs
            .iter()
            .position(|tab| tab.id == id)
            .expect("owning window contains the tab");
        let tab = window.tabs.remove(index);
        if window.active_tab == Some(id) {
            window.active_tab = window
                .tabs
                .get(index.min(window.tabs.len().saturating_sub(1)))
                .map(|tab| tab.id);
        }
        tab
    }

    fn repair_active_window(&mut self) {
        if !self
            .windows
            .iter()
            .any(|window| Some(window.id) == self.active_window)
        {
            self.active_window = self.windows.last().map(|window| window.id);
        }
    }

    fn check_loaded(&self) -> Result<(), StateError> {
        let mut ids = HashSet::new();
        let mut repositories = HashSet::new();
        for project in &self.projects {
            if !ids.insert(project.id.0) || !repositories.insert(&project.common_dir) {
                return Err(StateError("duplicate project"));
            }
            if !project.directory.is_absolute() || !project.common_dir.is_absolute() {
                return Err(StateError("repository paths must be absolute"));
            }
            project.last_review.check_loaded()?;
        }
        let projects: HashSet<_> = self.projects.iter().map(|project| project.id).collect();
        for window in &self.windows {
            if !ids.insert(window.id.0) {
                return Err(StateError("duplicate window ID"));
            }
            for tab in &window.tabs {
                if !ids.insert(tab.id.0) {
                    return Err(StateError("duplicate tab ID"));
                }
                for route in tab.routes() {
                    if let Route::Repository {
                        project,
                        review,
                        page,
                        ..
                    } = route
                    {
                        if !projects.contains(project) {
                            return Err(StateError("unknown route project"));
                        }
                        review.check_loaded()?;
                        if *page == RepositoryPage::Comparison && review.active_review().is_none() {
                            return Err(StateError("comparison page has no branch"));
                        }
                    }
                }
            }
            match window.active_tab {
                Some(id) if window.tabs.iter().any(|tab| tab.id == id) => {}
                None if window.tabs.is_empty() => {}
                _ => return Err(StateError("invalid active tab")),
            }
        }
        match self.active_window {
            Some(id) if self.windows.iter().any(|window| window.id == id) => {}
            None if self.windows.is_empty() => {}
            _ => return Err(StateError("invalid active window")),
        }
        if self.next_id == 0 || ids.iter().any(|id| *id == 0 || *id >= self.next_id) {
            return Err(StateError("invalid ID sequence"));
        }
        Ok(())
    }
}

mod local_path {
    use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error};
    use std::{
        ffi::OsString,
        path::{Path, PathBuf},
    };

    #[derive(Serialize, Deserialize)]
    enum EncodedPath {
        Unix(Vec<u8>),
        Windows(Vec<u16>),
    }

    pub fn serialize<S: Serializer>(path: &Path, serializer: S) -> Result<S::Ok, S::Error> {
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            EncodedPath::Unix(path.as_os_str().as_bytes().to_vec()).serialize(serializer)
        }
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            EncodedPath::Windows(path.as_os_str().encode_wide().collect()).serialize(serializer)
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<PathBuf, D::Error> {
        match EncodedPath::deserialize(deserializer)? {
            #[cfg(unix)]
            EncodedPath::Unix(bytes) => {
                use std::os::unix::ffi::OsStringExt;
                Ok(OsString::from_vec(bytes).into())
            }
            #[cfg(windows)]
            EncodedPath::Windows(units) => {
                use std::os::windows::ffi::OsStringExt;
                Ok(OsString::from_wide(&units).into())
            }
            _ => Err(D::Error::custom(
                "state contains paths from another platform",
            )),
        }
    }
}
