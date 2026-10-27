use gpui::{Context, Entity, Focusable, Subscription, Window, div, prelude::*};
use kestrel::state::{Project, RepositoryPage, Route, TabId, WindowId};

use crate::{
    branch_view::BranchView, comparison_view::ComparisonView, repository_model::RepositoryModel,
    workspace::Workspace,
};

pub struct RepositoryView {
    repository: Entity<RepositoryModel>,
    branches: Entity<BranchView>,
    comparison: Entity<ComparisonView>,
    painted_page: Option<RepositoryPage>,
    _subscription: Subscription,
}

impl RepositoryView {
    #[cfg(test)]
    pub fn comparison_freshness(&self, cx: &gpui::App) -> Option<bool> {
        self.comparison.read(cx).freshness()
    }

    pub fn new(
        workspace: Entity<Workspace>,
        window: WindowId,
        tab: TabId,
        project: Project,
        cx: &mut Context<Self>,
    ) -> Self {
        let subscription = cx.observe(&workspace, |_, _, cx| cx.notify());
        let repository = cx.new(|cx| RepositoryModel::new(workspace, window, tab, project, cx));
        let branches = cx.new(|cx| BranchView::new(repository.clone(), cx));
        let comparison = cx.new(|cx| ComparisonView::new(repository.clone(), cx));
        Self {
            repository,
            branches,
            comparison,
            painted_page: None,
            _subscription: subscription,
        }
    }
}

impl Render for RepositoryView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let page = match self.repository.read(cx).route(cx) {
            Some(Route::Repository { page, .. }) => *page,
            _ => RepositoryPage::Branches,
        };
        if self.painted_page.is_some_and(|previous| previous != page) {
            let focus = match page {
                RepositoryPage::Branches => self.branches.read(cx).focus_handle(cx),
                RepositoryPage::Comparison => self.comparison.read(cx).focus_handle(cx),
            };
            window.focus(&focus);
        }
        self.painted_page = Some(page);
        div()
            .size_full()
            .min_h_0()
            .flex()
            .flex_col()
            .child(match page {
                RepositoryPage::Branches => self.branches.clone().into_any_element(),
                RepositoryPage::Comparison => self.comparison.clone().into_any_element(),
            })
    }
}
