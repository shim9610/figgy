//! Matrix fields: heatmaps, contours, hexagonal grids and RGB images ([`FieldSeries`]).

use super::*;

/// A field over a matrix: the composed form of what were the heatmap,
/// contour and heatmap-contour render types (#160).
#[derive(Debug, Clone, PartialEq)]
pub struct FieldSeries {
    /// The columns that make up z; `x_column` / `y_column` are the grid
    /// coordinates.
    pub matrix: MatrixRef,
    /// The fill, or none.
    pub fill: Option<FieldFillConfig>,
    /// The contour lines, or none. Drawn over the fill, or on the box floor
    /// of a 3D chart.
    pub contour: Option<ContourConfig>,
    /// The look of the filled field drawn as a surface on a 3D chart
    /// (`Config.view_3d`); kept and unused on a flat chart. `None` draws the
    /// surface with [`SurfaceStyle::default`].
    pub surface: Option<SurfaceStyle>,
}

impl FieldSeries {
    /// The combinations a field can draw: a fill or contours (or both), and
    /// a surface look only with a fill.
    pub fn validate(&self) -> Result<(), crate::RuleKind> {
        if self.fill.is_none() && self.contour.is_none() {
            return Err(crate::RuleKind::FieldNothingDrawn);
        }
        if self.fill.is_none() && self.surface.is_some() {
            return Err(crate::RuleKind::FieldSurfaceWithoutFill);
        }
        Ok(())
    }
}

/// A 2-D grid declared as a bundle of column ids — and nothing more.
///
/// There is no separate matrix container, no renderer-side registry, and no new
/// data trait. The grid *is* those columns in the pool; this says which ones and
/// how to read them. Keeping the declaration inline in the render type is what
/// lets `Config` + `series` fully define the picture: an id pointing at
/// something the renderer holds privately would not.
///
/// z statistics come from the constituent columns' own cached `min` / `max` /
/// `min_positive`, combined. No GPU round trip, no second extent engine.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct MatrixRef {
    /// The grid's constituent columns, in grid order.
    pub columns: Vec<ColumnId>,
    /// Whether each constituent column is a slice along x or along y.
    pub orientation: MatrixOrientation,
    /// Whether the coordinate columns hold cell boundaries (n + 1 values) or
    /// cell centres (n). Never inferred from the lengths.
    pub grid_layout: GridLayout,
}

/// Which axis a constituent column runs along.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum MatrixOrientation {
    /// Each column is one x position, holding that position's y values.
    ColumnsAreX,
    /// Each column is one y position, holding that position's x values.
    ColumnsAreY,
}

/// Whether coordinate columns give cell edges, cell centres or the centres
/// of a hexagonal lattice.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum GridLayout {
    /// `n + 1` coordinates bounding `n` cells.
    Edges,
    /// `n` coordinates at the middle of `n` cells.
    Centers,
    /// `n` columns by `m` rows of flat-topped hexagons (a hexbin, #134): the
    /// cell of column `c`, row `r` is centred at `(x[c], y[r] + (c mod 2) ·
    /// (y[1] − y[0]) / 2)`, so the odd columns are raised by half the row
    /// pitch. Both coordinate columns are uniform (every step equal to the
    /// first within 1e-6 relative, at least two coordinates each); the cells
    /// are regular when the column pitch is `√3 / 2` of the row pitch and
    /// stretched otherwise. The host bins its samples; the renderer draws
    /// the grid it is given, shaded flat, as a `Heatmap` on linear axes of a
    /// flat chart: a contour, an interpolated fill, a grid vector and a 3D
    /// view read a rectangular lattice and are rejected
    /// ([`Config::validate_hexagonal_series`](crate::config::Config::validate_hexagonal_series)).
    Hexagonal,
}

impl MatrixRef {
    /// Cells along each axis, given the coordinate column lengths.
    ///
    /// Returns `(cols, rows)` in constituent-column order: `cols` is how many
    /// constituent columns are usable, `rows` how deep into each of them to
    /// read. A declaration that does not line up with the data is **not an
    /// error** — the smallest common extent is drawn and the caller reports the
    /// truncation. Refusing to draw would turn a data-shape surprise into a
    /// blank chart.
    pub fn effective_extent(
        &self,
        x_len: usize,
        y_len: usize,
        shortest_column_len: usize,
    ) -> (usize, usize) {
        let (along, across) = self.coordinate_cells(x_len, y_len);
        (
            self.columns.len().min(along),
            shortest_column_len.min(across),
        )
    }

    /// Cells the **coordinate columns alone** can bound, in constituent-column
    /// order: `(along, across)`.
    ///
    /// The half of [`Self::effective_extent`] that does not depend on the data,
    /// split out because a caller reporting truncation has to know what the
    /// coordinates offered before the grid columns capped it.
    pub fn coordinate_cells(&self, x_len: usize, y_len: usize) -> (usize, usize) {
        let (along, across) = match self.orientation {
            MatrixOrientation::ColumnsAreX => (x_len, y_len),
            MatrixOrientation::ColumnsAreY => (y_len, x_len),
        };
        let cells = |coordinates: usize| match self.grid_layout {
            GridLayout::Edges => coordinates.saturating_sub(1),
            GridLayout::Centers | GridLayout::Hexagonal => coordinates,
        };
        (cells(along), cells(across))
    }
}

/// How a field's cells are painted.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct FieldFillConfig {
    pub mode: FillMode,
    pub shading: Shading,
    /// Multiplied into the colormap's alpha — the way to put contour lines over
    /// a muted field without editing the ramp.
    pub opacity: f32,
}

/// Continuous ramp, or quantized into the contour levels' bands.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum FillMode {
    Continuous,
    /// One flat colour per interval between contour levels. With no levels this
    /// is a single band, i.e. a flat field.
    Bands,
}

#[cfg(feature = "serde")]
pub(super) fn is_default_image_range(range: &[f64; 2]) -> bool {
    *range == crate::defaults::image_range()
}

#[cfg(feature = "serde")]
pub(super) fn is_default_image_shading(shading: &Shading) -> bool {
    *shading == crate::defaults::image_shading()
}

#[cfg(feature = "serde")]
pub(super) fn is_default_image_opacity(opacity: &f32) -> bool {
    *opacity == crate::defaults::image_opacity()
}

/// Whether a cell is one colour or interpolated across its corners.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Shading {
    /// One colour per cell, from that cell's own z.
    Flat,
    /// Bilinear across the cell's corner values.
    Interpolated,
}

/// Maximum number of contour levels accepted by one series.
///
/// Explicit level lists and levels produced by [`ContourConfig::nice_levels`]
/// share this limit. Renderers reject longer explicit lists rather than
/// truncating them.
pub const MAX_CONTOUR_LEVELS: usize = 1024;

/// Maximum number of explicit label anchors one series accepts
/// ([`ContourLabelConfig::anchors`]). A longer list is rejected by
/// [`ContourLabelConfig::validate`] rather than truncated. Automatic
/// placement keeps at most this many labels too: both share the renderer's
/// label buffer.
pub const MAX_CONTOUR_LABEL_ANCHORS: usize = 1024;

// `ContourLabelConfig::TOO_MANY_ANCHORS_REASON` names the number.
const _: () = assert!(MAX_CONTOUR_LABEL_ANCHORS == 1024);

pub(super) fn push_contour_level(levels: &mut Vec<f64>, value: f64) -> Result<(), TickError> {
    if levels.len() == MAX_CONTOUR_LEVELS {
        return Err(TickError::InvalidRange);
    }
    levels.push(value);
    Ok(())
}

/// Contour lines over a field.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ContourConfig {
    /// The levels to draw, in data units, with at most
    /// [`MAX_CONTOUR_LEVELS`] entries. An explicit list only — there is no
    /// "auto" variant, because a level set inferred at draw time is a value the
    /// config does not contain. A helper computes levels and writes them here.
    pub levels: Vec<f64>,
    pub line: DataLineStyleConfig,
    /// Per-level colours, indexed by level. `None` means **every level is
    /// `line.line_color`** — colours are not derived from the colormap here. A
    /// helper that wants colormap colours computes them and writes them in.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub per_level_color: Option<Vec<Color>>,
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub labels: Option<ContourLabelConfig>,
}

impl ContourConfig {
    /// About `target_count` round levels inside a colourbar's z range.
    ///
    /// There is no `Auto` level variant, and this is why there does not need to
    /// be one: the levels a chart draws are always the ones in
    /// [`Self::levels`], and a caller who wants them chosen for it calls this and
    /// stores the answer. What is drawn is then still exactly what the config
    /// says.
    ///
    /// Levels are **strictly inside** the range. One sitting exactly on
    /// `axis.min` or `axis.max` coincides with the field's outer boundary or has
    /// no visible coverage at
    /// all, and it would also make the banded fill's `n + 1` bands come out
    /// wrong by one.
    ///
    /// The spacing is the axis machinery's own: `compute_nice_ticks` picks it,
    /// and the walk is the axes' [`crate::tick::LinearGrid`], anchored to
    /// absolute multiples of it, so the levels are round numbers even when the
    /// range ends are not. A logarithmic colourbar gets every step-th decade
    /// from [`crate::tick::LogDecades`], with no separate rule here.
    pub fn nice_levels(axis: &AxisOptions, target_count: usize) -> Result<Vec<f64>, TickError> {
        // `compute_nice_ticks` widens min/max to nice bounds; a level outside the
        // colourbar's range has no colour, so only its spacing is used.
        let plan = AxisOptions::compute_nice_ticks(
            axis.scale.clone(),
            axis.min,
            axis.max,
            target_count.max(1),
        )?;
        let mut out = Vec::new();
        // Half a step of slack keeps a level that lands on the boundary through
        // floating-point noise from being kept as an interior one.
        let inside = |v: f64| v > axis.min && v < axis.max;
        match axis.scale {
            AxisScale::Linear => {
                let step = plan.major_spacing;
                if !(step.is_finite() && step > 0.0) {
                    return Err(TickError::InvalidRange);
                }
                // No interval cap: `MAX_CONTOUR_LEVELS` bounds the walk.
                let Some(grid) = crate::tick::LinearGrid::new(axis.min, axis.max, step, usize::MAX)
                else {
                    // No multiple of the step lies inside, or the step is too
                    // small to move the range ends at `f64` precision; then
                    // the range holds more levels than the limit when it
                    // holds more steps.
                    let steps = axis.max / step - axis.min / step;
                    return if steps > MAX_CONTOUR_LEVELS as f64 {
                        Err(TickError::InvalidRange)
                    } else {
                        Ok(out)
                    };
                };
                for (_, value) in grid.iter() {
                    if inside(value) {
                        push_contour_level(&mut out, value)?;
                    }
                }
            }
            AxisScale::Logarithmic => {
                let step = plan.major_spacing.max(1.0);
                let start = axis.min.log10().ceil();
                let end = axis.max.log10().floor();
                if !(step.is_finite()
                    && step <= i32::MAX as f64
                    && start.is_finite()
                    && start >= i32::MIN as f64
                    && start <= i32::MAX as f64
                    && end.is_finite()
                    && end >= i32::MIN as f64
                    && end <= i32::MAX as f64)
                {
                    return Err(TickError::InvalidRange);
                }
                let decades = crate::tick::LogDecades::new(start as i32, end as i32, step as i32)
                    .ok_or(TickError::InvalidRange)?;
                for (_, value) in decades {
                    if inside(value) {
                        push_contour_level(&mut out, value)?;
                    }
                }
            }
            // The probability axis's own majors (#209); a colourbar refuses
            // the scale, so this serves another caller's axis.
            AxisScale::Probability(_) => {
                for value in crate::tick::major_ticks(axis) {
                    if inside(value) {
                        push_contour_level(&mut out, value)?;
                    }
                }
            }
        }
        Ok(out)
    }

    /// Replace [`Self::levels`] with [`Self::nice_levels`].
    ///
    /// [`Self::per_level_color`] is left alone: a colour list that no longer
    /// lines up with the new levels falls back to [`Self::line`]'s colour per
    /// level rather than being silently thrown away. Call
    /// [`Self::set_colormap_colors`] after this to re-derive it.
    pub fn set_nice_levels(
        &mut self,
        axis: &AxisOptions,
        target_count: usize,
    ) -> Result<(), TickError> {
        self.levels = Self::nice_levels(axis, target_count)?;
        Ok(())
    }

    /// Colour every level by where it sits on the colourbar's ramp.
    ///
    /// The helper the `per_level_color` doc means by "a helper that wants
    /// colormap colours computes them and writes them in" — the field's fill and
    /// the lines over it then agree about what a value looks like, because both
    /// go through `ColorBarOptions::color_for_z`. A level the ramp cannot place
    /// takes `nan_color`, the same as a cell with that value.
    pub fn set_colormap_colors(&mut self, bar: &crate::config::ColorBarOptions) {
        self.per_level_color = Some(
            self.levels
                .iter()
                .map(|level| bar.color_for_z(*level))
                .collect(),
        );
    }
}

/// Inline level labels on contour lines.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ContourLabelConfig {
    pub visible: bool,
    pub font_size: f32,
    /// Text colour, independent of the contour stroke and any per-level ramp
    /// colours. Labels are annotations; changing a line palette must not make
    /// their typography unreadable.
    #[cfg_attr(
        feature = "serde",
        serde(default = "crate::defaults::contour_label_color")
    )]
    pub color: Color,
    pub format: LabelFormat,
    pub significant_digits: u8,
    /// Target minimum screen distance between labels selected by the normal
    /// automatic sweep. Must be finite and greater than zero even when labels
    /// are hidden or explicit anchors override automatic placement.
    ///
    /// Read by the anchor pass, which seeds a lattice at this pitch over the data
    /// area, projects each seed onto its level's isoline, and keeps the ones that
    /// stay this far apart. A per-level fallback may keep a closer candidate
    /// rather than omit that level. Labels are not arc-length-even: the seed
    /// lattice is even, the curve is not. Ignored when the resolved override is
    /// non-empty.
    #[cfg_attr(
        feature = "serde",
        serde(deserialize_with = "deserialize_contour_label_spacing_px")
    )]
    pub spacing_px: f32,
    /// Where the labels go — an **override**.
    ///
    /// Empty is the normal case: the GPU projects a seed lattice onto each
    /// level's isoline and selects a separated subset. An explicit list holds at
    /// most [`MAX_CONTOUR_LABEL_ANCHORS`] anchors; a longer one fails
    /// [`ContourLabelConfig::validate`]. The renderer removes anchors whose
    /// `level_index` is outside `levels` and uses the rest, in input order, as
    /// the override. If that leaves it empty, automatic placement remains
    /// active. Automatic and explicit placement share the same 1024-label draw
    /// capacity. Either way the same anchor record reaches the same draw.
    pub anchors: Vec<ContourLabelAnchor>,
    /// Fill painted behind the text, so a label over a line stays readable.
    ///
    /// `None` draws the text alone. This is a property of the *label*, not of the
    /// chart — which is why it can live here at all: the renderer composites the
    /// chrome over the data and does not know what colour is behind it, but the
    /// label knows what it wants behind *itself*.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub bg_color: Option<Color>,
    /// Space around the text. This pads `bg_color` when one is present and is
    /// also the contour-line gap around a transparent label.
    #[cfg_attr(feature = "serde", serde(default))]
    pub bg_padding_px: f32,
}

impl ContourLabelConfig {
    pub const INVALID_SPACING_REASON: crate::RuleKind = crate::RuleKind::ContourLabelSpacing;
    pub const TOO_MANY_ANCHORS_REASON: crate::RuleKind = crate::RuleKind::ContourLabelAnchors;

    /// Validate invariants owned by the label configuration itself.
    pub fn validate(&self) -> Result<(), crate::RuleKind> {
        if !self.spacing_px.is_finite() || self.spacing_px <= 0.0 {
            return Err(Self::INVALID_SPACING_REASON);
        }
        if self.anchors.len() > MAX_CONTOUR_LABEL_ANCHORS {
            return Err(Self::TOO_MANY_ANCHORS_REASON);
        }
        Ok(())
    }
}

#[cfg(feature = "serde")]
pub(super) fn deserialize_contour_label_spacing_px<'de, D>(deserializer: D) -> Result<f32, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let spacing_px = <f32 as serde::Deserialize>::deserialize(deserializer)?;
    if !spacing_px.is_finite() || spacing_px <= 0.0 {
        return Err(serde::de::Error::custom(
            ContourLabelConfig::INVALID_SPACING_REASON,
        ));
    }
    Ok(spacing_px)
}

/// One label placement, in data space.
///
/// Data coordinates and a data-space tangent, not screen values: the draw
/// re-projects them, so the same record works whether the GPU projected it from
/// the implicit field or a host wrote it by hand. The screen angle is this tangent
/// projected through the current transform — exact on a linear axis, the local
/// direction on a logarithmic one.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ContourLabelAnchor {
    /// Index into [`ContourConfig::levels`].
    pub level_index: u32,
    pub x: f64,
    pub y: f64,
    /// Tangent direction in data space; the label's screen angle is this
    /// projected through the current transform.
    pub tx: f64,
    pub ty: f64,
}
