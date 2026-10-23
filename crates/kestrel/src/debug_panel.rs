mod timing;

use std::{
    cell::RefCell,
    collections::VecDeque,
    rc::Rc,
    time::{Duration, Instant},
};

use gpui::{Context, IntoElement, Render, SharedString, Window, div, prelude::*, px};

use crate::{appearance::Theme, controls::tooltip};
pub use timing::install;

const DISPLAY_INTERVAL: Duration = Duration::from_millis(250);
const SAMPLE_WINDOW: Duration = Duration::from_secs(1);
const MAX_SAMPLES: usize = 512;
const MIN_PERCENTILE_SAMPLES: usize = 20;
const KEY_WIDTH: usize = 10;
const VALUE_WIDTH: usize = 12;
const HELP: &str = concat!(
    "App FPS counts completed scene builds, not frames presented by macOS. ",
    "Low FPS under sparse input is normal. ",
    "CPU draw is mean elapsed scene-building time, excluding GPU submission and presentation. ",
    "p95 needs 20 samples; otherwise max is shown. ",
    "This one-second snapshot updates only when the app redraws and stays unchanged while idle. ",
    "At the sample limit, FPS is a lower bound and cost statistics are unavailable."
);

type SharedSamples = Rc<RefCell<Samples>>;

#[derive(Default)]
struct Samples {
    frames: VecDeque<(Instant, Duration)>,
    started: Option<Instant>,
    truncated_at: Option<Instant>,
}

#[derive(Debug, Default, PartialEq)]
struct Summary {
    fps: Option<usize>,
    mean_ms: Option<f64>,
    tail_ms: Option<f64>,
    count: usize,
    limited: bool,
}

impl Summary {
    fn label(&self) -> SharedString {
        let count = self.count.to_string();
        let fps = match self.fps {
            None => "warming up".into(),
            Some(fps) if self.limited => format!(">={fps}"),
            Some(fps) => fps.to_string(),
        };
        [
            "Recent frames (1s)".into(),
            text_row("app FPS:", &fps),
            metric_row("CPU draw:", self.mean_ms),
            metric_row(
                if self.count >= MIN_PERCENTILE_SAMPLES {
                    "p95 draw:"
                } else {
                    "max draw:"
                },
                self.tail_ms,
            ),
            text_row(
                "samples:",
                &if self.limited {
                    format!(">={count}")
                } else {
                    count
                },
            ),
        ]
        .join("\n")
        .into()
    }
}

fn text_row(key: &str, value: &str) -> String {
    format!("{key:<KEY_WIDTH$}{value:>VALUE_WIDTH$}")
}

fn metric_row(key: &str, value: Option<f64>) -> String {
    let value = value.map_or_else(
        || "-".into(),
        |value| {
            let decimal = format!("{value:.1} ms");
            if decimal.len() > VALUE_WIDTH {
                format!("{value:.1e} ms")
            } else {
                decimal
            }
        },
    );
    text_row(key, &value)
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
        self.started.get_or_insert(completed - duration);
        while self
            .frames
            .front()
            .is_some_and(|(time, _)| completed.duration_since(*time) >= SAMPLE_WINDOW)
        {
            self.frames.pop_front();
        }
        if self.frames.len() == MAX_SAMPLES {
            self.truncated_at = self.frames.pop_front().map(|(time, _)| time);
        }
        self.frames.push_back((completed, duration));
    }

    fn summary(&self, now: Instant) -> Summary {
        let mut costs: Vec<_> = self
            .frames
            .iter()
            .filter(|(time, _)| now.duration_since(*time) < SAMPLE_WINDOW)
            .map(|(_, duration)| duration.as_secs_f64() * 1000.)
            .collect();
        let count = costs.len();
        let limited = self
            .truncated_at
            .is_some_and(|time| now.duration_since(time) < SAMPLE_WINDOW);
        let fps = self
            .started
            .filter(|start| now.duration_since(*start) >= SAMPLE_WINDOW)
            .map(|_| count);
        // A capped window gives only a lower bound on FPS. Do not present a
        // biased subset of draw costs as statistics for the whole window.
        let (mean_ms, tail_ms) = if count == 0 || limited {
            (None, None)
        } else {
            let mean = costs.iter().sum::<f64>() / count as f64;
            costs.sort_unstable_by(f64::total_cmp);
            let rank = if count >= MIN_PERCENTILE_SAMPLES {
                (count * 95).div_ceil(100)
            } else {
                count
            };
            (Some(mean), Some(costs[rank - 1]))
        };
        Summary {
            fps,
            mean_ms,
            tail_ms,
            count,
            limited,
        }
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

        self.label = self.samples.borrow().summary(now).label();
        self.last_update = Some(now);
    }
}

impl Render for DebugPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::get(window, cx);
        // Associate the enclosing GPUI draw span with this window's samples.
        // The span, not this view's render method, measures the whole scene build.
        timing::attach(self.samples.clone());
        self.update_label(Instant::now());
        div()
            .id("debug-panel")
            .debug_selector(|| "debug-panel".into())
            .tooltip(tooltip(HELP))
            .absolute()
            .bottom_4()
            .right_4()
            .px_2()
            .py_1()
            .rounded_md()
            .border_1()
            .border_color(theme.border)
            .bg(theme.paper)
            .text_color(theme.muted)
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
