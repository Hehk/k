use super::*;

#[test]
fn accessibility_preferences_disable_translucency() {
    for appearance in [WindowAppearance::Light, WindowAppearance::Dark] {
        let normal = Theme::resolve(appearance, Preferences::default(), true);
        if cfg!(target_os = "macos") {
            assert_eq!(normal.background, WindowBackgroundAppearance::Blurred);
            assert!(normal.shell.a < 1.);
        } else {
            assert_eq!(normal.background, WindowBackgroundAppearance::Opaque);
            assert_eq!(normal.shell.a, 1.);
        }

        for (reduce_transparency, increase_contrast) in [(true, false), (false, true), (true, true)]
        {
            let theme = Theme::resolve(
                appearance,
                Preferences {
                    reduce_transparency,
                    increase_contrast,
                    ..Preferences::default()
                },
                true,
            );
            assert_eq!(theme.background, WindowBackgroundAppearance::Opaque);
            assert_eq!(theme.shell.a, 1.);
        }
    }
}

#[test]
fn legible_accents_are_not_adjusted() {
    let accent = rgb(0x0000ff).into();
    let background = rgb(0xffffff).into();
    let ink = rgb(0x000000).into();
    assert_eq!(contrasting(accent, background, ink, 3.), accent);
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

        let adjusted = contrasting(accent, background, ink, 3.);
        assert!(contrast(adjusted, background) >= 3.);
        assert_ne!(adjusted, accent);
        assert_ne!(adjusted, ink);
    }
}
