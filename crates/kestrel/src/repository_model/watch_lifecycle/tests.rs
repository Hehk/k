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
    assert_eq!(lifecycle.state, State::Starting { generation });
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
            generation: current
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
fn setup_failure_waits_for_release_and_request_before_retrying() {
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
    lifecycle.update(Input::Released);
    assert_eq!(lifecycle.error(), None);
    let generation = request(&mut lifecycle);
    lifecycle.update(Input::Started { generation });
    assert!(lifecycle.is_watching(generation));
    assert_eq!(lifecycle.error(), None);
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
