use gpui::{
    Bounds, ContentMask, Hsla, IntoElement, Styled, canvas, div, fill, linear_color_stop,
    linear_gradient, point, prelude::*, px, size,
};

const FADE_WIDTH: f32 = 16.;
pub(super) const TITLE_INSET: f32 = 12.;
pub(super) const CLOSE_BUTTON_SPACE: f32 = 30.;

pub(super) fn title(
    text: String,
    background: Hsla,
    hover_background: Option<Hsla>,
    centered: bool,
) -> impl IntoElement {
    let text = crate::controls::single_line(&text);
    let selector = format!("tab-title-{text}");
    div()
        .debug_selector(move || selector.clone())
        .flex_1()
        .min_w_0()
        .h_full()
        .child(
            canvas(
                move |_, window, _| {
                    let style = window.text_style();
                    let font_size = style.font_size.to_pixels(window.rem_size());
                    window.text_system().shape_line(
                        text.clone().into(),
                        font_size,
                        &[style.to_run(text.len())],
                        None,
                    )
                },
                move |bounds, line, window, cx| {
                    let overflowing = line.width > bounds.size.width;
                    let x = if centered && !overflowing {
                        (bounds.size.width - line.width) / 2.
                    } else {
                        px(0.)
                    };
                    window.with_content_mask(Some(ContentMask { bounds }), |window| {
                        let _ = line.paint(
                            bounds.origin + point(x, px(0.)),
                            bounds.size.height,
                            window,
                            cx,
                        );
                        if overflowing {
                            // Match the tab's hover surface, including its padding. Only the
                            // title is masked; the close button and pill edges stay intact.
                            let tab_bounds = Bounds::new(
                                bounds.origin - point(px(TITLE_INSET), px(0.)),
                                size(
                                    bounds.size.width + px(TITLE_INSET + CLOSE_BUTTON_SPACE),
                                    bounds.size.height,
                                ),
                            );
                            let background = hover_background
                                .filter(|_| tab_bounds.contains(&window.mouse_position()))
                                .unwrap_or(background);
                            let width = px(FADE_WIDTH).min(bounds.size.width);
                            window.paint_quad(fill(
                                Bounds::new(
                                    point(bounds.right() - width, bounds.top()),
                                    size(width, bounds.size.height),
                                ),
                                linear_gradient(
                                    90.,
                                    linear_color_stop(background.opacity(0.), 0.),
                                    linear_color_stop(background, 1.),
                                ),
                            ));
                        }
                    });
                },
            )
            .w_full()
            .min_w_0()
            .h_full(),
        )
}
