mod native;

use gpui::{App, Global, Hsla, Window, WindowAppearance, WindowBackgroundAppearance, rgb};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Preferences {
    pub accent: Hsla,
    pub increase_contrast: bool,
    pub reduce_transparency: bool,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            accent: rgb(0x007aff).into(),
            increase_contrast: false,
            reduce_transparency: false,
        }
    }
}

impl Global for Preferences {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiffTone {
    Context,
    Added,
    Deleted,
    Hunk,
    Metadata,
}

#[derive(Clone, Copy, Debug)]
pub struct Theme {
    pub shell: Hsla,
    pub paper: Hsla,
    pub text: Hsla,
    pub muted: Hsla,
    pub disabled: Hsla,
    pub border: Hsla,
    pub input_border: Hsla,
    pub control: Hsla,
    pub hover: Hsla,
    pub pressed: Hsla,
    pub accent: Hsla,
    pub selection: Hsla,
    pub error: Hsla,
    pub warning: Hsla,
    pub added: Hsla,
    pub deleted: Hsla,
    pub metadata: Hsla,
    pub background: WindowBackgroundAppearance,
}

impl Theme {
    pub fn get(window: &Window, cx: &App) -> Self {
        Self::resolve(
            window.appearance(),
            cx.try_global::<Preferences>().copied().unwrap_or_default(),
            window.is_window_active(),
        )
    }

    pub fn resolve(appearance: WindowAppearance, preferences: Preferences, active: bool) -> Self {
        let dark = matches!(
            appearance,
            WindowAppearance::Dark | WindowAppearance::VibrantDark
        );
        let color = |light, dark_color| Hsla::from(rgb(if dark { dark_color } else { light }));
        let paper = color(0xfffcf5, 0x24221e);
        let text = if preferences.increase_contrast {
            color(0x211e19, 0xf5f0e6)
        } else {
            color(0x302d27, 0xe8e2d5)
        };
        let control = color(0xeae5da, 0x34312a);
        let accent = contrasting(preferences.accent, paper, text, 3.);
        let glass = cfg!(target_os = "macos")
            && !preferences.reduce_transparency
            && !preferences.increase_contrast;
        Self {
            shell: color(0xeee8db, 0x171613).opacity(if glass { 0.82 } else { 1. }),
            paper,
            text,
            muted: if preferences.increase_contrast {
                text
            } else {
                color(0x595349, 0xbfb8aa)
            },
            disabled: if preferences.increase_contrast {
                color(0x716a5d, 0xaaa293)
            } else {
                color(0x888173, 0x898274)
            },
            border: if preferences.increase_contrast {
                text
            } else {
                color(0xd1cabd, 0x4c473e)
            },
            input_border: if preferences.increase_contrast {
                text
            } else {
                color(0x928b80, 0x817a6d)
            },
            control,
            hover: color(0xe0dacd, 0x403c33),
            pressed: color(0xd6cebf, 0x4b453a),
            accent,
            selection: if active {
                paper.blend(accent.opacity(0.18))
            } else {
                control
            },
            error: if preferences.increase_contrast {
                color(0x8f2b20, 0xf5bcae)
            } else {
                color(0xa0392b, 0xf0a494)
            },
            warning: color(0xf2e7c9, 0x443b25),
            added: color(0xe6eddb, 0x26362b),
            deleted: color(0xf4e3dd, 0x402b29),
            metadata: color(0xf1ebde, 0x2f2b23),
            background: if glass {
                WindowBackgroundAppearance::Blurred
            } else {
                WindowBackgroundAppearance::Opaque
            },
        }
    }

    pub fn diff(&self, tone: DiffTone) -> Hsla {
        match tone {
            DiffTone::Context => self.paper,
            DiffTone::Added => self.added,
            DiffTone::Deleted => self.deleted,
            DiffTone::Hunk | DiffTone::Metadata => self.metadata,
        }
    }
}

fn contrast(a: Hsla, b: Hsla) -> f32 {
    let luminance = |color: Hsla| {
        let color = color.to_rgb();
        let linear = |c: f32| {
            if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        linear(color.r) * 0.2126 + linear(color.g) * 0.7152 + linear(color.b) * 0.0722
    };
    let a = luminance(a);
    let b = luminance(b);
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

fn contrasting(color: Hsla, background: Hsla, ink: Hsla, minimum: f32) -> Hsla {
    // System accents can be too faint on custom paper surfaces. Mix toward our
    // ink only as much as legibility needs, rather than replacing the accent.
    for step in 0..=20 {
        let candidate = color.blend(ink.opacity(step as f32 / 20.));
        if contrast(candidate, background) >= minimum {
            return candidate;
        }
    }
    ink
}

pub fn install(cx: &mut App) {
    refresh(cx);
    native::observe(cx);
}

pub fn refresh(cx: &mut App) {
    let preferences = native::preferences();
    if cx.try_global::<Preferences>() != Some(&preferences) {
        cx.set_global(preferences);
        cx.refresh_windows();
    }
}

#[cfg(test)]
mod tests;
