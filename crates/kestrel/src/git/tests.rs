use std::{fs, process::Command};

use git2::{Index, IndexEntry, IndexTime, Oid, RepositoryInitOptions, Signature};
use tempfile::TempDir;

use super::*;

struct Fixture {
    directory: TempDir,
    repo: git2::Repository,
}

impl Fixture {
    fn new() -> Self {
        let directory = TempDir::new().unwrap();
        let repo = git2::Repository::init_opts(
            directory.path(),
            RepositoryInitOptions::new()
                .initial_head("main")
                .external_template(false),
        )
        .unwrap();
        Self { directory, repo }
    }

    fn reader(&self) -> Repository {
        Repository::open(self.directory.path()).unwrap()
    }

    fn commit(&self, reference: &str, parents: &[Oid], files: &[(&str, &str)]) -> Oid {
        let entries: Vec<_> = files
            .iter()
            .map(|(path, text)| {
                (
                    path.as_bytes(),
                    self.repo.blob(text.as_bytes()).unwrap(),
                    0o100644,
                )
            })
            .collect();
        self.commit_entries(reference, parents, &entries)
    }

    fn commit_entries(
        &self,
        reference: &str,
        parents: &[Oid],
        entries: &[(&[u8], Oid, u32)],
    ) -> Oid {
        let mut index = Index::new().unwrap();
        for (path, id, mode) in entries {
            index
                .add(&IndexEntry {
                    ctime: IndexTime::new(0, 0),
                    mtime: IndexTime::new(0, 0),
                    dev: 0,
                    ino: 0,
                    mode: *mode,
                    uid: 0,
                    gid: 0,
                    file_size: 0,
                    id: *id,
                    flags: 0,
                    flags_extended: 0,
                    path: path.to_vec(),
                })
                .unwrap();
        }
        let tree = self
            .repo
            .find_tree(index.write_tree_to(&self.repo).unwrap())
            .unwrap();
        let parents: Vec<_> = parents
            .iter()
            .map(|id| self.repo.find_commit(*id).unwrap())
            .collect();
        let signature = Signature::now("Kestrel tests", "tests@example.invalid").unwrap();
        let id = self
            .repo
            .commit(
                None,
                &signature,
                &signature,
                reference,
                &tree,
                &parents.iter().collect::<Vec<_>>(),
            )
            .unwrap();
        self.repo.reference(reference, id, true, "fixture").unwrap();
        id
    }
}

fn branch(name: &str) -> RefName {
    RefName::from(name)
}

#[test]
fn comparison_excludes_base_only_changes_and_matches_git() {
    let fixture = Fixture::new();
    let root = fixture.commit("refs/heads/main", &[], &[("shared", "before\n")]);
    fixture.commit(
        "refs/heads/main",
        &[root],
        &[("shared", "before\n"), ("base-only", "main\n")],
    );
    let head = fixture.commit("refs/heads/topic", &[root], &[("shared", "after\n")]);
    let repo = fixture.reader();
    let comparison = repo
        .compare(&branch("refs/heads/topic"), &branch("refs/heads/main"))
        .unwrap();
    assert_eq!(
        comparison.commits,
        Endpoints {
            old: ObjectId(root),
            new: ObjectId(head)
        }
    );
    let changes = repo.changes(&comparison.commits).unwrap();
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].new_file().unwrap().path, GitPath::from("shared"));
    let FilePatch::Hunks(hunks) = repo.patch(&changes[0], PreviewLimits::default()).unwrap() else {
        panic!()
    };
    assert_eq!(
        hunks[0].lines,
        [
            DiffLine::Deleted {
                old_line: 1,
                text: "before\n".into()
            },
            DiffLine::Added {
                new_line: 1,
                text: "after\n".into()
            },
        ]
    );

    let output = Command::new("git")
        .arg("-C")
        .arg(fixture.directory.path())
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .args([
            "merge-base",
            &comparison.base_tip.to_string(),
            &head.to_string(),
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap().trim(),
        root.to_string()
    );
    let output = Command::new("git")
        .arg("-C")
        .arg(fixture.directory.path())
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_COMMON_DIR")
        .args([
            "diff",
            "--no-ext-diff",
            "--no-textconv",
            "--no-color",
            "--no-renames",
            "--diff-algorithm=myers",
            "--unified=3",
            &root.to_string(),
            &head.to_string(),
            "--",
            "shared",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let patch = String::from_utf8(output.stdout).unwrap();
    let expected: String = hunks
        .iter()
        .map(|hunk| {
            let lines: String = hunk
                .lines
                .iter()
                .map(|line| match line {
                    DiffLine::Context { text, .. } => format!(" {text}"),
                    DiffLine::Added { text, .. } => format!("+{text}"),
                    DiffLine::Deleted { text, .. } => format!("-{text}"),
                    DiffLine::NoNewline(_) => "\\ No newline at end of file\n".into(),
                })
                .collect();
            format!("{}{lines}", hunk.header)
        })
        .collect();
    assert_eq!(&patch[patch.find("@@").unwrap()..], expected);
}

#[test]
fn branch_and_base_changes_do_not_mutate_pinned_comparisons() {
    let fixture = Fixture::new();
    let root = fixture.commit("refs/heads/main", &[], &[]);
    let head = fixture.commit("refs/heads/topic", &[root], &[("feature", "old\n")]);
    let repo = fixture.reader();
    let pinned = repo
        .compare(&branch("refs/heads/topic"), &branch("refs/heads/main"))
        .unwrap();
    let saved = pinned.clone();
    let new_head = fixture.commit("refs/heads/topic", &[head], &[("feature", "new\n")]);
    let new_base = fixture.commit("refs/heads/main", &[root], &[("other", "base\n")]);
    assert!(!repo.freshness(&pinned).unwrap().matches(&pinned));
    let _ = repo.snapshot().unwrap();
    assert_eq!(pinned, saved);
    let entry = repo
        .entry(pinned.commits.new, &GitPath::from("feature"))
        .unwrap();
    assert_eq!(
        repo.content(&entry, PreviewLimits::default()).unwrap(),
        FileContent::Text("old\n".into())
    );

    fixture
        .repo
        .reference("refs/heads/alternate", new_base, true, "fixture")
        .unwrap();
    let alternate = repo
        .change_base(&pinned, &branch("refs/heads/alternate"))
        .unwrap();
    assert_eq!(alternate.commits.new, ObjectId(head));
    assert_eq!(alternate.base_tip, ObjectId(new_base));
    let latest = repo.load_latest(&pinned).unwrap();
    assert_eq!(latest.commits.new, ObjectId(new_head));
    assert_eq!(latest.base_tip, ObjectId(new_base));
    assert_eq!(latest.base, pinned.base);

    fixture
        .repo
        .find_reference("refs/heads/topic")
        .unwrap()
        .delete()
        .unwrap();
    assert_eq!(repo.freshness(&pinned).unwrap().head, RefState::Missing);
    assert!(repo.changes(&pinned.commits).is_ok());
    assert!(matches!(
        repo.load_latest(&pinned),
        Err(Error::MissingRef(_))
    ));
}

#[test]
fn identical_refs_and_heads_behind_the_base_are_empty() {
    let fixture = Fixture::new();
    let root = fixture.commit("refs/heads/topic", &[], &[("file", "old")]);
    fixture.commit("refs/heads/main", &[root], &[("file", "new")]);
    let repo = fixture.reader();
    for base in ["refs/heads/topic", "refs/heads/main"] {
        let comparison = repo
            .compare(&branch("refs/heads/topic"), &branch(base))
            .unwrap();
        assert_eq!(comparison.commits.old, comparison.commits.new);
        assert!(repo.changes(&comparison.commits).unwrap().is_empty());
    }
}

#[test]
fn unrelated_and_criss_cross_histories_are_explicit_errors() {
    let fixture = Fixture::new();
    let root = fixture.commit("refs/heads/root", &[], &[]);
    let a = fixture.commit("refs/heads/a", &[root], &[("a", "a")]);
    let b = fixture.commit("refs/heads/b", &[root], &[("b", "b")]);
    fixture.commit("refs/heads/left", &[a, b], &[]);
    fixture.commit("refs/heads/right", &[b, a], &[]);
    fixture.commit("refs/heads/unrelated", &[], &[("other", "other")]);
    let repo = fixture.reader();
    assert!(matches!(
        repo.compare(&branch("refs/heads/a"), &branch("refs/heads/unrelated")),
        Err(Error::NoMergeBase)
    ));
    assert!(matches!(
        repo.compare(&branch("refs/heads/left"), &branch("refs/heads/right")),
        Err(Error::MultipleMergeBases(2))
    ));
}

#[test]
fn local_and_remote_branches_are_distinct_and_defaults_require_unambiguous_metadata() {
    let fixture = Fixture::new();
    let root = fixture.commit("refs/heads/main", &[], &[]);
    fixture
        .repo
        .reference("refs/remotes/origin/main", root, true, "fixture")
        .unwrap();
    let repo = fixture.reader();
    assert_eq!(repo.snapshot().unwrap().default_base, None);
    fixture
        .repo
        .reference_symbolic(
            "refs/remotes/origin/HEAD",
            "refs/remotes/origin/main",
            true,
            "fixture",
        )
        .unwrap();
    let snapshot = repo.snapshot().unwrap();
    assert_eq!(
        snapshot.default_base,
        Some(branch("refs/remotes/origin/main"))
    );
    assert_eq!(snapshot.branches.len(), 2);
    assert!(!snapshot.branches[0].remote);
    assert!(snapshot.branches[1].remote);
    fixture
        .repo
        .reference("refs/remotes/upstream/main", root, true, "fixture")
        .unwrap();
    fixture
        .repo
        .reference_symbolic(
            "refs/remotes/upstream/HEAD",
            "refs/remotes/upstream/main",
            true,
            "fixture",
        )
        .unwrap();
    assert_eq!(repo.snapshot().unwrap().default_base, None);
    assert!(matches!(
        repo.resolve(&branch("main")),
        Err(Error::InvalidRef)
    ));
}

#[test]
fn empty_detached_and_linked_worktree_repositories_are_discovered() {
    let fixture = Fixture::new();
    assert!(fixture.reader().snapshot().unwrap().branches.is_empty());
    assert_eq!(fixture.reader().info().unwrap().current_branch, None);
    let root = fixture.commit("refs/heads/main", &[], &[]);
    assert_eq!(
        fixture.reader().info().unwrap().current_branch,
        Some(branch("refs/heads/main"))
    );
    fixture.repo.set_head_detached(root).unwrap();
    assert_eq!(fixture.reader().info().unwrap().current_branch, None);
    fixture.repo.set_head("refs/heads/main").unwrap();
    let parent = TempDir::new().unwrap();
    let path = parent.path().join("linked");
    fixture.repo.worktree("linked", &path, None).unwrap();
    let linked = Repository::open(&path).unwrap();
    assert_eq!(
        linked.info().unwrap().common_dir,
        fixture.reader().info().unwrap().common_dir
    );
    assert_ne!(
        linked.info().unwrap().directory,
        fixture.reader().info().unwrap().directory
    );
    assert_eq!(
        linked.info().unwrap().current_branch,
        Some(branch("refs/heads/linked"))
    );
}

#[test]
fn tree_changes_keep_rename_paths_modes_and_deleted_content() {
    let fixture = Fixture::new();
    let root = fixture.commit(
        "refs/heads/main",
        &[],
        &[("old", "same\n"), ("deleted", "gone\n"), ("mode", "mode\n")],
    );
    let text = fixture.repo.blob(b"same\n").unwrap();
    let mode = fixture.repo.blob(b"mode\n").unwrap();
    fixture.commit_entries(
        "refs/heads/topic",
        &[root],
        &[(b"new", text, 0o100644), (b"mode", mode, 0o100755)],
    );
    let repo = fixture.reader();
    let comparison = repo
        .compare(&branch("refs/heads/topic"), &branch("refs/heads/main"))
        .unwrap();
    let changes = repo.changes(&comparison.commits).unwrap();
    let FileChange::Renamed { old, new } = changes
        .iter()
        .find(|change| matches!(change, FileChange::Renamed { .. }))
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(old.path, GitPath::from("old"));
    assert_eq!(new.path, GitPath::from("new"));
    let FileChange::Deleted { old } = changes
        .iter()
        .find(|change| matches!(change, FileChange::Deleted { .. }))
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(
        repo.content(old, PreviewLimits::default()).unwrap(),
        FileContent::Text("gone\n".into())
    );
    let mode = changes
        .iter()
        .find(|change| {
            change
                .new_file()
                .is_some_and(|entry| entry.path == GitPath::from("mode"))
        })
        .unwrap();
    let FileChange::Modified { old, new } = mode else {
        panic!()
    };
    assert_eq!(old.mode, 0o100644);
    assert_eq!(new.mode, 0o100755);
}

#[test]
fn previews_are_bounded_and_unsupported_content_is_not_an_empty_diff() {
    let fixture = Fixture::new();
    let root = fixture.commit("refs/heads/main", &[], &[]);
    let binary = fixture.repo.blob(b"binary\0content").unwrap();
    let encoding = fixture.repo.blob(b"bad\xfftext").unwrap();
    let text = fixture.repo.blob(b"1\n2\n3\n4\n").unwrap();
    fixture.commit_entries(
        "refs/heads/topic",
        &[root],
        &[
            (b"binary", binary, 0o100644),
            (b"encoding", encoding, 0o100644),
            (b"text", text, 0o100644),
            (b"module", root, 0o160000),
        ],
    );
    let repo = fixture.reader();
    let comparison = repo
        .compare(&branch("refs/heads/topic"), &branch("refs/heads/main"))
        .unwrap();
    let changes = repo.changes(&comparison.commits).unwrap();
    let change = |name: &str| {
        changes
            .iter()
            .find(|change| change.new_file().unwrap().path == GitPath::from(name))
            .unwrap()
    };
    for (name, reason) in [
        ("binary", PreviewUnavailable::Binary),
        ("encoding", PreviewUnavailable::Encoding),
        ("module", PreviewUnavailable::Submodule),
    ] {
        assert_eq!(
            repo.patch(change(name), PreviewLimits::default()).unwrap(),
            FilePatch::Unavailable(reason)
        );
    }
    assert_eq!(
        repo.patch(
            change("text"),
            PreviewLimits {
                blob_bytes: 2,
                ..PreviewLimits::default()
            }
        )
        .unwrap(),
        FilePatch::Unavailable(PreviewUnavailable::TooLarge)
    );
    assert_eq!(
        repo.patch(
            change("text"),
            PreviewLimits {
                patch_lines: 2,
                ..PreviewLimits::default()
            }
        )
        .unwrap(),
        FilePatch::Unavailable(PreviewUnavailable::TooLarge)
    );
}

#[test]
fn browsing_uses_raw_tree_paths_and_never_follows_symlinks() {
    let fixture = Fixture::new();
    let text = fixture.repo.blob(b"unchanged\n").unwrap();
    let symlink = fixture.repo.blob(b"/outside/the/repository").unwrap();
    let path = b"dir/spaces tabs\tnewlines\n\xff";
    let head = fixture.commit_entries(
        "refs/heads/main",
        &[],
        &[(path, text, 0o100644), (b"link", symlink, 0o120000)],
    );
    let repo = fixture.reader();
    let entries = repo
        .list_tree(ObjectId(head), Some(&GitPath::from("dir")))
        .unwrap();
    assert_eq!(entries[0].path.0, path);
    let entry = repo.entry(ObjectId(head), &GitPath(path.to_vec())).unwrap();
    assert_eq!(entry, entries[0]);
    assert_eq!(
        repo.content(&entry, PreviewLimits::default()).unwrap(),
        FileContent::Text("unchanged\n".into())
    );
    let link = repo.entry(ObjectId(head), &GitPath::from("link")).unwrap();
    assert_eq!(
        repo.content(&link, PreviewLimits::default()).unwrap(),
        FileContent::Text("/outside/the/repository".into())
    );
    assert!(matches!(
        repo.entry(ObjectId(head), &GitPath::from("../secret")),
        Err(Error::InvalidPath)
    ));
}

#[test]
fn inspection_leaves_dirty_checkout_index_and_refs_untouched() {
    let fixture = Fixture::new();
    let root = fixture.commit("refs/heads/main", &[], &[("file", "original\n")]);
    fixture
        .repo
        .checkout_head(Some(git2::build::CheckoutBuilder::new().force()))
        .unwrap();
    fixture.commit("refs/heads/topic", &[root], &[("file", "reviewed\n")]);
    fs::write(fixture.directory.path().join("file"), "staged\n").unwrap();
    let mut index = fixture.repo.index().unwrap();
    index.add_path(Path::new("file")).unwrap();
    index.write().unwrap();
    fs::write(fixture.directory.path().join("file"), "unstaged\n").unwrap();
    fs::write(fixture.directory.path().join("untracked"), "untouched\n").unwrap();
    fs::write(fixture.directory.path().join(".gitattributes"), "* -diff\n").unwrap();
    let index_before = fs::read(fixture.repo.path().join("index")).unwrap();
    let head_before = fs::read(fixture.repo.path().join("HEAD")).unwrap();
    let refs = || {
        fixture
            .repo
            .references()
            .unwrap()
            .map(|reference| {
                let reference = reference.unwrap();
                (reference.name_bytes().to_vec(), reference.target())
            })
            .collect::<Vec<_>>()
    };
    let refs_before = refs();
    let repo = fixture.reader();
    let comparison = repo
        .compare(&branch("refs/heads/topic"), &branch("refs/heads/main"))
        .unwrap();
    let changes = repo.changes(&comparison.commits).unwrap();
    let entry = changes[0].new_file().unwrap();
    assert_eq!(
        repo.content(entry, PreviewLimits::default()).unwrap(),
        FileContent::Text("reviewed\n".into())
    );
    assert!(matches!(
        repo.patch(&changes[0], PreviewLimits::default()).unwrap(),
        FilePatch::Hunks(_)
    ));
    assert_eq!(
        fs::read(fixture.repo.path().join("index")).unwrap(),
        index_before
    );
    assert_eq!(
        fs::read(fixture.repo.path().join("HEAD")).unwrap(),
        head_before
    );
    assert_eq!(refs(), refs_before);
    assert_eq!(
        fs::read_to_string(fixture.directory.path().join("file")).unwrap(),
        "unstaged\n"
    );
    assert_eq!(
        fs::read_to_string(fixture.directory.path().join("untracked")).unwrap(),
        "untouched\n"
    );
}

#[test]
fn edited_renames_and_type_changes_keep_structured_deltas() {
    let fixture = Fixture::new();
    let before = "one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\n";
    let root = fixture.commit(
        "refs/heads/main",
        &[],
        &[("old", before), ("file", "plain\n")],
    );
    let edited = fixture
        .repo
        .blob(b"one\ntwo\nthree\nfour\nfive\nsix\nseven\nchanged\n")
        .unwrap();
    let link = fixture.repo.blob(b"elsewhere").unwrap();
    fixture.commit_entries(
        "refs/heads/topic",
        &[root],
        &[(b"new", edited, 0o100644), (b"file", link, 0o120000)],
    );
    let repo = fixture.reader();
    let comparison = repo
        .compare(&branch("refs/heads/topic"), &branch("refs/heads/main"))
        .unwrap();
    let changes = repo.changes(&comparison.commits).unwrap();
    let rename = changes
        .iter()
        .find(|change| matches!(change, FileChange::Renamed { .. }))
        .unwrap();
    let FilePatch::Hunks(hunks) = repo.patch(rename, PreviewLimits::default()).unwrap() else {
        panic!()
    };
    assert!(
        hunks
            .iter()
            .flat_map(|hunk| &hunk.lines)
            .any(|line| matches!(line, DiffLine::Added { text, .. } if text == "changed\n"))
    );
    assert!(
        changes
            .iter()
            .any(|change| matches!(change, FileChange::TypeChanged { .. }))
    );
}

#[test]
fn raw_paths_survive_diff_selection_and_no_newline_markers() {
    let fixture = Fixture::new();
    let root = fixture.commit("refs/heads/main", &[], &[]);
    let blob = fixture.repo.blob(b"no final newline").unwrap();
    let raw = b"spaces\ttabs\nnon-utf8-\xff";
    fixture.commit_entries("refs/heads/topic", &[root], &[(raw, blob, 0o100644)]);
    let repo = fixture.reader();
    let comparison = repo
        .compare(&branch("refs/heads/topic"), &branch("refs/heads/main"))
        .unwrap();
    let changes = repo.changes(&comparison.commits).unwrap();
    assert_eq!(changes[0].new_file().unwrap().path.0, raw);
    let FilePatch::Hunks(hunks) = repo.patch(&changes[0], PreviewLimits::default()).unwrap() else {
        panic!()
    };
    assert_eq!(
        hunks[0].lines[0],
        DiffLine::Added {
            new_line: 1,
            text: "no final newline".into(),
        }
    );
    assert!(hunks[0].lines.contains(&DiffLine::NoNewline(DiffSide::New)));
}

#[test]
fn context_and_missing_newline_lines_have_explicit_sides() {
    for (old, new, side) in [
        ("same\nold", "same\nnew\n", DiffSide::Old),
        ("same\nold\n", "same\nnew", DiffSide::New),
        ("old\nlast", "new\nlast", DiffSide::Both),
    ] {
        let fixture = Fixture::new();
        let root = fixture.commit("refs/heads/main", &[], &[("file", old)]);
        fixture.commit("refs/heads/topic", &[root], &[("file", new)]);
        let repo = fixture.reader();
        let comparison = repo
            .compare(&branch("refs/heads/topic"), &branch("refs/heads/main"))
            .unwrap();
        let changes = repo.changes(&comparison.commits).unwrap();
        let FilePatch::Hunks(hunks) = repo.patch(&changes[0], PreviewLimits::default()).unwrap()
        else {
            panic!()
        };
        assert!(hunks[0].lines.contains(&DiffLine::NoNewline(side)));
        let (line, text) = if side == DiffSide::Both {
            (2, "last")
        } else {
            (1, "same\n")
        };
        assert!(hunks[0].lines.contains(&DiffLine::Context {
            old_line: line,
            new_line: line,
            text: text.into(),
        }));
    }
}

#[test]
fn missing_saved_objects_never_fall_back_to_current_refs() {
    let fixture = Fixture::new();
    let root = fixture.commit("refs/heads/main", &[], &[]);
    fixture.commit("refs/heads/topic", &[root], &[("file", "text\n")]);
    let repo = fixture.reader();
    let mut comparison = repo
        .compare(&branch("refs/heads/topic"), &branch("refs/heads/main"))
        .unwrap();
    comparison.commits.new = "1111111111111111111111111111111111111111".parse().unwrap();
    assert!(repo.changes(&comparison.commits).is_err());
    assert!(repo.list_tree(comparison.commits.new, None).is_err());
    assert!(
        repo.change_base(&comparison, &branch("refs/heads/main"))
            .is_err()
    );
}

#[test]
fn shallow_history_and_unsupported_repository_formats_report_errors() {
    let fixture = Fixture::new();
    let root = fixture.commit("refs/heads/main", &[], &[]);
    let head = fixture.commit("refs/heads/topic", &[root], &[("file", "text\n")]);
    fs::write(fixture.repo.path().join("shallow"), format!("{head}\n")).unwrap();
    assert!(matches!(
        fixture
            .reader()
            .compare(&branch("refs/heads/topic"), &branch("refs/heads/main")),
        Err(Error::NoMergeBase)
    ));
    fixture
        .repo
        .config()
        .unwrap()
        .set_i32("core.repositoryformatversion", 999)
        .unwrap();
    assert!(Repository::open(fixture.directory.path()).is_err());
}

#[test]
fn inspection_without_git_executable() {
    const CHILD: &str = "KESTREL_TEST_NO_GIT";
    if std::env::var_os(CHILD).is_none() {
        let output = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "git::tests::inspection_without_git_executable"])
            .env(CHILD, "1")
            .env("PATH", "")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let fixture = Fixture::new();
    let root = fixture.commit("refs/heads/main", &[], &[]);
    fixture.commit("refs/heads/topic", &[root], &[("file", "text\n")]);
    let repo = fixture.reader();
    let comparison = repo
        .compare(&branch("refs/heads/topic"), &branch("refs/heads/main"))
        .unwrap();
    let changes = repo.changes(&comparison.commits).unwrap();
    assert!(matches!(
        repo.patch(&changes[0], PreviewLimits::default()).unwrap(),
        FilePatch::Hunks(_)
    ));
}
