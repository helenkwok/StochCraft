//! StochCraft's look: IBM Plex (the Craft suite's interface face) in a fixed type scale, and a
//! restrained palette with a committed shell (an ink header, a cooler side rail) for light and
//! dark, each composed on its own rather than inverted.
//!
//! Colour has jobs, not decoration: the accent marks the primary action, selection and focus;
//! red and amber mark errors and warnings, always with words beside them.

use egui::epaint::text::{FontTweak, VariationCoords};
use egui::{Color32, FontData, FontDefinitions, FontFamily, FontId, TextStyle, Visuals};
use std::sync::Arc;

/// The family for headings and emphasis: Plex Sans at semibold.
pub const STRONG: &str = "plex-semibold";

const PLEX_SANS: &[u8] =
    include_bytes!("../../../assets/fonts/ibm-plex-sans/IBMPlexSans-Variable.ttf");
const PLEX_MONO: &[u8] =
    include_bytes!("../../../assets/fonts/ibm-plex-mono/IBMPlexMono-Regular.ttf");

/// The colours of one theme, by role.
#[derive(Debug, Clone, Copy)]
pub struct Palette {
    /// The header bar (the shell's committed colour) and the text on it.
    pub shell: Color32,
    pub on_shell: Color32,
    pub on_shell_weak: Color32,
    /// The side rail: a second neutral, cooler than the content.
    pub rail: Color32,
    /// The content surface.
    pub canvas: Color32,
    /// Raised surfaces: menus, dialogs.
    pub raised: Color32,
    pub text: Color32,
    pub text_weak: Color32,
    pub border: Color32,
    /// Alternate table rows.
    pub stripe: Color32,
    pub accent: Color32,
    pub on_accent: Color32,
    pub accent_soft: Color32,
    pub error: Color32,
    pub warning: Color32,
    /// Chart bars: one hue, validated against this theme's canvas (dataviz checks: lightness
    /// band, chroma floor, contrast ≥ 3:1), and its hovered state.
    pub chart: Color32,
    pub chart_hover: Color32,
}

pub const LIGHT: Palette = Palette {
    shell: Color32::from_rgb(0x17, 0x22, 0x2e),
    on_shell: Color32::from_rgb(0xf1, 0xf4, 0xf7),
    on_shell_weak: Color32::from_rgb(0xa9, 0xb6, 0xc4),
    rail: Color32::from_rgb(0xee, 0xf1, 0xf4),
    canvas: Color32::from_rgb(0xfb, 0xfb, 0xfa),
    raised: Color32::from_rgb(0xff, 0xff, 0xff),
    text: Color32::from_rgb(0x1b, 0x1f, 0x24),
    text_weak: Color32::from_rgb(0x55, 0x5e, 0x69),
    border: Color32::from_rgb(0xd9, 0xdd, 0xe3),
    stripe: Color32::from_rgb(0xf2, 0xf4, 0xf6),
    accent: Color32::from_rgb(0x0b, 0x6e, 0x86),
    on_accent: Color32::from_rgb(0xff, 0xff, 0xff),
    accent_soft: Color32::from_rgb(0xd7, 0xec, 0xf1),
    error: Color32::from_rgb(0xb4, 0x23, 0x18),
    warning: Color32::from_rgb(0x8f, 0x55, 0x00),
    chart: Color32::from_rgb(0x0a, 0x7f, 0x9e),
    chart_hover: Color32::from_rgb(0x07, 0x67, 0x80),
};

pub const DARK: Palette = Palette {
    shell: Color32::from_rgb(0x0c, 0x10, 0x15),
    on_shell: Color32::from_rgb(0xe8, 0xec, 0xf0),
    on_shell_weak: Color32::from_rgb(0x93, 0x9f, 0xac),
    rail: Color32::from_rgb(0x1a, 0x1f, 0x26),
    canvas: Color32::from_rgb(0x14, 0x17, 0x1b),
    raised: Color32::from_rgb(0x22, 0x28, 0x30),
    text: Color32::from_rgb(0xe6, 0xe9, 0xec),
    text_weak: Color32::from_rgb(0x9c, 0xa6, 0xb1),
    border: Color32::from_rgb(0x2e, 0x35, 0x3e),
    stripe: Color32::from_rgb(0x1a, 0x1e, 0x23),
    accent: Color32::from_rgb(0x4c, 0xc4, 0xdb),
    on_accent: Color32::from_rgb(0x06, 0x1a, 0x20),
    accent_soft: Color32::from_rgb(0x15, 0x34, 0x3c),
    error: Color32::from_rgb(0xff, 0x8a, 0x80),
    warning: Color32::from_rgb(0xf2, 0xb8, 0x4b),
    chart: Color32::from_rgb(0x1f, 0x9d, 0xb8),
    chart_hover: Color32::from_rgb(0x3d, 0xb5, 0xcf),
};

/// The palette for the theme egui is showing.
pub fn palette(ctx: &egui::Context) -> Palette {
    if ctx.theme() == egui::Theme::Dark {
        DARK
    } else {
        LIGHT
    }
}

fn plex(weight: f32) -> FontData {
    FontData::from_static(PLEX_SANS).tweak(FontTweak {
        coords: VariationCoords::new([("wght", weight)]),
        ..FontTweak::default()
    })
}

/// Installs the fonts, the type scale and both themes. Call once at start.
pub fn install(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert("plex".into(), Arc::new(plex(420.0)));
    fonts.font_data.insert(STRONG.into(), Arc::new(plex(600.0)));
    fonts.font_data.insert(
        "plex-mono".into(),
        Arc::new(FontData::from_static(PLEX_MONO)),
    );
    // Plex first; egui's own fonts stay behind it for symbols Plex lacks (⌘, ⇧).
    let proportional = fonts.families.entry(FontFamily::Proportional).or_default();
    proportional.insert(0, "plex".into());
    let fallback: Vec<String> = proportional.iter().skip(1).cloned().collect();
    fonts.families.insert(
        FontFamily::Name(STRONG.into()),
        std::iter::once(STRONG.to_owned()).chain(fallback).collect(),
    );
    fonts
        .families
        .entry(FontFamily::Monospace)
        .or_default()
        .insert(0, "plex-mono".into());
    ctx.set_fonts(fonts);

    ctx.set_theme(egui::ThemePreference::System);
    ctx.style_mut_of(egui::Theme::Light, |s| style(s, &LIGHT));
    ctx.style_mut_of(egui::Theme::Dark, |s| style(s, &DARK));
}

fn style(s: &mut egui::Style, p: &Palette) {
    // A fixed scale with a ratio of about 1.2: small 12, body 14, heading 20.
    s.text_styles = [
        (TextStyle::Small, FontId::proportional(12.0)),
        (TextStyle::Body, FontId::proportional(14.0)),
        (TextStyle::Button, FontId::proportional(14.0)),
        (TextStyle::Monospace, FontId::monospace(13.5)),
        (
            TextStyle::Heading,
            FontId::new(20.0, FontFamily::Name(STRONG.into())),
        ),
    ]
    .into();
    s.spacing.item_spacing = egui::vec2(8.0, 6.0);
    s.spacing.button_padding = egui::vec2(10.0, 5.0);
    s.spacing.menu_margin = egui::Margin::same(6);
    s.spacing.interact_size.y = 26.0;

    let v: &mut Visuals = &mut s.visuals;
    v.panel_fill = p.canvas;
    v.window_fill = p.raised;
    v.extreme_bg_color = p.raised;
    v.faint_bg_color = p.stripe;
    v.code_bg_color = p.stripe;
    v.window_stroke = egui::Stroke::new(1.0, p.border);
    v.window_corner_radius = egui::CornerRadius::same(10);
    v.menu_corner_radius = egui::CornerRadius::same(8);
    v.hyperlink_color = p.accent;
    v.error_fg_color = p.error;
    v.warn_fg_color = p.warning;
    v.selection.bg_fill = p.accent_soft;
    v.selection.stroke = egui::Stroke::new(1.0, p.accent);
    v.striped = true;

    let radius = egui::CornerRadius::same(6);
    let w = &mut v.widgets;
    w.noninteractive.bg_fill = p.canvas;
    w.noninteractive.weak_bg_fill = p.canvas;
    w.noninteractive.bg_stroke = egui::Stroke::new(1.0, p.border);
    w.noninteractive.fg_stroke = egui::Stroke::new(1.0, p.text);
    for (state, fill, stroke) in [
        (&mut w.inactive, p.stripe, p.border),
        (&mut w.hovered, p.accent_soft, p.accent),
        (&mut w.active, p.accent_soft, p.accent),
        (&mut w.open, p.accent_soft, p.border),
    ] {
        state.bg_fill = fill;
        state.weak_bg_fill = fill;
        state.bg_stroke = egui::Stroke::new(1.0, stroke);
        state.fg_stroke = egui::Stroke::new(1.0, p.text);
        state.corner_radius = radius;
    }
    // Controls at rest keep a hairline, so a choice box reads as one on the canvas.
    w.inactive.weak_bg_fill = Color32::TRANSPARENT;
}

/// The primary action's button: filled with the accent.
pub fn primary(ui: &egui::Ui, label: &str) -> egui::Button<'static> {
    let p = palette(ui.ctx());
    egui::Button::new(
        egui::RichText::new(label.to_owned())
            .color(p.on_accent)
            .family(FontFamily::Name(STRONG.into())),
    )
    .fill(p.accent)
    .min_size(egui::vec2(0.0, 30.0))
}

/// A secondary button: outlined, so it reads as a button on the canvas.
pub fn secondary(ui: &egui::Ui, label: &str) -> egui::Button<'static> {
    let p = palette(ui.ctx());
    egui::Button::new(label.to_owned())
        .stroke(egui::Stroke::new(1.0, p.border))
        .min_size(egui::vec2(0.0, 30.0))
}

/// Text in the semibold family.
pub fn strong(text: impl Into<String>) -> egui::RichText {
    egui::RichText::new(text).family(FontFamily::Name(STRONG.into()))
}

/// Secondary text, in the palette's weak colour (contrast at least 4.5:1 on the canvas and rail).
pub fn weak(ui: &egui::Ui, text: impl Into<String>) -> egui::RichText {
    egui::RichText::new(text).color(palette(ui.ctx()).text_weak)
}
