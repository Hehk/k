use super::*;
use serde_json::json;

const OLD: &str = "fn print_review(enabled: bool, files: &[&str]) {\n    println!(\"Review summary\");\n    println!(\"{} changed files\", files.len());\n\n    for file in files {\n        println!(\"  {file}\");\n    }\n\n    println!(\"Ready for review\");\n}\n";
const NEW: &str = "fn print_review(enabled: bool, files: &[&str]) {\n    if enabled {\n        println!(\"Review summary\");\n        println!(\"{} changed files\", files.len());\n\n        for file in files {\n            println!(\"  {file}\");\n        }\n\n        println!(\"Ready for review\");\n    }\n}\n";

fn input<'a>(old: &'a str, new: &'a str) -> Input<'a> {
    Input {
        old,
        new,
        path: "test.rs",
    }
}

fn compare(old: &str, new: &str) -> Diff {
    Engine::bundled()
        .compare(
            input(old, new),
            Limits {
                timeout: Duration::from_secs(5),
                ..Default::default()
            },
            &Cancellation::default(),
        )
        .expect("run tests in nix develop for the pinned Difftastic helper")
}

fn assert_sources(diff: &Diff, old: &str, new: &str) {
    let side = |old: bool| {
        diff.lines
            .iter()
            .filter_map(|line| {
                if old {
                    line.old.as_ref()
                } else {
                    line.new.as_ref()
                }
            })
            .map(|line| line.text.as_str())
            .collect::<String>()
    };
    assert_eq!(side(true), old);
    assert_eq!(side(false), new);
    for line in diff
        .lines
        .iter()
        .flat_map(|line| line.old.iter().chain(&line.new))
    {
        for range in &line.changes {
            assert!(line.text.get(range.clone()).is_some());
        }
    }
}

#[test]
fn real_helper_highlights_the_condition_not_the_reindented_block() {
    let diff = compare(OLD, NEW);
    assert_eq!(diff.language, "Rust");
    assert!(diff.has_changes);
    assert_sources(&diff, OLD, NEW);
    assert!(
        diff.lines
            .iter()
            .filter_map(|line| line.old.as_ref())
            .all(|line| line.changes.is_empty())
    );
    let novel = diff
        .lines
        .iter()
        .filter_map(|line| line.new.as_ref())
        .flat_map(|line| line.changes.iter().map(|range| &line.text[range.clone()]))
        .collect::<Vec<_>>();
    assert_eq!(novel, ["if", "enabled", "{", "}"]);
}

#[test]
fn real_helper_preserves_source_coordinates_and_normalization_edges() {
    for (old, new) in [
        ("fn main() { foo(1); }\n", "fn main() {\n    foo(2);\n}\n"),
        (
            "fn main() { let café = \"🦀\"; foo(1); }\n",
            "fn main() { let café = \"🦀\"; foo(2); }\n",
        ),
        ("fn main() {\n\tfoo(1);\n}\n", "fn main() {\n\tfoo(2);\n}\n"),
        ("fn main() {\rfoo(1); }\r\n", "fn main() {\rfoo(2); }\r\n"),
        (
            "\u{feff}fn main() { foo(1); }",
            "\u{feff}fn main() { foo(2); }",
        ),
        (
            "\nfn main() {\n\n    foo(1);\n\n}\n\n",
            "\n\nfn main() {\n    foo(2);\n\n}\n",
        ),
        ("", "fn main() {}\n"),
        ("fn main() {}", ""),
        ("fn main() {}\n", "// a comment\nfn main() {}\n"),
    ] {
        let diff = compare(old, new);
        assert!(diff.has_changes);
        assert_sources(&diff, old, new);
    }
    for (old, new) in [
        ("fn main() { foo(1); }\n", "fn main() {\n    foo(1);\n}\n"),
        ("fn main() {}", "fn main() {}\n"),
        ("", ""),
        ("", "\n \n"),
    ] {
        let diff = compare(old, new);
        assert!(!diff.has_changes);
        assert!(diff.lines.is_empty());
    }
}

#[test]
fn real_helper_reports_text_and_parse_fallbacks() {
    let engine = Engine::bundled();
    let token = Cancellation::default();
    assert!(matches!(
        engine.compare(
            Input {
                old: "old\n",
                new: "new\n",
                path: "unknown.xyz-kestrel"
            },
            Limits::default(),
            &token
        ),
        Err(Error::Unsupported)
    ));
    assert!(
        matches!(engine.compare(input("fn main() {}\n", "fn main() { @@@ }\n"), Limits::default(), &token), Err(Error::TextFallback(reason)) if reason.contains("parse error"))
    );
}

#[test]
fn real_helper_bounds_large_changes_and_json_output() {
    let engine = Engine::bundled();
    let token = Cancellation::default();
    let old = (0..2000)
        .map(|i| format!("fn f{i}() {{ foo(1); }}\n"))
        .collect::<String>();
    let new = (0..2000)
        .map(|i| format!("fn g{i}() {{ bar(2); }}\n"))
        .collect::<String>();
    assert!(
        matches!(engine.compare(input(&old, &new), Limits {timeout: Duration::from_secs(5), ..Default::default()}, &token), Err(Error::TextFallback(reason)) if reason.contains("GRAPH_LIMIT"))
    );
    assert!(matches!(
        engine.compare(
            input("fn main() { foo(1); }\n", "fn main() { foo(2); }\n"),
            Limits {
                output_bytes: 128,
                ..Default::default()
            },
            &token
        ),
        Err(Error::Limit)
    ));
}

fn token_json() -> serde_json::Value {
    json!({
        "language":"Rust", "path":"test.rs", "status":"changed",
        "aligned_lines":[[0,0],[1,1]],
        "chunks":[[{"lhs":{"line_number":0,"changes":[{"start":16,"end":17,"content":"1","highlight":"keyword"}]},
        "rhs":{"line_number":0,"changes":[{"start":16,"end":17,"content":"2","highlight":"keyword"}]}}]]
    })
}

#[test]
fn decoder_rejects_bad_schema_coordinates_alignment_and_status() {
    let old = "fn main() { foo(1); }\n";
    let new = "fn main() { foo(2); }\n";
    let decode = |value: serde_json::Value| {
        difftastic::decode(
            &serde_json::to_vec(&value).unwrap(),
            input(old, new),
            Limits::default(),
        )
    };
    assert!(decode(token_json()).is_ok());
    for (pointer, replacement) in [
        ("/status", json!("mystery")),
        ("/path", json!("other.rs")),
        ("/chunks/0/0/lhs/line_number", json!(900)),
        ("/chunks/0/0/lhs/changes/0/end", json!(999)),
        ("/chunks/0/0/lhs/changes/0/content", json!("wrong")),
        ("/chunks/0/0/lhs/changes/0/highlight", json!("unknown")),
        ("/aligned_lines", json!([[0, 0], [0, 1]])),
        ("/aligned_lines", json!([[1, 1]])),
        ("/aligned_lines", json!([[0, 0], [1, 1], [2, 2]])),
        ("/aligned_lines", json!([[null, null]])),
        ("/status", json!("unchanged")),
        ("/status", json!("created")),
    ] {
        let mut value = token_json();
        *value.pointer_mut(pointer).unwrap() = replacement;
        assert!(
            matches!(decode(value), Err(Error::InvalidOutput)),
            "{pointer}"
        );
    }
    let mut value = token_json();
    value["chunks"][0]
        .as_array_mut()
        .unwrap()
        .push(token_json()["chunks"][0][0].clone());
    assert!(matches!(decode(value), Err(Error::InvalidOutput)));
    let mut value = token_json();
    value["extra"] = json!(true);
    assert!(matches!(decode(value), Err(Error::InvalidOutput)));
    let mut value = token_json();
    value["chunks"][0][0]["lhs"]["changes"][0] =
        json!({"start": 0, "end": 1, "content":"é", "highlight":"normal"});
    assert!(matches!(
        difftastic::decode(
            &serde_json::to_vec(&value).unwrap(),
            input("é\n", "e\n"),
            Limits::default()
        ),
        Err(Error::InvalidOutput)
    ));
}

#[test]
fn limits_and_cancellation_are_checked_before_launch() {
    let engine = Engine::new("/missing/difft".into());
    let cancellation = Cancellation::default();
    assert!(matches!(
        engine.compare(
            input("abc", "def"),
            Limits {
                source_bytes: 2,
                ..Default::default()
            },
            &cancellation
        ),
        Err(Error::Limit)
    ));
    assert!(matches!(
        engine.compare(
            input("a\nb\n", ""),
            Limits {
                lines: 1,
                ..Default::default()
            },
            &cancellation
        ),
        Err(Error::Limit)
    ));
    assert!(matches!(
        engine.compare(input("a\0", ""), Limits::default(), &cancellation),
        Err(Error::Unsupported)
    ));
    cancellation.cancel();
    assert!(matches!(
        engine.compare(input("a", "b"), Limits::default(), &cancellation),
        Err(Error::Cancelled)
    ));
}

#[cfg(unix)]
mod supervision {
    use super::*;
    use std::{fs, os::unix::fs::PermissionsExt, process::Command, thread, time::Instant};

    fn helper(body: &str) -> (tempfile::TempDir, Engine) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("difft");
        fs::write(&path, format!("#!/bin/sh\nif [ \"$1\" = '--version' ]; then printf 'Difftastic 0.71.0\\n'; exit; fi\n{body}\n")).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        (dir, Engine::new(path))
    }

    #[test]
    fn failures_and_output_limits_are_explicit() {
        for (body, expected) in [
            ("exit 9", "failed"),
            ("printf 'bad json'", "invalid"),
            ("while :; do printf '12345678901234567890'; done", "limit"),
        ] {
            let (_dir, engine) = helper(body);
            let result = engine.compare(
                input("old", "new"),
                Limits {
                    output_bytes: 128,
                    ..Default::default()
                },
                &Cancellation::default(),
            );
            assert!(match expected {
                "failed" => matches!(result, Err(Error::Failed)),
                "invalid" => matches!(result, Err(Error::InvalidOutput)),
                _ => matches!(result, Err(Error::Limit)),
            });
        }
        let (dir, engine) = helper("exit 0");
        fs::write(
            dir.path().join("difft"),
            "#!/bin/sh\nprintf 'Difftastic 0.70.0\\n'\n",
        )
        .unwrap();
        assert!(matches!(
            engine.compare(input("a", "b"), Limits::default(), &Cancellation::default()),
            Err(Error::Version)
        ));
        assert!(matches!(
            Engine::new("/missing/kestrel-difft".into()).compare(
                input("a", "b"),
                Limits::default(),
                &Cancellation::default()
            ),
            Err(Error::Unavailable)
        ));
    }

    #[test]
    fn timeout_and_cancellation_reap_the_child_and_remove_sources() {
        for cancel in [false, true] {
            let records = tempfile::tempdir().unwrap();
            let pid_path = records.path().join("pid");
            let cwd_path = records.path().join("cwd");
            let (_dir, engine) = helper(&format!(
                "printf '%s' \"$$\" > '{}'\nprintf '%s' \"$PWD\" > '{}'\nexec /bin/sleep 30",
                pid_path.display(),
                cwd_path.display()
            ));
            let cancellation = Cancellation::default();
            let request = cancellation.clone();
            let worker = thread::spawn(move || {
                engine.compare(
                    input("old", "new"),
                    Limits {
                        timeout: Duration::from_millis(if cancel { 5000 } else { 500 }),
                        ..Default::default()
                    },
                    &request,
                )
            });
            let deadline = Instant::now() + Duration::from_secs(5);
            while fs::metadata(&cwd_path).map_or(true, |metadata| metadata.len() == 0)
                && Instant::now() < deadline
            {
                thread::sleep(Duration::from_millis(5));
            }
            assert!(fs::metadata(&cwd_path).is_ok_and(|metadata| metadata.len() > 0));
            if cancel {
                cancellation.cancel();
            }
            let result = worker.join().unwrap();
            assert!(if cancel {
                matches!(result, Err(Error::Cancelled))
            } else {
                matches!(result, Err(Error::Timeout))
            });
            let pid: u32 = fs::read_to_string(pid_path).unwrap().parse().unwrap();
            let probe = Command::new("/bin/ps")
                .args(["-p", &pid.to_string(), "-o", "pid="])
                .output()
                .unwrap();
            assert_eq!(
                probe.status.code(),
                Some(1),
                "helper must be killed and reaped"
            );
            assert!(probe.stdout.is_empty());
            assert!(!PathBuf::from(fs::read_to_string(cwd_path).unwrap()).exists());
        }
    }

    #[test]
    fn parent_environment_and_git_attributes_do_not_change_the_result() {
        const CHILD: &str = "KESTREL_DIFF_ENVIRONMENT_TEST";
        if std::env::var_os(CHILD).is_none() {
            let output = Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "tests::supervision::parent_environment_and_git_attributes_do_not_change_the_result", "--nocapture"])
                .env(CHILD, "1").env("DFT_IGNORE_COMMENTS", "true").env("DFT_CHECK_ONLY", "true")
                .env("DFT_OVERRIDE", "*:text").env("DFT_EXIT_CODE", "true").env("GIT_DIR", "/missing/git-dir")
                .output().unwrap();
            let stdout = String::from_utf8_lossy(&output.stdout);
            assert!(output.status.success(), "{stdout}");
            assert!(
                stdout.contains("1 passed"),
                "child test must actually run: {stdout}"
            );
            return;
        }
        let repo = tempfile::tempdir().unwrap();
        git2::Repository::init(repo.path()).unwrap();
        fs::write(repo.path().join(".gitattributes"), "*.rs -diff\n").unwrap();
        let path = repo.path().join("test.rs");
        let diff = Engine::bundled()
            .compare(
                Input {
                    old: "// old\nfn main() {}\n",
                    new: "// new\nfn main() {}\n",
                    path: path.to_str().unwrap(),
                },
                Limits {
                    timeout: Duration::from_secs(5),
                    ..Default::default()
                },
                &Cancellation::default(),
            )
            .unwrap();
        assert_eq!(diff.language, "Rust");
        assert!(diff.has_changes);
        assert_eq!(std::env::var("DFT_CHECK_ONLY").unwrap(), "true");
        assert_eq!(
            fs::read_to_string(repo.path().join(".gitattributes")).unwrap(),
            "*.rs -diff\n"
        );
    }
}
