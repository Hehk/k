mod timing;

use std::{
    cell::RefCell,
    collections::VecDeque,
    rc::Rc,
    time::{Duration, Instant},
};

use gpui::{Context, IntoElement, Render, SharedString, Window, div, prelude::*, px, rgb};

pub use timing::install;

const DISPLAY_INTERVAL: Duration = Duration::from_millis(250);
const SAMPLE_WINDOW: Duration = Duration::from_secs(1);
const MAX_SAMPLES: usize = 512;
const KEY_WIDTH: usize = 10;
const VALUE_WIDTH: usize = 8;

type SharedSamples = Rc<RefCell<Samples>>;

#[derive(Default)]
struct Samples {
    frames: VecDeque<(Instant, Duration)>,
    previous_frame: Option<Instant>,
}

#[derive(Debug, Default, PartialEq)]
struct Summary {
    renders_per_second: Option<f64>,
    estimated_fps: Option<f64>,
}

impl Summary {
    fn label(&self) -> SharedString {
        [
            metric_row("fps:", self.estimated_fps, 0),
            metric_row("render/s:", self.renders_per_second, 1),
        ]
        .join("\n")
        .into()
    }
}

fn metric_row(key: &str, value: Option<f64>, precision: usize) -> String {
    let value = value.map_or_else(
        || "-".into(),
        |value| {
            let decimal = format!("{value:.precision$}");
            if decimal.len() > VALUE_WIDTH {
                format!("{value:.1e}")
            } else {
                decimal
            }
        },
    );
    format!("{key:<KEY_WIDTH$}{value:>VALUE_WIDTH$}")
}

fn right_align_rows(label: &str) -> SharedString {
    let width = label
        .lines()
        .map(|row| row.chars().count())
        .max()
        .unwrap_or(0);
    label
        .lines()
        .map(|row| format!("{row:>width$}"))
        .collect::<Vec<_>>()
        .join("\n")
        .into()
}

impl Samples {
    fn record(&mut self, completed: Instant, duration: Duration) {
        while self.frames.front().is_some_and(|(time, _)| {
            completed.duration_since(*time) >= SAMPLE_WINDOW || self.frames.len() >= MAX_SAMPLES
        }) {
            self.previous_frame = self.frames.pop_front().map(|(time, _)| time);
        }
        self.frames.push_back((completed, duration));
    }

    fn summary(&self) -> Option<Summary> {
        let (first, _) = self.frames.front()?;
        let (last, _) = self.frames.back()?;
        // Keep the preceding completion as an interval boundary, including long gaps.
        let (start, intervals) = match self.previous_frame {
            Some(previous) => (previous, self.frames.len()),
            None => (*first, self.frames.len() - 1),
        };
        let elapsed = last.duration_since(start);
        let total: Duration = self.frames.iter().map(|(_, duration)| *duration).sum();
        Some(Summary {
            renders_per_second: (!elapsed.is_zero())
                .then(|| intervals as f64 / elapsed.as_secs_f64()),
            estimated_fps: (!total.is_zero())
                .then(|| self.frames.len() as f64 / total.as_secs_f64()),
        })
    }
}

pub struct DebugPanel {
    samples: SharedSamples,
    label: SharedString,
    last_update: Option<Instant>,
}

impl Default for DebugPanel {
    fn default() -> Self {
        Self {
            samples: Rc::default(),
            label: Summary::default().label(),
            last_update: None,
        }
    }
}

impl DebugPanel {
    fn update_label(&mut self, now: Instant) {
        if self
            .last_update
            .is_some_and(|last| now.duration_since(last) < DISPLAY_INTERVAL)
        {
            return;
        }

        if let Some(summary) = self.samples.borrow().summary() {
            self.label = summary.label();
            self.last_update = Some(now);
        }
    }
}

impl Render for DebugPanel {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        // Associate the enclosing GPUI draw span with this window's samples.
        // The span, not this view's render method, measures the whole scene build.
        timing::attach(self.samples.clone());
        self.update_label(Instant::now());
        div()
            .id("debug-panel")
            .debug_selector(|| "debug-panel".into())
            .absolute()
            .bottom_2()
            .right_2()
            .px_2()
            .py_1()
            .rounded_md()
            .bg(rgb(0x242830))
            .text_color(rgb(0xe4e7ec))
            .text_size(px(11.))
            .font_family(if cfg!(target_os = "macos") {
                "Menlo"
            } else {
                "monospace"
            })
            .whitespace_nowrap()
            .child(right_align_rows(&format!(
                "branch: {}\n{}",
                env!("KESTREL_BRANCH"),
                self.label
            )))
    }
}

#[cfg(test)]
mod tests;
