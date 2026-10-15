mod native;

use gpui::{App, Global, Hsla, Window, WindowAppearance, rgb};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Preferences {
    pub accent: Hsla,
    pub neutrals: Option<Neutrals>,
    pub increase_contrast: bool,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            accent: rgb(0x007aff).into(),
            neutrals: None,
            increase_contrast: false,
        }
    }
}

impl Global for Preferences {}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Neutrals {
    pub shell: Hsla,
    pub paper: Hsla,
    pub text: Hsla,
    pub muted: Hsla,
    pub disabled: Hsla,
    pub border: Hsla,
}

impl Neutrals {
    fn fallback(dark: bool) -> Self {
        let ink = Hsla::from(rgb(if dark { 0xffffff } else { 0x000000 }));
        Self {
            shell: rgb(if dark { 0x323232 } else { 0xececec }).into(),
            paper: rgb(if dark { 0x1e1e1e } else { 0xffffff }).into(),
            text: ink.opacity(0.85),
            muted: ink.opacity(0.55),
            disabled: ink.opacity(0.25),
            border: ink.opacity(0.1),
        }
    }
}

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
        let neutrals = preferences
            .neutrals
            .unwrap_or_else(|| Neutrals::fallback(dark));
        let paper = neutrals.paper.opacity(1.);
        let text = paper.blend(neutrals.text);
        let control = neutrals.shell.opacity(1.);
        let border = paper.blend(neutrals.border).blend(text.opacity(0.10));
        let accent = contrasting(preferences.accent, paper, text, 3.);
        Self {
            shell: control.blend(paper.opacity(0.4)),
            paper,
            text,
            muted: if preferences.increase_contrast {
                text
            } else {
                paper.blend(neutrals.muted)
            },
            disabled: neutrals.disabled,
            border: if preferences.increase_contrast {
                text
            } else {
                border
            },
            input_border: if preferences.increase_contrast {
                text
            } else {
                contrasting(border, paper, text, 3.)
            },
            control,
            hover: control.blend(text.opacity(0.06)),
            pressed: control.blend(text.opacity(0.12)),
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
            metadata: paper.blend(control.opacity(0.5)),
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
