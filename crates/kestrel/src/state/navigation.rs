use super::{
    AppState, BaseSelection, BranchList, Browsing, ProjectId, RepositoryPage, Route, StateError,
    TabId, resume_branch_list,
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
    SetComparison {
        base: BaseSelection,
        comparison: Comparison,
    },
    ComparisonLoaded {
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
                self.tab_mut(id)?.visit(Route::Home);
            }
            Navigation::OpenProject(project) => {
                let remembered = &self.project(project)?.last_review;
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
                self.tab_mut(id)?.visit(route);
            }
            Navigation::Back => {
                let tab = self.tab_mut(id)?;
                let Some(mut route) = tab.back.pop() else {
                    return Ok(());
                };
                resume_branch_list(&tab.route, &mut route);
                tab.forward.push(std::mem::replace(&mut tab.route, route));
                self.remember_tab(id);
            }
            Navigation::Forward => {
                let tab = self.tab_mut(id)?;
                let Some(mut route) = tab.forward.pop() else {
                    return Ok(());
                };
                resume_branch_list(&tab.route, &mut route);
                tab.back.push(std::mem::replace(&mut tab.route, route));
                self.remember_tab(id);
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
                self.tab_mut(id)?.visit(route);
                self.remember_tab(id);
            }
            Navigation::SetBranchList(list) => {
                let Route::Repository { branches, .. } = &mut self.tab_mut(id)?.route else {
                    return Err(StateError("tab is not a repository route"));
                };
                *branches = list;
            }
            Navigation::SetComparison { base, comparison }
            | Navigation::ComparisonLoaded {
                base, comparison, ..
            } => {
                if !comparison.base.is_branch() {
                    return Err(StateError("invalid base ref"));
                }
                if let BaseSelection::Explicit(base) = &base
                    && *base != comparison.base
                {
                    return Err(StateError("comparison does not match its selected base"));
                }
                self.edit_review(id, |context| {
                    let review = context
                        .reviews
                        .iter_mut()
                        .find(|review| review.branch == comparison.branch)
                        .ok_or(StateError("unknown branch review"))?;
                    review.base = base;
                    review.comparison = Some(comparison);
                    Ok(())
                })?;
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
}
