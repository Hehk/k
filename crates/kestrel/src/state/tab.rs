use std::collections::HashSet;

use serde::{Deserialize, Deserializer, Serialize, de::Error};

use super::{
    ProjectId, RepositoryPage, ReviewContext, Route, Snapshot, StateError, Tab, TabId, Visit,
    VisitId,
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Remembered {
    project: ProjectId,
    review: ReviewContext,
}

impl Tab {
    pub(super) fn new(id: TabId) -> Self {
        Self {
            id,
            route: Route::Home,
            visit_id: VisitId(1),
            next_visit: 2,
            activation: 0,
            back: Vec::new(),
            forward: Vec::new(),
            remembered: Vec::new(),
        }
    }

    pub fn visit_id(&self) -> VisitId {
        self.visit_id
    }
    pub fn activation(&self) -> u64 {
        self.activation
    }
    pub fn can_go_back(&self) -> bool {
        !self.back.is_empty()
    }
    pub fn can_go_forward(&self) -> bool {
        !self.forward.is_empty()
    }

    pub fn snapshot(&self) -> Result<Visit, StateError> {
        Ok(Visit {
            id: self.visit_id,
            snapshot: Snapshot::capture(&self.route)?,
        })
    }

    pub(super) fn remembered(&self, project: ProjectId) -> Option<&ReviewContext> {
        self.remembered
            .iter()
            .find(|entry| entry.project == project)
            .map(|entry| &entry.review)
    }

    fn remember(&mut self, route: &Route) {
        if let Route::Repository {
            project, review, ..
        } = route
        {
            if let Some(entry) = self
                .remembered
                .iter_mut()
                .find(|entry| entry.project == *project)
            {
                // Older route snapshots can lack branches remembered by later visits.
                for branch in &review.reviews {
                    entry.review.remember_branch(branch.clone());
                }
                entry.review.selected_branch = review.selected_branch.clone();
            } else {
                self.remembered.push(Remembered {
                    project: *project,
                    review: review.clone(),
                });
            }
        }
    }

    fn allocate_visit(&mut self) -> Result<VisitId, StateError> {
        let id = self.next_visit;
        self.next_visit = id.checked_add(1).ok_or(StateError("visit IDs exhausted"))?;
        Ok(VisitId(id))
    }

    pub(super) fn visit(&mut self, route: Route) -> Result<(), StateError> {
        let before = self.snapshot()?;
        let after = Snapshot::capture(&route)?;
        if before.snapshot.destination == after.destination {
            self.route = route;
            return Ok(());
        }
        self.push_visit(route, before)
    }

    pub(super) fn open_snapshot(&mut self, snapshot: Snapshot) -> Result<(), StateError> {
        let remembered = snapshot
            .project()
            .and_then(|project| {
                if self.route.project() == Some(project) {
                    self.route.review()
                } else {
                    self.remembered(project)
                }
            })
            .cloned()
            .unwrap_or_default();
        snapshot.view.browsing.check_paths()?;
        let route = snapshot.restore(remembered);
        Snapshot::capture(&route)?;
        // An explicit historical selection is a new visit, even for the same destination.
        self.push_visit(route, self.snapshot()?)
    }

    pub(super) fn seed_snapshot(&mut self, snapshot: Snapshot) -> Result<(), StateError> {
        if self != &Self::new(self.id) {
            return Err(StateError("only a new tab can be seeded"));
        }
        self.open_snapshot(snapshot)?;
        self.back.clear();
        Ok(())
    }

    fn push_visit(&mut self, route: Route, before: Visit) -> Result<(), StateError> {
        let activation = self
            .activation
            .checked_add(1)
            .ok_or(StateError("navigation generation exhausted"))?;
        let id = self.allocate_visit()?;
        self.remember(&self.route.clone());
        self.back.push(before);
        self.forward.clear();
        self.route = route;
        self.visit_id = id;
        self.activation = activation;
        Ok(())
    }

    pub(super) fn traverse(&mut self, back: bool) -> Result<bool, StateError> {
        let target = if back {
            self.back.last()
        } else {
            self.forward.last()
        };
        let Some(target) = target.cloned() else {
            return Ok(false);
        };
        let current = self.snapshot()?;
        let activation = self
            .activation
            .checked_add(1)
            .ok_or(StateError("navigation generation exhausted"))?;
        self.remember(&self.route.clone());
        let remembered = target
            .snapshot
            .project()
            .and_then(|project| self.remembered(project))
            .cloned()
            .unwrap_or_default();
        self.route = target.snapshot.restore(remembered);
        self.visit_id = target.id;
        self.activation = activation;
        if back {
            self.back.pop();
            self.forward.push(current);
        } else {
            self.forward.pop();
            self.back.push(current);
        }
        Ok(true)
    }

    // Used only at load/reinsertion boundaries. Do not rebuild routes on every save.
    pub(super) fn routes(&self) -> impl Iterator<Item = Route> + '_ {
        std::iter::once(self.route.clone())
            .chain(
                self.back
                    .iter()
                    .chain(&self.forward)
                    .map(|visit| visit.snapshot.restore(ReviewContext::default())),
            )
            .chain(self.remembered.iter().map(|entry| Route::Repository {
                project: entry.project,
                review: entry.review.clone(),
                page: RepositoryPage::Branches,
                branches: Default::default(),
            }))
    }

    pub(super) fn check_visits(&self) -> Result<(), StateError> {
        let mut ids = HashSet::new();
        for id in std::iter::once(self.visit_id)
            .chain(self.back.iter().chain(&self.forward).map(|visit| visit.id))
        {
            if id.0 == 0 || id.0 >= self.next_visit || !ids.insert(id) {
                return Err(StateError("invalid visit ID sequence"));
            }
        }
        let mut projects = HashSet::new();
        if self
            .remembered
            .iter()
            .any(|entry| !projects.insert(entry.project))
        {
            return Err(StateError("duplicate remembered repository"));
        }
        for visit in self.back.iter().chain(&self.forward) {
            visit.snapshot.view.browsing.check_paths()?;
        }
        Ok(())
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum SavedVisit {
    Visit(Visit),
    Route(Route),
}

impl<'de> Deserialize<'de> for Tab {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Fields {
            id: TabId,
            route: Route,
            #[serde(default)]
            visit_id: Option<VisitId>,
            #[serde(default)]
            next_visit: Option<u64>,
            #[serde(default)]
            activation: u64,
            #[serde(default)]
            back: Vec<SavedVisit>,
            #[serde(default)]
            forward: Vec<SavedVisit>,
            #[serde(default)]
            remembered: Vec<Remembered>,
        }
        let fields = Fields::deserialize(deserializer)?;
        let mut tab = Self {
            id: fields.id,
            route: fields.route,
            visit_id: fields.visit_id.unwrap_or(VisitId(0)),
            next_visit: fields.next_visit.unwrap_or(1),
            activation: fields.activation,
            back: Vec::new(),
            forward: Vec::new(),
            remembered: fields.remembered,
        };
        let mut convert = |entries: Vec<SavedVisit>| -> Result<Vec<Visit>, StateError> {
            entries
                .into_iter()
                .map(|entry| match entry {
                    SavedVisit::Visit(visit) => Ok(visit),
                    SavedVisit::Route(route) => {
                        let snapshot = Snapshot::capture(&route)?;
                        tab.remember(&route);
                        Ok(Visit {
                            id: tab.allocate_visit()?,
                            snapshot,
                        })
                    }
                })
                .collect()
        };
        let back = convert(fields.back).map_err(D::Error::custom)?;
        let forward = convert(fields.forward).map_err(D::Error::custom)?;
        tab.back = back;
        tab.forward = forward;
        if fields.visit_id.is_none() {
            tab.visit_id = tab.allocate_visit().map_err(D::Error::custom)?;
            tab.remember(&tab.route.clone());
        }
        tab.check_visits().map_err(D::Error::custom)?;
        Ok(tab)
    }
}
