use super::*;
use kestrel::viewing_history::ViewingId;

fn setup(cx: &mut TestAppContext) -> (tempfile::TempDir, Entity<Workspace>, TabId) {
    let dir = tempfile::tempdir().unwrap();
    let workspace = cx.new(|cx| {
        Workspace::new(
            LoadedWorkspace::load(Ok(Store::new(dir.path().join("state.json")))),
            cx,
        )
    });
    let tab = workspace.update(cx, |workspace, cx| {
        workspace
            .edit(
                |state| {
                    let project = project(state, "persisted-history-repo");
                    let window = state.new_window()?;
                    let tab = state.new_tab(window)?;
                    state.open_project(project, tab)?;
                    Ok(tab)
                },
                cx,
            )
            .unwrap()
    });
    (dir, workspace, tab)
}

fn saved_history(dir: &std::path::Path) -> ViewingHistory {
    history_storage::Store::new(dir.join("state.history.bin"))
        .load(&mut AppState::default())
        .unwrap()
        .history
}

fn finish_saving(cx: &mut TestAppContext) {
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_millis(151));
    cx.run_until_parked();
}

#[gpui::test]
fn history_only_changes_schedule_a_save_and_quit_fences_pending_writers(cx: &mut TestAppContext) {
    let (dir, workspace, tab) = setup(cx);
    finish_saving(cx);
    assert_eq!(saved_history(dir.path()).records().len(), 1);
    workspace.update(cx, |workspace, cx| {
        let before = workspace.state().clone();
        let revision = workspace.revision;
        workspace.navigate(tab, Navigation::ReopenTab, cx).unwrap();
        assert_eq!(workspace.state(), &before);
        assert_eq!(workspace.revision, revision + 1);
    });
    finish_saving(cx);
    assert_eq!(saved_history(dir.path()).records().len(), 2);
    let records = workspace.update(cx, |workspace, cx| {
        workspace.navigate(tab, Navigation::ReopenTab, cx).unwrap();
        workspace.flush_on_quit();
        assert!(workspace.navigate(tab, Navigation::ReopenTab, cx).is_err());
        workspace.viewing_history().records().to_vec()
    });
    finish_saving(cx);
    assert_eq!(saved_history(dir.path()).records(), records);
    assert_eq!(records.len(), 3);
}

#[gpui::test]
fn history_writer_fences_failed_newer_attempts_and_closed_writers(cx: &mut TestAppContext) {
    let (dir, workspace, _) = setup(cx);
    let (writer, first) = workspace.read_with(cx, |workspace, _| {
        (
            workspace.history_writer.clone().unwrap(),
            workspace.viewing_history().records().to_vec(),
        )
    });
    let mut second = first.clone();
    second.push(first[0].clone());
    second[1].id = ViewingId(1);
    let mut writer = writer.lock().unwrap();
    writer.save(2, &second).unwrap();
    writer.save(1, &first).unwrap();
    assert_eq!(saved_history(dir.path()).records(), second);
    let path = dir.path().join("state.history.bin");
    let previous = dir.path().join("previous.bin");
    fs::rename(&path, &previous).unwrap();
    fs::create_dir(&path).unwrap();
    assert!(writer.save(4, &first).is_err());
    fs::remove_dir(&path).unwrap();
    fs::rename(previous, path).unwrap();
    writer.save(3, &first).unwrap();
    assert_eq!(saved_history(dir.path()).records(), second);
    writer.save(4, &first).unwrap();
    assert_eq!(saved_history(dir.path()).records(), first);
    writer.closed = true;
    writer.save(5, &second).unwrap();
    assert_eq!(saved_history(dir.path()).records(), first);
}

#[gpui::test]
fn history_save_failure_does_not_block_workspace_saves_or_navigation(cx: &mut TestAppContext) {
    let (dir, workspace, tab) = setup(cx);
    let path = dir.path().join("state.history.bin");
    fs::create_dir(&path).unwrap();
    finish_saving(cx);
    workspace.update(cx, |workspace, cx| {
        assert!(workspace.save_error().unwrap().contains("Viewing history:"));
        workspace.navigate(tab, Navigation::Home, cx).unwrap();
    });
    finish_saving(cx);
    assert_eq!(
        Store::new(dir.path().join("state.json"))
            .load()
            .unwrap()
            .state
            .tab(tab)
            .unwrap()
            .route,
        Route::Home
    );
    fs::remove_dir(path).unwrap();
    workspace.update(cx, |workspace, cx| workspace.save(cx));
    finish_saving(cx);
    assert!(workspace.read_with(cx, |workspace, _| workspace.save_error().is_none()));
    assert_eq!(saved_history(dir.path()).records().len(), 1);
}

#[gpui::test]
fn workspace_save_failure_does_not_block_history_and_restart_reconciles_ids(
    cx: &mut TestAppContext,
) {
    let (dir, workspace, _) = setup(cx);
    let path = dir.path().join("state.json");
    fs::create_dir(&path).unwrap();
    finish_saving(cx);
    assert!(workspace.read_with(cx, |workspace, _| {
        workspace.save_error().unwrap().contains("Workspace:")
    }));
    let records = saved_history(dir.path()).records().to_vec();
    assert_eq!(records.len(), 1);
    let loaded = LoadedWorkspace::load(Ok(Store::new(path)));
    assert!(loaded.writer.is_none());
    assert!(loaded.history_writer.is_some());
    assert!(
        loaded
            .notice
            .unwrap()
            .contains("Saved state could not be loaded")
    );
    assert_eq!(loaded.viewing_history.records(), records);
    assert_eq!(loaded.state.projects().len(), 1);
    assert!(loaded.state.windows().is_empty());
}

#[gpui::test]
fn corrupt_or_unreadable_history_leaves_restored_workspace_usable(cx: &mut TestAppContext) {
    let (dir, workspace, _) = setup(cx);
    workspace.update(cx, |workspace, _| workspace.flush_on_quit());
    let state = workspace.read_with(cx, |workspace, _| workspace.state().clone());
    let path = dir.path().join("state.history.bin");
    fs::write(&path, b"corrupt history").unwrap();
    let loaded = LoadedWorkspace::load(Ok(Store::new(dir.path().join("state.json"))));
    assert_eq!(loaded.state, state);
    assert!(loaded.notice.unwrap().contains("Original preserved at"));
    assert!(loaded.writer.is_some());
    assert!(loaded.history_writer.is_some());
    assert_eq!(fs::read(&path).unwrap(), b"corrupt history");
    let backups = fs::read_dir(dir.path())
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with("kestrel-history-recovery-")
        })
        .collect::<Vec<_>>();
    assert_eq!(backups.len(), 1);
    assert_eq!(fs::read(backups[0].path()).unwrap(), b"corrupt history");
    fs::remove_file(&path).unwrap();
    fs::create_dir(&path).unwrap();
    let loaded = LoadedWorkspace::load(Ok(Store::new(dir.path().join("state.json"))));
    assert_eq!(loaded.state, state);
    assert!(loaded.history_writer.is_none());
    assert!(loaded.writer.is_some());
    assert!(
        loaded
            .notice
            .unwrap()
            .contains("History saving is disabled")
    );
}

#[gpui::test]
fn independently_saved_workspace_catalogs_do_not_retarget_history(cx: &mut TestAppContext) {
    let (dir, workspace, _) = setup(cx);
    workspace.update(cx, |workspace, _| workspace.flush_on_quit());
    let original = workspace.read_with(cx, |workspace, _| {
        workspace.viewing_history().records()[0].clone()
    });
    let mut unrelated = AppState::default();
    let other = project(&mut unrelated, "unrelated-repository");
    let window = unrelated.new_window().unwrap();
    let tab = unrelated.new_tab(window).unwrap();
    unrelated.open_project(other, tab).unwrap();
    let mut store = Store::new(dir.path().join("state.json"));
    store.load().unwrap();
    store.save(&unrelated).unwrap();
    let loaded = LoadedWorkspace::load(Ok(store));
    let reopened = cx.new(|cx| Workspace::new(loaded, cx));
    reopened.update(cx, |workspace, cx| {
        assert_eq!(workspace.state().windows(), unrelated.windows());
        assert_eq!(workspace.viewing_history().records().len(), 1);
        workspace
            .open_view(original.id, OpenViewIn::Tab(tab), cx)
            .unwrap();
        let project = workspace.state().tab(tab).unwrap().route.project().unwrap();
        assert_ne!(project, other);
        assert_eq!(
            workspace.state().project(project).unwrap().common_dir,
            original.viewed.repository.common_dir
        );
        assert_eq!(workspace.viewing_history().records().len(), 2);
    });
}

#[gpui::test]
fn workspace_replacement_cannot_retarget_existing_records_or_recent_keys(cx: &mut TestAppContext) {
    let (_dir, workspace, _) = setup(cx);
    workspace.update(cx, |workspace, cx| {
        let original = workspace.viewing_history().records()[0].clone();
        let mut other = AppState::default();
        let project = project(&mut other, "replacement-repository");
        let window = other.new_window().unwrap();
        let tab = other.new_tab(window).unwrap();
        other.open_project(project, tab).unwrap();
        workspace.replace(other, cx);
        workspace.navigate(tab, Navigation::ReopenTab, cx).unwrap();
        assert_eq!(workspace.viewing_history().recent(10).count(), 2);
        workspace
            .open_view(original.id, OpenViewIn::Tab(tab), cx)
            .unwrap();
        let project = workspace.state().tab(tab).unwrap().route.project().unwrap();
        assert_eq!(
            workspace.state().project(project).unwrap().common_dir,
            original.viewed.repository.common_dir
        );
        assert_eq!(workspace.viewing_history().recent(10).count(), 2);
    });
}
