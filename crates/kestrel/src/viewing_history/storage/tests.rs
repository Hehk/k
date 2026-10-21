use super::*;
use crate::{
    git::RepositoryInfo,
    review::{Comparison, Endpoints, GitPath, RefName},
    state::{BaseSelection, BranchList, BranchSort, Browsing, Destination, ReadingMode},
    viewing_history::{Foreground, ViewChange, ViewingId},
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
                    query: "topic\nquery".into(),
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

fn journal(records: &[Record]) -> Vec<u8> {
    let mut bytes = HEADER.to_vec();
    for record in records {
        serde_json::to_writer(&mut bytes, record).unwrap();
        bytes.push(b'\n');
    }
    bytes
}

#[cfg(unix)]
fn mode(path: &Path) -> u32 {
    fs::metadata(path).unwrap().permissions().mode() & 0o777
}

#[cfg(unix)]
#[test]
fn new_journals_are_private_even_with_a_permissive_umask() {
    const CHILD: &str = "KESTREL_HISTORY_PERMISSION_TEST";
    if std::env::var_os(CHILD).is_none() {
        // umask is process-wide: change it only in an isolated test process.
        let output = std::process::Command::new("/bin/sh")
            .args(["-c", "umask 000; exec \"$@\"", "history-permission-test"])
            .arg(std::env::current_exe().unwrap())
            .args(["--exact", "viewing_history::storage::tests::new_journals_are_private_even_with_a_permissive_umask", "--nocapture"])
            .env(CHILD, "1")
            .output().unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let (_, history) = history(dir.path());
    let path = dir.path().join("history.jsonl");
    let mut store = Store::new(path.clone());
    store.load(&mut AppState::default()).unwrap();
    store.append(history.records()).unwrap();
    assert_eq!(mode(&path), 0o600);
}

#[cfg(unix)]
#[test]
fn existing_journals_are_made_private_on_load_and_append() {
    let dir = tempfile::tempdir().unwrap();
    let (_, history) = history(dir.path());
    let path = dir.path().join("history.jsonl");
    let original = journal(history.records());
    fs::write(&path, &original).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    let mut store = Store::new(path.clone());
    let loaded = store.load(&mut AppState::default()).unwrap();
    assert_eq!(loaded.history.records(), history.records());
    assert_eq!(mode(&path), 0o600);
    assert_eq!(fs::read(&path).unwrap(), original);

    fs::set_permissions(&path, fs::Permissions::from_mode(0o666)).unwrap();
    store.append([&history.records()[0]]).unwrap();
    assert_eq!(mode(&path), 0o600);
    assert!(fs::read(&path).unwrap().starts_with(&original));

    fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
    store.load(&mut AppState::default()).unwrap();
    assert_eq!(mode(&path), 0o400, "do not expand the owner's permissions");
}

#[test]
fn append_updates_preserve_the_prefix_visit_order_and_lossless_values() {
    let dir = tempfile::tempdir().unwrap();
    let (_, history) = history(&dir.path().join("missing-repository"));
    let path = dir.path().join("history.jsonl");
    let mut store = Store::new(path.clone());
    store.load(&mut AppState::default()).unwrap();
    store.append(history.records()).unwrap();
    let original = fs::read(&path).unwrap();
    assert_eq!(original, journal(history.records()));
    let mut update = history.records()[0].clone();
    update.viewed.snapshot.view.branches.first_visible_row = 99;
    store.append([&update]).unwrap();
    let bytes = fs::read(&path).unwrap();
    assert!(bytes.starts_with(&original));
    assert_eq!(bytes.split(|byte| *byte == b'\n').count(), 5);
    let loaded = Store::new(path).load(&mut AppState::default()).unwrap();
    assert!(loaded.recovery.is_none());
    assert_eq!(
        loaded.history.records(),
        &[update, history.records()[1].clone()]
    );
    assert_eq!(loaded.history.recent(1).next().unwrap().id, ViewingId(1));
    assert!(loaded.history.bindings.is_empty());
    assert!(loaded.history.pending().is_empty());
}

#[test]
fn dirty_updates_coalesce_and_old_acknowledgements_cannot_clear_new_changes() {
    let dir = tempfile::tempdir().unwrap();
    let (_, mut history) = history(dir.path());
    let initial = history.pending();
    assert_eq!(initial.len(), 2);
    history.acknowledge(&initial);
    assert!(history.pending().is_empty());
    let source = history.bindings.values().next().unwrap().source;
    let mut viewed = history.records()[1].viewed.clone();
    viewed.snapshot.view.browsing.first_visible_line = 100;
    history.apply(
        [ViewChange::Update {
            source,
            viewed: Some(viewed.clone()),
        }],
        || 0,
    );
    let in_flight = history.pending();
    viewed.snapshot.view.browsing.first_visible_line = 200;
    history.apply(
        [ViewChange::Update {
            source,
            viewed: Some(viewed.clone()),
        }],
        || 0,
    );
    history.acknowledge(&in_flight);
    let latest = history.pending();
    assert_eq!(latest.len(), 1);
    assert_eq!(latest[0].1.viewed, viewed);
    history.acknowledge(&latest);
    assert!(history.pending().is_empty());
}

#[test]
fn replay_reconciles_ids_and_preserves_the_current_worktree() {
    let dir = tempfile::tempdir().unwrap();
    let (old, history) = history(&dir.path().join("historical"));
    let (mut state, _) = self::history(&dir.path().join("unrelated"));
    let windows = state.windows().to_vec();
    let expected = state
        .register_project(&RepositoryInfo {
            directory: dir.path().join("current-worktree"),
            common_dir: old.projects()[0].common_dir.clone(),
            current_branch: None,
        })
        .unwrap();
    let path = dir.path().join("history.jsonl");
    fs::write(&path, journal(history.records())).unwrap();
    let loaded = Store::new(path).load(&mut state).unwrap();
    assert_ne!(
        expected,
        history.records()[0].viewed.snapshot.project().unwrap()
    );
    assert_eq!(
        loaded.history.records()[0].viewed.snapshot.project(),
        Some(expected)
    );
    assert_eq!(
        state.project(expected).unwrap().directory,
        dir.path().join("current-worktree")
    );
    assert_eq!(state.windows(), windows);
    assert_eq!(
        loaded.history.records()[1].viewed.repository,
        history.records()[1].viewed.repository
    );
}

#[cfg(unix)]
#[test]
fn local_paths_unresolved_pins_and_all_reading_modes_round_trip() {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.jsonl");
    let (_, history) = history(&PathBuf::from(OsString::from_vec(
        b"/repositories/raw-\xff".to_vec(),
    )));
    for mode in [
        ReadingMode::Diff,
        ReadingMode::HeadFile,
        ReadingMode::BaseFile,
    ] {
        let mut records = history.records().to_vec();
        records[1].timestamp_ms = -123;
        records[1].viewed.snapshot.view.browsing.mode = mode;
        if let Destination::Comparison {
            base, comparison, ..
        } = &mut records[1].viewed.snapshot.destination
        {
            *base = BaseSelection::Default;
            *comparison = None;
        }
        fs::write(&path, journal(&records)).unwrap();
        let loaded = Store::new(path.clone())
            .load(&mut AppState::default())
            .unwrap();
        assert_eq!(loaded.history.records(), records);
    }
}

#[test]
fn interrupted_final_lines_are_backed_up_before_resuming_from_the_valid_prefix() {
    let dir = tempfile::tempdir().unwrap();
    let (_, history) = history(dir.path());
    let prefix = journal(history.records());
    let mut changed = history.records()[1].clone();
    changed.viewed.snapshot.view.browsing.first_visible_line = 99;
    let mut line = serde_json::to_vec(&changed).unwrap();
    line.push(b'\n');
    for cut in [1, line.len() / 2, line.len() - 1] {
        let mut interrupted = prefix.clone();
        interrupted.extend_from_slice(&line[..cut]);
        let path = dir.path().join("history.jsonl");
        fs::write(&path, &interrupted).unwrap();
        let mut store = Store::new(path.clone());
        let loaded = store.load(&mut AppState::default()).unwrap();
        assert_eq!(loaded.history.records(), history.records());
        let recovery = loaded.recovery.unwrap();
        assert_eq!(fs::read(&recovery.backup).unwrap(), interrupted);
        #[cfg(unix)]
        {
            assert_eq!(mode(&path), 0o600);
            assert_eq!(mode(&recovery.backup), 0o600);
        }
        assert_eq!(fs::read(&path).unwrap(), prefix);
        store.append([&changed]).unwrap();
        let recovered = Store::new(path).load(&mut AppState::default()).unwrap();
        assert!(recovered.recovery.is_none());
        assert_eq!(recovered.history.records()[1], changed);
    }
}

#[test]
fn corruption_never_skips_to_later_lines_and_unknown_formats_are_preserved() {
    let dir = tempfile::tempdir().unwrap();
    let (_, history) = history(dir.path());
    let prefix = journal(history.records());
    let mut bytes = prefix.clone();
    bytes.extend_from_slice(b"not json\n");
    bytes.extend_from_slice(&serde_json::to_vec(&history.records()[0]).unwrap());
    bytes.push(b'\n');
    let path = dir.path().join("history.jsonl");
    fs::write(&path, &bytes).unwrap();
    let loaded = Store::new(path.clone())
        .load(&mut AppState::default())
        .unwrap();
    assert_eq!(loaded.history.records(), history.records());
    assert_eq!(fs::read(loaded.recovery.unwrap().backup).unwrap(), bytes);
    assert_eq!(fs::read(&path).unwrap(), prefix);
    for bytes in [
        b"{\"format\":\"kestrel-viewing-history\",\"version\":99}\n".as_slice(),
        b"bad header\n",
        b"{",
        b"",
    ] {
        fs::write(&path, bytes).unwrap();
        let mut store = Store::new(path.clone());
        let loaded = store.load(&mut AppState::default()).unwrap();
        assert!(loaded.history.records().is_empty());
        assert_eq!(fs::read(loaded.recovery.unwrap().backup).unwrap(), bytes);
        store.append(history.records()).unwrap();
        assert!(
            Store::new(path.clone())
                .load(&mut AppState::default())
                .unwrap()
                .recovery
                .is_none()
        );
    }
}

#[test]
fn invalid_records_are_rejected_before_append_and_preserved_during_replay() {
    let dir = tempfile::tempdir().unwrap();
    let (_, history) = history(dir.path());
    for case in 0..4 {
        let mut invalid = history.records()[0].clone();
        match case {
            0 => invalid.id = ViewingId(99),
            1 => invalid.viewed.snapshot.destination = Destination::Home,
            2 => invalid.viewed.repository.directory = "relative/path".into(),
            3 => invalid.viewed.snapshot.view.browsing.file = Some("../escape".into()),
            _ => unreachable!(),
        }
        let path = dir.path().join(format!("invalid{case}.jsonl"));
        let mut store = Store::new(path.clone());
        store.load(&mut AppState::default()).unwrap();
        assert!(store.append([&invalid]).is_err());
        assert!(!path.exists());
        let bytes = journal(&[invalid]);
        fs::write(&path, &bytes).unwrap();
        let mut state = AppState::default();
        let before = state.clone();
        let loaded = store.load(&mut state).unwrap();
        assert_eq!(state, before);
        assert!(loaded.history.records().is_empty());
        assert_eq!(fs::read(loaded.recovery.unwrap().backup).unwrap(), bytes);
    }
}

#[test]
fn failed_partial_appends_roll_back_and_retry_without_losing_updates() {
    let dir = tempfile::tempdir().unwrap();
    let (_, history) = history(dir.path());
    let path = dir.path().join("history.jsonl");
    let mut store = Store::new(path.clone());
    store.load(&mut AppState::default()).unwrap();
    store.append(history.records()).unwrap();
    let prefix = fs::read(&path).unwrap();
    let mut changed = history.records()[0].clone();
    changed.viewed.snapshot.view.branches.query = "updated".into();
    assert!(
        store
            .append_with([&changed], |file, bytes| {
                file.write_all(&bytes[..bytes.len() / 2])?;
                Err(std::io::Error::other("injected write failure"))
            })
            .is_err()
    );
    assert_eq!(fs::read(&path).unwrap(), prefix);
    store.append([&changed]).unwrap();
    let loaded = Store::new(path).load(&mut AppState::default()).unwrap();
    assert_eq!(loaded.history.records()[0], changed);
}

#[test]
fn failed_rollback_disables_writes_until_restart_recovers_the_tail() {
    let dir = tempfile::tempdir().unwrap();
    let (_, history) = history(dir.path());
    let path = dir.path().join("history.jsonl");
    let mut store = Store::new(path.clone());
    store.load(&mut AppState::default()).unwrap();
    store.append(history.records()).unwrap();
    let result = store.append_with([&history.records()[0]], |file, bytes| {
        file.write_all(&bytes[..bytes.len() / 2])?;
        *file = fs::File::open(&path)?;
        Err(std::io::Error::other("injected write and rollback failure"))
    });
    assert!(matches!(result, Err(Error::Rollback { .. })));
    assert!(matches!(
        store.append(history.records()),
        Err(Error::NotLoaded)
    ));
    let loaded = Store::new(path).load(&mut AppState::default()).unwrap();
    assert_eq!(loaded.history.records(), history.records());
    assert!(loaded.recovery.is_some());
}

#[test]
fn missing_unreadable_and_externally_changed_journals_are_not_overwritten() {
    let dir = tempfile::tempdir().unwrap();
    let (_, history) = history(dir.path());
    let path = dir.path().join("history.jsonl");
    let mut store = Store::new(path.clone());
    assert!(matches!(
        store.append(history.records()),
        Err(Error::NotLoaded)
    ));
    assert!(
        store
            .load(&mut AppState::default())
            .unwrap()
            .history
            .records()
            .is_empty()
    );
    store.append([]).unwrap();
    assert!(!path.exists());
    store.append(history.records()).unwrap();
    let mut external = fs::read(&path).unwrap();
    external.extend_from_slice(b"external writer");
    fs::write(&path, &external).unwrap();
    assert!(matches!(
        store.append(history.records()),
        Err(Error::Changed)
    ));
    assert_eq!(fs::read(&path).unwrap(), external);
    fs::remove_file(&path).unwrap();
    fs::create_dir(&path).unwrap();
    assert!(store.load(&mut AppState::default()).is_err());
    assert!(matches!(
        store.append(history.records()),
        Err(Error::NotLoaded)
    ));
}

#[cfg(unix)]
#[test]
fn json_fixture_locks_direct_record_serialization_and_replay() {
    let (_, history) = history(Path::new("/repositories/example"));
    let bytes = journal(history.records());
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src/viewing_history/storage/fixtures/v1.jsonl");
    if std::env::var_os("UPDATE_HISTORY_FIXTURE").is_some() {
        fs::create_dir_all(fixture.parent().unwrap()).unwrap();
        fs::write(fixture, bytes).unwrap();
        return;
    }
    assert_eq!(fs::read(&fixture).unwrap(), bytes);
    // Load a copy: fixture files must never be repaired in place by a failing test.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.jsonl");
    fs::copy(fixture, &path).unwrap();
    let loaded = Store::new(path).load(&mut AppState::default()).unwrap();
    assert!(loaded.recovery.is_none());
    assert_eq!(loaded.history.records(), history.records());
}

#[test]
#[ignore = "manual JSON journal sizing/timing; run in release mode with --ignored --nocapture"]
fn measure_history_storage() {
    use std::time::Instant;
    let dir = tempfile::tempdir().unwrap();
    let (_, history) = history(&dir.path().join("representative-repository"));
    for count in [1, 1_000, 100_000] {
        let records = (0..count)
            .map(|index| {
                let mut record = history.records()[index % 2].clone();
                record.id = ViewingId(index as u64);
                record
            })
            .collect::<Vec<_>>();
        let path = dir.path().join(format!("{count}.jsonl"));
        let mut store = Store::new(path.clone());
        store.load(&mut AppState::default()).unwrap();
        let start = Instant::now();
        store.append(&records).unwrap();
        let initial = start.elapsed();
        let bytes = fs::metadata(&path).unwrap().len();
        let mut changed = records[count - 1].clone();
        changed.viewed.snapshot.view.branches.first_visible_row = 99;
        let start = Instant::now();
        store.append([&changed]).unwrap();
        let append = start.elapsed();
        let added = fs::metadata(&path).unwrap().len() - bytes;
        let start = Instant::now();
        let loaded = Store::new(path).load(&mut AppState::default()).unwrap();
        let replay = start.elapsed();
        assert_eq!(loaded.history.records().len(), count);
        assert_eq!(loaded.history.records()[count - 1], changed);
        println!(
            "{count} records: {bytes} bytes, initial {initial:?}; one update: {added} bytes, append {append:?}; replay {replay:?}"
        );
    }
}
