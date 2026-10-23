use super::*;

#[test]
fn invalidations_schedule_one_fixed_delay() {
    let mut scheduler = Scheduler::default();
    assert_eq!(scheduler.phase, Phase::Idle);
    let Some(Effect::ScheduleDelay(timer)) = scheduler.update(Input::Invalidated) else {
        panic!()
    };
    for _ in 0..100 {
        assert_eq!(scheduler.update(Input::Invalidated), None);
    }
    assert_eq!(scheduler.phase, Phase::Scheduled { timer });
    assert_eq!(
        scheduler.update(Input::DelayElapsed(timer)),
        Some(Effect::ReadRefs)
    );
    assert_eq!(scheduler.phase, Phase::Reading { invalidated: false });
    assert_eq!(scheduler.update(Input::ReadFinished), None);
    assert_eq!(scheduler.phase, Phase::Idle);
}

#[test]
fn requests_during_a_read_queue_exactly_one_followup() {
    let mut scheduler = Scheduler::default();
    assert_eq!(
        scheduler.update(Input::RefreshRequested),
        Some(Effect::ReadRefs)
    );
    for _ in 0..100 {
        assert_eq!(scheduler.update(Input::Invalidated), None);
        assert_eq!(scheduler.update(Input::RefreshRequested), None);
    }
    assert_eq!(scheduler.phase, Phase::Reading { invalidated: true });
    assert_eq!(
        scheduler.update(Input::ReadFinished),
        Some(Effect::ReadRefs)
    );
    assert_eq!(scheduler.phase, Phase::Reading { invalidated: false });
    assert_eq!(scheduler.update(Input::ReadFinished), None);
    assert_eq!(scheduler.phase, Phase::Idle);
}

#[test]
fn manual_refresh_bypasses_delay_and_old_expirations_cannot_start_reads() {
    let mut scheduler = Scheduler::default();
    let Some(Effect::ScheduleDelay(old)) = scheduler.update(Input::Invalidated) else {
        panic!()
    };
    assert_eq!(
        scheduler.update(Input::RefreshRequested),
        Some(Effect::ReadRefs)
    );
    assert_eq!(scheduler.update(Input::DelayElapsed(old)), None);
    assert_eq!(scheduler.update(Input::ReadFinished), None);
    assert_eq!(scheduler.update(Input::DelayElapsed(old)), None);

    let Some(Effect::ScheduleDelay(current)) = scheduler.update(Input::Invalidated) else {
        panic!()
    };
    assert_ne!(old, current);
    assert_eq!(scheduler.update(Input::DelayElapsed(old)), None);
    assert_eq!(scheduler.phase, Phase::Scheduled { timer: current });
    assert_eq!(
        scheduler.update(Input::DelayElapsed(current)),
        Some(Effect::ReadRefs)
    );
    assert_eq!(scheduler.update(Input::DelayElapsed(current)), None);
}

#[test]
fn invalidating_the_followup_read_queues_another_read() {
    let mut scheduler = Scheduler::default();
    scheduler.update(Input::RefreshRequested);
    scheduler.update(Input::Invalidated);
    assert_eq!(
        scheduler.update(Input::ReadFinished),
        Some(Effect::ReadRefs)
    );
    scheduler.update(Input::Invalidated);
    assert_eq!(
        scheduler.update(Input::ReadFinished),
        Some(Effect::ReadRefs)
    );
    assert_eq!(scheduler.update(Input::ReadFinished), None);
}

#[test]
fn unrelated_completions_do_not_change_idle_or_scheduled_state() {
    let mut scheduler = Scheduler::default();
    assert_eq!(scheduler.update(Input::ReadFinished), None);
    assert_eq!(scheduler.update(Input::DelayElapsed(99)), None);
    assert_eq!(scheduler.phase, Phase::Idle);
    let Some(Effect::ScheduleDelay(timer)) = scheduler.update(Input::Invalidated) else {
        panic!()
    };
    assert_eq!(scheduler.update(Input::ReadFinished), None);
    assert_eq!(scheduler.phase, Phase::Scheduled { timer });
}
