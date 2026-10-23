use super::*;
use std::time::{Duration, Instant};

#[test]
fn ref_changes_and_rescan_hints_invalidate_but_reads_and_unrelated_paths_do_not() {
    let common = Path::new("/repo/.git");
    for path in [
        "refs",
        "refs/heads/topic",
        "refs/remotes/origin/HEAD",
        "packed-refs",
        "",
    ] {
        let event = Event::new(EventKind::Any).add_path(common.join(path));
        assert!(affects_refs(common, &event), "{path}");
    }
    for path in [
        "objects/ab/cd",
        "logs/refs/heads/topic",
        "index",
        "packed-refs.lock",
    ] {
        let event = Event::new(EventKind::Any).add_path(common.join(path));
        assert!(!affects_refs(common, &event), "{path}");
    }
    let access = Event::new(EventKind::Access(notify::event::AccessKind::Read))
        .add_path(common.join("refs/heads/topic"));
    assert!(!affects_refs(common, &access));
    let rescan = Event::new(EventKind::Other)
        .add_path(common.join("objects"))
        .set_flag(notify::event::Flag::Rescan);
    assert!(affects_refs(common, &rescan));
    assert!(affects_refs(common, &Event::new(EventKind::Any)));
}

#[test]
fn a_full_notification_channel_does_not_lose_watcher_errors() {
    let watch = RefWatch::channel();
    let signal = watch.invalidation.clone();
    for _ in 0..100 {
        signal.send(None);
    }
    signal.send(Some("watch failed".into()));
    assert_eq!(watch.events.len(), 1);
    assert_eq!(watch.failure().as_deref(), Some("watch failed"));
    assert!(watch.events.try_recv().is_ok());
    assert!(watch.events.try_recv().is_err());
}

#[test]
fn native_watcher_observes_ref_file_replacement() {
    let dir = tempfile::tempdir().unwrap();
    let common = dir.path().canonicalize().unwrap();
    std::fs::create_dir_all(common.join("refs/heads")).unwrap();
    let watch = RefWatch::new(&common).unwrap();
    std::thread::sleep(Duration::from_millis(100));
    while watch.events.try_recv().is_ok() {}
    std::fs::write(common.join("refs/heads/topic.lock"), "new tip").unwrap();
    std::fs::rename(
        common.join("refs/heads/topic.lock"),
        common.join("refs/heads/topic"),
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if watch.events.try_recv().is_ok() {
            assert!(watch.failure().is_none());
            break;
        }
        assert!(
            Instant::now() < deadline,
            "no native ref notification received"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn missing_watch_root_reports_a_setup_failure() {
    let dir = tempfile::tempdir().unwrap();
    assert!(RefWatch::new(&dir.path().join("missing")).is_err());
}
