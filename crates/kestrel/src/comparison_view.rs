// TODO: Thoroughly review and improve the comparison page; deferred and non-blocking.
// Follow-up: kestrel/UI/comparison_view_improvements__plan.md in Obsidian.

use std::sync::Arc;

use diff::{Cancellation, Engine};
use gpui::{
    App, Context, Entity, FocusHandle, Focusable, Subscription, Task, Window, div, prelude::*, px,
    uniform_list,
};
use kestrel::{
    review::{Comparison, DiffLine, FileChange, FilePatch, GitPath, PreviewUnavailable, RefName},
    state::{BaseSelection, Browsing, RepositoryPage, Route},
};

use crate::{
    appearance::{DiffTone, Theme},
    commands::Command,
    controls::{control, tooltip},
    diff_selection::{CopySelection, DiffSelection, ROW_HEIGHT as DIFF_ROW_HEIGHT, RowSource},
    repository_model::{Load, RepositoryModel, branch_label, open},
};

mod gutter;
mod patch;
use patch::LoadedPatch;

const DIFF_NUMBER_WIDTH: f32 = 48.;
const DIFF_MARKER_WIDTH: f32 = 12.;
const DIFF_GUTTER_WIDTH: f32 = 2. * DIFF_NUMBER_WIDTH + DIFF_MARKER_WIDTH;
const DIFF_FONT: &str = if cfg!(target_os = "macos") {
    "Menlo"
} else {
    "monospace"
};

struct PendingPatch {
    _task: Task<()>,
    cancellation: Cancellation,
}

impl Drop for PendingPatch {
    fn drop(&mut self) {
        self.cancellation.cancel();
    }
}

pub struct ComparisonView {
    repository: Entity<RepositoryModel>,
    focus: FocusHandle,
    base_open: bool,
    review_key: Option<ReviewKey>,
    review: ReviewStatus,
    review_task: Option<Task<()>>,
    review_revision: u64,
    patch_key: Option<FileChange>,
    patch: Load<LoadedPatch>,
    patch_task: Option<PendingPatch>,
    engine: Arc<Engine>,
    patch_revision: u64,
    selection: Option<(Entity<DiffSelection>, [Subscription; 2])>,
    error: Option<String>,
    _subscriptions: Vec<Subscription>,
}

impl Focusable for ComparisonView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ReviewKey {
    branch: RefName,
    base: BaseSelection,
    comparison: Option<Comparison>,
}

struct ReviewData {
    comparison: Comparison,
    changes: Vec<FileChange>,
    fresh: bool,
}

enum ReviewStatus {
    ChooseBase,
    Loading,
    Ready(ReviewData),
    Failed(String),
}

struct DiffRow {
    old: gpui::SharedString,
    new: gpui::SharedString,
    text: gpui::SharedString,
    marker: &'static str,
    tone: DiffTone,
    source: RowSource,
}

enum PreparedPatch {
    Unavailable(PreviewUnavailable),
    Rows {
        rows: Arc<[DiffRow]>,
        widest: Option<usize>,
        text: gpui::SharedString,
    },
}

impl PreparedPatch {
    fn new(patch: FilePatch) -> Self {
        let hunks = match patch {
            FilePatch::Unavailable(reason) => return Self::Unavailable(reason),
            FilePatch::Hunks(hunks) => hunks,
        };
        let mut rows = Vec::new();
        let mut plain = String::new();
        let mut terminated = true;
        for hunk in hunks {
            rows.push(DiffRow {
                old: "".into(),
                new: "".into(),
                text: hunk.header.trim_end().to_owned().into(),
                marker: "",
                tone: DiffTone::Hunk,
                source: RowSource::Boundary(plain.len()),
            });
            for line in hunk.lines {
                let (old, new, text, tone, prefix) = match line {
                    DiffLine::Context {
                        old_line,
                        new_line,
                        text,
                    } => (
                        old_line.to_string(),
                        new_line.to_string(),
                        text,
                        DiffTone::Context,
                        Some(" "),
                    ),
                    DiffLine::Added { new_line, text } => (
                        String::new(),
                        new_line.to_string(),
                        text,
                        DiffTone::Added,
                        Some("+"),
                    ),
                    DiffLine::Deleted { old_line, text } => (
                        old_line.to_string(),
                        String::new(),
                        text,
                        DiffTone::Deleted,
                        Some("-"),
                    ),
                    DiffLine::NoNewline(side) => (
                        String::new(),
                        String::new(),
                        format!("\\ No newline at end of file ({side:?})"),
                        DiffTone::Metadata,
                        None,
                    ),
                };
                let (text, source) = if prefix.is_some() {
                    terminated = text.ends_with('\n');
                    let text = text
                        .strip_suffix('\n')
                        .map(|line| line.strip_suffix('\r').unwrap_or(line))
                        .unwrap_or(&text);
                    let start = plain.len();
                    plain.push_str(text);
                    let source = RowSource::Code(start..plain.len());
                    // Old and new rows remain separate even if either side lacks a newline.
                    plain.push('\n');
                    (text.to_owned(), source)
                } else {
                    (text, RowSource::Boundary(plain.len()))
                };
                rows.push(DiffRow {
                    old: old.into(),
                    new: new.into(),
                    text: text.into(),
                    marker: prefix.unwrap_or(""),
                    tone,
                    source,
                });
            }
        }
        if !terminated {
            plain.pop();
        }
        let widest = rows
            .iter()
            .enumerate()
            .max_by_key(|(_, row)| row.text.len())
            .map(|(index, _)| index);
        Self::Rows {
            rows: rows.into(),
            widest,
            text: plain.into(),
        }
    }
}

fn path(change: &FileChange) -> &GitPath {
    &change
        .new_file()
        .or_else(|| change.old_file())
        .expect("every change has a file")
        .path
}

struct FileLabel {
    status: &'static str,
    name: String,
    directory: Option<String>,
    tooltip: String,
}

impl FileLabel {
    fn new(change: &FileChange) -> Self {
        let label = path(change).label();
        let (directory, name) = label
            .rsplit_once('/')
            .map_or((None, label.as_str()), |(directory, name)| {
                (Some(directory.to_owned()), name)
            });
        Self {
            status: match change {
                FileChange::Added { .. } => "A",
                FileChange::Deleted { .. } => "D",
                FileChange::Modified { .. } => "M",
                FileChange::Renamed { .. } => "R",
                FileChange::TypeChanged { .. } => "T",
            },
            name: name.to_owned(),
            directory,
            tooltip: match change {
                FileChange::Renamed { old, new } => {
                    format!("{} → {}", old.path.label(), new.path.label())
                }
                _ => label,
            },
        }
    }
}

impl ComparisonView {
    pub fn new(repository: Entity<RepositoryModel>, cx: &mut Context<Self>) -> Self {
        let workspace = repository.read(cx).workspace();
        let subscriptions = vec![
            cx.observe(&workspace, |this, _, cx| this.synchronize(cx)),
            cx.observe(&repository, |this, _, cx| {
                this.refresh_review(cx);
                cx.notify();
            }),
        ];
        let mut this = Self {
            repository,
            focus: cx.focus_handle(),
            base_open: false,
            review_key: None,
            review: ReviewStatus::ChooseBase,
            review_task: None,
            review_revision: 0,
            patch_key: None,
            patch: Load::Loading,
            patch_task: None,
            engine: Arc::new(Engine::bundled()),
            patch_revision: 0,
            selection: None,
            error: None,
            _subscriptions: subscriptions,
        };
        this.synchronize(cx);
        this
    }

    fn synchronize(&mut self, cx: &mut Context<Self>) {
        let key = self.current_key(cx);
        if self.review_key != key {
            self.review_revision += 1;
            self.review_task.take();
            self.patch_revision += 1;
            self.patch_task.take();
            self.selection = None;
            self.patch_key = None;
            self.review_key = key;
            self.base_open = false;
            self.review = ReviewStatus::ChooseBase;
            if self.review_key.is_some() {
                self.load_review(None, false, cx);
            }
        }
        self.load_patch(cx);
        cx.notify();
    }

    fn current_key(&self, cx: &App) -> Option<ReviewKey> {
        let Route::Repository {
            page: RepositoryPage::Comparison,
            review,
            ..
        } = self.repository.read(cx).route(cx)?
        else {
            return None;
        };
        let review = review.active_review()?;
        Some(ReviewKey {
            branch: review.branch.clone(),
            base: review.base.clone(),
            comparison: review.comparison.clone(),
        })
    }

    fn refresh_review(&mut self, cx: &mut Context<Self>) {
        if let Some(snapshot) = self.repository.read(cx).snapshot().data()
            && let ReviewStatus::Ready(data) = &mut self.review
        {
            data.fresh = snapshot.branches.iter().any(|branch| {
                branch.name == data.comparison.branch && branch.tip == data.comparison.commits.new
            }) && snapshot.branches.iter().any(|branch| {
                branch.name == data.comparison.base && branch.tip == data.comparison.base_tip
            });
        }
        if self.review_key.is_some() && matches!(self.review, ReviewStatus::ChooseBase) {
            self.load_review(None, false, cx);
        }
    }

    fn load_review(
        &mut self,
        override_base: Option<RefName>,
        latest: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(key) = self.current_key(cx) else {
            return;
        };
        let base = override_base
            .as_ref()
            .map(|base| BaseSelection::Explicit(base.clone()))
            .unwrap_or_else(|| key.base.clone());
        let reference = match &base {
            BaseSelection::Explicit(base) => Some(base.clone()),
            BaseSelection::Default => self
                .repository
                .read(cx)
                .snapshot()
                .data()
                .and_then(|snapshot| snapshot.default_base.clone()),
        };
        if key.comparison.is_none() && reference.is_none() {
            self.review = ReviewStatus::ChooseBase;
            cx.notify();
            return;
        }
        self.review_revision += 1;
        let revision = self.review_revision;
        self.patch_revision += 1;
        self.patch_task.take();
        self.selection = None;
        self.patch_key = None;
        self.review = ReviewStatus::Loading;
        let project = self.repository.read(cx).project().clone();
        let request = key.clone();
        self.review_task = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let repository = open(&project)?;
                    let comparison = match (&request.comparison, override_base) {
                        (Some(comparison), _) if latest => repository.load_latest(comparison),
                        (Some(comparison), Some(base)) => repository.change_base(comparison, &base),
                        (Some(comparison), None) => Ok(comparison.clone()),
                        (None, _) => repository.compare(
                            &request.branch,
                            &reference.expect("new comparisons have a base"),
                        ),
                    }
                    .map_err(|error| error.to_string())?;
                    let changes = repository
                        .changes(&comparison.commits)
                        .map_err(|error| error.to_string())?;
                    let fresh = repository
                        .freshness(&comparison)
                        .map_err(|error| error.to_string())?
                        .matches(&comparison);
                    Ok::<_, String>(ReviewData {
                        comparison,
                        changes,
                        fresh,
                    })
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.finish_review(revision, key, base, result, cx)
            });
        }));
        cx.notify();
    }

    fn finish_review(
        &mut self,
        revision: u64,
        key: ReviewKey,
        base: BaseSelection,
        result: Result<ReviewData, String>,
        cx: &mut Context<Self>,
    ) {
        if revision != self.review_revision || self.current_key(cx).as_ref() != Some(&key) {
            return;
        }
        match result {
            Ok(data) => {
                let comparison = data.comparison.clone();
                let repository = self.repository.read(cx);
                let tab = repository.tab();
                let result = repository.workspace().update(cx, |workspace, cx| {
                    workspace.edit(
                        |state| state.set_comparison(tab, base.clone(), comparison.clone()),
                        cx,
                    )
                });
                if let Err(error) = result {
                    self.review = ReviewStatus::Failed(error.to_string());
                } else {
                    self.review_key = Some(ReviewKey {
                        branch: comparison.branch.clone(),
                        base,
                        comparison: Some(comparison),
                    });
                    self.review = ReviewStatus::Ready(data);
                    self.load_patch(cx);
                }
            }
            Err(error) => self.review = ReviewStatus::Failed(error),
        }
        cx.notify();
    }

    fn load_patch(&mut self, cx: &mut Context<Self>) {
        let Some(key) = self.current_key(cx) else {
            return;
        };
        let ReviewStatus::Ready(data) = &self.review else {
            return;
        };
        if key.comparison.as_ref() != Some(&data.comparison) {
            return;
        }
        let browsing = self
            .repository
            .read(cx)
            .route(cx)
            .and_then(Route::review)
            .and_then(|review| review.active_review())
            .map(|review| review.browsing.clone())
            .unwrap_or_default();
        let change = data
            .changes
            .iter()
            .find(|change| Some(path(change)) == browsing.file.as_ref())
            .or(data.changes.first())
            .cloned();
        if self.patch_key == change {
            return;
        }
        self.patch_revision += 1;
        self.patch_task.take();
        self.patch_key = change.clone();
        self.selection = None;
        self.patch = Load::Loading;
        let Some(change) = change else {
            return;
        };
        if browsing.file.as_ref() != Some(path(&change)) {
            self.select_file(path(&change).clone(), cx);
        }
        let project = self.repository.read(cx).project().clone();
        let revision = self.patch_revision;
        let selected_file = path(&change).clone();
        let cancellation = Cancellation::default();
        let request_cancellation = cancellation.clone();
        let engine = self.engine.clone();
        let task = cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    LoadedPatch::load(&open(&project)?, &change, &engine, &request_cancellation)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.finish_patch(revision, &key, &selected_file, result, cx);
            });
        });
        self.patch_task = Some(PendingPatch {
            _task: task,
            cancellation,
        });
    }

    fn finish_patch(
        &mut self,
        revision: u64,
        key: &ReviewKey,
        selected_file: &GitPath,
        result: Result<LoadedPatch, String>,
        cx: &mut Context<Self>,
    ) {
        if !self.patch_matches(revision, key, selected_file, cx) {
            return;
        }
        self.selection = None;
        if let Ok(LoadedPatch {
            content: PreparedPatch::Rows { text, .. },
            ..
        }) = &result
        {
            let selection =
                cx.new(|cx| DiffSelection::new(text.clone(), px(DIFF_GUTTER_WIDTH), cx));
            let observe = cx.observe(&selection, |_, _, cx| cx.notify());
            let key = key.clone();
            let selected_file = selected_file.clone();
            let copy = cx.subscribe(&selection, move |this, _, event: &CopySelection, cx| {
                if this.patch_matches(revision, &key, &selected_file, cx) {
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(event.0.clone()));
                }
            });
            self.selection = Some((selection, [observe, copy]));
        }
        self.patch = match result {
            Ok(patch) => Load::Ready(patch),
            Err(error) => Load::Failed(error),
        };
        cx.notify();
    }

    fn patch_matches(&self, revision: u64, key: &ReviewKey, file: &GitPath, cx: &App) -> bool {
        revision == self.patch_revision
            && self.current_key(cx).as_ref() == Some(key)
            && self
                .repository
                .read(cx)
                .route(cx)
                .and_then(Route::review)
                .and_then(|review| review.active_review())
                .and_then(|review| review.browsing.file.as_ref())
                == Some(file)
    }

    fn select_file(&mut self, file: GitPath, cx: &mut Context<Self>) {
        let Some(key) = self.current_key(cx) else {
            return;
        };
        let repository = self.repository.read(cx);
        let tab = repository.tab();
        let result = repository.workspace().update(cx, |workspace, cx| {
            workspace.edit(
                |state| {
                    state.set_browsing(
                        tab,
                        &key.branch,
                        Browsing {
                            file: Some(file),
                            ..Default::default()
                        },
                    )
                },
                cx,
            )
        });
        self.error = result.err().map(|error| error.to_string());
        cx.notify();
    }

    fn base_picker(&self, theme: Theme, window: &Window, cx: &Context<Self>) -> impl IntoElement {
        let label = self
            .current_key(cx)
            .and_then(|key| {
                key.comparison
                    .map(|comparison| format!("Base: {} ▾", comparison.base.label()))
            })
            .unwrap_or_else(|| "Choose base… ▾".into());
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .when_some(
                        window.root::<crate::workspace_window::Kestrel>().flatten(),
                        |row, root| {
                            let root = root.read(cx);
                            row.child(root.navigation_button(
                                "navigate-back",
                                "Back",
                                Command::Back,
                                theme,
                                cx,
                            ))
                            .child(root.navigation_button(
                                "navigate-forward",
                                "Forward",
                                Command::Forward,
                                theme,
                                cx,
                            ))
                        },
                    )
                    .child(div().flex_1())
                    .child(control("choose-base", label, theme).on_click(cx.listener(
                        |this, _, _, cx| {
                            this.base_open = !this.base_open;
                            cx.notify();
                        },
                    )))
                    .child(
                        control("refresh-review-refs", "Refresh local refs", theme).on_click(
                            cx.listener(|this, _, _, cx| {
                                this.repository
                                    .update(cx, |repository, cx| repository.refresh(cx))
                            }),
                        ),
                    )
                    .when(
                        matches!(
                            self.review,
                            ReviewStatus::Ready(_) | ReviewStatus::Failed(_)
                        ),
                        |row| {
                            row.child(control("latest-review", "Load latest", theme).on_click(
                                cx.listener(|this, _, _, cx| this.load_review(None, true, cx)),
                            ))
                        },
                    ),
            )
            .when(
                self.repository.read(cx).snapshot().is_refreshing(),
                |panel| {
                    panel.child(
                        div()
                            .text_color(theme.muted)
                            .child("Refreshing local refs…"),
                    )
                },
            )
            .when_some(
                self.repository
                    .read(cx)
                    .snapshot()
                    .error()
                    .map(str::to_owned),
                |panel, error| panel.child(div().text_color(theme.error).child(error)),
            )
            .when(self.base_open, |picker| {
                picker.child(match self.repository.read(cx).snapshot().data() {
                    Some(snapshot) => uniform_list(
                        "base-options",
                        snapshot.branches.len(),
                        cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                            let Some(snapshot) = this.repository.read(cx).snapshot().data() else {
                                return Vec::new();
                            };
                            range
                                .map(|index| {
                                    let branch = &snapshot.branches[index];
                                    let name = branch.name.clone();
                                    let selector =
                                        format!("base:{}", String::from_utf8_lossy(&name.0));
                                    div()
                                        .id(index)
                                        .debug_selector(move || selector.clone())
                                        .h(px(30.))
                                        .px_2()
                                        .cursor_pointer()
                                        .hover(|style| style.bg(theme.hover))
                                        .child(branch_label(branch))
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.base_open = false;
                                            this.load_review(Some(name.clone()), false, cx);
                                        }))
                                })
                                .collect()
                        }),
                    )
                    .h(px(150.))
                    .into_any_element(),
                    None => div()
                        .child("Bases unavailable or loading…")
                        .into_any_element(),
                })
            })
    }
}

impl Render for ComparisonView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::get(window, cx);
        let mut panel = div()
            .text_color(theme.text)
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_col()
            .gap_3()
            .when_some(self.error.clone(), |panel, error| {
                panel.child(div().text_color(theme.error).child(error))
            })
            .child(self.base_picker(theme, window, cx));
        panel = match &self.review {
            ReviewStatus::ChooseBase => {
                panel.child("Choose a comparison base; no unambiguous repository default is known.")
            }
            ReviewStatus::Loading => panel.child("Loading comparison…"),
            ReviewStatus::Failed(error) => panel
                .child(div().text_color(theme.error).child(error.clone()))
                .child(
                    control("retry-review", "Retry pinned comparison", theme)
                        .on_click(cx.listener(|this, _, _, cx| this.load_review(None, false, cx))),
                ),
            ReviewStatus::Ready(data) => {
                let label = format!(
                    "{} → {} · Changes since merge base",
                    &data.comparison.commits.old.to_string()[..8],
                    &data.comparison.commits.new.to_string()[..8]
                );
                panel
                    .child(div().text_sm().child(label))
                    .when(!data.fresh, |panel| {
                        panel.child(
                            "Local refs have changed or disappeared. This comparison is pinned.",
                        )
                    })
                    .when(data.changes.is_empty(), |panel| {
                        panel.child("No changed files.")
                    })
                    .when(!data.changes.is_empty(), |panel| {
                        panel.child(
                            div()
                                .flex()
                                .flex_1()
                                .min_h_0()
                                .gap_3()
                                .child(
                                    uniform_list(
                                        "changed-files",
                                        data.changes.len(),
                                        cx.processor(
                                            move |this, range: std::ops::Range<usize>, _, cx| {
                                                let ReviewStatus::Ready(data) = &this.review else {
                                                    return Vec::new();
                                                };
                                                range
                                                    .map(|index| {
                                                        this.file_row(
                                                            index,
                                                            &data.changes[index],
                                                            theme,
                                                            cx,
                                                        )
                                                    })
                                                    .collect()
                                            },
                                        ),
                                    )
                                    .w(px(250.))
                                    .h_full(),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .h_full()
                                        .child(self.file_preview(theme, cx)),
                                ),
                        )
                    })
            }
        };
        panel.into_any_element()
    }
}

impl ComparisonView {
    fn file_row(
        &self,
        index: usize,
        change: &FileChange,
        theme: Theme,
        cx: &Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let file = path(change).clone();
        let label = FileLabel::new(change);
        div()
            .id(index)
            .debug_selector(move || format!("changed-file-{index}"))
            .w_full()
            .h(px(46.))
            .px_2()
            .flex()
            .items_center()
            .gap_2()
            .overflow_hidden()
            .cursor_pointer()
            .when(self.patch_key.as_ref() == Some(change), |row| {
                row.bg(theme.selection)
            })
            .tooltip(tooltip(label.tooltip))
            .child(
                div()
                    .w(px(16.))
                    .flex_shrink_0()
                    .text_sm()
                    .child(label.status),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .debug_selector(move || format!("file-name-{index}"))
                            .text_sm()
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .text_ellipsis()
                            .child(label.name),
                    )
                    .when_some(label.directory, |column, directory| {
                        column.child(
                            div()
                                .debug_selector(move || format!("file-directory-{index}"))
                                .text_xs()
                                .text_color(theme.muted)
                                .text_ellipsis()
                                .child(directory),
                        )
                    }),
            )
            .on_click(cx.listener(move |this, _, _, cx| this.select_file(file.clone(), cx)))
    }

    fn file_preview(&self, theme: Theme, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .gap_2()
            .when_some(self.patch_key.as_ref(), |panel, change| {
                let old = change
                    .old_file()
                    .map_or_else(|| "—".into(), |file| format!("{:o}", file.mode));
                let new = change
                    .new_file()
                    .map_or_else(|| "—".into(), |file| format!("{:o}", file.mode));
                panel
                    .child(
                        div()
                            .id("diff-file-title")
                            .debug_selector(|| "diff-file-title".into())
                            .text_sm()
                            .child(path(change).label())
                            .when_some(
                                match &self.patch {
                                    Load::Ready(patch) => Some(patch.status.clone()),
                                    _ => None,
                                },
                                |title, status| title.tooltip(tooltip(status)),
                            ),
                    )
                    .when(old != new, |panel| {
                        panel.child(div().text_sm().child(format!("Mode: {old} → {new}")))
                    })
            })
            .child(div().flex_1().min_h_0().child(self.diff(theme, cx)))
    }

    fn diff(&self, theme: Theme, cx: &mut Context<Self>) -> gpui::AnyElement {
        match &self.patch {
            Load::Loading => div().child("Loading diff…").into_any_element(),
            Load::Failed(error) => div()
                .child(error.clone())
                .child(
                    control("retry-patch", "Retry diff", theme).on_click(cx.listener(
                        |this, _, _, cx| {
                            this.patch_key = None;
                            this.load_patch(cx);
                        },
                    )),
                )
                .into_any_element(),
            Load::Ready(LoadedPatch {
                content: PreparedPatch::Unavailable(reason),
                ..
            }) => {
                use kestrel::review::PreviewUnavailable::*;
                div()
                    .child(match reason {
                        Binary => "Binary file; text preview unavailable.",
                        Encoding => "This file is not UTF-8; text preview unavailable.",
                        TooLarge => "This file exceeds the preview limit.",
                        Submodule => "Submodule; text preview unavailable.",
                        NotAFile => "This entry is not a readable file.",
                    })
                    .into_any_element()
            }
            Load::Ready(LoadedPatch {
                content: PreparedPatch::Rows { rows, .. },
                ..
            }) if rows.is_empty() => div()
                .child("No text changes (metadata or mode change).")
                .into_any_element(),
            Load::Ready(LoadedPatch {
                content: PreparedPatch::Rows { rows, widest, .. },
                ..
            }) => {
                let rows = Arc::clone(rows);
                let selection = self
                    .selection
                    .as_ref()
                    .expect("loaded text patches have a selection controller")
                    .0
                    .clone();
                let row_selection = selection.clone();
                let gutter = gutter::DiffGutter(Arc::clone(&rows));
                let list = uniform_list("diff-lines", rows.len(), move |range, window, cx| {
                    range
                        .map(|index| {
                            let DiffRow {
                                text, tone, source, ..
                            } = &rows[index];
                            div()
                                .debug_selector(move || format!("diff-line-{index}"))
                                .h(px(DIFF_ROW_HEIGHT))
                                .pl(px(DIFF_GUTTER_WIDTH))
                                .flex()
                                .bg(theme.diff(*tone))
                                .font_family(DIFF_FONT)
                                .text_size(px(13.))
                                .child(row_selection.update(cx, |selection, cx| {
                                    selection
                                        .row(index, text.clone(), source.clone(), window, cx)
                                        .debug_selector(move || format!("diff-source-{index}"))
                                }))
                        })
                        .collect::<Vec<_>>()
                })
                .with_width_from_item(*widest)
                .with_decoration(gutter)
                .with_horizontal_sizing_behavior(gpui::ListHorizontalSizingBehavior::Unconstrained)
                .size_full()
                .into_any_element();
                selection
                    .update(cx, |selection, cx| selection.wrap(list, cx))
                    .into_any_element()
            }
        }
    }
}

#[cfg(test)]
mod tests;
