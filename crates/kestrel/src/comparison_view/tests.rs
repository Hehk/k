use super::*;
use crate::workspace::{LoadedWorkspace, Workspace};
use gpui::{Entity, TestAppContext};
use kestrel::{
    git::RepositoryInfo,
    review::{DiffSide, Endpoints, FileEntry, Hunk},
    state::TabId,
    storage::Store,
};

mod structural;

#[test]
fn changed_file_labels_prioritize_names_without_losing_path_or_status() {
    let entry = |path: GitPath| FileEntry {
        path,
        id: "1111111111111111111111111111111111111111".parse().unwrap(),
        mode: 0o100644,
    };
    let old = entry("crates/old_module/mod.rs".into());
    let new = entry("crates/deeply/nested/new_module/mod.rs".into());
    for (change, status, name, directory, tooltip) in [
        (
            FileChange::Added {
                new: entry("README.md".into()),
            },
            "A",
            "README.md",
            None,
            "README.md",
        ),
        (
            FileChange::Modified {
                old: new.clone(),
                new: new.clone(),
            },
            "M",
            "mod.rs",
            Some("crates/deeply/nested/new_module"),
            "crates/deeply/nested/new_module/mod.rs",
        ),
        (
            FileChange::Deleted { old: old.clone() },
            "D",
            "mod.rs",
            Some("crates/old_module"),
            "crates/old_module/mod.rs",
        ),
        (
            FileChange::Renamed {
                old: old.clone(),
                new: new.clone(),
            },
            "R",
            "mod.rs",
            Some("crates/deeply/nested/new_module"),
            "crates/old_module/mod.rs → crates/deeply/nested/new_module/mod.rs",
        ),
        (
            FileChange::TypeChanged { old, new },
            "T",
            "mod.rs",
            Some("crates/deeply/nested/new_module"),
            "crates/deeply/nested/new_module/mod.rs",
        ),
    ] {
        let label = FileLabel::new(&change);
        assert_eq!(
            (
                label.status,
                label.name.as_str(),
                label.directory.as_deref(),
                label.tooltip.as_str()
            ),
            (status, name, directory, tooltip)
        );
    }
    let raw_path = GitPath(b"nested/\xff.rs".to_vec());
    let change = FileChange::Added {
        new: entry(raw_path.clone()),
    };
    let label = FileLabel::new(&change);
    assert_eq!(label.name, "�.rs");
    assert_eq!(path(&change), &raw_path);
}

#[test]
fn prepared_rows_preserve_hunks_line_numbers_markers_and_whitespace() {
    let header = "@".repeat(200);
    let patch = FilePatch::Hunks(vec![
        Hunk {
            header: "@@ -10,2 +20,2 @@\n".into(),
            lines: vec![
                DiffLine::Context {
                    old_line: 10,
                    new_line: 20,
                    text: "keep λ  \r\n".into(),
                },
                DiffLine::Deleted {
                    old_line: 11,
                    text: "removed\n".into(),
                },
                DiffLine::Added {
                    new_line: 21,
                    text: "added\r\n".into(),
                },
                DiffLine::NoNewline(DiffSide::Old),
                DiffLine::NoNewline(DiffSide::New),
                DiffLine::NoNewline(DiffSide::Both),
            ],
        },
        Hunk {
            header: format!("{header}\n"),
            lines: vec![DiffLine::Added {
                new_line: 100,
                text: "tail".into(),
            }],
        },
    ]);
    let PreparedPatch::Rows { rows, widest, text } = PreparedPatch::new(patch) else {
        panic!()
    };
    let actual = rows
        .iter()
        .map(|row| {
            (
                row.old.as_ref(),
                row.new.as_ref(),
                row.text.as_ref(),
                row.tone,
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        actual,
        vec![
            ("", "", "@@ -10,2 +20,2 @@", DiffTone::Hunk),
            ("10", "20", "keep λ  ", DiffTone::Context),
            ("11", "", "removed", DiffTone::Deleted),
            ("", "21", "added", DiffTone::Added),
            (
                "",
                "",
                "\\ No newline at end of file (Old)",
                DiffTone::Metadata
            ),
            (
                "",
                "",
                "\\ No newline at end of file (New)",
                DiffTone::Metadata
            ),
            (
                "",
                "",
                "\\ No newline at end of file (Both)",
                DiffTone::Metadata
            ),
            ("", "", header.as_str(), DiffTone::Hunk),
            ("", "100", "tail", DiffTone::Added),
        ]
    );
    assert_eq!(widest, Some(7));
    assert_eq!(
        rows.iter().map(|row| row.marker).collect::<Vec<_>>(),
        vec!["", " ", "-", "+", "", "", "", "", "+"]
    );
    assert_eq!(text.as_ref(), "keep λ  \nremoved\nadded\ntail");
    for row in rows.iter() {
        if let RowSource::Code(source) = &row.source {
            assert_eq!(&text[source.clone()], row.text.as_ref());
        }
    }
}

#[test]
fn preparing_a_patch_preserves_empty_and_unavailable_previews() {
    let PreparedPatch::Rows { rows, widest, text } =
        PreparedPatch::new(FilePatch::Hunks(Vec::new()))
    else {
        panic!()
    };
    assert!(rows.is_empty());
    assert!(text.is_empty());
    assert_eq!(widest, None);
    for reason in [
        PreviewUnavailable::Binary,
        PreviewUnavailable::Encoding,
        PreviewUnavailable::TooLarge,
        PreviewUnavailable::Submodule,
        PreviewUnavailable::NotAFile,
    ] {
        let PreparedPatch::Unavailable(actual) = PreparedPatch::new(FilePatch::Unavailable(reason))
        else {
            panic!()
        };
        assert_eq!(actual, reason);
    }
}

#[test]
fn prepared_copy_text_preserves_source_markers_blank_lines_and_final_newlines() {
    for text in ["\n", "+-code\t  \n", "a\u{301}😀", "lone\r", "crlf\r\n"] {
        let PreparedPatch::Rows { text: plain, .. } =
            PreparedPatch::new(FilePatch::Hunks(vec![Hunk {
                header: "@@ header @@\n".into(),
                lines: vec![
                    DiffLine::Added {
                        new_line: 1,
                        text: text.into(),
                    },
                    DiffLine::NoNewline(DiffSide::New),
                ],
            }]))
        else {
            panic!()
        };
        assert_eq!(plain.as_ref(), text.replace("\r\n", "\n"));
    }
    let PreparedPatch::Rows { text, .. } = PreparedPatch::new(FilePatch::Hunks(vec![Hunk {
        header: "@@ header @@\n".into(),
        lines: vec![
            DiffLine::Deleted {
                old_line: 1,
                text: "old".into(),
            },
            DiffLine::NoNewline(DiffSide::Old),
            DiffLine::Added {
                new_line: 1,
                text: "new".into(),
            },
            DiffLine::NoNewline(DiffSide::New),
        ],
    }])) else {
        panic!()
    };
    assert_eq!(text.as_ref(), "old\nnew");
}

#[test]
fn prepared_width_keeps_the_last_longest_row() {
    let PreparedPatch::Rows { widest, .. } = PreparedPatch::new(FilePatch::Hunks(vec![Hunk {
        header: "@".into(),
        lines: vec![
            DiffLine::Added {
                new_line: 1,
                text: "é\n".into(),
            },
            DiffLine::Added {
                new_line: 2,
                text: "ab\n".into(),
            },
        ],
    }])) else {
        panic!()
    };
    assert_eq!(widest, Some(2));
}

fn setup(
    cx: &mut TestAppContext,
) -> (
    tempfile::TempDir,
    Entity<Workspace>,
    TabId,
    Entity<RepositoryModel>,
) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let workspace = cx.new(|cx| {
        Workspace::new(
            LoadedWorkspace::load(Ok(Store::new(dir.path().join("state.json")))),
            cx,
        )
    });
    let (window, tab, project) = workspace.update(cx, |workspace, cx| {
        workspace
            .edit(
                |state| {
                    let project = state.register_project(&RepositoryInfo {
                        directory: root.join("repo"),
                        common_dir: root.join("repo/.git"),
                        current_branch: None,
                    })?;
                    let window = state.new_window()?;
                    let tab = state.new_tab(window)?;
                    state.open_project(project, tab)?;
                    state.select_branch(tab, "refs/heads/topic".into())?;
                    Ok((window, tab, state.project(project)?.clone()))
                },
                cx,
            )
            .unwrap()
    });
    let repository = cx.new(|cx| RepositoryModel::new(workspace.clone(), window, tab, project, cx));
    (dir, workspace, tab, repository)
}

fn data(base: &str) -> ReviewData {
    let id = "1111111111111111111111111111111111111111".parse().unwrap();
    ReviewData {
        comparison: Comparison {
            branch: "refs/heads/topic".into(),
            base: base.into(),
            base_tip: id,
            commits: Endpoints { old: id, new: id },
        },
        changes: Vec::new(),
        fresh: true,
    }
}

#[gpui::test]
fn returning_to_the_same_visit_rejects_its_old_request_before_observers_run(
    cx: &mut TestAppContext,
) {
    let (_dir, workspace, tab, repository) = setup(cx);
    let (view, cx) = cx.add_window_view(|_, cx| ComparisonView::new(repository, cx));
    view.update(cx, |view, cx| {
        view.load_review(Some("refs/heads/main".into()), false, cx);
        let revision = view.review_revision;
        let key = view.current_key(cx).unwrap();
        workspace.update(cx, |workspace, cx| {
            workspace
                .edit(
                    |state| {
                        state.go_back(tab)?;
                        state.go_forward(tab)
                    },
                    cx,
                )
                .unwrap();
        });
        let before = workspace.read(cx).state().clone();
        assert_eq!(view.review_revision, revision);
        view.finish_review(
            revision,
            key,
            BaseSelection::Default,
            Ok(data("refs/heads/main")),
            cx,
        );
        assert_eq!(workspace.read(cx).state(), &before);
        assert!(view.current_key(cx).unwrap().comparison.is_none());
    });
}

#[gpui::test]
fn selecting_the_same_file_preserves_position_and_missing_files_are_not_substituted(
    cx: &mut TestAppContext,
) {
    let (_dir, workspace, tab, repository) = setup(cx);
    workspace.update(cx, |workspace, cx| {
        workspace
            .edit(
                |state| {
                    state.set_comparison(
                        tab,
                        BaseSelection::Default,
                        data("refs/heads/main").comparison,
                    )?;
                    state.set_browsing(
                        tab,
                        &"refs/heads/topic".into(),
                        Browsing {
                            file: Some("missing.rs".into()),
                            first_visible_line: 45,
                            ..Default::default()
                        },
                    )
                },
                cx,
            )
            .unwrap();
    });
    let (view, cx) = cx.add_window_view(|_, cx| ComparisonView::new(repository, cx));
    view.update(cx, |view, cx| {
        let before = workspace.read(cx).state().clone();
        view.select_file("missing.rs".into(), cx);
        assert_eq!(workspace.read(cx).state(), &before);
        let mut loaded = data("refs/heads/main");
        loaded.changes.push(FileChange::Added {
            new: kestrel::review::FileEntry {
                path: "different.rs".into(),
                id: loaded.comparison.commits.new,
                mode: 0o100644,
            },
        });
        view.review.complete(Ok(loaded));
        view.load_patch(cx);
        assert!(
            view.patch
                .error()
                .is_some_and(|error| error.contains("saved file"))
        );
        assert_eq!(workspace.read(cx).state(), &before);
    });
}

#[gpui::test]
fn late_comparisons_cannot_navigate_or_overwrite_another_branch(cx: &mut TestAppContext) {
    let (_dir, workspace, tab, repository) = setup(cx);
    let (view, cx) = cx.add_window_view(|_, cx| ComparisonView::new(repository, cx));
    let (revision, key) = view.update(cx, |view, cx| {
        view.load_review(Some("refs/heads/main".into()), false, cx);
        (view.review_revision, view.current_key(cx).unwrap())
    });
    workspace.update(cx, |workspace, cx| {
        workspace
            .edit(
                |state| state.select_branch(tab, "refs/heads/other".into()),
                cx,
            )
            .unwrap()
    });
    let before = workspace.read_with(cx, |workspace, _| workspace.state().clone());
    view.update(cx, |view, cx| {
        view.finish_review(
            revision,
            key.clone(),
            BaseSelection::Explicit("refs/heads/main".into()),
            Ok(data("refs/heads/main")),
            cx,
        )
    });
    assert_eq!(
        workspace.read_with(cx, |workspace, _| workspace.state().clone()),
        before
    );
    workspace.update(cx, |workspace, cx| {
        workspace
            .edit(
                |state| {
                    state.close_tab(tab);
                    Ok(())
                },
                cx,
            )
            .unwrap()
    });
    let closed = workspace.read_with(cx, |workspace, _| workspace.state().clone());
    view.update(cx, |view, cx| {
        view.finish_review(
            revision,
            key,
            BaseSelection::Explicit("refs/heads/main".into()),
            Ok(data("refs/heads/main")),
            cx,
        )
    });
    assert_eq!(
        workspace.read_with(cx, |workspace, _| workspace.state().clone()),
        closed
    );
}

#[gpui::test]
fn only_the_latest_base_request_can_complete_even_for_the_same_route(cx: &mut TestAppContext) {
    let (_dir, _, _, repository) = setup(cx);
    let (view, cx) = cx.add_window_view(|_, cx| ComparisonView::new(repository, cx));
    view.update(cx, |view, cx| {
        view.load_review(Some("refs/heads/old-base".into()), false, cx);
        let old_revision = view.review_revision;
        let key = view.current_key(cx).unwrap();
        view.load_review(Some("refs/heads/main".into()), false, cx);
        let revision = view.review_revision;
        view.finish_review(
            old_revision,
            key.clone(),
            BaseSelection::Explicit("refs/heads/old-base".into()),
            Ok(data("refs/heads/old-base")),
            cx,
        );
        assert!(view.current_key(cx).unwrap().comparison.is_none());
        view.finish_review(
            revision,
            key,
            BaseSelection::Explicit("refs/heads/main".into()),
            Ok(data("refs/heads/main")),
            cx,
        );
        assert_eq!(
            view.current_key(cx).unwrap().comparison.unwrap().base,
            RefName::from("refs/heads/main")
        );
    });
}

#[gpui::test]
fn comparison_query_waits_for_a_base_and_resets_for_new_requests(cx: &mut TestAppContext) {
    let (_dir, workspace, tab, repository) = setup(cx);
    let (view, cx) = cx.add_window_view(|_, cx| ComparisonView::new(repository, cx));
    view.update(cx, |view, cx| {
        assert!(view.review.is_pending());
        assert!(!view.review.is_fetching());
        view.load_review(Some("refs/heads/main".into()), false, cx);
        assert!(view.review.is_loading());
        let key = view.current_key(cx).unwrap();
        view.finish_review(
            view.review_revision,
            key.clone(),
            BaseSelection::Explicit("refs/heads/main".into()),
            Err("comparison unavailable".into()),
            cx,
        );
        assert_eq!(view.review.error(), Some("comparison unavailable"));
        assert!(!view.review.is_fetching());

        view.load_review(Some("refs/heads/main".into()), false, cx);
        assert!(view.review.is_loading());
        assert!(view.review.error().is_none());
        view.finish_review(
            view.review_revision,
            key,
            BaseSelection::Explicit("refs/heads/main".into()),
            Ok(data("refs/heads/main")),
            cx,
        );
        assert!(view.review.data().is_some());
        assert!(!view.review.is_fetching());

        view.load_review(Some("refs/heads/other-base".into()), false, cx);
        assert!(view.review.is_loading());
        assert!(view.review.data().is_none());
    });
    workspace.update(cx, |workspace, cx| {
        workspace
            .edit(
                |state| state.select_branch(tab, "refs/heads/other".into()),
                cx,
            )
            .unwrap();
    });
    view.update(cx, |view, cx| {
        view.synchronize(cx);
        assert!(view.review.is_pending());
        assert!(!view.review.is_fetching());
        assert!(view.review.data().is_none());
    });
}

#[gpui::test]
fn patch_queries_reset_on_retry_and_file_changes(cx: &mut TestAppContext) {
    let (_dir, _, _, repository) = setup(cx);
    let (view, cx) = cx.add_window_view(|_, cx| ComparisonView::new(repository, cx));
    view.update(cx, |view, cx| {
        view.load_review(Some("refs/heads/main".into()), false, cx);
        let mut review = data("refs/heads/main");
        review.changes = ["first.rs", "second.rs"]
            .map(|file| FileChange::Added {
                new: FileEntry {
                    path: file.into(),
                    id: review.comparison.commits.new,
                    mode: 0o100644,
                },
            })
            .into();
        view.finish_review(
            view.review_revision,
            view.current_key(cx).unwrap(),
            BaseSelection::Explicit("refs/heads/main".into()),
            Ok(review),
            cx,
        );
        assert!(view.patch.is_loading());
        let key = view.current_key(cx).unwrap();
        let file = path(view.patch_key.as_ref().unwrap()).clone();
        view.finish_patch(
            view.patch_revision,
            &key,
            &file,
            Err("read failed".into()),
            cx,
        );
        assert_eq!(view.patch.error(), Some("read failed"));
        assert!(!view.patch.is_fetching());

        view.patch_key = None;
        view.load_patch(cx);
        assert!(view.patch.is_loading());
        assert!(view.patch.error().is_none());
        let revision = view.patch_revision;
        view.finish_patch(
            revision,
            &key,
            &file,
            Ok(LoadedPatch {
                content: PreparedPatch::Unavailable(PreviewUnavailable::Binary),
                status: "First file".into(),
            }),
            cx,
        );
        assert_eq!(view.patch.data().unwrap().status, "First file");
        assert!(!view.patch.is_fetching());

        let cancellation = view.patch_task.as_ref().unwrap().cancellation.clone();
        view.select_file("second.rs".into(), cx);
        view.load_patch(cx);
        assert!(cancellation.check().is_err());
        assert!(view.patch.is_loading());
        assert!(view.patch.data().is_none());
        assert!(view.selection.is_none());
        view.finish_patch(revision, &key, &file, Err("obsolete first file".into()), cx);
        assert!(view.patch.is_loading());
        assert!(view.patch.error().is_none());

        view.finish_patch(
            view.patch_revision,
            &key,
            &"second.rs".into(),
            Ok(LoadedPatch {
                content: PreparedPatch::Unavailable(PreviewUnavailable::Binary),
                status: "Second file".into(),
            }),
            cx,
        );
        assert_eq!(view.patch.data().unwrap().status, "Second file");
        assert!(!view.patch.is_fetching());
    });
}
