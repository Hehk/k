use std::{
    path::Path,
    process::{Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};

use notify::{RecursiveMode, Watcher};

fn git(repo: &Path, args: &[&str]) -> Result<String, Box<dyn std::error::Error>> {
    let output = Command::new("git")
        .current_dir(repo)
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_AUTHOR_NAME", "Watch spike")
        .env("GIT_AUTHOR_EMAIL", "watch@example.invalid")
        .env("GIT_COMMITTER_NAME", "Watch spike")
        .env("GIT_COMMITTER_EMAIL", "watch@example.invalid")
        .args([
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .stdin(Stdio::null())
        .output()?;
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}

type Events = mpsc::Receiver<(Instant, notify::Result<notify::Event>)>;

fn watch(common: &Path) -> notify::Result<(notify::RecommendedWatcher, Events)> {
    let (tx, rx) = mpsc::channel();
    let mut watcher = notify::recommended_watcher(move |event| {
        let _ = tx.send((Instant::now(), event));
    })?;
    watcher.watch(common, RecursiveMode::Recursive)?;
    Ok((watcher, rx))
}

struct Case<'a> {
    name: &'a str,
    command: &'a [&'a str],
    event_path: &'a str,
    expected_tip: Option<&'a str>,
}

fn collect_events(
    rx: &Events,
    started: Instant,
    expected_path: &Path,
) -> Result<usize, Box<dyn std::error::Error>> {
    let deadline = started + Duration::from_secs(2);
    let mut matching = 0;
    while let Some(remaining) = deadline.checked_duration_since(Instant::now()) {
        let (received, event) = match rx.recv_timeout(remaining) {
            Ok(event) => event,
            Err(mpsc::RecvTimeoutError::Timeout) => break,
            Err(error) => return Err(error.into()),
        };
        let event = event?;
        println!(
            "  +{:?} {event:?}",
            received.saturating_duration_since(started)
        );
        if received >= started && event.paths.iter().any(|path| path == expected_path) {
            matching += 1;
        }
    }
    Ok(matching)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = tempfile::tempdir()?;
    let repo = fixture.path().canonicalize()?;
    git(&repo, &["init", "--template=", "--initial-branch=main"])?;
    let tree = git(&repo, &["mktree"])?;
    let first = git(&repo, &["commit-tree", &tree, "-m", "first"])?;
    let second = git(&repo, &["commit-tree", &tree, "-p", &first, "-m", "second"])?;
    let third = git(&repo, &["commit-tree", &tree, "-p", &second, "-m", "third"])?;
    git(&repo, &["update-ref", "refs/heads/main", &first])?;
    let common = repo
        .join(git(&repo, &["rev-parse", "--git-common-dir"])?)
        .canonicalize()?;
    let linked = repo.join("linked");
    let linked_path = linked.to_string_lossy();
    git(
        &repo,
        &["worktree", "add", "--detach", &linked_path, "main"],
    )?;
    let linked_common = linked
        .join(git(&linked, &["rev-parse", "--git-common-dir"])?)
        .canonicalize()?;
    let linked_git_dir = linked
        .join(git(&linked, &["rev-parse", "--git-dir"])?)
        .canonicalize()?;
    assert!(linked.join(".git").is_file());
    assert_eq!(linked_common, common);
    assert_ne!(linked_git_dir, common);
    println!("Linked worktree: {}", linked.display());
    println!("Private Git directory: {}", linked_git_dir.display());
    println!("Shared Git directory: {}", linked_common.display());

    let (watcher, rx) = watch(&common)?;
    println!(
        "backend={:?}, watch={}",
        notify::RecommendedWatcher::kind(),
        common.display()
    );
    // Separate watcher startup/fixture events from the first measured operation.
    std::thread::sleep(Duration::from_millis(500));
    while rx.try_recv().is_ok() {}

    let cases = [
        Case {
            name: "create topic",
            command: &["update-ref", "refs/heads/topic", &first],
            event_path: "refs/heads/topic",
            expected_tip: Some(&first),
        },
        Case {
            name: "advance topic",
            command: &["update-ref", "refs/heads/topic", &second],
            event_path: "refs/heads/topic",
            expected_tip: Some(&second),
        },
        Case {
            name: "delete loose topic",
            command: &["update-ref", "-d", "refs/heads/topic"],
            event_path: "refs/heads/topic",
            expected_tip: None,
        },
        Case {
            name: "recreate topic",
            command: &["update-ref", "refs/heads/topic", &second],
            event_path: "refs/heads/topic",
            expected_tip: Some(&second),
        },
        Case {
            name: "pack refs",
            command: &["pack-refs", "--all", "--prune"],
            event_path: "packed-refs",
            expected_tip: Some(&second),
        },
        Case {
            name: "advance packed topic",
            command: &["update-ref", "refs/heads/topic", &third],
            event_path: "refs/heads/topic",
            expected_tip: Some(&third),
        },
        Case {
            name: "delete topic from linked worktree",
            command: &["-C", &linked_path, "update-ref", "-d", "refs/heads/topic"],
            event_path: "refs/heads/topic",
            expected_tip: None,
        },
        Case {
            name: "create topic from linked worktree",
            command: &["-C", &linked_path, "update-ref", "refs/heads/topic", &first],
            event_path: "refs/heads/topic",
            expected_tip: Some(&first),
        },
        Case {
            name: "advance topic from linked worktree",
            command: &[
                "-C",
                &linked_path,
                "update-ref",
                "refs/heads/topic",
                &second,
            ],
            event_path: "refs/heads/topic",
            expected_tip: Some(&second),
        },
    ];
    for case in cases {
        println!("CASE {}", case.name);
        while rx.try_recv().is_ok() {}
        let started = Instant::now();
        git(&repo, case.command)?;
        let matching = collect_events(&rx, started, &common.join(case.event_path))?;
        assert!(
            matching > 0,
            "no event for {} during {}",
            case.event_path,
            case.name
        );

        let tip = git(
            &repo,
            &["for-each-ref", "--format=%(objectname)", "refs/heads/topic"],
        )?;
        assert_eq!(tip, case.expected_tip.unwrap_or(""), "{}", case.name);
        let linked_tip = git(
            &linked,
            &["for-each-ref", "--format=%(objectname)", "refs/heads/topic"],
        )?;
        assert_eq!(linked_tip, tip, "worktrees disagree during {}", case.name);
        if case.name == "pack refs" {
            assert!(!common.join("refs/heads/topic").exists());
            assert!(
                std::fs::read_to_string(common.join("packed-refs"))?
                    .lines()
                    .any(|line| line == format!("{second} refs/heads/topic"))
            );
        }
        println!(
            "PASS {}: {matching} matching event(s); Git state verified",
            case.name
        );
    }

    println!("CASE watched directory disappears and returns");
    let relocation = tempfile::tempdir()?;
    let moved = relocation.path().join("moved");
    let started = Instant::now();
    std::fs::rename(&repo, &moved)?;
    collect_events(&rx, started, &common)?;
    git(&moved, &["update-ref", "refs/heads/topic", &third])?;
    assert_eq!(git(&moved, &["rev-parse", "refs/heads/topic"])?, third);
    let started = Instant::now();
    std::fs::rename(&moved, &repo)?;
    collect_events(&rx, started, &common)?;
    assert_eq!(git(&repo, &["rev-parse", "refs/heads/topic"])?, third);
    println!("PASS Git reread recovers the ref changed while the path was absent");

    while rx.try_recv().is_ok() {}
    let started = Instant::now();
    git(&repo, &["update-ref", "refs/heads/topic", &first])?;
    let matching = collect_events(&rx, started, &common.join("refs/heads/topic"))?;
    println!("OBSERVATION original watch after return: {matching} matching event(s)");

    println!("CASE reattach before reconciliation, without journal replay");
    let before = git(
        &repo,
        &["for-each-ref", "--format=%(refname) %(objectname)"],
    )?;
    drop(watcher);
    drop(rx);
    git(&repo, &["update-ref", "refs/heads/topic", &third])?;
    git(&repo, &["update-ref", "refs/heads/transient", &second])?;
    git(&repo, &["update-ref", "-d", "refs/heads/transient"])?;
    let (_watcher, rx) = watch(&common)?;
    let after = git(
        &repo,
        &["for-each-ref", "--format=%(refname) %(objectname)"],
    )?;
    assert_ne!(before, after);
    assert!(
        after
            .lines()
            .any(|line| line == format!("refs/heads/topic {third}"))
    );
    assert!(
        !after
            .lines()
            .any(|line| line.starts_with("refs/heads/transient "))
    );
    println!(
        "PASS reconciliation recovers current refs; the transient branch is not reconstructible from this snapshot"
    );

    // This delay isolates the next event check; the recovery read above does not wait for an event.
    std::thread::sleep(Duration::from_millis(500));
    while rx.try_recv().is_ok() {}
    let started = Instant::now();
    git(&repo, &["update-ref", "refs/heads/topic", &second])?;
    let matching = collect_events(&rx, started, &common.join("refs/heads/topic"))?;
    assert!(
        matching > 0,
        "reattached watcher did not observe the next update"
    );
    assert_eq!(git(&repo, &["rev-parse", "refs/heads/topic"])?, second);
    println!("PASS reattached watcher observes subsequent updates");
    Ok(())
}
