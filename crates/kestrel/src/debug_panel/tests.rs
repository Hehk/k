use super::*;
use gpui::{Entity, Modifiers, TestAppContext, VisualTestContext};

#[test]
fn render_rate_changes_with_activity_but_estimated_fps_does_not() {
    let start = Instant::now();
    for spacing in [Duration::from_millis(8), Duration::from_secs(2)] {
        let mut samples = Samples::default();
        for frame in 0..20 {
            samples.record(start + spacing * frame, Duration::from_millis(2));
        }
        assert_eq!(
            samples.summary(),
            Some(Summary {
                renders_per_second: Some(1. / spacing.as_secs_f64()),
                estimated_fps: Some(500.),
            })
        );
    }
}

#[test]
fn estimated_fps_changes_with_cost_but_render_rate_does_not() {
    let start = Instant::now();
    for cost in [Duration::from_millis(2), Duration::from_millis(8)] {
        let mut samples = Samples::default();
        for frame in 0..=100 {
            samples.record(start + Duration::from_millis(20) * frame, cost);
        }
        assert_eq!(
            samples.summary(),
            Some(Summary {
                renders_per_second: Some(50.),
                estimated_fps: Some(1. / cost.as_secs_f64()),
            })
        );
    }
}

#[test]
fn long_frames_are_measured_not_discarded_as_idle() {
    let start = Instant::now();
    let mut samples = Samples::default();
    samples.record(start, Duration::from_millis(2));
    samples.record(
        start + Duration::from_millis(600),
        Duration::from_millis(600),
    );
    assert_eq!(
        samples.summary(),
        Some(Summary {
            renders_per_second: Some(1. / 0.6),
            estimated_fps: Some(2. / 0.602),
        })
    );
}

#[test]
fn expiring_cost_samples_preserves_the_preceding_render_interval() {
    let start = Instant::now();
    let mut samples = Samples::default();
    samples.record(start, Duration::from_millis(600));
    samples.record(start + SAMPLE_WINDOW, Duration::from_millis(2));
    assert_eq!(samples.frames.len(), 1);
    assert_eq!(
        samples.summary(),
        Some(Summary {
            renders_per_second: Some(1.),
            estimated_fps: Some(500.),
        })
    );
}

#[test]
fn storage_limit_does_not_underestimate_render_rate() {
    let start = Instant::now();
    let mut samples = Samples::default();
    for frame in 0..MAX_SAMPLES as u32 * 2 {
        samples.record(
            start + Duration::from_millis(1) * frame,
            Duration::from_millis(1),
        );
    }
    assert_eq!(samples.frames.len(), MAX_SAMPLES);
    assert_eq!(
        samples.summary(),
        Some(Summary {
            renders_per_second: Some(1000.),
            estimated_fps: Some(1000.),
        })
    );
}

#[test]
fn missing_or_zero_timings_never_produce_infinity() {
    let now = Instant::now();
    let mut samples = Samples::default();
    assert_eq!(samples.summary(), None);
    for _ in 0..MAX_SAMPLES * 2 {
        samples.record(now, Duration::ZERO);
    }
    assert_eq!(samples.frames.len(), MAX_SAMPLES);
    assert_eq!(samples.summary(), Some(Summary::default()));
    assert_eq!(
        samples.summary().unwrap().label().as_ref(),
        "fps:             -\nrender/s:        -"
    );
}

#[test]
fn label_waits_for_completed_work_and_updates_at_most_four_times_a_second() {
    let start = Instant::now();
    let mut panel = DebugPanel::default();
    panel.update_label(start);
    assert_eq!(
        panel.label.as_ref(),
        "fps:             -\nrender/s:        -"
    );
    panel
        .samples
        .borrow_mut()
        .record(start, Duration::from_millis(2));
    panel.update_label(start);
    assert_eq!(
        panel.label.as_ref(),
        "fps:           500\nrender/s:        -"
    );
    panel
        .samples
        .borrow_mut()
        .record(start + Duration::from_millis(10), Duration::from_millis(6));
    panel.update_label(start + Duration::from_millis(100));
    assert_eq!(
        panel.label.as_ref(),
        "fps:           500\nrender/s:        -"
    );
    panel.update_label(start + DISPLAY_INTERVAL);
    assert_eq!(
        panel.label.as_ref(),
        "fps:           250\nrender/s:    100.0"
    );
}

#[test]
fn rows_have_fixed_width_for_missing_small_and_large_values() {
    for value in [
        None,
        Some(0.),
        Some(9.),
        Some(99.9),
        Some(1000.),
        Some(1e12),
    ] {
        let label = Summary {
            estimated_fps: value,
            renders_per_second: value,
        }
        .label();
        let rows: Vec<_> = label.lines().collect();
        assert_eq!(rows.len(), 2);
        assert!(rows[0].starts_with("fps:"));
        assert!(rows[1].starts_with("render/s:"));
        for row in rows {
            assert!(row.is_ascii());
            assert_eq!(row.len(), KEY_WIDTH + VALUE_WIDTH);
        }
    }
    assert_eq!(metric_row("fps:", Some(1e12), 0), "fps:        1.0e12");
}

#[test]
fn timing_covers_the_enclosing_draw_and_records_only_when_it_finishes() {
    let _subscriber = tracing::subscriber::set_default(timing::subscriber());
    let samples = SharedSamples::default();
    let draw = tracing::info_span!(target: "gpui::window", "draw").entered();
    let before_attach = Instant::now();
    timing::attach(samples.clone());
    assert!(samples.borrow().frames.is_empty());
    let after_attach = Instant::now();
    drop(draw);
    let samples = samples.borrow();
    assert_eq!(samples.frames.len(), 1);
    assert!(samples.frames[0].1 >= after_attach.duration_since(before_attach));
}

#[test]
fn nested_draws_are_attributed_to_their_own_windows() {
    let _subscriber = tracing::subscriber::set_default(timing::subscriber());
    let outer = SharedSamples::default();
    let inner = SharedSamples::default();
    let outer_draw = tracing::info_span!(target: "gpui::window", "draw").entered();
    timing::attach(outer.clone());
    {
        let _inner_draw = tracing::info_span!(target: "gpui::window", "draw").entered();
        timing::attach(inner.clone());
    }
    assert_eq!(inner.borrow().frames.len(), 1);
    assert!(outer.borrow().frames.is_empty());
    drop(outer_draw);
    assert_eq!(outer.borrow().frames.len(), 1);
}

#[test]
fn ignores_other_spans_and_unassociated_draws() {
    let _subscriber = tracing::subscriber::set_default(timing::subscriber());
    let samples = SharedSamples::default();
    {
        let _present = tracing::info_span!(target: "gpui::window", "present").entered();
        timing::attach(samples.clone());
    }
    {
        let _draw = tracing::info_span!(target: "other", "draw").entered();
        timing::attach(samples.clone());
    }
    {
        let _draw = tracing::info_span!(target: "gpui::window", "draw").entered();
    }
    assert!(samples.borrow().frames.is_empty());
}

struct Host {
    counter: Entity<DebugPanel>,
    clicks: usize,
}

impl Host {
    fn new(_: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            counter: cx.new(|_| DebugPanel::default()),
            clicks: 0,
        }
    }
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .relative()
            .size_full()
            .child(
                div()
                    .id("content")
                    .debug_selector(|| "content".into())
                    .size_full()
                    .on_click(cx.listener(|this, _, _, _| this.clicks += 1)),
            )
            .child(self.counter.clone())
    }
}

fn redraw(cx: &mut VisualTestContext) {
    cx.update(|window, cx| window.draw(cx).clear());
    cx.run_until_parked();
}

#[gpui::test]
fn overlay_is_bottom_right_out_of_flow_and_click_through(cx: &mut TestAppContext) {
    let _subscriber = tracing::subscriber::set_default(timing::subscriber());
    let (host, cx) = cx.add_window_view(Host::new);
    redraw(cx);
    let overlay = cx.debug_bounds("debug-panel").unwrap();
    let content = cx.debug_bounds("content").unwrap();
    let viewport = cx.update(|window, _| window.viewport_size());
    assert_eq!(content.size, viewport);
    assert_eq!(overlay.right(), content.right() - px(8.));
    assert_eq!(overlay.bottom(), content.bottom() - px(8.));
    cx.simulate_click(overlay.center(), Modifiers::none());
    assert_eq!(host.read_with(cx, |host, _| host.clicks), 1);
}

#[gpui::test]
fn panel_bounds_stay_fixed_when_values_change(cx: &mut TestAppContext) {
    let (host, cx) = cx.add_window_view(Host::new);
    let panel = host.read_with(cx, |host, _| host.counter.clone());
    redraw(cx);
    let bounds = cx.debug_bounds("debug-panel").unwrap();
    for value in [
        Some(0.),
        Some(9.),
        Some(99.9),
        Some(1000.),
        Some(1e12),
        None,
    ] {
        panel.update(cx, |panel, _| {
            panel.label = Summary {
                estimated_fps: value,
                renders_per_second: value,
            }
            .label();
        });
        redraw(cx);
        assert_eq!(cx.debug_bounds("debug-panel").unwrap(), bounds);
    }
}

#[gpui::test]
fn real_gpui_draws_produce_cost_samples_without_creating_redraws(cx: &mut TestAppContext) {
    let _subscriber = tracing::subscriber::set_default(timing::subscriber());
    let (app, cx) = cx.add_window_view(|window, cx| crate::Kestrel::new(None, window, cx));
    let counter = app.read_with(cx, |app, _| app.debug_panel.clone());
    let samples = counter.read_with(cx, |counter, _| counter.samples.clone());
    redraw(cx);
    assert!(!samples.borrow().frames.is_empty());
    let before = samples.borrow().frames.len();
    redraw(cx);
    assert_eq!(samples.borrow().frames.len(), before + 1);
    assert!(samples.borrow().summary().unwrap().estimated_fps.is_some());
    assert!(counter.read_with(cx, |counter, _| counter.last_update.is_some()));
    cx.executor().advance_clock(Duration::from_secs(10));
    cx.run_until_parked();
    assert_eq!(samples.borrow().frames.len(), before + 1);
}
