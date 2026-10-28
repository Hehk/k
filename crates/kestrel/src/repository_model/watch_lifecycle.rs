#[derive(Debug, Default, PartialEq, Eq)]
pub(super) enum State {
    #[default]
    Stopped,
    Starting {
        generation: u64,
    },
    Watching {
        generation: u64,
    },
    Failed {
        error: String,
    },
}

pub(super) enum Input {
    Requested,
    Released,
    Started { generation: u64 },
    Failed { generation: u64, error: String },
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Effect {
    Start { generation: u64 },
    Stop,
    RefreshRefs,
}

#[derive(Default)]
pub(super) struct Lifecycle {
    state: State,
    next_generation: u64,
}

impl Lifecycle {
    pub fn error(&self) -> Option<&str> {
        match &self.state {
            State::Failed { error } => Some(error),
            _ => None,
        }
    }

    pub fn is_watching(&self, generation: u64) -> bool {
        self.state == State::Watching { generation }
    }

    pub fn update(&mut self, input: Input) -> Vec<Effect> {
        match (&self.state, input) {
            (State::Stopped, Input::Requested) => {
                let generation = self.next_generation;
                self.next_generation = self.next_generation.wrapping_add(1);
                self.state = State::Starting { generation };
                vec![Effect::Start { generation }]
            }
            (
                State::Starting { generation },
                Input::Started {
                    generation: started,
                },
            ) if *generation == started => {
                self.state = State::Watching {
                    generation: started,
                };
                vec![Effect::RefreshRefs]
            }
            (
                State::Starting { generation },
                Input::Failed {
                    generation: failed,
                    error,
                },
            ) if *generation == failed => {
                self.state = State::Failed { error };
                vec![Effect::Stop]
            }
            (
                State::Watching { generation },
                Input::Failed {
                    generation: failed,
                    error,
                },
            ) if *generation == failed => {
                self.state = State::Failed { error };
                vec![Effect::Stop, Effect::RefreshRefs]
            }
            (State::Stopped, Input::Released) => vec![],
            (_, Input::Released) => {
                self.state = State::Stopped;
                vec![Effect::Stop]
            }
            // Repeated requests do not retry a failure; release/request is the explicit reset.
            // Completions from stopped or superseded generations are ignored.
            _ => vec![],
        }
    }
}

#[cfg(test)]
mod tests;
