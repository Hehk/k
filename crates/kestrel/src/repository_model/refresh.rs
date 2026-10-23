#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Phase {
    #[default]
    Idle,
    Scheduled {
        timer: u64,
    },
    Reading {
        invalidated: bool,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Input {
    Invalidated,
    RefreshRequested,
    DelayElapsed(u64),
    ReadFinished,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Effect {
    ScheduleDelay(u64),
    ReadRefs,
}

#[derive(Default)]
pub(super) struct Scheduler {
    phase: Phase,
    next_timer: u64,
}

impl Scheduler {
    pub fn update(&mut self, input: Input) -> Option<Effect> {
        match (self.phase, input) {
            (Phase::Idle, Input::Invalidated) => {
                let timer = self.next_timer;
                self.next_timer = self.next_timer.wrapping_add(1);
                self.phase = Phase::Scheduled { timer };
                Some(Effect::ScheduleDelay(timer))
            }
            (Phase::Idle | Phase::Scheduled { .. }, Input::RefreshRequested) => self.start_read(),
            (Phase::Scheduled { timer }, Input::DelayElapsed(expired)) if timer == expired => {
                self.start_read()
            }
            (Phase::Reading { .. }, Input::Invalidated | Input::RefreshRequested) => {
                self.phase = Phase::Reading { invalidated: true };
                None
            }
            (Phase::Reading { invalidated: true }, Input::ReadFinished) => self.start_read(),
            (Phase::Reading { invalidated: false }, Input::ReadFinished) => {
                self.phase = Phase::Idle;
                None
            }
            _ => None,
        }
    }

    fn start_read(&mut self) -> Option<Effect> {
        self.phase = Phase::Reading { invalidated: false };
        Some(Effect::ReadRefs)
    }
}

#[cfg(test)]
mod tests;
