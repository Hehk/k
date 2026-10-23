use super::*;
use gpui::{Entity, Modifiers, TestAppContext, VisualTestContext};

#[test]
fn cadence_and_draw_cost_are_independent() {
    let start = Instant::now();
    for (spacing, fps) in [(10, 100), (20, 50)] {
        for cost in [2, 8] {
            let mut samples = Samples::default();
            for frame in 0..=2000 / spacing {
                samples.record(
                    start + Duration::from_millis(spacing * frame),
                    Duration::from_millis(cost),
                );
            }
            assert_eq!(
                samples.summary(start + Duration::from_secs(2)),
                Summary {
                    fps: Some(fps),
                    mean_ms: Some(cost as f64),
                    tail_ms: Some(cost as f64),
                    count: fps,
                    limited: false,
                }
            );
        }
    }
}

#[test]
fn warm_up_requires_a_full_observation_window() {
    let start = Instant::now();
    let mut samples = Samples::default();
    assert_eq!(samples.summary(start), Summary::default());
    samples.record(start + Duration::from_millis(2), Duration::from_millis(2));
    let summary = samples.summary(start + Duration::from_millis(999));
    assert_eq!(summary.fps, None);
    assert_eq!(summary.mean_ms, Some(2.));
    assert!(summary.label().contains("warming up"));
    assert_eq!(samples.summary(start + SAMPLE_WINDOW).fps, Some(1));
}

#[test]
fn expiry_uses_now_even_without_another_completed_frame() {
    let start = Instant::now();
    let mut samples = Samples::default();
    samples.record(start, Duration::from_millis(2));
    assert_eq!(
        samples.summary(start + SAMPLE_WINDOW),
        Summary {
            fps: Some(0),
            ..Default::default()
        }
    );
    samples.record(start + Duration::from_secs(10), Duration::from_millis(4));
    assert_eq!(samples.frames.len(), 1);
    let summary = samples.summary(start + Duration::from_secs(10));
    assert_eq!(summary.fps, Some(1));
    assert_eq!(summary.mean_ms, Some(4.));
    assert_eq!(summary.count, 1);
}

#[test]
fn sparse_updates_do_not_exclude_gaps_to_inflate_fps() {
    let start = Instant::now();
    let mut samples = Samples::default();
    samples.record(start, Duration::ZERO);
    samples.record(
        start + Duration::from_millis(1800),
        Duration::from_millis(2),
    );
    samples.record(
        start + Duration::from_millis(1900),
        Duration::from_millis(2),
    );
    assert_eq!(samples.summary(start + Duration::from_secs(2)).fps, Some(2));
}

#[test]
fn a_long_draw_keeps_its_full_cost_even_when_it_spans_the_window() {
    let start = Instant::now();
    let mut samples = Samples::default();
    samples.record(start + Duration::from_secs(2), Duration::from_secs(2));
    assert_eq!(
        samples.summary(start + Duration::from_secs(2)),
        Summary {
            fps: Some(1),
            mean_ms: Some(2000.),
            tail_ms: Some(2000.),
            count: 1,
            limited: false,
        }
    );
}

#[test]
fn tail_uses_max_for_sparse_samples_and_nearest_rank_p95_otherwise() {
    let start = Instant::now();
    let mut samples = Samples::default();
    for i in 1..20 {
        samples.record(
            start + Duration::from_millis(i * 10),
            Duration::from_millis(i),
        );
    }
    let summary = samples.summary(start + Duration::from_millis(190));
    assert_eq!(summary.tail_ms, Some(19.));
    assert!(summary.label().contains("max draw:"));
    samples.record(
        start + Duration::from_millis(200),
        Duration::from_millis(200),
    );
    let summary = samples.summary(start + Duration::from_millis(200));
    assert_eq!(summary.tail_ms, Some(19.));
    assert!(summary.label().contains("p95 draw:"));
    samples.record(
        start + Duration::from_millis(210),
        Duration::from_millis(210),
    );
    assert_eq!(
        samples.summary(start + Duration::from_millis(210)).tail_ms,
        Some(200.)
    );
}

#[test]
fn sample_limit_is_explicit_and_never_reports_biased_costs() {
    let start = Instant::now();
    let mut samples = Samples::default();
    samples.record(start, Duration::ZERO);
    let completed = start + Duration::from_secs(2);
    for _ in 0..MAX_SAMPLES {
        samples.record(completed, Duration::ZERO);
    }
    assert!(!samples.summary(completed).limited);
    samples.record(completed, Duration::from_millis(100));
    assert_eq!(samples.frames.len(), MAX_SAMPLES);
    assert_eq!(
        samples.summary(completed),
        Summary {
            fps: Some(MAX_SAMPLES),
            count: MAX_SAMPLES,
            limited: true,
            ..Default::default()
        }
    );
    assert!(samples.summary(completed).label().contains(">=512"));
    // Truncation remains visible until the omitted completion leaves the window.
    assert!(
        samples
            .summary(completed + Duration::from_millis(999))
            .limited
    );
    assert_eq!(
        samples.summary(completed + SAMPLE_WINDOW),
        Summary {
            fps: Some(0),
            ..Default::default()
        }
    );
    samples.record(completed + SAMPLE_WINDOW, Duration::from_millis(2));
    assert_eq!(samples.summary(completed + SAMPLE_WINDOW).mean_ms, Some(2.));
}

#[test]
fn zero_costs_and_identical_timestamps_are_finite() {
    let start = Instant::now();
    let mut samples = Samples::default();
    for _ in 0..20 {
        samples.record(start, Duration::ZERO);
    }
    let summary = samples.summary(start);
    assert_eq!(summary.fps, None);
    assert_eq!(summary.mean_ms, Some(0.));
    assert_eq!(summary.tail_ms, Some(0.));
}

#[test]
fn label_uses_completed_samples_at_most_four_times_a_second() {
    let start = Instant::now();
    let mut panel = DebugPanel::default();
    panel.update_label(start);
    let initial = panel.label.clone();
    panel
        .samples
        .borrow_mut()
        .record(start, Duration::from_millis(2));
    panel.update_label(start + Duration::from_millis(100));
    assert_eq!(panel.label, initial);
    panel.update_label(start + DISPLAY_INTERVAL);
    assert!(panel.label.contains("2.0 ms"));
    let active = panel.label.clone();
    panel
        .samples
        .borrow_mut()
        .record(start + Duration::from_millis(300), Duration::from_millis(6));
    panel.update_label(start + Duration::from_millis(400));
    assert_eq!(panel.label, active);
    panel.update_label(start + Duration::from_millis(500));
    assert!(panel.label.contains("4.0 ms"));
    // No new completion is needed to expire stale values when rendering resumes.
    panel.update_label(start + Duration::from_secs(10));
    assert_eq!(
        panel.label,
        Summary {
            fps: Some(0),
            ..Default::default()
        }
        .label()
    );
}

fn labels() -> Vec<SharedString> {
    let mut labels = vec![Summary::default().label()];
    for value in [
        None,
        Some(0.),
        Some(9.),
        Some(99.9),
        Some(1000.),
        Some(1e12),
    ] {
        for count in [0, 1, 19, 20, MAX_SAMPLES] {
            for limited in [false, true] {
                labels.push(
                    Summary {
                        fps: Some(count),
                        mean_ms: value,
                        tail_ms: value,
                        count,
                        limited,
                    }
                    .label(),
                );
            }
        }
    }
    labels
}

#[test]
fn rows_have_fixed_width_in_all_states_and_explicit_units() {
    for label in labels() {
        let rows: Vec<_> = label.lines().collect();
        assert_eq!(rows.len(), 5);
        assert_eq!(rows[0], "Recent frames (1s)");
        for row in &rows[1..] {
            assert!(row.is_ascii());
            assert_eq!(row.len(), KEY_WIDTH + VALUE_WIDTH);
        }
    }
    assert_eq!(
        metric_row("CPU draw:", Some(1e12)),
        "CPU draw:    1.0e12 ms"
    );
}

#[test]
fn panel_rows_are_left_padded_to_the_widest_row() {
    for branch in ["main", "feat/frame-cost-info-with-a-long-name"] {
        let label = right_align_rows(&format!("branch: {branch}\n{}", Summary::default().label()));
        let width = label.lines().map(str::len).max().unwrap();
        assert!(label.lines().all(|line| line.len() == width));
        assert_eq!(
            label.lines().next().unwrap().trim(),
            format!("branch: {branch}")
        );
    }
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
fn finished_spans_do_not_retain_closed_panels() {
    let _subscriber = tracing::subscriber::set_default(timing::subscriber());
    let samples = SharedSamples::default();
    let weak = Rc::downgrade(&samples);
    {
        let _draw = tracing::info_span!(target: "gpui::window", "draw").entered();
        timing::attach(samples.clone());
        drop(samples);
    }
    assert!(weak.upgrade().is_none());
}

#[test]
fn ignores_other_spans_and_unassociated_draws() {
    let _subscriber = tracing::subscriber::set_default(timing::subscriber());
    let samples = SharedSamples::default();
    {
        let _span = tracing::info_span!(target: "gpui::window", "present").entered();
        timing::attach(samples.clone());
    }
    {
        let _span = tracing::info_span!(target: "other", "draw").entered();
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
    assert_eq!(overlay.right(), content.right() - px(16.));
    assert_eq!(overlay.bottom(), content.bottom() - px(16.));
    cx.simulate_click(overlay.center(), Modifiers::none());
    assert_eq!(host.read_with(cx, |host, _| host.clicks), 1);
}

#[gpui::test]
fn panel_bounds_stay_fixed_when_values_change(cx: &mut TestAppContext) {
    let (host, cx) = cx.add_window_view(Host::new);
    let panel = host.read_with(cx, |host, _| host.counter.clone());
    redraw(cx);
    let bounds = cx.debug_bounds("debug-panel").unwrap();
    for label in labels() {
        panel.update(cx, |panel, _| {
            panel.label = label;
            panel.last_update = Some(Instant::now());
        });
        redraw(cx);
        assert_eq!(cx.debug_bounds("debug-panel").unwrap(), bounds);
    }
}

#[gpui::test]
fn real_gpui_draws_produce_samples_without_creating_redraws(cx: &mut TestAppContext) {
    let _subscriber = tracing::subscriber::set_default(timing::subscriber());
    let (host, cx) = cx.add_window_view(Host::new);
    let counter = host.read_with(cx, |host, _| host.counter.clone());
    let samples = counter.read_with(cx, |counter, _| counter.samples.clone());
    redraw(cx);
    assert!(!samples.borrow().frames.is_empty());
    let before = samples.borrow().frames.len();
    redraw(cx);
    assert_eq!(samples.borrow().frames.len(), before + 1);
    assert!(samples.borrow().summary(Instant::now()).mean_ms.is_some());
    assert!(counter.read_with(cx, |counter, _| counter.last_update.is_some()));
    cx.executor().advance_clock(Duration::from_secs(10));
    cx.run_until_parked();
    assert_eq!(samples.borrow().frames.len(), before + 1);
}

#[gpui::test]
fn window_entities_have_separate_samples(cx: &mut TestAppContext) {
    let _subscriber = tracing::subscriber::set_default(timing::subscriber());
    let (first, first_cx) = cx.add_window_view(Host::new);
    let first_samples = first.read_with(first_cx, |host, cx| host.counter.read(cx).samples.clone());
    redraw(first_cx);
    let (second, second_cx) = cx.add_window_view(Host::new);
    let second_samples =
        second.read_with(second_cx, |host, cx| host.counter.read(cx).samples.clone());
    redraw(second_cx);
    assert!(!Rc::ptr_eq(&first_samples, &second_samples));
    let first_count = first_samples.borrow().frames.len();
    let second_count = second_samples.borrow().frames.len();
    redraw(second_cx);
    assert_eq!(first_samples.borrow().frames.len(), first_count);
    assert_eq!(second_samples.borrow().frames.len(), second_count + 1);
}
