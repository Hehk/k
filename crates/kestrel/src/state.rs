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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct WindowId(u64);

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
    pub selected_branch: Option<RefName>,
    pub reviews: Vec<BranchReview>,
}

impl Project {
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

    pub fn active_review(&self) -> Option<&BranchReview> {
        self.reviews
            .iter()
            .find(|review| Some(&review.branch) == self.selected_branch.as_ref())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceWindow {
    pub id: WindowId,
    pub tabs: Vec<ProjectId>,
    pub active_tab: Option<ProjectId>,
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

    pub fn project(&self, id: ProjectId) -> Result<&Project, StateError> {
        self.projects
            .iter()
            .find(|project| project.id == id)
            .ok_or(StateError("unknown project"))
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
            selected_branch: None,
            reviews: Vec::new(),
        };
        if let Some(branch) = &info.current_branch {
            project.select_branch(branch.clone())?;
        }
        let id = project.id;
        self.projects.push(project);
        Ok(id)
    }

    pub fn select_branch(&mut self, project: ProjectId, branch: RefName) -> Result<(), StateError> {
        self.projects
            .iter_mut()
            .find(|item| item.id == project)
            .ok_or(StateError("unknown project"))?
            .select_branch(branch)
    }

    pub fn set_comparison(
        &mut self,
        project: ProjectId,
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
        let review = self.review_mut(project, &comparison.branch)?;
        review.base = base;
        review.comparison = Some(comparison);
        Ok(())
    }

    pub fn set_browsing(
        &mut self,
        project: ProjectId,
        branch: &RefName,
        browsing: Browsing,
    ) -> Result<(), StateError> {
        browsing.check_paths()?;
        self.review_mut(project, branch)?.browsing = browsing;
        Ok(())
    }

    fn review_mut(
        &mut self,
        project: ProjectId,
        branch: &RefName,
    ) -> Result<&mut BranchReview, StateError> {
        self.projects
            .iter_mut()
            .find(|item| item.id == project)
            .ok_or(StateError("unknown project"))?
            .reviews
            .iter_mut()
            .find(|review| review.branch == *branch)
            .ok_or(StateError("unknown branch review"))
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

    pub fn open_project(
        &mut self,
        project: ProjectId,
        window: WindowId,
    ) -> Result<WindowId, StateError> {
        self.project(project)?;
        let target = self
            .windows
            .iter()
            .position(|item| item.id == window)
            .ok_or(StateError("unknown window"))?;
        let index = self
            .windows
            .iter()
            .position(|window| window.tabs.contains(&project));
        let window = &mut self.windows[index.unwrap_or(target)];
        if index.is_none() {
            window.tabs.push(project);
        }
        window.active_tab = Some(project);
        self.active_window = Some(window.id);
        Ok(window.id)
    }

    pub fn close_project(&mut self, project: ProjectId) {
        self.windows.retain_mut(|window| {
            let Some(index) = window.tabs.iter().position(|tab| *tab == project) else {
                return true;
            };
            window.tabs.remove(index);
            if window.active_tab == Some(project) {
                window.active_tab = window
                    .tabs
                    .get(index.min(window.tabs.len().saturating_sub(1)))
                    .copied();
            }
            !window.tabs.is_empty()
        });
        self.repair_active_window();
    }

    pub fn close_window(&mut self, id: WindowId) {
        self.windows.retain(|window| window.id != id);
        self.repair_active_window();
    }

    pub fn move_project(
        &mut self,
        project: ProjectId,
        target: WindowId,
        index: usize,
    ) -> Result<(), StateError> {
        self.project(project)?;
        if !self.windows.iter().any(|window| window.id == target) {
            return Err(StateError("unknown target window"));
        }
        self.windows.retain_mut(|window| {
            let contained = window.tabs.contains(&project);
            window.tabs.retain(|tab| *tab != project);
            if window.active_tab == Some(project) {
                window.active_tab = window.tabs.last().copied();
            }
            window.id == target || !contained || !window.tabs.is_empty()
        });
        let window = self
            .windows
            .iter_mut()
            .find(|window| window.id == target)
            .unwrap();
        window.tabs.insert(index.min(window.tabs.len()), project);
        window.active_tab = Some(project);
        self.active_window = Some(target);
        Ok(())
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
            let mut branches = HashSet::new();
            for review in &project.reviews {
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
            if let Some(branch) = &project.selected_branch
                && !branches.contains(branch)
            {
                return Err(StateError("selected branch has no review context"));
            }
        }
        let projects: HashSet<_> = self.projects.iter().map(|project| project.id).collect();
        let mut open = HashSet::new();
        for window in &self.windows {
            if !ids.insert(window.id.0) {
                return Err(StateError("duplicate window ID"));
            }
            for tab in &window.tabs {
                if !projects.contains(tab) || !open.insert(tab) {
                    return Err(StateError("unknown or duplicate open project"));
                }
            }
            match window.active_tab {
                Some(tab) if window.tabs.contains(&tab) => {}
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
