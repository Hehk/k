use super::*;

fn request(lifecycle: &mut Lifecycle) -> u64 {
    let effects = lifecycle.update(Input::Requested);
    let [Effect::Start { generation }] = effects.as_slice() else {
        panic!("expected one start effect")
    };
    *generation
}

#[test]
fn attachment_requests_reconciliation_once() {
    let mut lifecycle = Lifecycle::default();
    let generation = request(&mut lifecycle);
    assert_eq!(
        lifecycle.state,
        State::Starting {
            generation,
            error: None,
        }
    );
    assert!(lifecycle.update(Input::Requested).is_empty());
    assert_eq!(
        lifecycle.update(Input::Started { generation }),
        vec![Effect::RefreshRefs]
    );
    assert!(lifecycle.is_watching(generation));
    assert!(lifecycle.update(Input::Requested).is_empty());
    assert!(lifecycle.update(Input::Started { generation }).is_empty());
    assert_eq!(lifecycle.update(Input::Released), vec![Effect::Stop]);
    assert_eq!(lifecycle.state, State::Stopped);
    assert!(lifecycle.update(Input::Released).is_empty());
}

#[test]
fn release_during_setup_rejects_the_old_completion() {
    let mut lifecycle = Lifecycle::default();
    let old = request(&mut lifecycle);
    assert_eq!(lifecycle.update(Input::Released), vec![Effect::Stop]);
    assert!(
        lifecycle
            .update(Input::Started { generation: old })
            .is_empty()
    );
    let current = request(&mut lifecycle);
    assert_ne!(current, old);
    assert!(
        lifecycle
            .update(Input::Started { generation: old })
            .is_empty()
    );
    assert!(
        lifecycle
            .update(Input::Failed {
                generation: old,
                error: "old setup".into()
            })
            .is_empty()
    );
    assert_eq!(
        lifecycle.state,
        State::Starting {
            generation: current,
            error: None,
        }
    );
    lifecycle.update(Input::Started {
        generation: current,
    });
    assert!(lifecycle.is_watching(current));
    assert!(!lifecycle.is_watching(old));
    assert!(
        lifecycle
            .update(Input::Failed {
                generation: old,
                error: "old runtime".into()
            })
            .is_empty()
    );
    assert!(lifecycle.is_watching(current));
}

#[test]
fn setup_failure_retains_its_error_until_retry_succeeds() {
    let mut lifecycle = Lifecycle::default();
    let generation = request(&mut lifecycle);
    assert_eq!(
        lifecycle.update(Input::Failed {
            generation,
            error: "setup failed".into()
        }),
        vec![Effect::Stop]
    );
    assert_eq!(lifecycle.error(), Some("setup failed"));
    assert!(lifecycle.update(Input::Requested).is_empty());
    assert!(lifecycle.update(Input::Started { generation }).is_empty());
    let effects = lifecycle.update(Input::RetryRequested);
    let [Effect::Start { generation: retry }] = effects.as_slice() else {
        panic!("expected a retry")
    };
    assert_ne!(*retry, generation);
    assert_eq!(lifecycle.error(), Some("setup failed"));
    assert!(lifecycle.update(Input::RetryRequested).is_empty());
    assert!(lifecycle.update(Input::Started { generation }).is_empty());
    assert!(
        lifecycle
            .update(Input::Failed {
                generation,
                error: "obsolete failure".into(),
            })
            .is_empty()
    );
    assert_eq!(lifecycle.error(), Some("setup failed"));
    assert_eq!(
        lifecycle.update(Input::Started { generation: *retry }),
        vec![Effect::RefreshRefs]
    );
    assert!(lifecycle.is_watching(*retry));
    assert_eq!(lifecycle.error(), None);
}

#[test]
fn retries_only_start_failed_watchers() {
    let mut lifecycle = Lifecycle::default();
    assert!(lifecycle.update(Input::RetryRequested).is_empty());
    let generation = request(&mut lifecycle);
    assert!(lifecycle.update(Input::RetryRequested).is_empty());
    lifecycle.update(Input::Started { generation });
    assert!(lifecycle.update(Input::RetryRequested).is_empty());
    lifecycle.update(Input::Failed {
        generation,
        error: "failed".into(),
    });
    lifecycle.update(Input::Released);
    assert!(lifecycle.update(Input::RetryRequested).is_empty());
    assert_eq!(lifecycle.state, State::Stopped);
    assert_eq!(lifecycle.error(), None);
}

#[test]
fn failed_retries_wait_for_another_recovery_request() {
    let mut lifecycle = Lifecycle::default();
    let mut generation = request(&mut lifecycle);
    for attempt in 0..10 {
        let error = format!("failure {attempt}");
        assert_eq!(
            lifecycle.update(Input::Failed {
                generation,
                error: error.clone(),
            }),
            vec![Effect::Stop]
        );
        assert_eq!(lifecycle.error(), Some(error.as_str()));
        assert!(lifecycle.update(Input::Requested).is_empty());
        let effects = lifecycle.update(Input::RetryRequested);
        let [Effect::Start { generation: retry }] = effects.as_slice() else {
            panic!("expected a retry")
        };
        assert_ne!(*retry, generation);
        generation = *retry;
    }
    lifecycle.update(Input::Released);
    assert!(lifecycle.update(Input::Started { generation }).is_empty());
    assert!(
        lifecycle
            .update(Input::Failed {
                generation,
                error: "late retry failure".into(),
            })
            .is_empty()
    );
    assert_eq!(lifecycle.state, State::Stopped);
}

#[test]
fn runtime_failure_stops_watching_and_requests_a_git_reread() {
    let mut lifecycle = Lifecycle::default();
    let generation = request(&mut lifecycle);
    lifecycle.update(Input::Started { generation });
    assert_eq!(
        lifecycle.update(Input::Failed {
            generation,
            error: "watch failed".into()
        }),
        vec![Effect::Stop, Effect::RefreshRefs]
    );
    assert!(!lifecycle.is_watching(generation));
    assert_eq!(lifecycle.error(), Some("watch failed"));
    assert!(lifecycle.update(Input::Requested).is_empty());
    assert!(
        lifecycle
            .update(Input::Failed {
                generation,
                error: "late error".into()
            })
            .is_empty()
    );
    assert_eq!(lifecycle.error(), Some("watch failed"));
}
