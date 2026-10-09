use std::{cell::RefCell, time::Instant};

use tracing::{Subscriber, span::Id};
use tracing_subscriber::{Layer, filter::filter_fn, layer::Context, prelude::*};

use super::SharedSamples;

struct Draw {
    id: Id,
    started: Instant,
    samples: Option<SharedSamples>,
}

thread_local! {
    static DRAWS: RefCell<Vec<Draw>> = const { RefCell::new(Vec::new()) };
}

struct DrawTimings;

impl<S: Subscriber> Layer<S> for DrawTimings {
    fn on_enter(&self, id: &Id, _: Context<'_, S>) {
        DRAWS.with_borrow_mut(|draws| {
            draws.push(Draw {
                id: id.clone(),
                started: Instant::now(),
                samples: None,
            });
        });
    }

    fn on_exit(&self, id: &Id, _: Context<'_, S>) {
        let completed = Instant::now();
        let draw = DRAWS.with_borrow_mut(|draws| draws.pop());
        if let Some(draw) = draw {
            debug_assert_eq!(&draw.id, id);
            if let Some(samples) = draw.samples {
                samples
                    .borrow_mut()
                    .record(completed, completed.duration_since(draw.started));
            }
        }
    }
}

pub(super) fn attach(samples: SharedSamples) {
    DRAWS.with_borrow_mut(|draws| {
        if let Some(draw) = draws.last_mut() {
            draw.samples = Some(samples);
        }
    });
}

pub(super) fn subscriber() -> impl Subscriber + Send + Sync {
    tracing_subscriber::registry().with(DrawTimings.with_filter(filter_fn(|metadata| {
        metadata.is_span() && metadata.target() == "gpui::window" && metadata.name() == "draw"
    })))
}

pub fn install() {
    tracing::subscriber::set_global_default(subscriber()).expect("install UI frame timings");
}
