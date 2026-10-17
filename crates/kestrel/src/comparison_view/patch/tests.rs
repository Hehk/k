use super::*;
use kestrel::{review::FileEntry, structural_diff::SourceLine};

mod structural;

struct Fixture {
    dir: tempfile::TempDir,
    repo: git2::Repository,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        Self { dir, repo }
    }

    fn change(&self, name: &str, old: &str, new: &str) -> FileChange {
        let entry = |source: &str| FileEntry {
            path: name.into(),
            id: self
                .repo
                .blob(source.as_bytes())
                .unwrap()
                .to_string()
                .parse()
                .unwrap(),
            mode: 0o100644,
        };
        FileChange::Modified {
            old: entry(old),
            new: entry(new),
        }
    }

    fn load(&self, change: &FileChange, engine: &Engine) -> LoadedPatch {
        LoadedPatch::load(
            &Repository::open(self.dir.path()).unwrap(),
            change,
            engine,
            &Cancellation::default(),
        )
        .unwrap()
    }
}

fn highlights(patch: &LoadedPatch) -> Vec<(String, DiffTone)> {
    let PreparedPatch::Rows { rows, .. } = &patch.content else {
        panic!()
    };
    rows.iter()
        .flat_map(|row| {
            row.highlights
                .iter()
                .map(|(range, tone)| (row.text[range.clone()].to_owned(), *tone))
        })
        .collect()
}

#[test]
fn structural_and_fallback_diffs_use_the_same_blobs_and_line_colors() {
    let fixture = Fixture::new();
    let change = fixture.change(
        "review.rs",
        "fn review(enabled: bool) {\n    show();\n}\n",
        "fn review(enabled: bool) {\n    if enabled {\n        show();\n    }\n}\n",
    );
    std::fs::write(
        fixture.dir.path().join("review.rs"),
        "dirty unrelated checkout",
    )
    .unwrap();
    std::fs::write(fixture.dir.path().join(".gitattributes"), "*.rs -diff\n").unwrap();
    let snapshot = || {
        ["review.rs", ".gitattributes", ".git/HEAD", ".git/config"]
            .map(|path| std::fs::read(fixture.dir.path().join(path)).unwrap())
    };
    let before = snapshot();
    let engine = Engine::bundled();
    let structural = fixture.load(&change, &engine);
    assert_eq!(structural.status, "Difftastic · Rust · Structural");
    assert_eq!(
        highlights(&structural),
        vec![
            ("if".into(), DiffTone::Added),
            ("enabled".into(), DiffTone::Added),
            ("{".into(), DiffTone::Added),
            ("}".into(), DiffTone::Added)
        ]
    );
    let PreparedPatch::Rows { rows, .. } = &structural.content else {
        panic!()
    };
    let show = rows
        .iter()
        .filter(|row| row.text.contains("show()"))
        .collect::<Vec<_>>();
    assert_eq!(
        show.len(),
        1,
        "indentation-only changes should not duplicate the code"
    );
    assert_eq!((show[0].old.as_ref(), show[0].new.as_ref()), ("2", "3"));
    assert!(show[0].highlights.is_empty());
    assert_eq!(show[0].tone, DiffTone::Context);
    for row in rows.iter().filter(|row| !row.highlights.is_empty()) {
        assert_eq!(row.tone, DiffTone::Added);
        assert!(row.highlights.iter().all(|(_, tone)| *tone == row.tone));
    }
    let text = fixture.load(&change, &Engine::new("/missing/difft".into()));
    assert!(text.status.starts_with("Text fallback"));
    let PreparedPatch::Rows { rows, .. } = text.content else {
        panic!()
    };
    assert_eq!(
        rows.iter()
            .filter(|row| row.text.contains("show()"))
            .count(),
        2
    );
    assert!(rows.iter().any(|row| row.tone == DiffTone::Deleted));
    assert_eq!(snapshot(), before);
    assert!(!fixture.dir.path().join(".git/index").exists());
}

#[test]
fn formatting_and_final_newline_changes_keep_structural_results_and_metadata() {
    let fixture = Fixture::new();
    for (old, new) in [
        ("fn main() { foo(1); }\n", "fn main() {\n    foo(1);\n}\n"),
        ("fn main() {}", "fn main() {}\n"),
    ] {
        let change = fixture.change("test.rs", old, new);
        let loaded = fixture.load(&change, &Engine::bundled());
        assert!(loaded.status.starts_with("Difftastic"));
        let PreparedPatch::Rows { rows, .. } = loaded.content else {
            panic!()
        };
        assert_eq!(rows[0].text.as_ref(), "No structural changes.");
        assert!(
            rows.iter()
                .all(|row| !row.text.contains("Text mode") && !row.text.contains("use Text"))
        );
        if !old.ends_with('\n') {
            assert!(
                rows.iter()
                    .any(|row| row.text.contains("Final newline changed"))
            );
        }
    }
}

#[test]
fn failures_fall_back_explicitly_and_unavailable_content_does_not_launch_the_helper() {
    let fixture = Fixture::new();
    let missing = Engine::new("/missing/difft".into());
    let change = fixture.change("test.rs", "fn main() {}\n", "fn main() { foo(); }\n");
    let loaded = fixture.load(&change, &missing);
    assert!(
        loaded
            .status
            .starts_with("Text fallback · Difftastic is unavailable")
    );
    assert!(matches!(loaded.content, PreparedPatch::Rows { .. }));
    for (name, old, new, reason) in [
        (
            "unknown.xyz-kestrel",
            "old\n",
            "new\n",
            "No structural parser",
        ),
        (
            "bad.rs",
            "fn main() {}\n",
            "fn main() { @@@ }\n",
            "parse error",
        ),
    ] {
        let loaded = fixture.load(&fixture.change(name, old, new), &Engine::bundled());
        assert!(loaded.status.contains(reason), "{}", loaded.status);
        assert!(loaded.status.starts_with("Text fallback"));
    }
    for (source, reason) in [
        ("binary\0".to_owned(), PreviewUnavailable::Binary),
        ("x".repeat(1024 * 1024 + 1), PreviewUnavailable::TooLarge),
    ] {
        let loaded = fixture.load(&fixture.change("test.rs", "", &source), &missing);
        assert_eq!(loaded.status, "Preview unavailable");
        assert!(matches!(loaded.content, PreparedPatch::Unavailable(actual) if actual == reason));
    }
}

#[test]
fn one_sided_files_and_edited_renames_keep_source_side_identity() {
    let fixture = Fixture::new();
    let FileChange::Modified { old, new } =
        fixture.change("new.rs", "fn main() {}\n", "fn main() { foo(); }\n")
    else {
        panic!()
    };
    let changes = [
        FileChange::Added { new: new.clone() },
        FileChange::Deleted { old: old.clone() },
        FileChange::Renamed {
            old: FileEntry {
                path: "old.rs".into(),
                ..old
            },
            new,
        },
    ];
    for change in changes {
        let loaded = fixture.load(&change, &Engine::bundled());
        assert!(loaded.status.starts_with("Difftastic"), "{}", loaded.status);
        let PreparedPatch::Rows { rows, .. } = &loaded.content else {
            panic!()
        };
        for row in rows.iter().filter(|row| !row.highlights.is_empty()) {
            assert!(row.highlights.iter().all(|(_, tone)| *tone == row.tone));
        }
        let spans = highlights(&loaded);
        assert!(!spans.is_empty());
        if matches!(change, FileChange::Deleted { .. }) {
            assert!(spans.iter().all(|(_, tone)| *tone == DiffTone::Deleted));
        } else {
            assert!(spans.iter().all(|(_, tone)| *tone == DiffTone::Added));
        }
    }
}

#[test]
#[allow(clippy::single_range_in_vec_init)]
fn structural_rows_are_bounded_even_when_alignment_expands_to_two_display_rows() {
    let diff = structural_diff::Diff {
        language: "Rust".into(),
        has_changes: true,
        lines: vec![structural_diff::AlignedLine {
            old: Some(SourceLine {
                number: 1,
                text: "old\n".into(),
                changes: vec![0..3],
            }),
            new: Some(SourceLine {
                number: 1,
                text: "new\n".into(),
                changes: vec![0..3],
            }),
        }],
    };
    assert!(matches!(
        PreparedPatch::structural(
            diff,
            "old\n",
            "new\n",
            PreviewLimits {
                patch_lines: 2,
                ..Default::default()
            }
        ),
        Err(structural_diff::Error::Limit)
    ));
}
