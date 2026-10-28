use super::*;

// AppKit Aqua/Dark Aqua samples from macOS 26.7.1, converted to sRGB.
// These exercise color calculations, not native visual acceptance. Increased
// Contrast and alternate accents below are synthetic preference inputs.
fn sampled_neutrals(dark: bool) -> Neutrals {
    let ink = Hsla::from(rgb(if dark { 0xffffff } else { 0x000000 }));
    let paper = rgb(if dark { 0x1e1e1e } else { 0xffffff }).into();
    Neutrals {
        shell: paper,
        paper,
        text: ink.opacity(216. / 255.),
        muted: ink.opacity(if dark { 140. / 255. } else { 127. / 255. }),
        disabled: ink.opacity(63. / 255.),
        border: ink.opacity(25. / 255.),
    }
}

fn text_surfaces(theme: Theme) -> [(&'static str, Hsla); 10] {
    [
        ("paper", theme.diff(DiffTone::Context)),
        ("added gutter", theme.diff(DiffTone::Added)),
        ("deleted gutter", theme.diff(DiffTone::Deleted)),
        ("hunk gutter", theme.diff(DiffTone::Hunk)),
        ("metadata gutter", theme.diff(DiffTone::Metadata)),
        ("shell", theme.shell),
        ("inactive tab", theme.control),
        ("hover", theme.hover),
        ("pressed", theme.pressed),
        ("selection", theme.selection),
    ]
}

#[test]
fn primary_and_secondary_text_meet_contrast_on_their_surfaces() {
    for appearance in [
        WindowAppearance::Light,
        WindowAppearance::VibrantLight,
        WindowAppearance::Dark,
        WindowAppearance::VibrantDark,
    ] {
        let dark = matches!(
            appearance,
            WindowAppearance::Dark | WindowAppearance::VibrantDark
        );
        for neutrals in [Some(sampled_neutrals(dark)), None] {
            for increase_contrast in [false, true] {
                for active in [false, true] {
                    for accent in [
                        0x007aff, 0xff0000, 0xff9500, 0xffff00, 0x00ff00, 0x8000ff, 0xff00ff,
                        0x808080, 0x000000, 0xffffff,
                    ] {
                        let preferences = Preferences {
                            accent: rgb(accent).into(),
                            neutrals,
                            increase_contrast,
                        };
                        let theme = Theme::resolve(appearance, preferences, active);
                        if increase_contrast {
                            assert_eq!(theme.muted, theme.text);
                            assert_eq!(theme.border, theme.text);
                            assert_eq!(theme.input_border, theme.text);
                        }
                        for (surface, background) in text_surfaces(theme) {
                            assert_eq!(background.a, 1.);
                            for (role, foreground) in
                                [("primary", theme.text), ("secondary", theme.muted)]
                            {
                                assert_eq!(foreground.a, 1.);
                                let ratio = contrast(foreground, background);
                                assert!(
                                    ratio >= 4.5,
                                    "{appearance:?}, {preferences:?}, active={active}: {role}/{surface} = {ratio:.4}:1"
                                );
                            }
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn faint_semantic_secondary_text_is_adjusted_without_being_replaced() {
    let neutrals = sampled_neutrals(false);
    let theme = Theme::resolve(
        WindowAppearance::Light,
        Preferences {
            neutrals: Some(neutrals),
            ..Preferences::default()
        },
        true,
    );
    let original = neutrals.paper.blend(neutrals.muted);
    assert!(contrast(original, theme.paper) < 4.5);
    assert_ne!(theme.muted, original);
    assert_ne!(theme.muted, theme.text);
    assert_eq!(theme.text, neutrals.paper.blend(neutrals.text));
    assert_eq!(theme.paper, neutrals.paper);
    assert_eq!(theme.control, neutrals.shell);
    assert_eq!(theme.disabled, neutrals.disabled);
}

#[test]
fn legible_semantic_secondary_text_is_not_adjusted() {
    let neutrals = Neutrals {
        muted: Hsla::from(rgb(0xd9e6ff)).opacity(0.75),
        ..sampled_neutrals(true)
    };
    let theme = Theme::resolve(
        WindowAppearance::Dark,
        Preferences {
            neutrals: Some(neutrals),
            ..Preferences::default()
        },
        true,
    );
    assert_eq!(theme.muted, neutrals.paper.blend(neutrals.muted));
    assert_ne!(theme.muted, theme.text);
    assert_eq!(theme.text, neutrals.paper.blend(neutrals.text));
    assert_eq!(theme.paper, neutrals.paper);
    assert_eq!(theme.control, neutrals.shell);
    assert_eq!(theme.disabled, neutrals.disabled);
}

#[test]
fn contrast_matches_srgb_reference_ratios() {
    assert!((contrast(rgb(0x000000).into(), rgb(0xffffff).into()) - 21.).abs() < 0.001);
    assert!((contrast(rgb(0x777777).into(), rgb(0xffffff).into()) - 4.478).abs() < 0.001);
}

#[test]
fn legible_accents_are_not_adjusted() {
    let accent = rgb(0x0000ff).into();
    let background = rgb(0xffffff).into();
    let ink = rgb(0x000000).into();
    assert_eq!(contrasting(accent, &[background], ink, 3.), accent);
}

#[test]
fn faint_accents_are_adjusted_toward_ink_without_being_replaced() {
    for (accent, background, ink) in [
        (0xdddddd, 0xffffff, 0x000000),
        (0x222222, 0x000000, 0xffffff),
    ] {
        let accent = rgb(accent).into();
        let background = rgb(background).into();
        let ink = rgb(ink).into();
        assert!(contrast(accent, background) < 3.);

        let adjusted = contrasting(accent, &[background], ink, 3.);
        assert!(contrast(adjusted, background) >= 3.);
        assert_ne!(adjusted, accent);
        assert_ne!(adjusted, ink);
    }
}
