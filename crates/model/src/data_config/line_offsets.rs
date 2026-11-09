//! Offset spectra ([`LineOffsets`]).

use super::*;

/// Offset spectra (a ridgeline or waterfall of curves): line series drawn
/// shifted from each other so their traces separate instead of overlapping,
/// `Config::line_offsets`.
///
/// Member `k` of `series` (in stacking order, the first member unshifted) is
/// drawn shifted by `k · step_y` along y and `k · step_x` along x. The steps
/// are display parameters in data units, or decades on a logarithmic axis:
/// the host supplies the curves as they are and nothing is shifted on the
/// CPU. Hover, picking, selection details and `step_selection` report the
/// unshifted values; an auto-fit of the axes includes the shifts.
///
/// The renderer draws member `k` as an internal panel of its own, the base
/// panel (or, for a member an overlay layer scales, that overlay's panel)
/// with its X and Y ranges moved by the shift and every axis element hidden
/// (`Config::expand`), so lines, fills, markers, error bars, dashes and the
/// picked highlight follow without a transform of their own. Later members
/// draw over earlier ones, fill included, which gives the ridgeline
/// occlusion, and a pick answers with what is painted under the cursor: the
/// last member painted there first, else the nearest hit of any series.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(default = "LineOffsets::omitted")
)]
pub struct LineOffsets {
    /// The member series ids in stacking order: member `k` is shifted by
    /// `k` steps. Line kinds only (`Line`, `ScatterLine` and the
    /// `LineScatterErrorbar*` kinds), each id once, with no cap on the
    /// number of members (#107): each is drawn as an internal panel of its
    /// own (`Config::expand`) with the textures of an overlay layer, and a
    /// member whose textures cannot be allocated fails with the GPU
    /// allocation error.
    pub series: Vec<SeriesRef>,
    /// The step along y between consecutive members: data units, or decades
    /// on a logarithmic axis.
    pub step_y: f64,
    /// The step along x for a skewed waterfall, in the same units as
    /// `step_y`. Omitted from JSON at `0.0`.
    #[cfg_attr(feature = "serde", serde(skip_serializing_if = "is_zero_step"))]
    pub step_x: f64,
}

#[cfg(feature = "serde")]
pub(super) fn is_zero_step(step: &f64) -> bool {
    *step == 0.0
}

impl LineOffsets {
    /// The value this type takes where a document omits it: the one source
    /// of its default (#266). `Default`, for hosts, returns it.
    #[cfg_attr(not(feature = "helpers"), allow(dead_code))]
    pub(crate) fn omitted() -> Self {
        Self {
            series: Vec::new(),
            step_y: 0.0,
            step_x: 0.0,
        }
    }
}

#[cfg(feature = "helpers")]
impl Default for LineOffsets {
    fn default() -> Self {
        Self::omitted()
    }
}

impl LineOffsets {
    /// The rules that do not depend on the series list: finite steps, each
    /// member id once and non-empty, any number of members.
    pub fn validate(&self) -> Result<(), crate::RuleKind> {
        if !self.step_y.is_finite() || !self.step_x.is_finite() {
            return Err(crate::RuleKind::LineOffsetSteps);
        }
        for (index, id) in self.series.iter().enumerate() {
            if id.is_empty() || self.series[..index].contains(id) {
                return Err(crate::RuleKind::LineOffsetMember);
            }
        }
        Ok(())
    }

    /// The stacking position of `series_id`, `None` for a series that is not
    /// a member.
    pub fn member(&self, series_id: &str) -> Option<usize> {
        self.series.iter().position(|id| id == series_id)
    }

    /// The shift of member `k`: `[k · step_x, k · step_y]`.
    pub fn shift(&self, k: usize) -> [f64; 2] {
        let k = k as f64;
        [k * self.step_x, k * self.step_y]
    }
}

impl DataRenderType {
    /// Whether a series of this kind may be a [`LineOffsets`] member: the
    /// kinds that draw a line.
    pub fn takes_line_offset(&self) -> bool {
        match self {
            DataRenderType::Xy(xy) => xy.line.is_some(),
            DataRenderType::Histogram { .. }
            | DataRenderType::Field(_)
            | DataRenderType::RgbImage { .. }
            | DataRenderType::Vector { .. }
            | DataRenderType::CategoryBars { .. }
            | DataRenderType::BoxPlot { .. }
            | DataRenderType::RangeBars { .. }
            | DataRenderType::EnergyLevels(_)
            | DataRenderType::TriMesh(_)
            | DataRenderType::Feynman(_)
            | DataRenderType::Radar { .. }
            // A field's streamlines share its axes; they are not a member.
            | DataRenderType::Streamlines { .. }
            // A ternary chart has no line offsets.
            | DataRenderType::Ternary { .. }
            // Nor has a parallel chart.
            | DataRenderType::Parallel { .. } => false,
        }
    }

    /// The fill of a line kind, `None` for a line without one and for every
    /// other kind.
    fn line_fill(&self) -> Option<&LineFill> {
        match self {
            DataRenderType::Xy(crate::data_config::XySeries {
                line: Some(line), ..
            }) => line.fill.as_ref(),
            _ => None,
        }
    }
}

impl crate::config::Config {
    /// The line offset group of the chart that draws `series_id` (the
    /// parent's for its base layer and overlays, a subplot's own for the
    /// series it draws) and the series' stacking position in it.
    fn offset_member(&self, series_id: &str) -> Option<(Option<usize>, usize)> {
        use crate::layer::LayerRef;
        let (cell, chart) = match self.layer_of_series(series_id) {
            LayerRef::Subplot(index) | LayerRef::SubplotOverlay(index, _) => {
                (Some(index), &self.subplots.get(index)?.config)
            }
            LayerRef::Base | LayerRef::Overlay(_) | LayerRef::Annotations => (None, self),
        };
        let k = chart.line_offsets.as_ref()?.member(series_id)?;
        Some((cell, k))
    }

    /// The display shift of `series_id` as `[dx, dy]`: `k · step` of its
    /// position `k` in the line offset group of the chart that draws it, in
    /// data units or decades on a logarithmic axis. `None` for a series that
    /// is not a member.
    pub fn line_offset_of(&self, series_id: &str) -> Option<[f64; 2]> {
        let (cell, k) = self.offset_member(series_id)?;
        let chart = match cell {
            Some(index) => &self.subplots[index].config,
            None => self,
        };
        Some(chart.line_offsets.as_ref()?.shift(k))
    }

    /// The line offset rules that do not depend on the series list, for the
    /// chart and every subplot: [`LineOffsets::validate`], no polar chart,
    /// and every member drawn by the chart that lists it (the parent's base
    /// layer or one of its overlays, or the subplot itself).
    pub fn validate_line_offset_layout(&self) -> Result<(), crate::RuleKind> {
        use crate::layer::LayerRef;
        let charts = std::iter::once((None, self)).chain(
            self.subplots
                .iter()
                .enumerate()
                .map(|(index, subplot)| (Some(index), &subplot.config)),
        );
        for (cell, chart) in charts {
            let Some(offsets) = &chart.line_offsets else {
                continue;
            };
            offsets.validate()?;
            if offsets.series.is_empty() {
                continue;
            }
            if chart.polar().is_some() {
                return Err(crate::RuleKind::PolarLineOffsets);
            }
            for id in &offsets.series {
                let drawn = match (cell, self.layer_of_series(id)) {
                    (None, LayerRef::Base | LayerRef::Overlay(_)) => true,
                    (Some(index), LayerRef::Subplot(at) | LayerRef::SubplotOverlay(at, _)) => {
                        at == index
                    }
                    _ => false,
                };
                if !drawn {
                    return Err(crate::RuleKind::LineOffsetMemberLayer);
                }
            }
        }
        Ok(())
    }

    /// The line offset rules with the series list the chart draws:
    /// [`Self::validate_line_offset_layout`], every member a declared series
    /// of a line kind ([`DataRenderType::takes_line_offset`]), and no
    /// [`LineFill::ToSeries`] fill between two series of which only one is a
    /// member or that are different members: each member is drawn by a panel
    /// of its own, so such a fill would span two panels.
    pub fn validate_line_offsets(&self, series: &[SeriesConfig]) -> Result<(), crate::RuleKind> {
        self.validate_line_offset_layout()?;
        let charts = std::iter::once(self).chain(self.subplots.iter().map(|s| &s.config));
        for chart in charts {
            let Some(offsets) = &chart.line_offsets else {
                continue;
            };
            for id in &offsets.series {
                let Some(member) = series.iter().find(|s| s.series_id == *id) else {
                    return Err(crate::RuleKind::LineOffsetMemberUnknown);
                };
                if !member.render_type.takes_line_offset() {
                    return Err(crate::RuleKind::LineOffsetMemberNotLine);
                }
            }
        }
        for item in series {
            if let Some(LineFill::ToSeries(target)) = item.render_type.line_fill()
                && self.offset_member(&item.series_id) != self.offset_member(target)
            {
                return Err(crate::RuleKind::LineOffsetFill);
            }
        }
        Ok(())
    }
}
