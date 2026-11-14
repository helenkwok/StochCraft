//! Chart palettes: the colours of the series in a chart, as built-in presets from published sources
//! and the user's own, with a check for colour-vision deficiency and contrast.
//!
//! The check follows published methods: OKLab (Björn Ottosson, 2020), colour-vision simulation with
//! the Machado, Oliveira and Fernandes (2009) matrices at severity 1.0, and WCAG 2.x contrast.
//! Where each preset's colours come from is recorded in ATTRIBUTION.md.

use egui::Color32;

use crate::i18n::{t, tf};
use crate::theme;

/// What a palette is for. Categorical colours tell separate series apart; sequential colours run
/// from low to high; diverging colours run from one extreme through a neutral middle to the other.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PaletteKind {
    Categorical,
    Sequential,
    Diverging,
}

impl PaletteKind {
    /// The kind's heading in the editor.
    pub fn label(self) -> &'static str {
        match self {
            Self::Categorical => t("Categorical"),
            Self::Sequential => t("Sequential"),
            Self::Diverging => t("Diverging"),
        }
    }

    /// The fewest colours a palette of this kind may have: a diverging scale needs both ends and a
    /// middle, and a categorical one needs at least two series to tell apart.
    pub fn min_colours(self) -> usize {
        match self {
            Self::Categorical => 2,
            Self::Sequential | Self::Diverging => 3,
        }
    }
}

/// A named list of colours in order: slot 0 is the first series, or the low end of a ramp.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ChartPalette {
    pub name: String,
    pub kind: PaletteKind,
    #[serde(with = "theme::colour_list")]
    pub colors: Vec<Color32>,
}

/// The palette series use when the preferences name none, or one that doesn't exist.
pub const DEFAULT: &str = "Okabe–Ito";

/// Colours from a `0xRRGGBB` literal, so the presets read like the sources' hex values.
const fn hex(v: u32) -> Color32 {
    Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

fn colours(values: &[u32]) -> Vec<Color32> {
    values.iter().map(|v| hex(*v)).collect()
}

/// The built-in palettes, the default first. Sources (ATTRIBUTION.md):
/// - Okabe–Ito: Okabe and Ito, "Color Universal Design", Fig. 16, R,G,B (0-255). Order:
///   orange, sky blue, bluish green, yellow, blue, vermilion, reddish purple, black (the authors
///   list black first; it is last here so the first series stays visible on a dark canvas).
/// - Tol bright, Tol muted, Tol sunset: Paul Tol, "Colour schemes" (sronpersonalpages.nl/~pault).
/// - ColorBrewer Dark2, Set2, Blues, RdBu: Cynthia Brewer, colorbrewer2.org (9/11-class, or the
///   8-class qualitative schemes).
/// - Viridis: matplotlib's colormaps (BIDS/colormap, CC0), sampled at 9 evenly spaced steps.
pub fn presets() -> Vec<ChartPalette> {
    let p = |name: &str, kind, values: &[u32]| ChartPalette {
        name: name.to_owned(),
        kind,
        colors: colours(values),
    };
    use PaletteKind::{Categorical, Diverging, Sequential};
    vec![
        p(
            DEFAULT,
            Categorical,
            &[
                0xE69F00, 0x56B4E9, 0x009E73, 0xF0E442, 0x0072B2, 0xD55E00, 0xCC79A7, 0x000000,
            ],
        ),
        p(
            "Tol bright",
            Categorical,
            &[
                0x4477AA, 0xEE6677, 0x228833, 0xCCBB44, 0x66CCEE, 0xAA3377, 0xBBBBBB,
            ],
        ),
        p(
            "Tol muted",
            Categorical,
            &[
                0xCC6677, 0x332288, 0xDDCC77, 0x117733, 0x88CCEE, 0x882255, 0x44AA99, 0x999933,
                0xAA4499,
            ],
        ),
        p(
            "ColorBrewer Dark2",
            Categorical,
            &[
                0x1B9E77, 0xD95F02, 0x7570B3, 0xE7298A, 0x66A61E, 0xE6AB02, 0xA6761D, 0x666666,
            ],
        ),
        p(
            "ColorBrewer Set2",
            Categorical,
            &[
                0x66C2A5, 0xFC8D62, 0x8DA0CB, 0xE78AC3, 0xA6D854, 0xFFD92F, 0xE5C494, 0xB3B3B3,
            ],
        ),
        p(
            "Viridis",
            Sequential,
            &[
                0x440154, 0x472D7B, 0x3B528B, 0x2C728E, 0x21918C, 0x27AD81, 0x5CC863, 0xAADC32,
                0xFDE725,
            ],
        ),
        p(
            "ColorBrewer Blues",
            Sequential,
            &[
                0xF7FBFF, 0xDEEBF7, 0xC6DBEF, 0x9ECAE1, 0x6BAED6, 0x4292C6, 0x2171B5, 0x08519C,
                0x08306B,
            ],
        ),
        p(
            "ColorBrewer RdBu",
            Diverging,
            &[
                0x67001F, 0xB2182B, 0xD6604D, 0xF4A582, 0xFDDBC7, 0xF7F7F7, 0xD1E5F0, 0x92C5DE,
                0x4393C3, 0x2166AC, 0x053061,
            ],
        ),
        p(
            "Tol sunset",
            Diverging,
            &[
                0x125A56, 0x00767B, 0x238F9D, 0x42A7C6, 0x60BCE9, 0x9DCCEF, 0xC6DBED, 0xDEE6E7,
                0xECEADA, 0xF0E6B2, 0xF9D576, 0xFFB954, 0xFD9A44, 0xF57634, 0xE94C1F, 0xD11807,
                0xA01813,
            ],
        ),
    ]
}

/// The palette called `name`: a built-in one, else one of `custom`, else the default. Built-in
/// names come first, so a custom palette can't take one.
pub fn find(name: &str, custom: &[ChartPalette]) -> ChartPalette {
    presets()
        .into_iter()
        .chain(custom.iter().cloned())
        .find(|p| p.name == name)
        .unwrap_or_else(default_palette)
}

fn default_palette() -> ChartPalette {
    presets()
        .into_iter()
        .next()
        .unwrap_or_else(|| ChartPalette {
            name: DEFAULT.to_owned(),
            kind: PaletteKind::Categorical,
            colors: Vec::new(),
        })
}

/// The palette series colours come from: the one named, when it is categorical. A sequential or
/// diverging palette can't colour series, so the default is used instead.
pub fn in_use(name: &str, custom: &[ChartPalette]) -> ChartPalette {
    let palette = find(name, custom);
    if palette.kind == PaletteKind::Categorical {
        palette
    } else {
        default_palette()
    }
}

/// The active categorical palette's colours, kept in egui's context for [`series_color`].
static ACTIVE: std::sync::LazyLock<egui::Id> =
    std::sync::LazyLock::new(|| egui::Id::unique("stochcraft-chart-palette"));

/// Makes `palette` the one [`series_color`] reads. The app calls it every frame with the palette in
/// use, as it does for colour themes.
pub fn use_palette(ctx: &egui::Context, palette: &ChartPalette) {
    let colors = palette.colors.clone();
    ctx.data_mut(|d| d.insert_temp(*ACTIVE, colors));
}

/// The colour of series `i` in the palette in use. Beyond the palette's last colour this returns
/// the last colour: a chart with more series than colours should fold the rest into "Other"
/// rather than reuse a colour.
pub fn series_color(ctx: &egui::Context, i: usize) -> Color32 {
    let colors: Vec<Color32> = ctx
        .data(|d| d.get_temp::<Vec<Color32>>(*ACTIVE))
        .unwrap_or_else(|| colours(&[0x000000]));
    colors
        .get(i)
        .or(colors.last())
        .copied()
        .unwrap_or(Color32::BLACK)
}

/// How serious a finding is. A warning says the chart may be hard to read; a failure says it
/// can't be read reliably by everyone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Warn,
    Fail,
}

/// One result of [`check`]: the slots it concerns (0-based), and a plain-English message naming the
/// measure and the threshold.
#[derive(Debug, Clone, PartialEq)]
pub struct Finding {
    pub level: Level,
    pub slots: Vec<usize>,
    pub message: String,
}

// Thresholds, as ΔE (OKLab distance × 100), contrast ratio, chroma and lightness difference.
/// Adjacent slots that differ by less than this under red-green colour blindness are a warning
/// (below [`CVD_FAIL`] they are a failure).
const CVD_WARN: f64 = 8.0;
const CVD_FAIL: f64 = 6.0;
/// Adjacent slots must differ by at least this under normal colour vision.
const NORMAL_MIN: f64 = 15.0;
/// Each colour must contrast with the chart surface by at least this (WCAG graphical objects).
const SURFACE_CONTRAST_MIN: f32 = 3.0;
/// Below this a colour all but vanishes into the surface (black on the dark canvas is 1.1:1): a
/// failure rather than a warning. StochCraft's own line, not a WCAG figure.
const SURFACE_CONTRAST_FAIL: f32 = 1.5;
/// Colours with less OKLCH chroma than this read as grey.
const CHROMA_MIN: f64 = 0.10;
/// Colours darker than this are black or near it: a deliberate neutral, not a grey, so the
/// chroma check leaves them alone (their contrast is still checked).
const NEUTRAL_L: f64 = 0.2;
/// Adjacent ramp steps must differ in lightness by at least this.
const LIGHTNESS_STEP_MIN: f64 = 0.06;

/// A colour in OKLab: lightness and the two opponent axes.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Lab {
    l: f64,
    a: f64,
    b: f64,
}

impl Lab {
    /// OKLCH chroma: the distance from the neutral axis.
    fn chroma(self) -> f64 {
        self.a.hypot(self.b)
    }
}

/// Machado, Oliveira and Fernandes (2009), Table 1 at severity 1.0, applied to linear RGB.
const PROTANOPIA: [[f64; 3]; 3] = [
    [0.152_286, 1.052_583, -0.204_868],
    [0.114_503, 0.786_281, 0.099_216],
    [-0.003_882, -0.048_116, 1.051_998],
];
const DEUTERANOPIA: [[f64; 3]; 3] = [
    [0.367_322, 0.860_646, -0.227_968],
    [0.280_085, 0.672_501, 0.047_413],
    [-0.011_820, 0.042_940, 0.968_881],
];

/// The sRGB transfer function, with the workspace's own `powf` so every target agrees.
fn srgb_to_linear(v: f64) -> f64 {
    if v <= 0.040_45 {
        v / 12.92
    } else {
        stochcraft_random::math::powf((v + 0.055) / 1.055, 2.4)
    }
}

fn to_linear(c: Color32) -> [f64; 3] {
    [c.r(), c.g(), c.b()].map(|v| srgb_to_linear(f64::from(v) / 255.0))
}

/// Ottosson's linear sRGB → OKLab, with the matrices of his published post.
fn oklab(rgb: [f64; 3]) -> Lab {
    let [r, g, b] = rgb;
    let l = (0.412_221_470_8 * r + 0.536_332_536_3 * g + 0.051_445_992_9 * b).cbrt();
    let m = (0.211_903_498_2 * r + 0.680_699_545_1 * g + 0.107_396_956_6 * b).cbrt();
    let s = (0.088_302_461_9 * r + 0.281_718_837_6 * g + 0.629_978_700_5 * b).cbrt();
    Lab {
        l: 0.210_454_255_3 * l + 0.793_617_785_0 * m - 0.004_072_046_8 * s,
        a: 1.977_998_495_1 * l - 2.428_592_205_0 * m + 0.450_593_709_9 * s,
        b: 0.025_904_037_1 * l + 0.782_771_766_2 * m - 0.808_675_766_0 * s,
    }
}

fn lab(c: Color32) -> Lab {
    oklab(to_linear(c))
}

/// Distance in OKLab, scaled so that 1 unit is 0.01 (the ΔE the thresholds are written in).
fn delta_e(x: Lab, y: Lab) -> f64 {
    100.0 * (x.l - y.l).hypot(x.a - y.a).hypot(x.b - y.b)
}

/// The linear RGB a colour-blind viewer sees: the matrix applied, clamped to the displayable range.
fn simulate_linear(c: Color32, matrix: &[[f64; 3]; 3]) -> [f64; 3] {
    let lin = to_linear(c);
    matrix.map(|row| (row[0] * lin[0] + row[1] * lin[1] + row[2] * lin[2]).clamp(0.0, 1.0))
}

/// The colour a viewer with the deficiency sees, as OKLab.
fn simulated_lab(c: Color32, matrix: &[[f64; 3]; 3]) -> Lab {
    oklab(simulate_linear(c, matrix))
}

/// A ratio or difference rounded down to one decimal, so a shown value never reads as passing.
fn down(v: f64) -> String {
    down_by(v, 10.0)
}

/// `v` rounded down to the places `scale` gives: 10 for one decimal, 100 for two.
fn down_by(v: f64, scale: f64) -> String {
    let places = if scale >= 100.0 { 2 } else { 1 };
    format!("{:.*}", places, (v * scale).floor() / scale)
}

fn slot_name(i: usize) -> String {
    tf("Colour {n}", &[("n", &(i + 1).to_string())])
}

/// Checks `p` against the chart `surface` it is drawn on. Categorical: each colour contrasts with
/// the surface, colours differ enough for neighbours with normal vision and with red-green colour
/// blindness, and no colour reads as grey. Sequential and diverging: lightness goes one way
/// (diverging: from each end towards the middle) and neighbours differ enough in lightness.
pub fn check(p: &ChartPalette, surface: Color32) -> Vec<Finding> {
    match p.kind {
        PaletteKind::Categorical => categorical(&p.colors, surface),
        PaletteKind::Sequential => ramp(&p.colors, false),
        PaletteKind::Diverging => ramp(&p.colors, true),
    }
}

fn categorical(colors: &[Color32], surface: Color32) -> Vec<Finding> {
    let mut found = Vec::new();
    let normal: Vec<Lab> = colors.iter().map(|c| lab(*c)).collect();
    for (i, c) in colors.iter().enumerate() {
        let ratio = theme::contrast(*c, surface);
        // A warning, not a failure: pale colours are fine for fills and bars with labels, and the
        // well-tested sets (Okabe–Ito's orange, sky blue and yellow on white) are below 3:1.
        if ratio < SURFACE_CONTRAST_MIN {
            found.push(Finding {
                level: if ratio < SURFACE_CONTRAST_FAIL {
                    Level::Fail
                } else {
                    Level::Warn
                },
                slots: vec![i],
                message: tf(
                    "{colour} has contrast {ratio}:1 on the chart background, below 3:1: label its series or show the numbers in a table.",
                    &[
                        ("colour", &slot_name(i)),
                        ("ratio", &down(f64::from(ratio))),
                    ],
                ),
            });
        }
        let l = normal[i];
        if l.l >= NEUTRAL_L && l.chroma() < CHROMA_MIN {
            found.push(Finding {
                level: Level::Warn,
                slots: vec![i],
                message: tf(
                    "{colour} reads as grey (chroma {chroma}, needs 0.10).",
                    &[
                        ("colour", &slot_name(i)),
                        ("chroma", &down_by(l.chroma(), 100.0)),
                    ],
                ),
            });
        }
    }
    for i in 0..colors.len().saturating_sub(1) {
        let (a, b) = (i, i + 1);
        let slots = vec![a, b];
        let names = [slot_name(a), slot_name(b)];
        let de_normal = delta_e(normal[a], normal[b]);
        if de_normal < NORMAL_MIN {
            found.push(Finding {
                level: Level::Fail,
                slots: slots.clone(),
                message: tf(
                    "{first} and {second} differ by {de} to viewers with normal colour vision, needs 15.",
                    &[
                        ("first", &names[0]),
                        ("second", &names[1]),
                        ("de", &down(de_normal)),
                    ],
                ),
            });
        }
        // The worse of protanopia and deuteranopia: both are red-green deficiencies.
        let de_cvd = [PROTANOPIA, DEUTERANOPIA]
            .iter()
            .map(|m| delta_e(simulated_lab(colors[a], m), simulated_lab(colors[b], m)))
            .fold(f64::INFINITY, f64::min);
        if de_cvd < CVD_FAIL {
            found.push(Finding {
                level: Level::Fail,
                slots: slots.clone(),
                message: tf(
                    "{first} and {second} differ by {de} to viewers with red-green colour blindness, needs 8.",
                    &[
                        ("first", &names[0]),
                        ("second", &names[1]),
                        ("de", &down(de_cvd)),
                    ],
                ),
            });
        } else if de_cvd < CVD_WARN {
            found.push(Finding {
                level: Level::Warn,
                slots: slots.clone(),
                message: tf(
                    "{first} and {second} differ by {de} to viewers with red-green colour blindness, needs 8: tell them apart only with labels.",
                    &[
                        ("first", &names[0]),
                        ("second", &names[1]),
                        ("de", &down(de_cvd)),
                    ],
                ),
            });
        }
    }
    found
}

/// The index where the direction of change first reverses, if it does.
fn first_reversal(values: &[f64]) -> Option<usize> {
    let steps: Vec<(usize, f64)> = values
        .windows(2)
        .enumerate()
        .map(|(i, w)| (i, w[1] - w[0]))
        .filter(|(_, d)| d.abs() > 1e-9)
        .collect();
    let direction = steps.first()?.1.signum();
    steps
        .iter()
        .find(|(_, d)| d.signum() != direction)
        .map(|(i, _)| *i)
}

fn ramp(colors: &[Color32], diverging: bool) -> Vec<Finding> {
    let lightness: Vec<f64> = colors.iter().map(|c| lab(*c).l).collect();
    let mut found = Vec::new();
    // Diverging: each half must be monotone, so lightness moves towards the middle from each end.
    let segments: Vec<(usize, &[f64])> = if diverging && lightness.len() > 2 {
        let mid = lightness.len() / 2;
        vec![(0, &lightness[..=mid]), (mid, &lightness[mid..])]
    } else {
        vec![(0, &lightness[..])]
    };
    for (offset, values) in segments {
        if let Some(i) = first_reversal(values) {
            let (a, b) = (offset + i, offset + i + 1);
            found.push(Finding {
                level: Level::Fail,
                slots: vec![a, b],
                message: tf(
                    "Lightness turns back between {first} and {second}; a ramp must only get lighter or only darker.",
                    &[("first", &slot_name(a)), ("second", &slot_name(b))],
                ),
            });
        }
    }
    for i in 0..lightness.len().saturating_sub(1) {
        let step = (lightness[i + 1] - lightness[i]).abs();
        if step < LIGHTNESS_STEP_MIN {
            found.push(Finding {
                level: Level::Warn,
                slots: vec![i, i + 1],
                message: tf(
                    "{first} and {second} differ in lightness by {step}, needs 0.06.",
                    &[
                        ("first", &slot_name(i)),
                        ("second", &slot_name(i + 1)),
                        ("step", &down_by(step, 100.0)),
                    ],
                ),
            });
        }
    }
    found
}

#[cfg(test)]
// A missing preset is a bug in the test's setup, so it fails the test.
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    const WHITE: Color32 = Color32::WHITE;

    fn linear_to_srgb(v: f64) -> f64 {
        if v <= 0.003_130_8 {
            12.92 * v
        } else {
            1.055 * stochcraft_random::math::powf(v, 1.0 / 2.4) - 0.055
        }
    }

    /// The colour a viewer with the deficiency sees, as an 8-bit sRGB colour (for the tests).
    fn simulate_srgb(c: Color32, matrix: &[[f64; 3]; 3]) -> Color32 {
        let [r, g, b] =
            simulate_linear(c, matrix).map(|v| (linear_to_srgb(v) * 255.0).round() as u8);
        Color32::from_rgb(r, g, b)
    }

    fn palette(kind: PaletteKind, colors: Vec<Color32>) -> ChartPalette {
        ChartPalette {
            name: "test".to_owned(),
            kind,
            colors,
        }
    }

    fn preset(name: &str) -> ChartPalette {
        presets()
            .into_iter()
            .find(|p| p.name == name)
            .expect("a preset")
    }

    #[test]
    fn okabe_ito_passes_the_colour_vision_checks_on_white() {
        // Only the surface-contrast warning, for orange, sky blue and yellow (slots 0, 1 and 3),
        // which are too pale for 3:1 on white. Neighbours still differ enough for everyone, and
        // no colour reads as grey (black is a neutral and exempt).
        let found = check(&preset(DEFAULT), WHITE);
        assert!(
            found
                .iter()
                .all(|f| f.level == Level::Warn && f.message.contains("contrast")),
            "{found:?}"
        );
        let slots: Vec<usize> = found.iter().flat_map(|f| f.slots.clone()).collect();
        assert_eq!(slots, vec![0, 1, 3], "{found:?}");
    }

    #[test]
    fn two_near_identical_reds_fail_normal_vision() {
        let p = palette(PaletteKind::Categorical, vec![hex(0xFF0000), hex(0xF00505)]);
        let found = check(&p, WHITE);
        assert!(
            found.iter().any(|f| f.level == Level::Fail
                && f.slots == vec![0, 1]
                && f.message.contains("normal colour vision")),
            "{found:?}"
        );
    }

    #[test]
    fn viridis_is_monotone_and_passes() {
        let found = check(&preset("Viridis"), WHITE);
        assert!(found.iter().all(|f| f.level != Level::Fail), "{found:?}");
    }

    #[test]
    fn a_hand_made_non_monotone_ramp_is_flagged() {
        let p = palette(
            PaletteKind::Sequential,
            vec![hex(0x222222), hex(0x888888), hex(0x444444), hex(0xEEEEEE)],
        );
        let found = check(&p, WHITE);
        assert!(
            found
                .iter()
                .any(|f| f.level == Level::Fail && f.slots == vec![1, 2]),
            "{found:?}"
        );
    }

    #[test]
    fn oklab_matches_ottosson_reference_values() {
        // Ottosson's reference values: white (L 1, a 0, b 0), red, green and blue.
        let cases = [
            (WHITE, 1.0, 0.0, 0.0),
            (hex(0xFF0000), 0.627_955, 0.224_863, 0.125_846),
            (hex(0x00FF00), 0.866_432, -0.233_888, 0.179_498),
            (hex(0x0000FF), 0.452_014, -0.032_457, -0.311_528),
        ];
        for (c, l, a, b) in cases {
            let got = lab(c);
            assert!((got.l - l).abs() < 1e-3, "{c:?} L {got:?}");
            assert!((got.a - a).abs() < 1e-3, "{c:?} a {got:?}");
            assert!((got.b - b).abs() < 1e-3, "{c:?} b {got:?}");
        }
    }

    #[test]
    fn machado_protanopia_of_pure_red_matches_the_hand_derivation() {
        // Pure red is linear (1, 0, 0), so the output is the first column of the matrix:
        // (0.152286, 0.114503, -0.003882), clamped to (0.152286, 0.114503, 0). sRGB encoding:
        // 1.055 * 0.152286^(1/2.4) - 0.055 = 0.42661, times 255 = 108.8 → 109;
        // 1.055 * 0.114503^(1/2.4) - 0.055 = 0.37171, times 255 = 94.8 → 95; blue 0.
        let got = simulate_srgb(hex(0xFF0000), &PROTANOPIA);
        assert!(got.r().abs_diff(109) <= 1, "{got:?}");
        assert!(got.g().abs_diff(95) <= 1, "{got:?}");
        assert_eq!(got.b(), 0, "{got:?}");
    }

    #[test]
    fn a_diverging_ramp_must_rise_and_fall_about_its_middle() {
        let monotone_ends = palette(
            PaletteKind::Diverging,
            vec![
                hex(0x67001F),
                hex(0xD6604D),
                hex(0xF7F7F7),
                hex(0x4393C3),
                hex(0x053061),
            ],
        );
        let found = check(&monotone_ends, WHITE);
        assert!(found.iter().all(|f| f.level != Level::Fail), "{found:?}");
        let bump = palette(
            PaletteKind::Diverging,
            vec![
                hex(0x67001F),
                hex(0xF7F7F7),
                hex(0x222222),
                hex(0xF7F7F7),
                hex(0x053061),
            ],
        );
        let found = check(&bump, WHITE);
        assert!(found.iter().any(|f| f.level == Level::Fail), "{found:?}");
    }
}
