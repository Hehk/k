use serde::{Deserialize, Serialize};

use super::{
    BaseSelection, BranchList, BranchReview, Browsing, ProjectId, RepositoryPage, ReviewContext,
    Route, StateError,
};
use crate::review::{Comparison, RefName};

#[cfg(test)]
mod tests;

/// Stable within a tab; activation generations distinguish repeated loads of a visit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct VisitId(pub(super) u64);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Destination {
    Home,
    Branches(ProjectId),
    Comparison {
        project: ProjectId,
        branch: RefName,
        base: BaseSelection,
        comparison: Option<Comparison>,
    },
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViewState {
    pub branches: BranchList,
    pub browsing: Browsing,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    pub destination: Destination,
    pub view: ViewState,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Visit {
    pub id: VisitId,
    pub snapshot: Snapshot,
}

impl Snapshot {
    pub fn capture(route: &Route) -> Result<Self, StateError> {
        match route {
            Route::Home => Ok(Self {
                destination: Destination::Home,
                view: ViewState::default(),
            }),
            Route::Repository {
                project,
                review,
                page,
                branches,
            } => {
                review.check_loaded()?;
                let mut view = ViewState {
                    branches: branches.clone(),
                    ..Default::default()
                };
                let destination = match page {
                    RepositoryPage::Branches => Destination::Branches(*project),
                    RepositoryPage::Comparison => {
                        let review = review
                            .active_review()
                            .ok_or(StateError("comparison page has no branch"))?;
                        view.browsing = review.browsing.clone();
                        Destination::Comparison {
                            project: *project,
                            branch: review.branch.clone(),
                            base: review.base.clone(),
                            comparison: review.comparison.clone(),
                        }
                    }
                };
                Ok(Self { destination, view })
            }
        }
    }

    pub fn project(&self) -> Option<ProjectId> {
        match self.destination {
            Destination::Home => None,
            Destination::Branches(project) | Destination::Comparison { project, .. } => {
                Some(project)
            }
        }
    }

    pub(super) fn restore(&self, mut remembered: ReviewContext) -> Route {
        let (project, page) = match &self.destination {
            Destination::Home => return Route::Home,
            Destination::Branches(project) => (*project, RepositoryPage::Branches),
            Destination::Comparison {
                project,
                branch,
                base,
                comparison,
            } => {
                let restored = BranchReview {
                    branch: branch.clone(),
                    base: base.clone(),
                    comparison: comparison.clone(),
                    browsing: self.view.browsing.clone(),
                };
                // Other branches remain remembered, but never overwrite this visit's pins or view.
                remembered.remember_branch(restored);
                remembered.selected_branch = Some(branch.clone());
                (*project, RepositoryPage::Comparison)
            }
        };
        Route::Repository {
            project,
            review: remembered,
            page,
            branches: self.view.branches.clone(),
        }
    }
}
