use std::fs;

use gpui::{AppContext, TestAppContext};
use kestrel::{git::RepositoryInfo, review::RefName};

use super::*;

mod persistence;
mod viewing;

fn project(state: &mut AppState, name: &str) -> kestrel::state::ProjectId {
    let directory = std::env::temp_dir().join(name);
    state
        .register_project(&RepositoryInfo {
            common_dir: directory.join(".git"),
            directory,
            current_branch: Some(RefName::from("refs/heads/review")),
        })
        .unwrap()
}

#[test]
fn tab_layout_keeps_project_context_and_rejects_foreign_or_duplicate_tabs() {
    let mut state = AppState::default();
    let project = project(&mut state, "repo");
    let window = state.new_window().unwrap();
    let a = state.new_tab(window).unwrap();
    let b = state.new_tab(window).unwrap();
    state.open_project(project, a).unwrap();
    state.open_project(project, b).unwrap();
    let other = state.new_window().unwrap();
    let c = state.new_tab(other).unwrap();
    let before = state.clone();
    for (tabs, active) in [
        (vec![c.as_u64()], Some(c.as_u64())),
        (vec![a.as_u64(), a.as_u64()], Some(a.as_u64())),
        (vec![a.as_u64()], Some(b.as_u64())),
        (vec![], Some(a.as_u64())),
    ] {
        assert!(set_tab_layout(&mut state, window, &tabs, active).is_err());
        assert_eq!(state, before);
    }
    set_tab_layout(
        &mut state,
        window,
        &[b.as_u64(), a.as_u64()],
        Some(a.as_u64()),
    )
    .unwrap();
    assert_eq!(
        state
            .window(window)
            .unwrap()
            .tabs
            .iter()
            .map(|tab| tab.id)
            .collect::<Vec<_>>(),
        [b, a]
    );
    assert_eq!(state.window(window).unwrap().active_tab, Some(a));
    set_tab_layout(&mut state, window, &[b.as_u64()], Some(b.as_u64())).unwrap();
    assert_eq!(
        state.project(project).unwrap(),
        before.project(project).unwrap()
    );
    assert_eq!(state.window(other).unwrap(), before.window(other).unwrap());
    set_tab_layout(&mut state, window, &[], None).unwrap();
    assert!(state.window(window).is_err());
    assert_eq!(state.projects(), before.projects());
}

#[test]
fn writer_never_replaces_a_newer_attempt_with_an_older_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.json");
    let mut loaded = LoadedWorkspace::load(Ok(Store::new(path.clone())));
    let writer = loaded.writer.as_mut().unwrap();
    let first = AppState::default();
    let mut second = first.clone();
    second.new_window().unwrap();
    writer.save(2, &second).unwrap();
    writer.save(1, &first).unwrap();
    assert_eq!(Store::new(path.clone()).load().unwrap().state, second);

    fs::rename(&path, dir.path().join("previous")).unwrap();
    fs::create_dir(&path).unwrap();
    assert!(writer.save(4, &first).is_err());
    fs::remove_dir(&path).unwrap();
    fs::rename(dir.path().join("previous"), &path).unwrap();
    writer.save(3, &first).unwrap();
    assert_eq!(Store::new(path.clone()).load().unwrap().state, second);
    writer.save(4, &first).unwrap();
    assert_eq!(Store::new(path.clone()).load().unwrap().state, first);
    writer.closed = true;
    writer.save(5, &second).unwrap();
    assert_eq!(Store::new(path).load().unwrap().state, first);
}

#[gpui::test]
fn rapid_edits_are_saved_automatically_and_quit_flushes_pending_changes(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.json");
    let loaded = LoadedWorkspace::load(Ok(Store::new(path.clone())));
    let workspace = cx.new(|cx| Workspace::new(loaded, cx));
    for _ in 0..3 {
        workspace.update(cx, |workspace, cx| {
            workspace.edit(AppState::new_window, cx).unwrap()
        });
    }
    cx.run_until_parked();
    assert!(!path.exists());
    cx.executor().advance_clock(Duration::from_millis(151));
    cx.run_until_parked();
    assert_eq!(
        Store::new(path.clone())
            .load()
            .unwrap()
            .state
            .windows()
            .len(),
        3
    );
    let last = workspace.update(cx, |workspace, cx| {
        workspace.edit(AppState::new_window, cx).unwrap()
    });
    workspace.update(cx, |workspace, cx| {
        workspace.flush_on_quit();
        workspace.close_window(last, cx);
        assert_eq!(workspace.state().windows().len(), 4);
    });
    cx.executor().advance_clock(Duration::from_secs(1));
    cx.run_until_parked();
    assert_eq!(Store::new(path).load().unwrap().state.windows().len(), 4);
}

#[gpui::test]
fn save_failure_is_persistent_until_a_successful_retry(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.json");
    let loaded = LoadedWorkspace::load(Ok(Store::new(path.clone())));
    let workspace = cx.new(|cx| Workspace::new(loaded, cx));
    fs::create_dir(&path).unwrap();
    workspace.update(cx, |workspace, cx| {
        workspace.edit(AppState::new_window, cx).unwrap()
    });
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_secs(1));
    cx.run_until_parked();
    assert!(workspace.read_with(cx, |workspace, _| workspace.save_error().is_some()));
    fs::remove_dir(&path).unwrap();
    workspace.update(cx, |workspace, cx| workspace.save(cx));
    assert!(workspace.read_with(cx, |workspace, _| workspace.save_error().is_some()));
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_secs(1));
    cx.run_until_parked();
    assert!(workspace.read_with(cx, |workspace, _| workspace.save_error().is_none()));
    assert_eq!(Store::new(path).load().unwrap().state.windows().len(), 1);
}

#[gpui::test]
fn poisoned_writer_reports_failure_and_preserves_the_saved_snapshot(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.json");
    let mut store = Store::new(path.clone());
    store.load().unwrap();
    store.save(&AppState::default()).unwrap();
    let loaded = LoadedWorkspace::load(Ok(store));
    let workspace = cx.new(|cx| Workspace::new(loaded, cx));
    let writer = workspace.read_with(cx, |workspace, _| workspace.writer.clone().unwrap());
    assert!(
        std::thread::spawn(move || {
            let _guard = writer.lock().unwrap();
            panic!("simulate a panic while saving");
        })
        .join()
        .is_err()
    );
    workspace.update(cx, |workspace, cx| {
        workspace.edit(AppState::new_window, cx).unwrap()
    });
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_secs(1));
    cx.run_until_parked();
    workspace.update(cx, |workspace, _| {
        assert!(workspace.save_error().unwrap().contains("poisoned"));
        assert_eq!(workspace.state().windows().len(), 1);
        workspace.flush_on_quit();
    });
    assert_eq!(Store::new(path).load().unwrap().state, AppState::default());
}

#[test]
fn failed_startup_load_disables_writes_without_losing_the_original() {
    let dir = tempfile::tempdir().unwrap();
    let loaded = LoadedWorkspace::load(Ok(Store::new(dir.path().to_owned())));
    assert!(loaded.writer.is_none());
    assert!(
        loaded
            .notice
            .unwrap()
            .contains("Workspace saving is disabled")
    );
    assert!(dir.path().is_dir());
}

#[gpui::test]
fn failed_edits_leave_the_workspace_and_save_schedule_unchanged(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let loaded = LoadedWorkspace::load(Ok(Store::new(dir.path().join("state.json"))));
    let workspace = cx.new(|cx| Workspace::new(loaded, cx));
    workspace.update(cx, |workspace, cx| {
        assert!(
            workspace
                .edit(
                    |state| {
                        state.new_window()?;
                        Err::<(), _>(StateError("aborted"))
                    },
                    cx
                )
                .is_err()
        );
        assert_eq!(workspace.state(), &AppState::default());
        assert!(workspace.save_task.is_none());
    });
}

#[gpui::test]
fn closed_history_is_bounded_transactional_and_not_persisted(cx: &mut TestAppContext) {
    const CLOSED_HISTORY_LIMIT: usize = 100;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.json");
    let loaded = LoadedWorkspace::load(Ok(Store::new(path.clone())));
    let workspace = cx.new(|cx| Workspace::new(loaded, cx));
    workspace.update(cx, |workspace, cx| {
        let window = workspace
            .edit(
                |state| {
                    let window = state.new_window()?;
                    state.new_tab(window)?;
                    Ok(window)
                },
                cx,
            )
            .unwrap();
        for _ in 0..CLOSED_HISTORY_LIMIT + 3 {
            let tab = workspace.edit(|state| state.new_tab(window), cx).unwrap();
            workspace
                .edit(
                    |state| {
                        state.close_tab(tab);
                        Ok(())
                    },
                    cx,
                )
                .unwrap();
        }
        assert_eq!(workspace.model.closed_count(), CLOSED_HISTORY_LIMIT);
        let before = workspace.state().clone();
        let revision = workspace.model.revision();
        let (_, restored) = workspace.reopen_state(Some(window)).unwrap().unwrap();
        assert_eq!(restored, window);
        assert_eq!(workspace.state(), &before);
        assert_eq!(workspace.model.revision(), revision);
        assert_eq!(workspace.model.closed_count(), CLOSED_HISTORY_LIMIT);
        workspace.flush_on_quit();
        assert!(!workspace.can_reopen());
        assert!(workspace.reopen_state(None).is_err());
    });
    let restored = cx.new(|cx| Workspace::new(LoadedWorkspace::load(Ok(Store::new(path))), cx));
    assert!(!restored.read_with(cx, |workspace, _| workspace.can_reopen()));
}
