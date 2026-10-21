use super::{
    AppState, BaseSelection, BranchList, Browsing, ProjectId, RepositoryPage, Route, Snapshot,
    StateError, TabId, WindowId,
};
use crate::review::{Comparison, RefName};

mod viewing;
use viewing::ViewPolicy;

#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ComparisonChange {
    #[default]
    Resolve,
    ChangeBase,
    LoadLatest,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Navigation {
    Home,
    OpenProject(ProjectId),
    Back,
    Forward,
    SelectTab,
    ReopenTab,
    OpenSnapshot(Snapshot),
    /// Seed a pristine tab without retaining its placeholder Home visit.
    SeedSnapshot(Snapshot),
    SelectBranch(RefName),
    SetBranchList(BranchList),
    /// Initial resolution; explicit retargets use ComparisonLoaded with their intent.
    SetComparison {
        base: BaseSelection,
        comparison: Comparison,
    },
    ComparisonLoaded {
        activation: u64,
        change: ComparisonChange,
        base: BaseSelection,
        comparison: Comparison,
    },
    SetBrowsing {
        branch: RefName,
        browsing: Browsing,
    },
}

impl AppState {
    /// The navigation policy lives here, not in UI handlers or state observers.
    /// Structural operations (new/move/close/restore tab) retain their own lifecycle.
    pub fn navigate(&mut self, id: TabId, action: Navigation) -> Result<(), StateError> {
        let before = self.tab(id)?.visit_id();
        let selected = self.selected(id);
        let focused = self.active_window;
        let policy = match action {
            Navigation::Home => {
                self.tab(id)?;
                self.remember_tab(id);
                self.tab_mut(id)?.visit(Route::Home)?;
                ViewPolicy::Navigate
            }
            Navigation::OpenProject(project) => {
                let fallback = &self.project(project)?.last_review;
                let remembered = self.tab(id)?.remembered(project).unwrap_or(fallback);
                let route = match &self.tab(id)?.route {
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
                self.remember_tab(id);
                self.tab_mut(id)?.visit(route)?;
                ViewPolicy::Navigate
            }
            Navigation::Back => {
                if self.tab_mut(id)?.traverse(true)? {
                    self.remember_tab(id);
                }
                ViewPolicy::Navigate
            }
            Navigation::Forward => {
                if self.tab_mut(id)?.traverse(false)? {
                    self.remember_tab(id);
                }
                ViewPolicy::Navigate
            }
            Navigation::SelectTab => {
                let window = self
                    .windows
                    .iter_mut()
                    .find(|window| window.tabs.iter().any(|tab| tab.id == id))
                    .ok_or(StateError("unknown tab"))?;
                window.active_tab = Some(id);
                self.active_window = Some(window.id);
                ViewPolicy::Select
            }
            Navigation::ReopenTab => ViewPolicy::Reopen,
            Navigation::OpenSnapshot(snapshot) => {
                if let Some(project) = snapshot.project() {
                    self.project(project)?;
                }
                self.tab_mut(id)?.open_snapshot(snapshot)?;
                self.remember_tab(id);
                ViewPolicy::Navigate
            }
            Navigation::SeedSnapshot(snapshot) => {
                if let Some(project) = snapshot.project() {
                    self.project(project)?;
                }
                self.tab_mut(id)?.seed_snapshot(snapshot)?;
                self.remember_tab(id);
                ViewPolicy::Navigate
            }
            Navigation::SelectBranch(branch) => {
                let mut route = self.tab(id)?.route.clone();
                let Route::Repository { review, page, .. } = &mut route else {
                    return Err(StateError("tab is not a repository route"));
                };
                review.select_branch(branch)?;
                *page = RepositoryPage::Comparison;
                self.tab_mut(id)?.visit(route)?;
                self.remember_tab(id);
                ViewPolicy::Navigate
            }
            Navigation::SetBranchList(list) => {
                let Route::Repository { branches, .. } = &mut self.tab_mut(id)?.route else {
                    return Err(StateError("tab is not a repository route"));
                };
                *branches = list;
                ViewPolicy::Update
            }
            Navigation::SetComparison { base, comparison } => {
                self.complete_comparison(id, ComparisonChange::Resolve, base, comparison)?;
                ViewPolicy::Update
            }
            Navigation::ComparisonLoaded {
                activation,
                change,
                base,
                comparison,
            } => {
                let tab = self.tab(id)?;
                if tab.activation() != activation {
                    return Ok(());
                }
                if !matches!(
                    tab.route,
                    Route::Repository {
                        page: RepositoryPage::Comparison,
                        ..
                    }
                ) {
                    return Err(StateError("tab is not a comparison page"));
                }
                self.complete_comparison(id, change, base, comparison)?;
                ViewPolicy::Navigate
            }
            Navigation::SetBrowsing { branch, browsing } => {
                browsing.check_paths()?;
                self.edit_review(id, |context| {
                    let review = context
                        .reviews
                        .iter_mut()
                        .find(|review| review.branch == branch)
                        .ok_or(StateError("unknown branch review"))?;
                    review.browsing = browsing;
                    Ok(())
                })?;
                ViewPolicy::Update
            }
        };
        self.apply_view_policy(id, before, selected, focused, policy)
    }

    pub fn set_tab_layout(
        &mut self,
        id: WindowId,
        tabs: &[u64],
        active: Option<u64>,
    ) -> Result<(), StateError> {
        let previous = self
            .window(id)?
            .tabs
            .iter()
            .map(|tab| tab.id)
            .collect::<Vec<_>>();
        let tabs = tabs
            .iter()
            .map(|id| {
                previous
                    .iter()
                    .find(|tab| tab.as_u64() == *id)
                    .copied()
                    .ok_or(StateError("tab is not in this window"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        if tabs
            .iter()
            .enumerate()
            .any(|(index, id)| tabs[..index].contains(id))
        {
            return Err(StateError("duplicate tab"));
        }
        let selected = active.and_then(|id| tabs.iter().find(|tab| tab.as_u64() == id).copied());
        if (tabs.is_empty() && active.is_some()) || (!tabs.is_empty() && selected.is_none()) {
            return Err(StateError("invalid active tab"));
        }
        let structural = previous != tabs;
        for tab in previous {
            if !tabs.contains(&tab) {
                self.close_tab(tab);
            }
        }
        if let Some(tab) = selected {
            let window = self
                .windows
                .iter_mut()
                .find(|window| window.id == id)
                .expect("remaining tabs retain their window");
            window.tabs.sort_by_key(|tab| {
                tabs.iter()
                    .position(|id| *id == tab.id)
                    .expect("layout contains every remaining tab")
            });
            if structural {
                // Reorder/close chooses a displayed tab structurally, not a new user tab switch.
                window.active_tab = Some(tab);
                self.active_window = Some(id);
            } else {
                self.navigate(tab, Navigation::SelectTab)?;
            }
        }
        Ok(())
    }

    fn complete_comparison(
        &mut self,
        id: TabId,
        change: ComparisonChange,
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
        let mut route = self.tab(id)?.route.clone();
        let Route::Repository { review, .. } = &mut route else {
            return Err(StateError("tab is not a repository route"));
        };
        if review.selected_branch.as_ref() != Some(&comparison.branch) {
            return Err(StateError("comparison does not match the selected branch"));
        }
        let review = review
            .reviews
            .iter_mut()
            .find(|review| review.branch == comparison.branch)
            .ok_or(StateError("unknown branch review"))?;
        let changed = review
            .comparison
            .as_ref()
            .is_some_and(|old| old != &comparison);
        review.base = base;
        review.comparison = Some(comparison);
        // Revalidating restored pins must not replace another tab's newer remembered context.
        if self.tab(id)?.route == route {
            return Ok(());
        }
        match (change, changed) {
            (ComparisonChange::Resolve, true) => {
                return Err(StateError(
                    "resolving a visit cannot advance its pinned comparison",
                ));
            }
            (ComparisonChange::ChangeBase | ComparisonChange::LoadLatest, true) => {
                self.tab_mut(id)?.visit(route)?;
            }
            (
                ComparisonChange::Resolve
                | ComparisonChange::ChangeBase
                | ComparisonChange::LoadLatest,
                false,
            ) => {
                self.tab_mut(id)?.route = route;
            }
        }
        self.remember_tab(id);
        Ok(())
    }
}
