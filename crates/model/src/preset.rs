//! Selectable style presets — axis frame styles and series color cycles.
//!
//! Both enums are **fieldless** on purpose: they cross the wasm boundary as
//! plain integers (`wasm_bindgen` exposes C-like enums directly), and a host
//! UI can list variants in a dropdown without knowing anything else about the
//! model. Names describe the *style*, not the tool that popularized it.
//!
//! Applying a preset only mutates `Config` / `SeriesConfig` — the usual dirty
//! rules apply on top: wrap axis-preset application in
//! `Chart::with_decoration_change` (raster only), and rebuild each affected
//! series' GPU style (`Renderer::create_style_for_series`) after recoloring.

use crate::color::Color;
use crate::config::{AxisOptions, Config, TickVisibility};
use crate::data_config::{DataRenderType, SeriesConfig};

const fn rgb8(r: u8, g: u8, b: u8) -> Color {
    Color::new(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0)
}

// ============================================================================
// Axis presets.
// ============================================================================

/// Whole-chart axis frame styles, applied to all four axes at once via
/// [`Config::apply_axis_preset`].
///
/// Every preset keeps labels and titles on the bottom/left axes only and
/// leaves data ranges, margins, tick lengths, colors, and title text
/// untouched — it only switches which axis lines are drawn, the tick
/// direction, and which sides show tick-value labels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AxisPreset {
    /// Closed box frame with ticks pointing **inward** on all four sides —
    /// the classic lab-instrument / scientific-plotting default.
    BoxedInward,
    /// Closed box frame, ticks pointing **outward** on the labeled
    /// bottom/left axes only — the common plotting-library default.
    BoxedOutward,
    /// Open L-frame: bottom/left axes only, ticks outward — the typical
    /// publication / biostatistics style.
    OpenOutward,
    /// Open L-frame with inward ticks — journals that want an open frame
    /// without ticks protruding into the margins.
    OpenInward,
    /// No axis lines and no ticks; tick-value labels only. Pair with a grid
    /// for a minimal, web-dashboard look.
    Minimal,
}

/// Per-side knobs a preset controls.
#[derive(Clone)]
struct SideStyle {
    line: bool,
    tick: TickVisibility,
    labels: bool,
}

fn apply_side(axis: &mut AxisOptions, s: SideStyle) {
    axis.line_visible = s.line;
    axis.tick = s.tick;
    axis.label_style.label_visible = s.labels;
}

impl Config {
    /// Apply `preset` to all four axes at once. See [`AxisPreset`] for what
    /// is (and is not) touched.
    pub fn apply_axis_preset(&mut self, preset: AxisPreset) {
        use TickVisibility::{Inside, None as NoTick, Outside};

        // (labeled = bottom/left, unlabeled = top/right) side styles.
        let (labeled, unlabeled) = match preset {
            AxisPreset::BoxedInward => (
                SideStyle { line: true, tick: Inside, labels: true },
                SideStyle { line: true, tick: Inside, labels: false },
            ),
            AxisPreset::BoxedOutward => (
                SideStyle { line: true, tick: Outside, labels: true },
                SideStyle { line: true, tick: NoTick, labels: false },
            ),
            AxisPreset::OpenOutward => (
                SideStyle { line: true, tick: Outside, labels: true },
                SideStyle { line: false, tick: NoTick, labels: false },
            ),
            AxisPreset::OpenInward => (
                SideStyle { line: true, tick: Inside, labels: true },
                SideStyle { line: false, tick: NoTick, labels: false },
            ),
            AxisPreset::Minimal => (
                SideStyle { line: false, tick: NoTick, labels: true },
                SideStyle { line: false, tick: NoTick, labels: false },
            ),
        };

        apply_side(&mut self.bottom_x, labeled.clone());
        apply_side(&mut self.left_y, labeled);
        apply_side(&mut self.top_x, unlabeled.clone());
        apply_side(&mut self.right_y, unlabeled);
    }
}

// ============================================================================
// Series color cycles.
// ============================================================================

/// Color rotation rules for multi-series plots. `color(i)` wraps around, so
/// any number of series can be colored consistently.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorCycle {
    /// Strong primary rotation starting at black — the classic
    /// lab-instrument increment list.
    Classic,
    /// High-saturation, high-contrast rotation — the look of biostatistics
    /// plotting tools, optimized for telling few series apart at a glance.
    Vivid,
    /// Ten balanced mid-saturation hues (the public "tab10" palette) — the
    /// modern general-purpose default.
    Balanced,
    /// The Okabe–Ito eight-color palette, designed to stay distinguishable
    /// under the common forms of color-vision deficiency.
    ColorblindSafe,
    /// Grayscale steps for print-safe single-ink figures.
    Monochrome,
}

const CLASSIC: &[Color] = &[
    rgb8(0, 0, 0),       // black
    rgb8(255, 0, 0),     // red
    rgb8(0, 128, 0),     // green
    rgb8(0, 0, 255),     // blue
    rgb8(255, 0, 255),   // magenta
    rgb8(0, 139, 139),   // dark cyan
    rgb8(184, 134, 11),  // dark yellow
    rgb8(0, 0, 128),     // navy
    rgb8(128, 0, 128),   // purple
    rgb8(128, 0, 0),     // wine
    rgb8(128, 128, 0),   // olive
    rgb8(255, 102, 0),   // orange
];

const VIVID: &[Color] = &[
    rgb8(0, 32, 240),    // blue
    rgb8(230, 16, 16),   // red
    rgb8(0, 153, 0),     // green
    rgb8(128, 0, 192),   // purple
    rgb8(255, 128, 0),   // orange
    rgb8(0, 0, 0),       // black
    rgb8(230, 0, 172),   // pink
    rgb8(0, 153, 153),   // teal
];

const BALANCED: &[Color] = &[
    rgb8(0x1f, 0x77, 0xb4),
    rgb8(0xff, 0x7f, 0x0e),
    rgb8(0x2c, 0xa0, 0x2c),
    rgb8(0xd6, 0x27, 0x28),
    rgb8(0x94, 0x67, 0xbd),
    rgb8(0x8c, 0x56, 0x4b),
    rgb8(0xe3, 0x77, 0xc2),
    rgb8(0x7f, 0x7f, 0x7f),
    rgb8(0xbc, 0xbd, 0x22),
    rgb8(0x17, 0xbe, 0xcf),
];

const COLORBLIND_SAFE: &[Color] = &[
    rgb8(0xe6, 0x9f, 0x00), // orange
    rgb8(0x56, 0xb4, 0xe9), // sky blue
    rgb8(0x00, 0x9e, 0x73), // bluish green
    rgb8(0xf0, 0xe4, 0x42), // yellow
    rgb8(0x00, 0x72, 0xb2), // blue
    rgb8(0xd5, 0x5e, 0x00), // vermillion
    rgb8(0xcc, 0x79, 0xa7), // reddish purple
    rgb8(0x00, 0x00, 0x00), // black
];

const MONOCHROME: &[Color] = &[
    rgb8(0, 0, 0),
    rgb8(89, 89, 89),
    rgb8(140, 140, 140),
    rgb8(191, 191, 191),
];

impl ColorCycle {
    /// The full palette in rotation order.
    pub fn colors(self) -> &'static [Color] {
        match self {
            Self::Classic => CLASSIC,
            Self::Vivid => VIVID,
            Self::Balanced => BALANCED,
            Self::ColorblindSafe => COLORBLIND_SAFE,
            Self::Monochrome => MONOCHROME,
        }
    }

    /// Color for the `index`-th series — wraps past the palette end, so it is
    /// total over any series count.
    pub fn color(self, index: usize) -> Color {
        let palette = self.colors();
        palette[index % palette.len()]
    }

    pub fn len(self) -> usize {
        self.colors().len()
    }

    pub fn is_empty(self) -> bool {
        false
    }

    /// Recolor one series as the `index`-th member of this cycle: line,
    /// scatter, and errorbar sub-styles all take the same cycle color.
    /// Rebuild the series' GPU style afterwards
    /// (`Renderer::create_style_for_series`).
    pub fn apply_to_series(self, series: &mut SeriesConfig, index: usize) {
        let c = self.color(index);
        match &mut series.render_type {
            DataRenderType::Scatter { scatter } => {
                scatter.point_color = c;
            }
            DataRenderType::Line { line } => {
                line.line_color = c;
            }
            DataRenderType::ScatterLine { scatter, line } => {
                scatter.point_color = c;
                line.line_color = c;
            }
            DataRenderType::ScatterErrorbarX { scatter, err_style, .. }
            | DataRenderType::ScatterErrorbarY { scatter, err_style, .. }
            | DataRenderType::ScatterErrorbarXY { scatter, err_style, .. } => {
                scatter.point_color = c;
                err_style.error_bar_color = c;
            }
            DataRenderType::LineScatterErrorbarX { scatter, line, err_style, .. }
            | DataRenderType::LineScatterErrorbarY { scatter, line, err_style, .. }
            | DataRenderType::LineScatterErrorbarXY { scatter, line, err_style, .. } => {
                scatter.point_color = c;
                line.line_color = c;
                err_style.error_bar_color = c;
            }
        }
    }

    /// Recolor a whole series list in order: series `i` gets `color(i)`.
    pub fn apply_to_all(self, series: &mut [SeriesConfig]) {
        for (i, s) in series.iter_mut().enumerate() {
            self.apply_to_series(s, i);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data_config::{
        DataErrorBarStyleConfig, DataLineStyleConfig, DataScatterStyleConfig, ErrorRef,
        ScatterShape,
    };
    use crate::default::default_config;
    use crate::line::LineStylePreset;

    #[test]
    fn boxed_inward_shows_full_frame_with_inside_ticks() {
        let mut cfg = default_config();
        cfg.apply_axis_preset(AxisPreset::BoxedInward);
        for axis in [&cfg.top_x, &cfg.bottom_x, &cfg.left_y, &cfg.right_y] {
            assert!(axis.line_visible);
            assert_eq!(axis.tick, TickVisibility::Inside);
        }
        assert!(cfg.bottom_x.label_style.label_visible);
        assert!(cfg.left_y.label_style.label_visible);
        assert!(!cfg.top_x.label_style.label_visible);
        assert!(!cfg.right_y.label_style.label_visible);
    }

    #[test]
    fn open_outward_hides_top_right_entirely() {
        let mut cfg = default_config();
        cfg.apply_axis_preset(AxisPreset::OpenOutward);
        assert!(cfg.bottom_x.line_visible);
        assert_eq!(cfg.bottom_x.tick, TickVisibility::Outside);
        assert!(!cfg.top_x.line_visible);
        assert_eq!(cfg.top_x.tick, TickVisibility::None);
        assert!(!cfg.right_y.line_visible);
    }

    #[test]
    fn minimal_drops_lines_and_ticks_but_keeps_labels() {
        let mut cfg = default_config();
        cfg.apply_axis_preset(AxisPreset::Minimal);
        for axis in [&cfg.top_x, &cfg.bottom_x, &cfg.left_y, &cfg.right_y] {
            assert!(!axis.line_visible);
            assert_eq!(axis.tick, TickVisibility::None);
        }
        assert!(cfg.bottom_x.label_style.label_visible);
    }

    // A preset must not disturb anything outside frame/tick/label visibility.
    #[test]
    fn preset_leaves_ranges_margins_titles_untouched() {
        let mut cfg = default_config();
        let before = cfg.clone();
        cfg.apply_axis_preset(AxisPreset::OpenOutward);

        assert_eq!(cfg.bottom_x.min, before.bottom_x.min);
        assert_eq!(cfg.bottom_x.max, before.bottom_x.max);
        assert_eq!(cfg.bottom_x.out_margin, before.bottom_x.out_margin);
        assert_eq!(cfg.left_y.major_tick_length, before.left_y.major_tick_length);
        assert_eq!(cfg.chart_title, before.chart_title);
        assert_eq!(cfg.grid, before.grid);
        assert_eq!(cfg.data_area().unwrap(), before.data_area().unwrap());
    }

    #[test]
    fn cycle_color_wraps_around() {
        for cycle in [
            ColorCycle::Classic,
            ColorCycle::Vivid,
            ColorCycle::Balanced,
            ColorCycle::ColorblindSafe,
            ColorCycle::Monochrome,
        ] {
            let n = cycle.len();
            assert!(n >= 4, "{cycle:?} palette too small");
            assert_eq!(cycle.color(0), cycle.color(n));
            assert_eq!(cycle.color(3), cycle.color(3 + n * 2));
            // Adjacent rotation entries must differ.
            assert_ne!(cycle.color(0), cycle.color(1));
        }
    }

    fn full_series() -> SeriesConfig {
        SeriesConfig {
            series_id: "s".into(),
            label: None,
            x_column: "x".into(),
            y_column: "y".into(),
            render_type: DataRenderType::LineScatterErrorbarXY {
                scatter: DataScatterStyleConfig {
                    point_color: Color::BLACK,
                    point_shape: ScatterShape::Circle,
                    point_size: 4.0,
                },
                line: DataLineStyleConfig {
                    line_style: LineStylePreset::Solid,
                    line_color: Color::BLACK,
                    line_width: 1.0,
                },
                err_x: ErrorRef::Symmetric { column: "ex".into() },
                err_y: ErrorRef::Symmetric { column: "ey".into() },
                err_style: DataErrorBarStyleConfig {
                    error_bar_color: Color::BLACK,
                    error_bar_width: 1.0,
                    error_bar_cap_size: 3.0,
                    cap_width: 1.0,
                },
            },
        }
    }

    #[test]
    fn apply_to_series_recolors_every_sub_style() {
        let mut s = full_series();
        ColorCycle::Balanced.apply_to_series(&mut s, 2);
        let expected = ColorCycle::Balanced.color(2);
        let DataRenderType::LineScatterErrorbarXY { scatter, line, err_style, .. } =
            &s.render_type
        else {
            unreachable!()
        };
        assert_eq!(scatter.point_color, expected);
        assert_eq!(line.line_color, expected);
        assert_eq!(err_style.error_bar_color, expected);
    }

    #[test]
    fn apply_to_all_assigns_sequential_rotation() {
        let mut list = vec![full_series(), full_series(), full_series()];
        ColorCycle::Classic.apply_to_all(&mut list);
        for (i, s) in list.iter().enumerate() {
            let DataRenderType::LineScatterErrorbarXY { line, .. } = &s.render_type else {
                unreachable!()
            };
            assert_eq!(line.line_color, ColorCycle::Classic.color(i));
        }
    }
}
