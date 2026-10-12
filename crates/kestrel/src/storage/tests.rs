use super::*;
use crate::{git::RepositoryInfo, review::RefName};
use tempfile::TempDir;

fn state() -> AppState {
    let mut state = AppState::default();
    let directory = std::env::temp_dir().join("kestrel-saved-project");
    let project = state
        .register_project(&RepositoryInfo {
            common_dir: directory.join(".git"),
            directory,
            current_branch: Some(RefName::from("refs/heads/topic")),
        })
        .unwrap();
    let window = state.new_window().unwrap();
    let tab = state.new_tab(window).unwrap();
    state.open_project(project, tab).unwrap();
    state
}

#[test]
fn configuration_location_prefers_xdg_then_dot_config_then_native() {
    let root = std::env::temp_dir();
    let home = root.join("home");
    let xdg = root.join("custom-config");
    let native = root.join("native-config");
    assert_eq!(
        state_path(Some(xdg.clone()), Some(home.clone()), Some(native.clone())).unwrap(),
        xdg.join("kestrel/state.json"),
    );
    for config_home in [None, Some(PathBuf::new()), Some(PathBuf::from("relative"))] {
        assert_eq!(
            state_path(config_home, Some(home.clone()), Some(native.clone())).unwrap(),
            home.join(".config/kestrel/state.json"),
        );
    }
    assert_eq!(
        state_path(None, None, Some(native.clone())).unwrap(),
        native.join("kestrel/state.json")
    );
    assert!(matches!(
        state_path(None, None, None),
        Err(Error::NoConfigDirectory)
    ));
}

#[test]
fn first_launch_and_successive_atomic_saves_round_trip() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("nested/state.json");
    let mut store = Store::new(path.clone());
    let loaded = store.load().unwrap();
    assert_eq!(loaded.state, AppState::default());
    assert!(loaded.recovery.is_none());
    let mut state = state();
    store.save(&state).unwrap();
    let restored = Store::new(path.clone()).load().unwrap();
    assert_eq!(restored.state, state);
    state.close_window(state.windows()[0].id);
    store.save(&state).unwrap();
    assert_eq!(Store::new(path).load().unwrap().state, state);
}

#[test]
fn corrupt_or_unsupported_state_is_preserved_before_replacement() {
    for bytes in [
        b"{broken".as_slice(),
        br#"{"version":1,"state":{"projects":[],"windows":[],"active_window":null,"next_id":1}}"#,
        br#"{"version":999,"future-data":"keep me"}"#,
    ] {
        let directory = TempDir::new().unwrap();
        let path = directory.path().join("state.json");
        fs::write(&path, bytes).unwrap();
        let mut store = Store::new(path.clone());
        let loaded = store.load().unwrap();
        assert_eq!(loaded.state, AppState::default());
        let recovery = loaded.recovery.unwrap();
        assert!(!recovery.reason.is_empty());
        assert_eq!(fs::read(&recovery.backup).unwrap(), bytes);
        assert_eq!(fs::read(&path).unwrap(), bytes);
        store.save(&state()).unwrap();
        assert_eq!(fs::read(&recovery.backup).unwrap(), bytes);
        assert_eq!(Store::new(path).load().unwrap().state, state());
    }
}

#[test]
fn structurally_invalid_state_is_recovered_not_partially_restored() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("state.json");
    let mut value = serde_json::to_value(Snapshot {
        version: VERSION,
        state: &state(),
    })
    .unwrap();
    value["state"]["next_id"] = serde_json::json!(0);
    fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
    let loaded = Store::new(path).load().unwrap();
    assert!(loaded.recovery.is_some());
    assert_eq!(loaded.state, AppState::default());
}

#[test]
fn save_requires_loading_and_recovery_first() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("state.json");
    fs::write(&path, b"precious broken state").unwrap();
    let mut store = Store::new(path.clone());
    assert!(matches!(store.save(&state()), Err(Error::NotLoaded)));
    assert_eq!(fs::read(path).unwrap(), b"precious broken state");
}

#[test]
fn failed_load_never_authorizes_overwriting_the_original() {
    let directory = TempDir::new().unwrap();
    let mut store = Store::new(directory.path().to_path_buf());
    assert!(store.load().is_err());
    assert!(matches!(store.save(&state()), Err(Error::NotLoaded)));
    assert!(directory.path().is_dir());
}

#[test]
fn rejected_transitions_do_not_change_the_saved_snapshot() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("state.json");
    let mut store = Store::new(path.clone());
    store.load().unwrap();
    store.save(&state()).unwrap();
    let previous = fs::read(&path).unwrap();
    let mut current = state();
    let tab = current.windows()[0].active_tab.unwrap();
    assert!(
        current
            .select_branch(tab, RefName::from("invalid"))
            .is_err()
    );
    store.save(&current).unwrap();
    assert_eq!(fs::read(path).unwrap(), previous);
}

#[test]
fn interrupted_temporary_writes_do_not_affect_the_saved_snapshot() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("state.json");
    let mut store = Store::new(path.clone());
    store.load().unwrap();
    store.save(&state()).unwrap();
    fs::write(
        directory.path().join(".tmp-interrupted"),
        b"{\"version\":1,\"state\":",
    )
    .unwrap();
    assert_eq!(Store::new(path).load().unwrap().state, state());
}

#[test]
fn failed_replace_leaves_the_destination_untouched_and_can_be_retried() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("state.json");
    let mut store = Store::new(path.clone());
    store.load().unwrap();
    fs::create_dir(&path).unwrap();
    fs::write(path.join("keep"), "do not remove").unwrap();
    assert!(store.save(&state()).is_err());
    assert_eq!(
        fs::read_to_string(path.join("keep")).unwrap(),
        "do not remove"
    );
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    fs::remove_file(path.join("keep")).unwrap();
    fs::remove_dir(&path).unwrap();
    store.save(&state()).unwrap();
    assert_eq!(Store::new(path).load().unwrap().state, state());
}
