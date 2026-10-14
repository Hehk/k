// TODO: Thoroughly review and improve the branch-list page; deferred and non-blocking.
// Follow-up: kestrel/UI/branch_list_improvements__plan.md in Obsidian.

use gpui::{
    App, Context, Entity, FocusHandle, Focusable, Subscription, Window, deferred, div, prelude::*,
    px, rgb, uniform_list,
};
use kestrel::{
    git::Snapshot,
    review::RefName,
    state::{BranchList, BranchSort, RepositoryPage, Route},
};

use crate::{
    controls::control,
    repository_model::{Load, RepositoryModel, branch_label},
    text_input::{InputEvent, TextInput},
};

pub struct BranchView {
    repository: Entity<RepositoryModel>,
    focus: FocusHandle,
    search: Entity<TextInput>,
    filters: BranchList,
    visible: Vec<usize>,
    sort_open: bool,
    error: Option<String>,
    _subscriptions: Vec<Subscription>,
}

impl Focusable for BranchView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

fn ordered_branches(snapshot: &Snapshot, filters: &BranchList) -> Vec<usize> {
    let query = filters.query.to_lowercase();
    let mut rows = snapshot
        .branches
        .iter()
        .enumerate()
        .filter(|(_, branch)| branch.name.label().to_lowercase().contains(&query))
        .map(|(index, branch)| (index, branch.name.label().to_lowercase()))
        .collect::<Vec<_>>();
    rows.sort_by(|(a, name_a), (b, name_b)| {
        let a = &snapshot.branches[*a];
        let b = &snapshot.branches[*b];
        let by_name = || name_a.cmp(name_b).then_with(|| a.name.0.cmp(&b.name.0));
        match filters.sort {
            BranchSort::Newest => b.committed_at.cmp(&a.committed_at).then_with(by_name),
            BranchSort::Name => by_name(),
        }
    });
    rows.into_iter().map(|(index, _)| index).collect()
}

impl BranchView {
    pub fn new(repository: Entity<RepositoryModel>, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| TextInput::new("branch-search", "Search branches…", cx));
        let workspace = repository.read(cx).workspace();
        let subscriptions = vec![
            cx.observe(&workspace, |this, _, cx| this.synchronize(cx)),
            cx.observe(&repository, |this, _, cx| {
                this.filter(cx);
                cx.notify();
            }),
            cx.subscribe(&search, |this, _, event: &InputEvent, cx| match event {
                InputEvent::Changed(query) => this.change_filters(
                    BranchList {
                        query: query.clone(),
                        ..this.filters.clone()
                    },
                    cx,
                ),
                InputEvent::Submit => {
                    if let Load::Ready(snapshot) = this.repository.read(cx).snapshot()
                        && let Some(index) = this.visible.first()
                    {
                        this.select_branch(snapshot.branches[*index].name.clone(), cx);
                    }
                }
            }),
        ];
        let mut this = Self {
            repository,
            focus: cx.focus_handle(),
            search,
            filters: BranchList::default(),
            visible: Vec::new(),
            sort_open: false,
            error: None,
            _subscriptions: subscriptions,
        };
        this.synchronize(cx);
        this.filter(cx);
        this
    }

    fn synchronize(&mut self, cx: &mut Context<Self>) {
        if let Some(Route::Repository { branches, page, .. }) = self.repository.read(cx).route(cx) {
            if *page != RepositoryPage::Branches {
                self.sort_open = false;
            }
            let branches = branches.clone();
            self.search
                .update(cx, |search, cx| search.set_text(&branches.query, cx));
            if self.filters != branches {
                self.filters = branches;
                self.filter(cx);
            }
        }
        cx.notify();
    }

    fn filter(&mut self, cx: &App) {
        self.visible = match self.repository.read(cx).snapshot() {
            Load::Ready(snapshot) => ordered_branches(snapshot, &self.filters),
            _ => Vec::new(),
        };
    }

    fn change_filters(&mut self, filters: BranchList, cx: &mut Context<Self>) {
        let repository = self.repository.read(cx);
        if !matches!(
            repository.route(cx),
            Some(Route::Repository {
                page: RepositoryPage::Branches,
                ..
            })
        ) {
            return;
        }
        let tab = repository.tab();
        let result = repository.workspace().update(cx, |workspace, cx| {
            workspace.edit(|state| state.set_branch_list(tab, filters), cx)
        });
        self.error = result.err().map(|error| error.to_string());
        cx.notify();
    }

    fn select_branch(&mut self, branch: RefName, cx: &mut Context<Self>) {
        let repository = self.repository.read(cx);
        if !matches!(
            repository.route(cx),
            Some(Route::Repository {
                page: RepositoryPage::Branches,
                ..
            })
        ) {
            return;
        }
        let tab = repository.tab();
        let result = repository.workspace().update(cx, |workspace, cx| {
            workspace.edit(|state| state.select_branch(tab, branch), cx)
        });
        self.error = result.err().map(|error| error.to_string());
        self.synchronize(cx);
    }

    fn sort_menu(&self, cx: &Context<Self>) -> impl IntoElement {
        div()
            .relative()
            .child(
                control(
                    "branch-sort",
                    match self.filters.sort {
                        BranchSort::Newest => "Sort: Newest commit ▾",
                        BranchSort::Name => "Sort: Name (A–Z) ▾",
                    },
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.sort_open = !this.sort_open;
                    cx.notify();
                })),
            )
            .when(self.sort_open, |menu| {
                menu.child(
                    deferred(
                        div()
                            .id("branch-sort-menu")
                            .absolute()
                            .top_full()
                            .right_0()
                            .w_56()
                            .occlude()
                            .bg(rgb(0xffffff))
                            .border_1()
                            .border_color(rgb(0xc8cbd2))
                            .rounded_md()
                            .shadow_md()
                            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                                this.sort_open = false;
                                cx.notify();
                            }))
                            .children(
                                [
                                    (BranchSort::Newest, "sort-newest", "Newest commit"),
                                    (BranchSort::Name, "sort-name", "Name (A–Z)"),
                                ]
                                .into_iter()
                                .map(|(sort, id, label)| {
                                    control(id, label).w_full().on_click(cx.listener(
                                        move |this, _, _, cx| {
                                            this.sort_open = false;
                                            let filters = BranchList {
                                                sort,
                                                ..this.filters.clone()
                                            };
                                            this.change_filters(filters, cx);
                                        },
                                    ))
                                }),
                            ),
                    )
                    .with_priority(1),
                )
            })
    }
}

impl Render for BranchView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let header = div()
            .flex()
            .gap_3()
            .items_center()
            .child(div().flex_1().min_w_0().child(self.search.clone()))
            .child(self.sort_menu(cx))
            .child(
                control("refresh-branches", "Refresh local refs").on_click(cx.listener(
                    |this, _, _, cx| {
                        this.repository
                            .update(cx, |repository, cx| repository.refresh(cx))
                    },
                )),
            );
        let body = match self.repository.read(cx).snapshot() {
            Load::Loading => div().child("Loading branches…").into_any_element(),
            Load::Failed(error) => div()
                .text_color(rgb(0xaa3030))
                .child(error.clone())
                .into_any_element(),
            Load::Ready(snapshot) if snapshot.branches.is_empty() => div()
                .child("No branches yet. This repository may be empty.")
                .into_any_element(),
            Load::Ready(_) if self.visible.is_empty() => div()
                .child("No branches match your search.")
                .into_any_element(),
            Load::Ready(_) => uniform_list(
                "branches",
                self.visible.len(),
                cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                    let repository = this.repository.read(cx);
                    let Load::Ready(snapshot) = repository.snapshot() else {
                        return Vec::new();
                    };
                    range
                        .map(|row| {
                            let branch = &snapshot.branches[this.visible[row]];
                            let name = branch.name.clone();
                            let selector = format!("branch:{}", String::from_utf8_lossy(&name.0));
                            div()
                                .id(row)
                                .debug_selector(move || selector.clone())
                                .h(px(38.))
                                .px_2()
                                .flex()
                                .items_center()
                                .justify_between()
                                .cursor_pointer()
                                .hover(|style| style.bg(rgb(0xf1f4fa)))
                                .when(
                                    repository
                                        .route(cx)
                                        .and_then(Route::review)
                                        .and_then(|review| review.selected_branch.as_ref())
                                        == Some(&branch.name),
                                    |row| row.bg(rgb(0xe7edf9)),
                                )
                                .child(branch_label(branch))
                                .child(
                                    div()
                                        .text_sm()
                                        .text_color(rgb(0x777980))
                                        .child(branch.tip.to_string()[..8].to_owned()),
                                )
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    window.focus(&this.focus);
                                    this.select_branch(name.clone(), cx);
                                }))
                        })
                        .collect()
                }),
            )
            .flex_1()
            .min_h_0()
            .into_any_element(),
        };
        div()
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_col()
            .gap_3()
            .when_some(self.error.clone(), |panel, error| {
                panel.child(div().text_color(rgb(0xaa3030)).child(error))
            })
            .child(header)
            .child(body)
    }
}

#[cfg(test)]
mod tests;
