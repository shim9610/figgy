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
use crate::config::{AxisBreakStyle, AxisOptions, Config, PieceSizing, TickVisibility};
use crate::data_config::{
    BoxSeries, CategoryBarSeries, DataRenderType, HistogramSeries, ParallelSeries,
    RadarPolygonSeries, RangeBarSeries, SeriesConfig, StreamlineSeries, TernarySeries,
    VectorSeries,
};

const fn rgb8(r: u8, g: u8, b: u8) -> Color {
    Color::new(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0)
}

// ============================================================================
// Axis presets.
// ============================================================================

/// Whole-chart axis frame styles, applied to all four axes and every
/// overlay's own axes at once via [`Config::apply_axis_preset`].
///
/// Every preset leaves data ranges, margins, tick lengths, colors and title
/// text untouched. It switches which axis lines are drawn, the tick
/// direction and which sides show tick-value labels; every preset but
/// [`AxisPreset::Hidden`] keeps labels on the bottom/left axes only. An
/// overlay's `x_axis` and `y_axis` are data axes and take the bottom/left
/// style on the side they are drawn on (#283).
/// `Hidden`, which shows none, and [`AxisPreset::Grotrian`] also switch the
/// grid off, and `Grotrian` frames the energy axis alone and gives a
/// continuous `left_y` break a thin lowest piece (#205, #357).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
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
    /// No axis lines, ticks, tick-value labels or grid: the frame of a figure
    /// whose data is its own picture, such as a Feynman diagram (#234). The
    /// one preset that also switches the grid off; titles stay as set.
    Hidden,
    /// A Grotrian (energy-level) diagram
    /// ([#205](https://github.com/shim9610/figgy/issues/205),
    /// [#357](https://github.com/shim9610/figgy/issues/357)), framed as
    /// NIST ASD Grotrian diagrams and textbook term diagrams are: the
    /// energy axis on the left alone carries a line, outward ticks and
    /// labels; the term-column axis (`bottom_x`) keeps its labels, the term
    /// symbols under each column, without a line or ticks, since its
    /// position is no value; the top and right sides draw nothing, unless
    /// `right_y` prints another unit (`unit` set, such as cm⁻¹ beside eV),
    /// which then takes the energy axis' frame. No grid, and a thin
    /// ground-state strip: a `Continuous` break on `left_y` shares the
    /// height `1 : 9 : …` between its lowest piece and the rest
    /// (`PieceSizing::Weights`, one weight per piece). A `Panels` break or
    /// an unbroken axis keeps its sizing.
    Grotrian,
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
    /// Apply `preset` to all four axes and every overlay's own axes at once.
    /// See [`AxisPreset`] for what is (and is not) touched.
    pub fn apply_axis_preset(&mut self, preset: AxisPreset) {
        use TickVisibility::{Inside, None as NoTick, Outside};

        // (labeled = bottom/left, unlabeled = top/right) side styles.
        let (labeled, unlabeled) = match preset {
            AxisPreset::BoxedInward => (
                SideStyle {
                    line: true,
                    tick: Inside,
                    labels: true,
                },
                SideStyle {
                    line: true,
                    tick: Inside,
                    labels: false,
                },
            ),
            AxisPreset::BoxedOutward => (
                SideStyle {
                    line: true,
                    tick: Outside,
                    labels: true,
                },
                SideStyle {
                    line: true,
                    tick: NoTick,
                    labels: false,
                },
            ),
            AxisPreset::OpenOutward => (
                SideStyle {
                    line: true,
                    tick: Outside,
                    labels: true,
                },
                SideStyle {
                    line: false,
                    tick: NoTick,
                    labels: false,
                },
            ),
            AxisPreset::OpenInward => (
                SideStyle {
                    line: true,
                    tick: Inside,
                    labels: true,
                },
                SideStyle {
                    line: false,
                    tick: NoTick,
                    labels: false,
                },
            ),
            AxisPreset::Minimal => (
                SideStyle {
                    line: false,
                    tick: NoTick,
                    labels: true,
                },
                SideStyle {
                    line: false,
                    tick: NoTick,
                    labels: false,
                },
            ),
            AxisPreset::Hidden => {
                let bare = SideStyle {
                    line: false,
                    tick: NoTick,
                    labels: false,
                };
                self.grid.show_major_x = false;
                self.grid.show_major_y = false;
                self.grid.show_minor_x = false;
                self.grid.show_minor_y = false;
                (bare.clone(), bare)
            }
            AxisPreset::Grotrian => {
                self.grid.show_major_x = false;
                self.grid.show_major_y = false;
                self.grid.show_minor_x = false;
                self.grid.show_minor_y = false;
                if let Some(breaks) = self.left_y.breaks.as_mut()
                    && breaks.style == AxisBreakStyle::Continuous
                    && !breaks.ranges.is_empty()
                {
                    // One weight per piece (#205): the lowest piece thin,
                    // the rest sharing the height equally.
                    let mut weights = vec![9.0; breaks.ranges.len() + 1];
                    weights[0] = 1.0;
                    breaks.sizing = PieceSizing::Weights(weights);
                }
                // The energy axis alone is framed (#357): the column axis
                // names its columns without a value frame, and the right
                // side reads only when it prints a second unit.
                let energy = SideStyle {
                    line: true,
                    tick: Outside,
                    labels: true,
                };
                let bare = SideStyle {
                    line: false,
                    tick: NoTick,
                    labels: false,
                };
                for overlay in &mut self.overlays {
                    apply_side(&mut overlay.x_axis, energy.clone());
                    apply_side(&mut overlay.y_axis, energy.clone());
                }
                let second_unit = self.right_y.unit.is_some();
                apply_side(&mut self.left_y, energy.clone());
                apply_side(
                    &mut self.bottom_x,
                    SideStyle {
                        line: false,
                        tick: NoTick,
                        labels: true,
                    },
                );
                apply_side(&mut self.top_x, bare.clone());
                apply_side(&mut self.right_y, if second_unit { energy } else { bare });
                return;
            }
        };

        // An overlay's own axes are data axes, so they take the labeled
        // side's frame wherever they are drawn (#283).
        for overlay in &mut self.overlays {
            apply_side(&mut overlay.x_axis, labeled.clone());
            apply_side(&mut overlay.y_axis, labeled.clone());
        }
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
///
/// The categorical presets (#357) come from established public palettes;
/// each has eight or more colours, no two alike, and its source and licence
/// are noted on its variant and in `crates/renderer/USAGE.md` ("Palette
/// presets"). [`ColorCycle::DEFAULT`] is the cycle a series takes when the
/// host does not colour it. `Classic`, `Vivid` and `Monochrome` are figgy's
/// own rotations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorCycle {
    /// Strong primary rotation starting at black — the classic
    /// lab-instrument increment list.
    Classic,
    /// High-saturation, high-contrast rotation — the look of biostatistics
    /// plotting tools, optimized for telling few series apart at a glance.
    Vivid,
    /// Ten balanced mid-saturation hues — the modern general-purpose
    /// default ([`ColorCycle::DEFAULT`]). The values are D3's
    /// `schemeCategory10`, which matplotlib ships as `tab10` and Vega as
    /// `category10`. Source: d3-scale-chromatic,
    /// <https://github.com/d3/d3-scale-chromatic> (ISC licence).
    Balanced,
    /// The Okabe–Ito eight-color palette, designed to stay distinguishable
    /// under the common forms of color-vision deficiency. Source: M. Okabe
    /// and K. Ito, "Color Universal Design (CUD)", 2002/2008,
    /// <https://jfly.uni-koeln.de/color/>; published as a free
    /// recommendation, no licence terms.
    ColorblindSafe,
    /// Grayscale steps for print-safe single-ink figures. Not a categorical
    /// preset: four steps, for one ink.
    Monochrome,
    /// Tableau 10, the ten-colour palette Tableau introduced in 2016.
    /// Source: d3-scale-chromatic `schemeTableau10`,
    /// <https://github.com/d3/d3-scale-chromatic> (ISC licence).
    Tableau10,
    /// ColorBrewer Set2: eight soft qualitative colours. Source: Cynthia
    /// Brewer, Mark Harrower and The Pennsylvania State University,
    /// <https://colorbrewer2.org> (Apache-2.0 licence; this product
    /// includes color specifications and designs developed by Cynthia
    /// Brewer).
    Set2,
    /// ColorBrewer Dark2: eight dark qualitative colours that hold up on
    /// white. Source: Cynthia Brewer, Mark Harrower and The Pennsylvania
    /// State University, <https://colorbrewer2.org> (Apache-2.0 licence;
    /// this product includes color specifications and designs developed by
    /// Cynthia Brewer).
    Dark2,
    /// Plotly's default ten-colour qualitative sequence. Source: plotly.py
    /// `plotly.colors.qualitative.Plotly`,
    /// <https://github.com/plotly/plotly.py> (MIT licence).
    Plotly,
    /// A high-contrast set of 24 dark, saturated colours that stand off a
    /// light background, for charts with many series. Source: plotly.py
    /// `plotly.colors.qualitative.Dark24`,
    /// <https://github.com/plotly/plotly.py> (MIT licence).
    HighContrast,
}

const CLASSIC: &[Color] = &[
    rgb8(0, 0, 0),      // black
    rgb8(255, 0, 0),    // red
    rgb8(0, 128, 0),    // green
    rgb8(0, 0, 255),    // blue
    rgb8(255, 0, 255),  // magenta
    rgb8(0, 139, 139),  // dark cyan
    rgb8(184, 134, 11), // dark yellow
    rgb8(0, 0, 128),    // navy
    rgb8(128, 0, 128),  // purple
    rgb8(128, 0, 0),    // wine
    rgb8(128, 128, 0),  // olive
    rgb8(255, 102, 0),  // orange
];

const VIVID: &[Color] = &[
    rgb8(0, 32, 240),  // blue
    rgb8(230, 16, 16), // red
    rgb8(0, 153, 0),   // green
    rgb8(128, 0, 192), // purple
    rgb8(255, 128, 0), // orange
    rgb8(0, 0, 0),     // black
    rgb8(230, 0, 172), // pink
    rgb8(0, 153, 153), // teal
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

// Tableau 10 (d3-scale-chromatic `schemeTableau10`, ISC licence).
const TABLEAU10: &[Color] = &[
    rgb8(0x4e, 0x79, 0xa7),
    rgb8(0xf2, 0x8e, 0x2c),
    rgb8(0xe1, 0x57, 0x59),
    rgb8(0x76, 0xb7, 0xb2),
    rgb8(0x59, 0xa1, 0x4f),
    rgb8(0xed, 0xc9, 0x49),
    rgb8(0xaf, 0x7a, 0xa1),
    rgb8(0xff, 0x9d, 0xa7),
    rgb8(0x9c, 0x75, 0x5f),
    rgb8(0xba, 0xb0, 0xab),
];

// ColorBrewer Set2. Apache-2.0: this product includes color specifications
// and designs developed by Cynthia Brewer (http://colorbrewer.org/).
const SET2: &[Color] = &[
    rgb8(0x66, 0xc2, 0xa5),
    rgb8(0xfc, 0x8d, 0x62),
    rgb8(0x8d, 0xa0, 0xcb),
    rgb8(0xe7, 0x8a, 0xc3),
    rgb8(0xa6, 0xd8, 0x54),
    rgb8(0xff, 0xd9, 0x2f),
    rgb8(0xe5, 0xc4, 0x94),
    rgb8(0xb3, 0xb3, 0xb3),
];

// ColorBrewer Dark2. Apache-2.0: this product includes color specifications
// and designs developed by Cynthia Brewer (http://colorbrewer.org/).
const DARK2: &[Color] = &[
    rgb8(0x1b, 0x9e, 0x77),
    rgb8(0xd9, 0x5f, 0x02),
    rgb8(0x75, 0x70, 0xb3),
    rgb8(0xe7, 0x29, 0x8a),
    rgb8(0x66, 0xa6, 0x1e),
    rgb8(0xe6, 0xab, 0x02),
    rgb8(0xa6, 0x76, 0x1d),
    rgb8(0x66, 0x66, 0x66),
];

// Plotly (plotly.py `qualitative.Plotly`, MIT licence).
const PLOTLY: &[Color] = &[
    rgb8(0x63, 0x6e, 0xfa),
    rgb8(0xef, 0x55, 0x3b),
    rgb8(0x00, 0xcc, 0x96),
    rgb8(0xab, 0x63, 0xfa),
    rgb8(0xff, 0xa1, 0x5a),
    rgb8(0x19, 0xd3, 0xf3),
    rgb8(0xff, 0x66, 0x92),
    rgb8(0xb6, 0xe8, 0x80),
    rgb8(0xff, 0x97, 0xff),
    rgb8(0xfe, 0xcb, 0x52),
];

// High contrast (plotly.py `qualitative.Dark24`, MIT licence).
const HIGH_CONTRAST: &[Color] = &[
    rgb8(0x2e, 0x91, 0xe5),
    rgb8(0xe1, 0x5f, 0x99),
    rgb8(0x1c, 0xa7, 0x1c),
    rgb8(0xfb, 0x0d, 0x0d),
    rgb8(0xda, 0x16, 0xff),
    rgb8(0x22, 0x2a, 0x2a),
    rgb8(0xb6, 0x81, 0x00),
    rgb8(0x75, 0x0d, 0x86),
    rgb8(0xeb, 0x66, 0x3b),
    rgb8(0x51, 0x1c, 0xfb),
    rgb8(0x00, 0xa0, 0x8b),
    rgb8(0xfb, 0x00, 0xd1),
    rgb8(0xfc, 0x00, 0x80),
    rgb8(0xb2, 0x82, 0x8d),
    rgb8(0x6c, 0x7c, 0x32),
    rgb8(0x77, 0x8a, 0xae),
    rgb8(0x86, 0x2a, 0x16),
    rgb8(0xa7, 0x77, 0xf1),
    rgb8(0x62, 0x00, 0x42),
    rgb8(0x16, 0x16, 0xa7),
    rgb8(0xda, 0x60, 0xca),
    rgb8(0x6c, 0x45, 0x16),
    rgb8(0x0d, 0x2a, 0x63),
    rgb8(0xaf, 0x00, 0x38),
];

impl ColorCycle {
    /// The default palette (#357): the series constructors start in its
    /// first colour, and the default flows apply it to the series list
    /// ([`Config::apply_palette`]), so series `i` takes its colour `i`.
    pub const DEFAULT: Self = Self::Balanced;

    /// Every cycle, in declaration order.
    pub const ALL: [Self; 10] = [
        Self::Classic,
        Self::Vivid,
        Self::Balanced,
        Self::ColorblindSafe,
        Self::Monochrome,
        Self::Tableau10,
        Self::Set2,
        Self::Dark2,
        Self::Plotly,
        Self::HighContrast,
    ];

    /// The categorical presets from public palettes (#357), each with eight
    /// or more distinct colours.
    pub const PRESETS: [Self; 7] = [
        Self::Balanced,
        Self::ColorblindSafe,
        Self::Tableau10,
        Self::Set2,
        Self::Dark2,
        Self::Plotly,
        Self::HighContrast,
    ];

    /// The cycle's name, in kebab case (`"balanced"`, `"set2"`, ...).
    pub const fn name(self) -> &'static str {
        match self {
            Self::Classic => "classic",
            Self::Vivid => "vivid",
            Self::Balanced => "balanced",
            Self::ColorblindSafe => "colorblind-safe",
            Self::Monochrome => "monochrome",
            Self::Tableau10 => "tableau10",
            Self::Set2 => "set2",
            Self::Dark2 => "dark2",
            Self::Plotly => "plotly",
            Self::HighContrast => "high-contrast",
        }
    }

    /// The cycle named `name` ([`Self::name`]), or `None`.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|cycle| cycle.name() == name)
    }

    /// The full palette in rotation order.
    pub const fn colors(self) -> &'static [Color] {
        match self {
            Self::Classic => CLASSIC,
            Self::Vivid => VIVID,
            Self::Balanced => BALANCED,
            Self::ColorblindSafe => COLORBLIND_SAFE,
            Self::Monochrome => MONOCHROME,
            Self::Tableau10 => TABLEAU10,
            Self::Set2 => SET2,
            Self::Dark2 => DARK2,
            Self::Plotly => PLOTLY,
            Self::HighContrast => HIGH_CONTRAST,
        }
    }

    /// Color for the `index`-th series — wraps past the palette end, so it is
    /// total over any series count.
    pub const fn color(self, index: usize) -> Color {
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
        recolor_series(series, self.color(index));
    }

    /// Recolor a whole series list in order: series `i` gets `color(i)`.
    pub fn apply_to_all(self, series: &mut [SeriesConfig]) {
        for (i, s) in series.iter_mut().enumerate() {
            self.apply_to_series(s, i);
        }
    }
}

impl Config {
    /// Apply `palette` to this chart and `series`, its series list (#357):
    /// series `i` takes `palette[i % palette.len()]` in every part its kind
    /// draws in a palette colour (the parts [`ColorCycle::apply_to_series`]
    /// writes), so a chart of many series draws each in its own colour. A
    /// kind whose default ink is not a palette colour keeps its colours: a
    /// field or mesh (its fill comes from the colour map and its contour
    /// lines are dark ink), an RGB image, an energy-level diagram, a
    /// Feynman diagram and dependency links (#357). Such a series still takes its index, so series `i`
    /// always takes colour `i`.
    ///
    /// The colours are written into the series, so the document says what
    /// is drawn: there is no unset colour and nothing is chosen at draw
    /// time. The legends derived from the series (the base legend, each
    /// overlay's and each subplot's) redraw their symbols from them
    /// ([`crate::legend::update_legend_symbols_preserving_text`]); their
    /// text, corner and offsets stay. Nothing else changes: the layout is
    /// neither fitted nor marked. An empty palette changes nothing. The
    /// default flows call it ([`crate::default::config_for_series`]); call
    /// it again with another palette, such as [`ColorCycle::colors`] of a
    /// preset, to recolour.
    pub fn apply_palette(&mut self, series: &mut [SeriesConfig], palette: &[Color]) {
        if palette.is_empty() {
            return;
        }
        for (i, s) in series.iter_mut().enumerate() {
            if takes_palette(s) {
                recolor_series(s, palette[i % palette.len()]);
            }
        }
        refresh_legend_symbols(self, series);
    }
}

/// Whether [`Config::apply_palette`] colours `series`: every kind but the
/// ones whose default ink is not a palette colour.
fn takes_palette(series: &SeriesConfig) -> bool {
    !matches!(
        series.render_type,
        DataRenderType::Field(_)
            | DataRenderType::TriMesh(_)
            | DataRenderType::RgbImage(_)
            | DataRenderType::EnergyLevels(_)
            | DataRenderType::Feynman(_)
            // Dependency links keep their own accent (#357).
            | DataRenderType::RangeLinks(_)
    )
}

/// Redraw the symbols of every legend of `config` derived from `series`:
/// the base legend, each overlay's and, recursively, each subplot's.
fn refresh_legend_symbols(config: &mut Config, series: &[SeriesConfig]) {
    let colormap = config.colorbar.as_ref().map(|bar| bar.colormap.clone());
    crate::legend::update_legend_symbols_preserving_text(
        &mut config.legend.content,
        series,
        colormap.as_ref(),
    );
    for overlay in &mut config.overlays {
        if let Some(legend) = &mut overlay.legend {
            crate::legend::update_legend_symbols_preserving_text(
                &mut legend.content,
                series,
                colormap.as_ref(),
            );
        }
    }
    for subplot in &mut config.subplots {
        refresh_legend_symbols(&mut subplot.config, series);
    }
}

/// Give `series` the colour `c` where a palette colours it: the line,
/// marker, error bar and stem colour of an x-y series, a histogram's fill,
/// the series colour of a category, box, range or radar series, a contour
/// line, the arrows and streamlines, a parallel series' rows, a monotonic
/// channel, an energy diagram's level strokes and a Feynman diagram's
/// lines. A field fill and an RGB image keep their colours, which come
/// from the colour map or the image; per-item overrides stay. Shared by
/// [`ColorCycle::apply_to_series`] and the themes.
pub(crate) fn recolor_series(series: &mut SeriesConfig, c: Color) {
    match &mut series.render_type {
        // Markers, line, error bars and stems all take the cycle colour.
        DataRenderType::Xy(xy) => {
            if let Some(scatter) = &mut xy.scatter {
                scatter.point_color = c;
            }
            if let Some(line) = &mut xy.line {
                line.line_color = c;
            }
            if let Some(errors) = &mut xy.errors {
                errors.style.error_bar_color = c;
            }
            if let Some(stem) = &mut xy.stem {
                stem.color = c;
            }
        }
        DataRenderType::Histogram(HistogramSeries { bar }) => {
            bar.fill_color = c;
        }
        // The level strokes take the cycle colour; the arrows keep theirs
        // (#203).
        DataRenderType::EnergyLevels(levels) => {
            levels.style.level.color = c;
        }
        // The lines take the cycle colour; per-line overrides stay
        // (#234).
        DataRenderType::Feynman(diagram) => {
            diagram.style.color = c;
        }
        // The contour line joins the cycle like any other line. The fill
        // does not: its colours come from the colormap, and overwriting one
        // of them with a cycle entry would make the colourbar lie.
        DataRenderType::Field(crate::data_config::FieldSeries {
            contour: Some(contour),
            ..
        }) => {
            contour.line.line_color = c;
        }
        // A mesh's contour line joins the cycle as a field's does (#210).
        DataRenderType::TriMesh(crate::tri_mesh::TriMeshSeries {
            contour: Some(contour),
            ..
        }) => {
            contour.line.line_color = c;
        }
        DataRenderType::Field(crate::data_config::FieldSeries { contour: None, .. })
        | DataRenderType::TriMesh(crate::tri_mesh::TriMeshSeries { contour: None, .. })
        | DataRenderType::RgbImage(_) => {}
        // The arrow colour joins the cycle; a colour map, when set, still
        // draws from the colourbar.
        DataRenderType::Vector(VectorSeries { style, .. }) => {
            style.color = c;
        }
        // The bar fill joins the cycle; per-bar override colours stay.
        DataRenderType::CategoryBars(CategoryBarSeries { color, .. }) => {
            *color = c;
        }
        // The box fill joins the cycle; per-box override colours stay.
        DataRenderType::BoxPlot(BoxSeries { color, .. }) => {
            *color = c;
        }
        // The series fill joins the cycle; direction and state colours
        // stay, since they carry the bars' meaning.
        DataRenderType::RangeBars(RangeBarSeries { color, .. }) => {
            *color = c;
        }
        // Dependency links keep their own accent (#357): a cycle colour
        // would read as one more task series.
        DataRenderType::RangeLinks(_) => {}
        // The outline and markers join the cycle, and the fill follows
        // them unless the style names its own.
        DataRenderType::Radar(RadarPolygonSeries { color, .. }) => {
            *color = c;
        }
        DataRenderType::Ternary(TernarySeries { scatter, line, .. }) => {
            if let Some(scatter) = scatter {
                scatter.point_color = c;
            }
            if let Some(line) = line {
                line.line_color = c;
            }
        }
        // The line and its arrowheads join the cycle; a colour map, when
        // set, still draws from the colourbar.
        DataRenderType::Streamlines(StreamlineSeries { line, .. }) => {
            line.line_color = c;
        }
        // The rows join the cycle; a colour column, when set, still
        // draws from the colourbar.
        DataRenderType::Parallel(ParallelSeries { line, .. }) => {
            line.line_color = c;
        }
        // Each channel of a monotonic group joins the cycle (#271).
        DataRenderType::MonotonicLine(series) => {
            series.line.line_color = c;
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
    use crate::line::LineStylePreset;

    #[test]
    fn boxed_inward_shows_full_frame_with_inside_ticks() {
        let mut cfg = crate::defaults::config();
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
        let mut cfg = crate::defaults::config();
        cfg.apply_axis_preset(AxisPreset::OpenOutward);
        assert!(cfg.bottom_x.line_visible);
        assert_eq!(cfg.bottom_x.tick, TickVisibility::Outside);
        assert!(!cfg.top_x.line_visible);
        assert_eq!(cfg.top_x.tick, TickVisibility::None);
        assert!(!cfg.right_y.line_visible);
    }

    #[test]
    fn minimal_drops_lines_and_ticks_but_keeps_labels() {
        let mut cfg = crate::defaults::config();
        cfg.apply_axis_preset(AxisPreset::Minimal);
        for axis in [&cfg.top_x, &cfg.bottom_x, &cfg.left_y, &cfg.right_y] {
            assert!(!axis.line_visible);
            assert_eq!(axis.tick, TickVisibility::None);
        }
        assert!(cfg.bottom_x.label_style.label_visible);
    }

    // #205, #357: the Grotrian preset frames the energy axis alone (line,
    // outward ticks, labels), keeps the term columns' labels without a
    // line or ticks, draws nothing on the top and right, drops the grid,
    // and gives a continuous `left_y` break a thin lowest piece; a `Panels`
    // break or an unbroken axis keeps its sizing.
    #[test]
    fn grotrian_frames_the_energy_axis_alone_and_thins_the_ground_strip() {
        use crate::config::{AxisBreak, AxisBreaks};
        let mut cfg = crate::defaults::config();
        cfg.left_y.min = 0.0;
        cfg.left_y.max = 25.0;
        cfg.left_y.breaks = Some(AxisBreaks {
            ranges: vec![AxisBreak::new(0.5, 10.0), AxisBreak::new(12.0, 20.0)],
            ..crate::defaults::axis_breaks()
        });
        cfg.apply_axis_preset(AxisPreset::Grotrian);
        assert!(cfg.left_y.line_visible);
        for axis in [&cfg.top_x, &cfg.bottom_x, &cfg.right_y] {
            assert!(!axis.line_visible);
            assert_eq!(axis.tick, TickVisibility::None);
        }
        assert_eq!(cfg.left_y.tick, TickVisibility::Outside);
        assert!(cfg.bottom_x.label_style.label_visible && cfg.left_y.label_style.label_visible);
        assert!(!cfg.top_x.label_style.label_visible && !cfg.right_y.label_style.label_visible);
        assert!(!cfg.grid.show_major_x && !cfg.grid.show_major_y);
        assert!(!cfg.grid.show_minor_x && !cfg.grid.show_minor_y);
        assert_eq!(
            cfg.left_y.breaks.as_ref().map(|b| b.sizing.clone()),
            Some(PieceSizing::Weights(vec![1.0, 9.0, 9.0]))
        );
        let mut panels = crate::defaults::config();
        panels.left_y.breaks = Some(AxisBreaks {
            ranges: vec![AxisBreak::new(0.5, 10.0)],
            style: AxisBreakStyle::Panels,
            ..crate::defaults::axis_breaks()
        });
        panels.apply_axis_preset(AxisPreset::Grotrian);
        assert_eq!(
            panels.left_y.breaks.as_ref().map(|b| b.sizing.clone()),
            Some(PieceSizing::ByValue)
        );
        let mut plain = crate::defaults::config();
        plain.apply_axis_preset(AxisPreset::Grotrian);
        assert!(plain.left_y.breaks.is_none());
    }

    // #357: a right axis that prints a second unit (cm⁻¹ beside eV) takes
    // the energy axis' frame; without a unit the right side draws nothing.
    #[test]
    fn grotrian_frames_a_right_axis_in_a_second_unit() {
        use crate::config::AxisUnit;
        let mut cfg = crate::defaults::config();
        cfg.right_y.unit = Some(AxisUnit::new(8065.544, 0.0));
        cfg.apply_axis_preset(AxisPreset::Grotrian);
        assert!(cfg.right_y.line_visible && cfg.right_y.label_style.label_visible);
        assert_eq!(cfg.right_y.tick, TickVisibility::Outside);
        assert!(!cfg.top_x.line_visible && !cfg.top_x.label_style.label_visible);
    }

    #[test]
    fn hidden_drops_lines_ticks_labels_and_the_grid_and_keeps_the_layout() {
        let mut cfg = crate::defaults::config();
        cfg.grid.show_major_x = true;
        cfg.grid.show_minor_y = true;
        let before = cfg.clone();
        cfg.apply_axis_preset(AxisPreset::Hidden);
        for axis in [&cfg.top_x, &cfg.bottom_x, &cfg.left_y, &cfg.right_y] {
            assert!(!axis.line_visible);
            assert_eq!(axis.tick, TickVisibility::None);
            assert!(!axis.label_style.label_visible);
        }
        assert!(!cfg.grid.show_major_x && !cfg.grid.show_major_y);
        assert!(!cfg.grid.show_minor_x && !cfg.grid.show_minor_y);
        assert_eq!(cfg.chart_title, before.chart_title);
        assert_eq!(cfg.bottom_x.title_option, before.bottom_x.title_option);
        assert_eq!(cfg.data_area().unwrap(), before.data_area().unwrap());
    }

    // An overlay's own axes are data axes: they take the labeled side's
    // frame, so a minimal or hidden chart has no stray overlay frame (#283).
    #[test]
    fn an_overlay_s_own_axes_take_the_labeled_frame() {
        let mut cfg = crate::defaults::config();
        cfg.overlays.push(crate::defaults::overlay_layer());
        cfg.overlays[0].link_x = false;
        cfg.apply_axis_preset(AxisPreset::Minimal);
        for axis in [&cfg.overlays[0].x_axis, &cfg.overlays[0].y_axis] {
            assert!(!axis.line_visible);
            assert_eq!(axis.tick, TickVisibility::None);
            assert!(axis.label_style.label_visible);
        }
        cfg.apply_axis_preset(AxisPreset::BoxedOutward);
        for axis in [&cfg.overlays[0].x_axis, &cfg.overlays[0].y_axis] {
            assert!(axis.line_visible);
            assert_eq!(axis.tick, TickVisibility::Outside);
            assert!(axis.label_style.label_visible);
        }
        cfg.apply_axis_preset(AxisPreset::Hidden);
        for axis in [&cfg.overlays[0].x_axis, &cfg.overlays[0].y_axis] {
            assert!(!axis.line_visible && !axis.label_style.label_visible);
        }
    }

    // A preset must not disturb anything outside frame/tick/label visibility.
    #[test]
    fn preset_leaves_ranges_margins_titles_untouched() {
        let mut cfg = crate::defaults::config();
        let before = cfg.clone();
        cfg.apply_axis_preset(AxisPreset::OpenOutward);

        assert_eq!(cfg.bottom_x.min, before.bottom_x.min);
        assert_eq!(cfg.bottom_x.max, before.bottom_x.max);
        assert_eq!(cfg.bottom_x.out_margin, before.bottom_x.out_margin);
        assert_eq!(
            cfg.left_y.major_tick_length,
            before.left_y.major_tick_length
        );
        assert_eq!(cfg.chart_title, before.chart_title);
        assert_eq!(cfg.grid, before.grid);
        assert_eq!(cfg.data_area().unwrap(), before.data_area().unwrap());
    }

    #[test]
    fn presets_have_eight_distinct_colours_and_names() {
        // #357: every categorical preset has eight or more colours, no two
        // alike, and a name that reads back.
        for cycle in ColorCycle::PRESETS {
            let colors = cycle.colors();
            assert!(colors.len() >= 8, "{cycle:?} has {}", colors.len());
            for (i, a) in colors.iter().enumerate() {
                assert!(
                    colors[i + 1..].iter().all(|b| a != b),
                    "{cycle:?} repeats a colour"
                );
                assert_eq!(a.a, 1.0);
            }
        }
        for cycle in ColorCycle::ALL {
            assert_eq!(ColorCycle::from_name(cycle.name()), Some(cycle));
        }
        assert_eq!(ColorCycle::from_name("nope"), None);
        assert!(ColorCycle::PRESETS.contains(&ColorCycle::DEFAULT));
    }

    #[test]
    fn preset_values_match_their_sources() {
        // Spot checks against the published values: d3-scale-chromatic
        // (category10, Tableau10, Set2, Dark2) and plotly.py (Plotly,
        // Dark24).
        assert_eq!(ColorCycle::Balanced.color(0), rgb8(0x1f, 0x77, 0xb4));
        assert_eq!(ColorCycle::Tableau10.color(5), rgb8(0xed, 0xc9, 0x49));
        assert_eq!(ColorCycle::Set2.color(7), rgb8(0xb3, 0xb3, 0xb3));
        assert_eq!(ColorCycle::Dark2.color(0), rgb8(0x1b, 0x9e, 0x77));
        assert_eq!(ColorCycle::Plotly.color(9), rgb8(0xfe, 0xcb, 0x52));
        assert_eq!(ColorCycle::HighContrast.len(), 24);
        assert_eq!(ColorCycle::HighContrast.color(23), rgb8(0xaf, 0x00, 0x38));
        assert_eq!(ColorCycle::ColorblindSafe.color(0), rgb8(0xe6, 0x9f, 0x00));
    }

    #[test]
    fn cycle_color_wraps_around() {
        for cycle in ColorCycle::ALL {
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
            source_id: None,
            label: None,
            x_column: "x".into(),
            y_column: "y".into(),
            z_column: None,
            render_type: DataRenderType::Xy(crate::data_config::XySeries {
                scatter: Some(DataScatterStyleConfig {
                    point_color: Color::BLACK,
                    point_shape: ScatterShape::Circle,
                    point_size: 4.0,
                    point_style_table: None,
                    point_style_index_column: None,
                    point_style_overrides: None,
                    point_color_map: None,
                    point_size_map: None,
                }),
                line: Some(DataLineStyleConfig {
                    line_style: LineStylePreset::Solid,
                    line_color: Color::BLACK,
                    line_width: 1.0,
                    fill: None,
                    fill_color: None,
                    interpolation: crate::data_config::LineInterpolation::Linear,
                }),
                errors: Some(crate::data_config::ErrorBars {
                    x: Some(ErrorRef::Symmetric {
                        column: "ex".into(),
                    }),
                    y: Some(ErrorRef::Symmetric {
                        column: "ey".into(),
                    }),
                    style: DataErrorBarStyleConfig {
                        error_bar_color: Color::BLACK,
                        error_bar_width: 1.0,
                        error_bar_cap_size: 3.0,
                        cap_width: 1.0,
                        error_bar_style_table: None,
                        error_bar_style_index_column: None,
                        error_bar_style_overrides: None,
                    },
                }),
                stem: None,
            }),
        }
    }

    #[test]
    fn apply_to_series_recolors_every_sub_style() {
        let mut s = full_series();
        ColorCycle::Balanced.apply_to_series(&mut s, 2);
        let expected = ColorCycle::Balanced.color(2);
        let DataRenderType::Xy(crate::data_config::XySeries {
            scatter: Some(scatter),
            line: Some(line),
            errors:
                Some(crate::data_config::ErrorBars {
                    x: Some(_),
                    y: Some(_),
                    style: err_style,
                }),
            ..
        }) = &s.render_type
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
            let DataRenderType::Xy(crate::data_config::XySeries {
                scatter: Some(_),
                line: Some(line),
                errors:
                    Some(crate::data_config::ErrorBars {
                        x: Some(_),
                        y: Some(_),
                        style: _,
                    }),
                ..
            }) = &s.render_type
            else {
                unreachable!()
            };
            assert_eq!(line.line_color, ColorCycle::Classic.color(i));
        }
    }

    // The contour line joins the cycle like any other line; the field's colours
    // come from the colormap and the cycle must not reach into them.
    #[test]
    fn the_cycle_recolors_bars_and_contour_lines_but_never_a_field() {
        use crate::data_config::{
            BarOrientation, ContourConfig, DataBarStyleConfig, FieldFillConfig, FillMode,
            GridLayout, MatrixOrientation, MatrixRef, Shading,
        };

        let matrix = || MatrixRef {
            columns: vec!["z0".into()],
            orientation: MatrixOrientation::ColumnsAreX,
            grid_layout: GridLayout::Centers,
        };
        let fill = || FieldFillConfig {
            mode: FillMode::Continuous,
            shading: Shading::Flat,
            opacity: 1.0,
        };
        let contour = || ContourConfig {
            levels: vec![1.0],
            line: DataLineStyleConfig {
                line_style: LineStylePreset::Solid,
                line_color: Color::BLACK,
                line_width: 1.0,
                fill: None,
                fill_color: None,
                interpolation: crate::data_config::LineInterpolation::Linear,
            },
            per_level_color: None,
            labels: None,
        };
        let series = |render_type| SeriesConfig {
            series_id: "s".into(),
            source_id: None,
            label: None,
            x_column: "x".into(),
            y_column: "y".into(),
            z_column: None,
            render_type,
        };
        let expected = ColorCycle::Classic.color(2);

        let mut histogram = series(DataRenderType::Histogram(HistogramSeries {
            bar: DataBarStyleConfig {
                fill_color: Color::BLACK,
                border_color: Color::BLACK,
                border_width: 1.0,
                baseline: 0.0,
                gap_px: 1.0,
                width_ratio: 1.0,
                orientation: BarOrientation::Vertical,
                bar_style_overrides: None,
            },
        }));
        ColorCycle::Classic.apply_to_series(&mut histogram, 2);
        let DataRenderType::Histogram(HistogramSeries { bar }) = &histogram.render_type else {
            unreachable!()
        };
        assert_eq!(bar.fill_color, expected);

        for render_type in [
            DataRenderType::Field(crate::data_config::FieldSeries {
                matrix: matrix(),
                fill: None,
                contour: Some(contour()),
                surface: None,
            }),
            DataRenderType::Field(crate::data_config::FieldSeries {
                matrix: matrix(),
                fill: Some(fill()),
                contour: Some(contour()),
                surface: None,
            }),
        ] {
            let mut item = series(render_type);
            ColorCycle::Classic.apply_to_series(&mut item, 2);
            let line_color = match &item.render_type {
                DataRenderType::Field(crate::data_config::FieldSeries {
                    contour: Some(contour),
                    ..
                }) => contour.line.line_color,
                _ => unreachable!(),
            };
            assert_eq!(line_color, expected);
        }

        // A fill is left exactly as declared — including inside the combined
        // variant, where only the line was recoloured above.
        let mut heatmap = series(DataRenderType::Field(crate::data_config::FieldSeries {
            matrix: matrix(),
            fill: Some(fill()),
            contour: None,
            surface: None,
        }));
        let before = heatmap.clone();
        ColorCycle::Classic.apply_to_series(&mut heatmap, 2);
        assert_eq!(heatmap, before);
    }
}
