//! Vector fields and streamlines.

use super::*;

/// The arrowheads of a [`DataRenderType::Streamlines`] series: the vector
/// glyph's head, every `spacing_px` along the drawn path. Logical pixels,
/// like `point_size`; an export at a larger scale scales them with it.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct StreamlineArrows {
    /// The arc length between two arrowheads along the drawn path. The
    /// first sits half a spacing from the start of the series.
    pub spacing_px: f32,
    /// Head length along the path; the head is centred on its position.
    pub head_length_px: f32,
    /// Full head width across the path.
    pub head_width_px: f32,
}

impl StreamlineArrows {
    /// The value this type takes where a document omits it: the one source
    /// of its default (#266). `Default`, for hosts, returns it.
    #[cfg_attr(not(feature = "helpers"), allow(dead_code))]
    pub(crate) fn omitted() -> Self {
        Self {
            spacing_px: 80.0,
            head_length_px: 8.0,
            head_width_px: 7.0,
        }
    }
}

#[cfg(feature = "helpers")]
impl Default for StreamlineArrows {
    fn default() -> Self {
        Self::omitted()
    }
}

impl StreamlineArrows {
    /// The validation reason for a bad arrow size or spacing.
    pub const INVALID_REASON: crate::RuleKind = crate::RuleKind::StreamlineArrows;

    /// Validate invariants owned by the arrows themselves: finite sizes, a
    /// positive spacing and no head longer than the spacing, so two heads
    /// never overlap along a straight path.
    pub fn validate(&self) -> Result<(), crate::RuleKind> {
        let finite_non_negative = |v: f32| v.is_finite() && v >= 0.0;
        if !(self.spacing_px.is_finite() && self.spacing_px > 0.0)
            || !finite_non_negative(self.head_length_px)
            || !finite_non_negative(self.head_width_px)
            || self.spacing_px < self.head_length_px
        {
            return Err(Self::INVALID_REASON);
        }
        Ok(())
    }
}

/// The fields of a [`DataRenderType::Streamlines`] series, borrowed.
#[derive(Clone, Copy, Debug)]
pub struct StreamlinesRef<'a> {
    pub line: &'a DataLineStyleConfig,
    pub arrows: Option<&'a StreamlineArrows>,
    pub color_map: Option<&'a PointColorMap>,
}

impl DataRenderType {
    /// The fields of a streamline series, or `None` for every other kind.
    pub fn streamlines(&self) -> Option<StreamlinesRef<'_>> {
        match self {
            DataRenderType::Streamlines {
                line,
                arrows,
                color_map,
            } => Some(StreamlinesRef {
                line,
                arrows: arrows.as_ref(),
                color_map: color_map.as_ref(),
            }),
            _ => None,
        }
    }
}

/// The rules one streamline series is held to on its own: a linearly
/// interpolated line without a fill, valid arrows, and named coordinate and
/// colour columns. The chart rules (no polar chart, a colourbar for a
/// colour map) are checked with the chart.
pub fn validate_streamline_series(series: &SeriesConfig) -> Result<(), crate::RuleKind> {
    let Some(streamlines) = series.render_type.streamlines() else {
        return Ok(());
    };
    if !streamlines.line.interpolation.is_linear() {
        return Err(crate::RuleKind::StreamlineInterpolation);
    }
    if streamlines.line.fill.is_some() {
        return Err(crate::RuleKind::StreamlineFill);
    }
    if let Some(arrows) = streamlines.arrows {
        arrows.validate()?;
    }
    if series.x_column.is_empty() || series.y_column.is_empty() {
        return Err(crate::RuleKind::StreamlineColumns);
    }
    if streamlines
        .color_map
        .is_some_and(|map| map.column.is_empty())
    {
        return Err(crate::RuleKind::StreamlineColorMapColumn);
    }
    Ok(())
}

/// Where a vector series' components come from.
///
/// The anchors are the series' `x_column` / `y_column` in both forms. The grid
/// form shares a heatmap's coordinate columns, orientation and layout, so
/// arrows over a field are one more series naming the same columns.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum VectorComponents {
    /// `u` and `v` are index-aligned with the anchor columns. The draw count
    /// is the shortest of the four columns, or of five with a
    /// [`VectorColorMap::Column`]; nothing is length-inferred.
    Points {
        /// The x component column.
        u: ColumnId,
        /// The y component column.
        v: ColumnId,
        /// The z component column (#211): with the anchor height in
        /// [`SeriesConfig::z_column`], a 3D chart (`Config.view_3d`) draws
        /// each row as an arrow from `(x, y, z)` along `(u, v, w)`. A flat
        /// chart keeps it and draws `(u, v)`. Omitted from JSON when `None`.
        #[cfg_attr(
            feature = "serde",
            serde(default, skip_serializing_if = "Option::is_none")
        )]
        w: Option<ColumnId>,
    },
    /// `u` and `v` are grids over the coordinate columns, both with the same
    /// orientation and layout. With `Centers` the arrow sits on the
    /// coordinate; with `Edges` it sits at the cell centre, where a heatmap
    /// cell's value sits.
    Grid {
        /// The x component grid.
        u: MatrixRef,
        /// The y component grid.
        v: MatrixRef,
    },
}

impl VectorComponents {
    /// The grid declarations of the grid form.
    pub fn grid(&self) -> Option<(&MatrixRef, &MatrixRef)> {
        match self {
            Self::Grid { u, v } => Some((u, v)),
            Self::Points { .. } => None,
        }
    }
}

/// How an arrow's length is scaled.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum VectorScale {
    /// The tip is `origin + factor * vector` in data units. Meaningful when
    /// both axes share a unit; the arrows zoom with the data.
    Data {
        /// Data units of tip offset per unit of vector magnitude.
        factor: f64,
    },
    /// The screen length is `px_per_unit * |vector|` logical pixels along the
    /// data-space direction projected through the live transform, so arrows
    /// keep their length under zoom.
    Pixels {
        /// Logical pixels of arrow length per unit of vector magnitude.
        px_per_unit: f64,
    },
}

impl VectorScale {
    /// The value this type takes where a document omits it: the one source
    /// of its default (#266). `Default`, for hosts, returns it.
    #[cfg_attr(not(feature = "helpers"), allow(dead_code))]
    pub(crate) fn omitted() -> Self {
        Self::Pixels { px_per_unit: 1.0 }
    }
}

#[cfg(feature = "helpers")]
impl Default for VectorScale {
    fn default() -> Self {
        Self::omitted()
    }
}

/// Which point of the arrow sits on the anchor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum VectorPivot {
    /// The tail sits on the anchor (the default).
    Tail,
    /// The arrow is centred on the anchor.
    Middle,
    /// The tip sits on the anchor.
    Tip,
}

impl VectorPivot {
    /// The value this type takes where a document omits it: the one source
    /// of its default (#266). `Default`, for hosts, returns it.
    #[cfg_attr(not(feature = "helpers"), allow(dead_code))]
    pub(crate) const fn omitted() -> Self {
        Self::Tail
    }
}

#[cfg(feature = "helpers")]
impl Default for VectorPivot {
    fn default() -> Self {
        Self::omitted()
    }
}

/// A value mapped to arrow colour through `Config.colorbar`, in the same form
/// as the components: a column for [`VectorComponents::Points`], a grid for
/// [`VectorComponents::Grid`].
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum VectorColorMap {
    /// A value column index-aligned with the anchors.
    Column(ColumnId),
    /// A value grid over the coordinate columns.
    Grid(MatrixRef),
}

/// Arrow appearance of a vector series. Pixel sizes are logical pixels, like
/// `point_size`.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct VectorStyle {
    /// The arrow colour when `color_map` is `None`.
    pub color: Color,
    /// Shaft thickness.
    pub width_px: f32,
    /// How the vector's magnitude becomes a length on screen.
    pub scale: VectorScale,
    /// Which point of the arrow sits on the anchor.
    #[cfg_attr(feature = "serde", serde(default = "VectorPivot::omitted"))]
    pub pivot: VectorPivot,
    /// Head length along the shaft. An arrow shorter than its head scales the
    /// head down with it.
    pub head_length_px: f32,
    /// Full head width across the shaft.
    pub head_width_px: f32,
    /// Arrows shorter than this on screen are not drawn.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "VectorStyle::is_zero")
    )]
    pub min_length_px: f32,
    /// Colour each arrow by a value through `Config.colorbar`; the colourbar
    /// is the only authority on the range, scale, ramp and `nan_color`, as for
    /// a heatmap. A series with a colour map and no colourbar is rejected.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub color_map: Option<VectorColorMap>,
}

impl VectorStyle {
    /// The validation reason for a non-finite or negative pixel size.
    pub const INVALID_PIXELS_REASON: crate::RuleKind = crate::RuleKind::VectorPixels;
    /// The validation reason for a non-finite or zero scale.
    pub const INVALID_SCALE_REASON: crate::RuleKind = crate::RuleKind::VectorScale;

    #[cfg(feature = "serde")]
    fn is_zero(value: &f32) -> bool {
        *value == 0.0
    }

    /// Validate invariants owned by the style itself.
    pub fn validate(&self) -> Result<(), crate::RuleKind> {
        let pixels = [
            self.width_px,
            self.head_length_px,
            self.head_width_px,
            self.min_length_px,
        ];
        if pixels.iter().any(|px| !px.is_finite() || *px < 0.0) {
            return Err(Self::INVALID_PIXELS_REASON);
        }
        let scale = match self.scale {
            VectorScale::Data { factor } => factor,
            VectorScale::Pixels { px_per_unit } => px_per_unit,
        };
        if !scale.is_finite() || scale == 0.0 {
            return Err(Self::INVALID_SCALE_REASON);
        }
        Ok(())
    }

    /// Set the scale so the longest vector the bounds allow,
    /// `(max |u|, max |v|)` over `u_bounds` × `v_bounds`, draws `target` long:
    /// logical pixels for [`VectorScale::Pixels`], data units for
    /// [`VectorScale::Data`]. No arrow is longer than `target`; the longest
    /// drawn arrow is shorter when the largest `|u|` and `|v|` come from
    /// different samples. The bounds are the component columns' statistics,
    /// which the host already has.
    ///
    /// A display helper, like `PointSizeMap::fit_domain`: it writes only the
    /// scale value. A non-finite or zero extent, or a `target` that is not
    /// finite and positive, leaves the style unchanged and returns `false`.
    pub fn fit_scale(&mut self, u_bounds: (f64, f64), v_bounds: (f64, f64), target: f64) -> bool {
        let reach = |(min, max): (f64, f64)| {
            if !min.is_finite() || !max.is_finite() {
                return f64::NAN;
            }
            min.abs().max(max.abs())
        };
        let longest = reach(u_bounds).hypot(reach(v_bounds));
        if !longest.is_finite() || longest <= 0.0 || !target.is_finite() || target <= 0.0 {
            return false;
        }
        match &mut self.scale {
            VectorScale::Data { factor } => *factor = target / longest,
            VectorScale::Pixels { px_per_unit } => *px_per_unit = target / longest,
        }
        true
    }

    /// [`fit_scale`](Self::fit_scale) for a 3D vector field (#211): the
    /// longest vector the bounds allow is `hypot(max |u|, max |v|, max |w|)`
    /// over the three component columns' statistics, and it draws `target`
    /// long. In [`VectorScale::Pixels`] that is its length in the screen plane
    /// at its anchor; an arrow out of that plane is foreshortened by the
    /// camera. The same display helper: a non-finite or zero extent, or a
    /// `target` that is not finite and positive, leaves the style unchanged
    /// and returns `false`.
    pub fn fit_scale_3d(
        &mut self,
        u_bounds: (f64, f64),
        v_bounds: (f64, f64),
        w_bounds: (f64, f64),
        target: f64,
    ) -> bool {
        let reach = |(min, max): (f64, f64)| {
            if !min.is_finite() || !max.is_finite() {
                return f64::NAN;
            }
            min.abs().max(max.abs())
        };
        let (u, v, w) = (reach(u_bounds), reach(v_bounds), reach(w_bounds));
        let longest = (u * u + v * v + w * w).sqrt();
        if !longest.is_finite() || longest <= 0.0 || !target.is_finite() || target <= 0.0 {
            return false;
        }
        match &mut self.scale {
            VectorScale::Data { factor } => *factor = target / longest,
            VectorScale::Pixels { px_per_unit } => *px_per_unit = target / longest,
        }
        true
    }
}
