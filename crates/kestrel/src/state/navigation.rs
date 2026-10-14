use super::{
    AppState, BaseSelection, BranchList, Browsing, ProjectId, RepositoryPage, Route, StateError,
    TabId,
};
use crate::review::{Comparison, RefName};

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
        match action {
            Navigation::Home => {
                self.tab(id)?;
                self.remember_tab(id);
                self.tab_mut(id)?.visit(Route::Home)?;
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
            }
            Navigation::Back => {
                if self.tab_mut(id)?.traverse(true)? {
                    self.remember_tab(id);
                }
            }
            Navigation::Forward => {
                if self.tab_mut(id)?.traverse(false)? {
                    self.remember_tab(id);
                }
            }
            Navigation::SelectTab => {
                let window = self
                    .windows
                    .iter_mut()
                    .find(|window| window.tabs.iter().any(|tab| tab.id == id))
                    .ok_or(StateError("unknown tab"))?;
                window.active_tab = Some(id);
                self.active_window = Some(window.id);
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
            }
            Navigation::SetBranchList(list) => {
                let Route::Repository { branches, .. } = &mut self.tab_mut(id)?.route else {
                    return Err(StateError("tab is not a repository route"));
                };
                *branches = list;
            }
            Navigation::SetComparison { base, comparison } => {
                self.complete_comparison(id, ComparisonChange::Resolve, base, comparison)?;
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
