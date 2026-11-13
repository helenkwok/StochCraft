//! StochCraft's look: IBM Plex (the Craft suite's interface face) in a fixed type scale, and a
//! restrained palette with a committed shell (an ink header, a cooler side rail) for light and
//! dark, each composed on its own rather than inverted.
//!
//! Colour has jobs, not decoration: the accent marks the primary action, selection and focus;
//! red and amber mark errors and warnings, always with words beside them.

use egui::epaint::text::{FontTweak, VariationCoords};
use egui::{Color32, FontData, FontFamily, FontId, TextStyle, Visuals};

use crate::i18n::{t, tf};

/// The family for headings and emphasis: Plex Sans at semibold.
pub const STRONG: &str = "plex-semibold";

const PLEX_SANS: &[u8] =
    include_bytes!("../../../assets/fonts/ibm-plex-sans/IBMPlexSans-Variable.ttf");
const PLEX_MONO: &[u8] =
    include_bytes!("../../../assets/fonts/ibm-plex-mono/IBMPlexMono-Regular.ttf");

/// The colours of one palette, by role. Saved as `"#rrggbb"` text.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Palette {
    /// The header bar (the shell's committed colour) and the text on it.
    #[serde(with = "colour")]
    pub shell: Color32,
    #[serde(with = "colour")]
    pub on_shell: Color32,
    #[serde(with = "colour")]
    pub on_shell_weak: Color32,
    /// The side rail: a second neutral, cooler than the content.
    #[serde(with = "colour")]
    pub rail: Color32,
    /// The content surface.
    #[serde(with = "colour")]
    pub canvas: Color32,
    /// Raised surfaces: menus, dialogs.
    #[serde(with = "colour")]
    pub raised: Color32,
    #[serde(with = "colour")]
    pub text: Color32,
    #[serde(with = "colour")]
    pub text_weak: Color32,
    #[serde(with = "colour")]
    pub border: Color32,
    /// Alternate table rows.
    #[serde(with = "colour")]
    pub stripe: Color32,
    #[serde(with = "colour")]
    pub accent: Color32,
    #[serde(with = "colour")]
    pub on_accent: Color32,
    #[serde(with = "colour")]
    pub accent_soft: Color32,
    #[serde(with = "colour")]
    pub error: Color32,
    #[serde(with = "colour")]
    pub warning: Color32,
    /// Chart bars: one hue, validated against this theme's canvas (dataviz checks: lightness
    /// band, chroma floor, contrast ≥ 3:1), and its hovered state.
    #[serde(with = "colour")]
    pub chart: Color32,
    #[serde(with = "colour")]
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

/// A colour theme: one palette for light mode and one for dark mode, under a name.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Theme {
    pub name: String,
    pub light: Palette,
    pub dark: Palette,
}

/// The name of the theme the app starts with.
pub const DEFAULT: &str = "StochCraft";

/// High contrast: near-black text on white and near-white on black, strong borders, and a
/// saturated accent.
const HIGH_CONTRAST_LIGHT: Palette = Palette {
    shell: Color32::from_rgb(0x00, 0x00, 0x00),
    on_shell: Color32::from_rgb(0xff, 0xff, 0xff),
    on_shell_weak: Color32::from_rgb(0xe0, 0xe0, 0xe0),
    rail: Color32::from_rgb(0xf0, 0xf0, 0xf0),
    canvas: Color32::from_rgb(0xff, 0xff, 0xff),
    raised: Color32::from_rgb(0xff, 0xff, 0xff),
    text: Color32::from_rgb(0x00, 0x00, 0x00),
    text_weak: Color32::from_rgb(0x33, 0x33, 0x33),
    border: Color32::from_rgb(0x00, 0x00, 0x00),
    stripe: Color32::from_rgb(0xe6, 0xe6, 0xe6),
    accent: Color32::from_rgb(0x00, 0x38, 0xa8),
    on_accent: Color32::from_rgb(0xff, 0xff, 0xff),
    accent_soft: Color32::from_rgb(0xcf, 0xe0, 0xff),
    error: Color32::from_rgb(0xb0, 0x00, 0x00),
    warning: Color32::from_rgb(0x8a, 0x4b, 0x00),
    chart: Color32::from_rgb(0x00, 0x38, 0xa8),
    chart_hover: Color32::from_rgb(0x00, 0x2a, 0x80),
};

const HIGH_CONTRAST_DARK: Palette = Palette {
    shell: Color32::from_rgb(0x00, 0x00, 0x00),
    on_shell: Color32::from_rgb(0xff, 0xff, 0xff),
    on_shell_weak: Color32::from_rgb(0xd0, 0xd0, 0xd0),
    rail: Color32::from_rgb(0x0a, 0x0a, 0x0a),
    canvas: Color32::from_rgb(0x00, 0x00, 0x00),
    raised: Color32::from_rgb(0x11, 0x11, 0x11),
    text: Color32::from_rgb(0xff, 0xff, 0xff),
    text_weak: Color32::from_rgb(0xd4, 0xd4, 0xd4),
    border: Color32::from_rgb(0xbd, 0xbd, 0xbd),
    stripe: Color32::from_rgb(0x1a, 0x1a, 0x1a),
    accent: Color32::from_rgb(0x66, 0xb3, 0xff),
    on_accent: Color32::from_rgb(0x00, 0x00, 0x00),
    accent_soft: Color32::from_rgb(0x00, 0x33, 0x66),
    error: Color32::from_rgb(0xff, 0x6b, 0x6b),
    warning: Color32::from_rgb(0xff, 0xc2, 0x47),
    chart: Color32::from_rgb(0x5c, 0xb8, 0xff),
    chart_hover: Color32::from_rgb(0x9f, 0xd4, 0xff),
};

/// Warm: sand and stone neutrals with a terracotta accent. Errors stay red and warnings amber.
const WARM_LIGHT: Palette = Palette {
    shell: Color32::from_rgb(0x3b, 0x31, 0x28),
    on_shell: Color32::from_rgb(0xf7, 0xf1, 0xe8),
    on_shell_weak: Color32::from_rgb(0xcb, 0xbf, 0xae),
    rail: Color32::from_rgb(0xef, 0xe7, 0xdc),
    canvas: Color32::from_rgb(0xfa, 0xf6, 0xf0),
    raised: Color32::from_rgb(0xff, 0xfd, 0xf9),
    text: Color32::from_rgb(0x2b, 0x26, 0x22),
    text_weak: Color32::from_rgb(0x6b, 0x5f, 0x55),
    border: Color32::from_rgb(0xd8, 0xcd, 0xbf),
    stripe: Color32::from_rgb(0xf3, 0xec, 0xe2),
    accent: Color32::from_rgb(0xa4, 0x45, 0x2a),
    on_accent: Color32::from_rgb(0xff, 0xff, 0xff),
    accent_soft: Color32::from_rgb(0xf6, 0xdd, 0xd3),
    error: Color32::from_rgb(0xb4, 0x23, 0x1a),
    warning: Color32::from_rgb(0x9a, 0x5b, 0x00),
    chart: Color32::from_rgb(0x8e, 0x3b, 0x20),
    chart_hover: Color32::from_rgb(0x6d, 0x2b, 0x16),
};

const WARM_DARK: Palette = Palette {
    shell: Color32::from_rgb(0x1d, 0x19, 0x15),
    on_shell: Color32::from_rgb(0xf3, 0xec, 0xe2),
    on_shell_weak: Color32::from_rgb(0xa8, 0x9b, 0x8b),
    rail: Color32::from_rgb(0x26, 0x21, 0x1c),
    canvas: Color32::from_rgb(0x1a, 0x17, 0x14),
    raised: Color32::from_rgb(0x2a, 0x25, 0x20),
    text: Color32::from_rgb(0xec, 0xe4, 0xd8),
    text_weak: Color32::from_rgb(0xb3, 0xa5, 0x96),
    border: Color32::from_rgb(0x44, 0x3b, 0x32),
    stripe: Color32::from_rgb(0x20, 0x1c, 0x18),
    accent: Color32::from_rgb(0xe0, 0x8a, 0x66),
    on_accent: Color32::from_rgb(0x2a, 0x13, 0x07),
    accent_soft: Color32::from_rgb(0x3d, 0x2a, 0x20),
    error: Color32::from_rgb(0xff, 0x8a, 0x80),
    warning: Color32::from_rgb(0xf2, 0xb8, 0x4b),
    chart: Color32::from_rgb(0xd9, 0x82, 0x5c),
    chart_hover: Color32::from_rgb(0xe8, 0xa1, 0x83),
};

/// StochCraft's own theme: [`LIGHT`] and [`DARK`].
pub fn default_theme() -> Theme {
    Theme {
        name: DEFAULT.to_owned(),
        light: LIGHT,
        dark: DARK,
    }
}

/// The built-in themes, the default first.
pub fn presets() -> Vec<Theme> {
    vec![
        default_theme(),
        Theme {
            name: "High contrast".to_owned(),
            light: HIGH_CONTRAST_LIGHT,
            dark: HIGH_CONTRAST_DARK,
        },
        Theme {
            name: "Warm".to_owned(),
            light: WARM_LIGHT,
            dark: WARM_DARK,
        },
    ]
}

/// The theme called `name`: a built-in one, else one of `custom`, else the default. Built-in
/// names come first, so a custom theme can't take one.
pub fn find(name: &str, custom: &[Theme]) -> Theme {
    presets()
        .into_iter()
        .chain(custom.iter().cloned())
        .find(|t| t.name == name)
        .unwrap_or_else(default_theme)
}

/// A theme's name as the menus and the editor show it. Built-in names are translated (the brand
/// may stay as it is in any language); a custom name is the user's own and shown as it is.
pub fn shown_name(name: &str) -> String {
    match name {
        DEFAULT => t("StochCraft").to_owned(),
        "High contrast" => t("High contrast").to_owned(),
        "Warm" => t("Warm").to_owned(),
        other => other.to_owned(),
    }
}

/// Every role of a palette with its plain-English name, for the theme editor. Destructuring means
/// a palette with a new role doesn't build until the role is listed here.
pub fn roles(p: &mut Palette) -> [(&'static str, &mut Color32); 17] {
    let Palette {
        shell,
        on_shell,
        on_shell_weak,
        rail,
        canvas,
        raised,
        text,
        text_weak,
        border,
        stripe,
        accent,
        on_accent,
        accent_soft,
        error,
        warning,
        chart,
        chart_hover,
    } = p;
    [
        (t("Header"), shell),
        (t("Text on header"), on_shell),
        (t("Weak text on header"), on_shell_weak),
        (t("Side rail"), rail),
        (t("Canvas"), canvas),
        (t("Raised surfaces"), raised),
        (t("Text"), text),
        (t("Weak text"), text_weak),
        (t("Borders"), border),
        (t("Table stripes"), stripe),
        (t("Accent"), accent),
        (t("Text on accent"), on_accent),
        (t("Soft accent"), accent_soft),
        (t("Error"), error),
        (t("Warning"), warning),
        (t("Chart"), chart),
        (t("Chart hover"), chart_hover),
    ]
}

/// The WCAG 2.x relative luminance of an sRGB colour, from 0 (black) to 1 (white).
fn luminance(c: Color32) -> f32 {
    let linear = |v: u8| {
        let s = f32::from(v) / 255.0;
        if s <= 0.03928 {
            s / 12.92
        } else {
            ((s + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(c.r()) + 0.7152 * linear(c.g()) + 0.0722 * linear(c.b())
}

/// The WCAG 2.x contrast ratio of two colours, from 1 to 21.
pub fn contrast(a: Color32, b: Color32) -> f32 {
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

/// The text and background pairs of a palette that fall below their contrast minimum, in words:
/// one line per pair. Text needs 4.5:1; the colours that carry meaning on the canvas need 3:1.
pub fn problems(p: &Palette) -> Vec<String> {
    let pairs = [
        (t("Text"), p.text, t("Canvas"), p.canvas, 4.5),
        (t("Text"), p.text, t("Raised surfaces"), p.raised, 4.5),
        (t("Text"), p.text, t("Side rail"), p.rail, 4.5),
        (t("Text"), p.text, t("Table stripes"), p.stripe, 4.5),
        (t("Weak text"), p.text_weak, t("Canvas"), p.canvas, 4.5),
        (
            t("Weak text"),
            p.text_weak,
            t("Raised surfaces"),
            p.raised,
            4.5,
        ),
        (t("Weak text"), p.text_weak, t("Side rail"), p.rail, 4.5),
        (
            t("Weak text"),
            p.text_weak,
            t("Table stripes"),
            p.stripe,
            4.5,
        ),
        (t("Text on header"), p.on_shell, t("Header"), p.shell, 4.5),
        (
            t("Weak text on header"),
            p.on_shell_weak,
            t("Header"),
            p.shell,
            3.0,
        ),
        (t("Text on accent"), p.on_accent, t("Accent"), p.accent, 4.5),
        (t("Accent"), p.accent, t("Canvas"), p.canvas, 3.0),
        (t("Error"), p.error, t("Canvas"), p.canvas, 3.0),
        (t("Warning"), p.warning, t("Canvas"), p.canvas, 3.0),
        (t("Chart"), p.chart, t("Canvas"), p.canvas, 3.0),
    ];
    pairs
        .into_iter()
        .filter_map(|(fg_role, fg, bg_role, bg, min)| {
            let ratio = contrast(fg, bg);
            if ratio >= min {
                return None;
            }
            // Rounded down, so a shown ratio never reads as passing.
            let ratio = format!("{:.1}", (ratio * 10.0).floor() / 10.0);
            let min = format!("{min:.1}");
            Some(tf(
                "{text} on {background}: {ratio}:1, needs {min}:1",
                &[
                    ("text", fg_role),
                    ("background", bg_role),
                    ("ratio", &ratio),
                    ("min", &min),
                ],
            ))
        })
        .collect()
}

/// Colours as `"#rrggbb"` text, so a saved theme can be read and edited by hand.
mod colour {
    use egui::Color32;
    use serde::de::Error;

    pub fn serialize<S: serde::Serializer>(c: &Color32, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&format!("#{:02x}{:02x}{:02x}", c.r(), c.g(), c.b()))
    }

    pub fn deserialize<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Color32, D::Error> {
        let text = <String as serde::Deserialize>::deserialize(d)?;
        let hex = text.strip_prefix('#').unwrap_or(text.as_str());
        if hex.len() != 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(D::Error::custom(format!("expected #rrggbb, not {text:?}")));
        }
        let value = u32::from_str_radix(hex, 16).map_err(D::Error::custom)?;
        Ok(Color32::from_rgb(
            (value >> 16) as u8,
            (value >> 8) as u8,
            value as u8,
        ))
    }
}

/// The palettes of the theme in use, kept in egui's context for [`palette`].
#[derive(Clone, Copy)]
struct Active {
    light: Palette,
    dark: Palette,
}

/// The key the palettes are stored under: made once, so every frame reads the same one.
static ACTIVE: std::sync::LazyLock<egui::Id> =
    std::sync::LazyLock::new(|| egui::Id::unique("stochcraft-palettes"));

/// The palette for the theme egui is showing: the theme in use's, or StochCraft's before one is.
pub fn palette(ctx: &egui::Context) -> Palette {
    let active = ctx
        .data(|d| d.get_temp::<Active>(*ACTIVE))
        .unwrap_or(Active {
            light: LIGHT,
            dark: DARK,
        });
    if ctx.theme() == egui::Theme::Dark {
        active.dark
    } else {
        active.light
    }
}

/// Makes `theme`'s palettes the ones [`palette`] reads. The app calls it every frame, so the
/// palettes it draws with are always those of the theme in use.
pub fn use_palettes(ctx: &egui::Context, theme: &Theme) {
    let active = Active {
        light: theme.light,
        dark: theme.dark,
    };
    ctx.data_mut(|d| d.insert_temp(*ACTIVE, active));
}

fn plex(weight: f32) -> FontData {
    FontData::from_static(PLEX_SANS).tweak(FontTweak {
        coords: VariationCoords::new([("wght", weight)]),
        ..FontTweak::default()
    })
}

// The generated tables are bare `EmbeddedFace { … }` or `ServedFace { … }` literals.
#[cfg(not(target_arch = "wasm32"))]
use craft_fonts_build::EmbeddedFace;
#[cfg(target_arch = "wasm32")]
use craft_ui::fonts::ServedFace;

/// The craft-fonts faces this build embeds (build.rs; empty without `CRAFT_FONTS_DIR`).
#[cfg(not(target_arch = "wasm32"))]
static CRAFT_FONTS: &[EmbeddedFace] = include!(concat!(env!("OUT_DIR"), "/craft_fonts.rs"));

/// The craft-fonts faces the web build serves beside the wasm, fetched when needed (build.rs;
/// empty without `CRAFT_FONTS_DIR`).
#[cfg(target_arch = "wasm32")]
static CRAFT_FONTS: &[ServedFace] = include!(concat!(env!("OUT_DIR"), "/craft_fonts.rs"));

/// The interface's font stack for `ui_language` (a locale code), from craft-ui: IBM Plex
/// (regular, semibold and mono) first, then egui's Hack and emoji faces for the symbols Plex
/// lacks (⌘ ⇧ ●), then craft-fonts' faces for other scripts, the UI language's first, then the
/// installed system fonts (desktop builds with craft-ui's `system_fonts`; none on the web), so a
/// file name or label in any script draws. On the web the craft-fonts faces are fetched: the UI
/// language's at once, any other the first time a character needs it.
pub fn fonts(ui_language: &str) -> craft_ui::fonts::Stack {
    use craft_ui::fonts::{Family, I18nScripts, UiFonts};
    let offered: Vec<&str> = crate::i18n::languages()
        .into_iter()
        .map(|(lang, _)| crate::i18n::code(lang))
        .collect();
    let strong = FontFamily::Name(STRONG.into());
    let fonts = UiFonts::new(&I18nScripts)
        .app_face("plex", plex(420.0), &[FontFamily::Proportional])
        .app_face(STRONG, plex(600.0), std::slice::from_ref(&strong))
        .family(Family::named(STRONG).weight(600))
        .app_face(
            "plex-mono",
            FontData::from_static(PLEX_MONO),
            &[FontFamily::Monospace],
        )
        .egui_face("Hack", &[FontFamily::Proportional, strong.clone()])
        .egui_face("NotoEmoji-Regular", &[FontFamily::Proportional, strong]);
    #[cfg(not(target_arch = "wasm32"))]
    let fonts = {
        let craft: Vec<craft_ui::fonts::CraftFace> = CRAFT_FONTS
            .iter()
            .copied()
            .map(craft_ui::fonts::CraftFace::from)
            .collect();
        fonts.craft_faces(&craft)
    };
    #[cfg(target_arch = "wasm32")]
    let fonts = fonts.served_faces(CRAFT_FONTS);
    let mut fonts = fonts.languages(&offered).ui_language(ui_language);
    // `None` on the web and without craft-ui's `system_fonts` feature.
    if let Some(system) = craft_ui::fonts::system_font_provider(ui_language) {
        fonts = fonts.system_provider(system);
    }
    fonts.stack()
}

/// Installs the font stack for `ui_language`. Call again when the interface language changes.
pub fn install_fonts(ctx: &egui::Context, ui_language: &str) {
    fonts(ui_language).install(ctx);
}

/// The offered languages whose names, in their own script, would fetch a font if drawn now: on
/// the web, those with a character that only a served face draws (日本語, 한국어, हिन्दी, ไทย…)
/// while none of the faces that draw it has been fetched. View ▸ Language shows these by their
/// English names, so opening it fetches nothing; once the face arrives (the language was chosen,
/// or text needed it) the name shows in its own script. Always empty on the desktop, whose faces
/// are embedded or the system's.
pub fn names_waiting_for_fonts(ctx: &egui::Context) -> Vec<craft_i18n::Lang> {
    #[cfg(target_arch = "wasm32")]
    {
        use std::sync::LazyLock;
        /// For each language, per character that would fetch: the served faces, any of which
        /// draws it. Which faces draw a character doesn't depend on the UI language.
        static NEEDS: LazyLock<Vec<(craft_i18n::Lang, Vec<Vec<String>>)>> = LazyLock::new(|| {
            let stack = fonts("en");
            crate::i18n::languages()
                .into_iter()
                .map(|(lang, name)| {
                    let needs = name
                        .chars()
                        .map(|c| {
                            stack
                                .downloads_for(&FontFamily::Proportional, c)
                                .into_iter()
                                .map(ServedFace::name)
                                .collect::<Vec<_>>()
                        })
                        .filter(|faces| !faces.is_empty())
                        .collect();
                    (lang, needs)
                })
                .filter(|(_, needs): &(_, Vec<Vec<String>>)| !needs.is_empty())
                .collect()
        });
        // A served face's data is in the installed definitions once fetched (craft-ui adds it
        // under the face's name); asking egui for the glyph instead would start the fetch.
        ctx.fonts(|f| {
            let loaded = |face: &String| f.definitions().font_data.contains_key(face);
            NEEDS
                .iter()
                .filter(|(_, needs)| needs.iter().any(|faces| !faces.iter().any(loaded)))
                .map(|(lang, _)| *lang)
                .collect()
        })
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = ctx;
        Vec::new()
    }
}

/// Installs the type scale and `theme`'s palettes, following the system's light or dark mode until
/// the app chooses one. Call once at start, after [`install_fonts`].
pub fn install(ctx: &egui::Context, theme: &Theme) {
    ctx.set_theme(egui::ThemePreference::System);
    apply(ctx, theme);
}

/// Styles both egui themes with `theme`'s palettes, so a change of colour theme shows at once.
pub fn apply(ctx: &egui::Context, theme: &Theme) {
    use_palettes(ctx, theme);
    ctx.style_mut_of(egui::Theme::Light, |s| style(s, &theme.light));
    ctx.style_mut_of(egui::Theme::Dark, |s| style(s, &theme.dark));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_built_in_palette_meets_the_contrast_minimums() {
        for theme in presets() {
            for p in [theme.light, theme.dark] {
                assert_eq!(problems(&p), Vec::<String>::new(), "{}", theme.name);
            }
        }
    }

    #[test]
    fn a_pair_below_its_minimum_is_named_with_its_ratio() {
        let mut p = LIGHT;
        p.text_weak = Color32::from_rgb(0xc8, 0xc8, 0xc8);
        let found = problems(&p);
        assert!(
            found
                .iter()
                .any(|m| m.starts_with("Weak text on Canvas: ") && m.ends_with(":1, needs 4.5:1")),
            "{found:?}"
        );
        assert!(
            found.iter().all(|m| !m.starts_with("Text on Canvas")),
            "{found:?}"
        );
    }

    #[test]
    fn black_on_white_is_twenty_one_to_one() {
        assert!((contrast(Color32::BLACK, Color32::WHITE) - 21.0).abs() < 0.01);
    }

    #[test]
    fn plex_leads_and_egui_faces_fill_in() {
        let stack = fonts("en");
        let chain = stack.chain(&FontFamily::Proportional);
        assert_eq!(chain.first().map(String::as_str), Some("plex"), "{chain:?}");
        assert!(chain.iter().any(|f| f == "Hack"), "{chain:?}");
        let strong = stack.chain(&FontFamily::Name(STRONG.into()));
        assert_eq!(
            strong.first().map(String::as_str),
            Some(STRONG),
            "{strong:?}"
        );
        assert_eq!(
            stack
                .chain(&FontFamily::Monospace)
                .first()
                .map(String::as_str),
            Some("plex-mono")
        );
    }
}
