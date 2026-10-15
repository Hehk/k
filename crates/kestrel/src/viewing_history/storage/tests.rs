use super::*;
use crate::{
    git::RepositoryInfo,
    review::{Endpoints, GitPath},
    state::{BranchList, BranchSort, Browsing, ReadingMode},
    viewing_history::Foreground,
};

fn history(directory: &Path) -> (AppState, ViewingHistory) {
    let mut state = AppState::default();
    let (_, changes) = state
        .collect_view_changes(Foreground::SelectedWindow, |state| {
            let project = state.register_project(&RepositoryInfo {
                directory: directory.to_owned(),
                common_dir: directory.join(".git"),
                current_branch: None,
            })?;
            let window = state.new_window()?;
            let tab = state.new_tab(window)?;
            state.open_project(project, tab)?;
            state.set_branch_list(
                tab,
                BranchList {
                    query: "topic".into(),
                    sort: BranchSort::Name,
                    first_visible_row: 12,
                },
            )?;
            let branch = RefName(b"refs/heads/topic-\xff".to_vec());
            state.select_branch(tab, branch.clone())?;
            state.set_comparison(
                tab,
                BaseSelection::Explicit("refs/remotes/origin/main".into()),
                Comparison {
                    branch: branch.clone(),
                    base: "refs/remotes/origin/main".into(),
                    base_tip: "3333333333333333333333333333333333333333".parse().unwrap(),
                    commits: Endpoints {
                        old: "1111111111111111111111111111111111111111".parse().unwrap(),
                        new: "2222222222222222222222222222222222222222".parse().unwrap(),
                    },
                },
            )?;
            state.set_browsing(
                tab,
                &branch,
                Browsing {
                    file: Some(GitPath(b"src/file-\xff.rs".to_vec())),
                    directory: Some("src".into()),
                    mode: ReadingMode::HeadFile,
                    first_visible_line: 73,
                },
            )
        })
        .unwrap();
    let mut history = ViewingHistory::default();
    history.apply(changes, || 1_700_000_000_123);
    (state, history)
}

#[test]
fn round_trip_pins_reading_state_raw_git_bytes_and_saved_labels() {
    let dir = tempfile::tempdir().unwrap();
    let (state, original) = history(&dir.path().join("missing-repository"));
    let bytes = encode(original.records()).unwrap();
    let mut restored_state = AppState::default();
    let restored = decode(&bytes, &mut restored_state).unwrap();
    assert_eq!(restored.records(), original.records());
    assert!(restored.bindings.is_empty());
    assert!(restored_state.windows().is_empty());
    assert_eq!(
        restored_state.projects()[0].directory,
        state.projects()[0].directory
    );
    assert_eq!(
        restored.records()[1]
            .viewed
            .snapshot
            .view
            .browsing
            .first_visible_line,
        73
    );
    assert_eq!(restored.recent(10).count(), 2);
}

#[test]
fn unresolved_comparisons_and_all_reading_modes_survive_without_inventing_pins() {
    let dir = tempfile::tempdir().unwrap();
    let (_, history) = history(&dir.path().join("repo"));
    for mode in [
        ReadingMode::Diff,
        ReadingMode::HeadFile,
        ReadingMode::BaseFile,
    ] {
        let mut records = history.records().to_vec();
        let record = &mut records[1];
        record.timestamp_ms = -123;
        record.viewed.snapshot.view.browsing.mode = mode;
        if let Destination::Comparison {
            base, comparison, ..
        } = &mut record.viewed.snapshot.destination
        {
            *base = BaseSelection::Default;
            *comparison = None;
        }
        let restored = decode(&encode(&records).unwrap(), &mut AppState::default()).unwrap();
        assert_eq!(restored.records(), records);
        assert_eq!(restored.recent(1).next().unwrap().id, ViewingId(1));
    }
}

#[cfg(unix)]
#[test]
fn local_repository_paths_are_lossless_too() {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};
    let path = PathBuf::from(OsString::from_vec(b"/repositories/non-utf8-\xff".to_vec()));
    let (_, original) = history(&path);
    let restored = decode(
        &encode(original.records()).unwrap(),
        &mut AppState::default(),
    )
    .unwrap();
    assert_eq!(restored.records(), original.records());
}

#[test]
fn unrelated_or_newer_workspace_ids_cannot_alias_history_repositories() {
    let dir = tempfile::tempdir().unwrap();
    let (_, original) = history(&dir.path().join("historical"));
    let (mut other, _) = history(&dir.path().join("unrelated"));
    let before = other.clone();
    let restored = decode(&encode(original.records()).unwrap(), &mut other).unwrap();
    let id = restored.records()[1].viewed.snapshot.project().unwrap();
    assert_ne!(id, original.records()[1].viewed.snapshot.project().unwrap());
    assert_eq!(
        other.project(id).unwrap().directory,
        dir.path().join("historical")
    );
    assert_eq!(other.windows(), before.windows());
    let repeated = decode(&encode(original.records()).unwrap(), &mut other).unwrap();
    assert_eq!(repeated.records(), restored.records());
    assert_eq!(other.projects().len(), 2);
    assert!(restored.bindings.is_empty());
}

#[test]
fn matching_common_directory_keeps_the_workspaces_current_worktree() {
    let dir = tempfile::tempdir().unwrap();
    let (state, original) = history(&dir.path().join("old-worktree"));
    let mut current = AppState::default();
    let project = current
        .register_project(&RepositoryInfo {
            directory: dir.path().join("current-worktree"),
            common_dir: state.projects()[0].common_dir.clone(),
            current_branch: None,
        })
        .unwrap();
    let restored = decode(&encode(original.records()).unwrap(), &mut current).unwrap();
    assert_eq!(
        restored.records()[0].viewed.snapshot.project(),
        Some(project)
    );
    assert_eq!(
        current.project(project).unwrap().directory,
        dir.path().join("current-worktree")
    );
    assert_eq!(
        restored.records()[0].viewed.repository.directory,
        dir.path().join("old-worktree")
    );
}

#[test]
fn corrupt_unsupported_and_invalid_logs_are_preserved_before_replacement() {
    let dir = tempfile::tempdir().unwrap();
    let (_, history) = history(&dir.path().join("repo"));
    let valid = encode(history.records()).unwrap();
    let mut future = valid.clone();
    future[8..12].copy_from_slice(&99u32.to_le_bytes());
    let mut damaged = valid.clone();
    *damaged.last_mut().unwrap() ^= 1;
    let mut trailing = valid.clone();
    trailing.push(0);
    let mut invalid = history
        .records()
        .iter()
        .map(SavedRecord::capture)
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    invalid[1].view.browsing.file = Some("../escape".into());
    for bytes in [
        b"not history".to_vec(),
        future,
        damaged,
        trailing,
        valid[..valid.len() - 1].to_vec(),
        frame(&invalid).unwrap(),
    ] {
        let path = dir.path().join("history.bin");
        fs::write(&path, &bytes).unwrap();
        let mut state = AppState::default();
        let before = state.clone();
        let mut store = Store::new(path.clone());
        let loaded = store.load(&mut state).unwrap();
        assert_eq!(
            state, before,
            "failed decoding must not partly register projects"
        );
        assert!(loaded.history.records().is_empty());
        let recovery = loaded.recovery.unwrap();
        assert_eq!(fs::read(&recovery.backup).unwrap(), bytes);
        assert_eq!(fs::read(&path).unwrap(), bytes);
        store.save(&[]).unwrap();
        assert_eq!(fs::read(&recovery.backup).unwrap(), bytes);
        assert!(
            Store::new(path)
                .load(&mut state)
                .unwrap()
                .recovery
                .is_none()
        );
    }
}

#[test]
fn missing_files_start_empty_and_unreadable_files_disable_saving() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.bin");
    let mut store = Store::new(path.clone());
    assert!(matches!(store.save(&[]), Err(Error::NotLoaded)));
    assert!(
        store
            .load(&mut AppState::default())
            .unwrap()
            .history
            .records()
            .is_empty()
    );
    assert!(!path.exists());
    fs::create_dir(&path).unwrap();
    assert!(store.load(&mut AppState::default()).is_err());
    assert!(matches!(store.save(&[]), Err(Error::NotLoaded)));
    assert!(path.is_dir());
}

#[test]
fn incomplete_temporary_writes_and_invalid_snapshots_leave_the_last_file_intact() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.bin");
    let (_, history) = history(&dir.path().join("repo"));
    let mut store = Store::new(path.clone());
    store.load(&mut AppState::default()).unwrap();
    store.save(history.records()).unwrap();
    let before = fs::read(&path).unwrap();
    {
        let mut interrupted = tempfile::NamedTempFile::new_in(dir.path()).unwrap();
        interrupted.write_all(&before[..before.len() / 2]).unwrap();
        interrupted.as_file().sync_all().unwrap();
        assert_eq!(fs::read(&path).unwrap(), before);
    }
    let mut invalid = history.records().to_vec();
    invalid[0].id = ViewingId(99);
    assert!(store.save(&invalid).is_err());
    assert_eq!(fs::read(&path).unwrap(), before);
    let previous = dir.path().join("previous.bin");
    fs::rename(&path, &previous).unwrap();
    fs::create_dir(&path).unwrap();
    assert!(store.save(&[]).is_err());
    assert_eq!(fs::read(&previous).unwrap(), before);
    fs::remove_dir(&path).unwrap();
    fs::rename(previous, &path).unwrap();
    assert_eq!(
        Store::new(path)
            .load(&mut AppState::default())
            .unwrap()
            .history
            .records(),
        history.records()
    );
}

#[cfg(unix)]
#[test]
fn v1_fixture_locks_the_wire_schema() {
    let (_, history) = history(Path::new("/repositories/example"));
    let bytes = encode(history.records()).unwrap();
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src/viewing_history/storage/fixtures/v1.bin");
    if std::env::var_os("UPDATE_HISTORY_FIXTURE").is_some() {
        fs::create_dir_all(fixture.parent().unwrap()).unwrap();
        fs::write(fixture, bytes).unwrap();
        return;
    }
    let saved = fs::read(fixture).unwrap();
    assert_eq!(
        bytes, saved,
        "v1 schema changed; review versioning before regenerating the fixture"
    );
    let restored = decode(&saved, &mut AppState::default()).unwrap();
    assert_eq!(restored.records(), history.records());
}

#[test]
#[ignore = "manual storage sizing/timing; run in release mode with --ignored --nocapture"]
fn measure_history_storage() {
    use std::time::Instant;
    let dir = tempfile::tempdir().unwrap();
    let (_, history) = history(&dir.path().join("representative-repository"));
    for (kind, record) in ["branches", "pinned comparison"]
        .into_iter()
        .zip(history.records())
    {
        let mut record = record.clone();
        record.id = ViewingId(0);
        println!(
            "One {kind}: {} bytes including framing",
            encode(&[record]).unwrap().len()
        );
    }
    for count in [1, 1_000, 100_000] {
        let records = (0..count)
            .map(|index| {
                let mut record = history.records()[index % 2].clone();
                record.id = ViewingId(index as u64);
                record.timestamp_ms += index as i64;
                record
            })
            .collect::<Vec<_>>();
        let path = dir.path().join("history.bin");
        let mut store = Store::new(path.clone());
        store.load(&mut AppState::default()).unwrap();
        let start = Instant::now();
        let snapshot = records.clone();
        let clone = start.elapsed();
        let start = Instant::now();
        store.save(&snapshot).unwrap();
        let save = start.elapsed();
        let bytes = fs::metadata(&path).unwrap().len();
        let start = Instant::now();
        let loaded = Store::new(path).load(&mut AppState::default()).unwrap();
        let load = start.elapsed();
        assert_eq!(loaded.history.records(), records);
        println!(
            "{count} records: {bytes} bytes, {:.1} bytes/record, clone {clone:?}, save {save:?}, load {load:?}",
            bytes as f64 / count as f64
        );
    }
}
