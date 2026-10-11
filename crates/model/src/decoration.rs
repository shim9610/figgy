//! Editable decoration positions for bounded charts. The same `HitMap`,
//! `Selectable`, `Draggable` and `SelectionBox` policy as Cartesian charts applies.
use crate::{
    drag::{DragConfig, Draggable},
    layout::{NudgeReject, NudgeResult, RectF, Side},
    select::Selectable,
    text::MeasureText,
};

/// Stable identity within a chart. Category/series IDs survive reordering;
/// radial indices refer to the original arrays, including zero-valued slices.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum DecorationId {
    ChartTitle,
    Axis(Side),
    AxisLabel(Side),
    AxisTitle(Side),
    Legend,
    RadialLabel(crate::radial::RadialTarget),
    DataLabel {
        category_id: String,
        series_id: String,
    },
}
impl DecorationId {
    pub fn element_id(&self) -> String {
        let side = |s: &Side| match s {
            Side::Top => "top",
            Side::Bottom => "bottom",
            Side::Left => "left",
            Side::Right => "right",
        };
        match self {
            Self::ChartTitle => "chart_title".into(),
            Self::Axis(s) => format!("axis_{}", side(s)),
            Self::AxisLabel(s) => format!("tick_labels_{}", side(s)),
            Self::AxisTitle(s) => format!("axis_title_{}", side(s)),
            Self::Legend => "legend".into(),
            Self::RadialLabel(t) => format!("radial_label_{}_{}", t.group, t.index),
            Self::DataLabel {
                category_id,
                series_id,
            } => format!(
                "data_label_{}:{}{}:{}",
                category_id.len(),
                category_id,
                series_id.len(),
                series_id
            ),
        }
    }
}
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct DecorationOffset {
    pub element: DecorationId,
    /// Chart-local logical pixels, independent of DPR/export scale.
    pub offset: [f32; 2],
}
#[derive(Clone, Debug, Default, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(default)
)]
pub struct DecorationLayout {
    pub offsets: Vec<DecorationOffset>,
    /// Decoration selection is separate from data selection and hover.
    pub selected: Option<DecorationId>,
}
impl DecorationLayout {
    pub fn offset(&self, id: &DecorationId) -> [f32; 2] {
        self.offsets
            .iter()
            .find(|o| &o.element == id)
            .map_or([0.; 2], |o| o.offset)
    }
    /// Axis labels follow detached axes, plus their own freely editable offset.
    pub fn effective_offset(&self, id: &DecorationId) -> [f32; 2] {
        let mut p = self.offset(id);
        if let DecorationId::AxisLabel(side) = id {
            let q = self.offset(&DecorationId::Axis(side.clone()));
            p[0] += q[0];
            p[1] += q[1];
        }
        p
    }
    pub fn reset(&mut self, id: &DecorationId) {
        self.offsets.retain(|o| &o.element != id);
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.offsets.len() > 1024 {
            return Err("too many decoration offsets");
        }
        for (i, o) in self.offsets.iter().enumerate() {
            if o.offset.iter().any(|x| !x.is_finite())
                || self.offsets[..i].iter().any(|p| p.element == o.element)
            {
                return Err("decoration offsets must be finite and unique");
            }
            if let DecorationId::Axis(s) = &o.element {
                let parallel = match s {
                    Side::Top | Side::Bottom => o.offset[0],
                    _ => o.offset[1],
                };
                if parallel != 0.0 {
                    return Err("axes only move perpendicular to their direction");
                }
            }
        }
        Ok(())
    }
}
/// Renderer-measured base bounds, before user offsets. A fresh frame supplies
/// these after resizing/reflow; hosts never reconstruct text or axis geometry.
#[derive(Clone, Debug)]
pub struct DecorationElement {
    pub id: DecorationId,
    pub base_bounds: RectF,
    pub canvas_size: (u32, u32),
    /// Parts that move together with this target (e.g. a detached axis's labels).
    pub followers: Vec<(DecorationId, RectF)>,
}
impl Selectable<DecorationLayout> for DecorationElement {
    fn bounds(&self, cfg: &DecorationLayout, _: &dyn MeasureText) -> Option<RectF> {
        let p = cfg.effective_offset(&self.id);
        Some(self.base_bounds.translated(p[0], p[1]))
    }
    fn as_draggable(&self) -> Option<&dyn Draggable<DecorationLayout>> {
        Some(self)
    }
    fn element_id(&self) -> String {
        self.id.element_id()
    }
}
impl Draggable<DecorationLayout> for DecorationElement {
    fn nudge_element(&self) -> DecorationElement {
        self.clone()
    }
}
impl DragConfig for DecorationLayout {
    type Element = DecorationElement;
    fn nudge(&mut self, element: DecorationElement, mut dx: f32, mut dy: f32) -> NudgeResult {
        let reject = NudgeResult::Rejected(NudgeReject::OutOfBounds);
        if !dx.is_finite() || !dy.is_finite() {
            return reject;
        }
        if let DecorationId::Axis(side) = &element.id {
            match side {
                Side::Top | Side::Bottom => dx = 0.,
                _ => dy = 0.,
            }
        }
        let inside = |rect: RectF, offset: [f32; 2]| {
            let r = rect.translated(offset[0] + dx, offset[1] + dy);
            r.x >= 0.
                && r.y >= 0.
                && r.x + r.width <= element.canvas_size.0 as f32
                && r.y + r.height <= element.canvas_size.1 as f32
        };
        if !inside(element.base_bounds, self.effective_offset(&element.id))
            || element
                .followers
                .iter()
                .any(|(id, rect)| !inside(*rect, self.effective_offset(id)))
        {
            return reject;
        }
        let old = self.offset(&element.id);
        let next = [old[0] + dx, old[1] + dy];
        self.reset(&element.id);
        if next != [0., 0.] {
            self.offsets.push(DecorationOffset {
                element: element.id,
                offset: next,
            });
        }
        NudgeResult::Moved
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn element(id: DecorationId) -> DecorationElement {
        DecorationElement {
            id,
            base_bounds: RectF {
                x: 20.,
                y: 20.,
                width: 100.,
                height: 20.,
            },
            canvas_size: (300, 200),
            followers: vec![],
        }
    }
    #[test]
    fn cartesian_hit_map_constructors_keep_type_inference_without_annotations() {
        let empty = crate::HitMap::new();
        let default = crate::HitMap::default();
        assert!(empty.is_empty() && default.is_empty());
    }
    #[test]
    fn drag_rejects_nonfinite_outside_and_axis_follower_overflow_atomically() {
        let mut state = DecorationLayout::default();
        let mut axis = element(DecorationId::Axis(Side::Left));
        axis.followers.push((
            DecorationId::AxisLabel(Side::Left),
            RectF {
                x: 1.,
                y: 20.,
                width: 20.,
                height: 30.,
            },
        ));
        for delta in [[f32::NAN, 0.], [0., f32::INFINITY], [-2., 0.], [1000., 0.]] {
            assert!(matches!(
                axis.drag_by(&mut state, delta[0], delta[1]),
                NudgeResult::Rejected(_)
            ));
            assert_eq!(state, DecorationLayout::default());
        }
        assert_eq!(axis.drag_by(&mut state, 5., 80.), NudgeResult::Moved);
        assert_eq!(state.offset(&axis.id), [5., 0.]);
        assert_eq!(
            state.effective_offset(&DecorationId::AxisLabel(Side::Left)),
            [5., 0.]
        );
    }
    #[test]
    fn duplicate_nonfinite_and_parallel_axis_offsets_are_rejected() {
        let mut state = DecorationLayout {
            offsets: vec![DecorationOffset {
                element: DecorationId::Legend,
                offset: [f32::NAN, 0.],
            }],
            selected: None,
        };
        assert!(state.validate().is_err());
        state.offsets[0].offset = [1., 2.];
        state.offsets.push(state.offsets[0].clone());
        assert!(state.validate().is_err());
        state.offsets.pop();
        state.offsets[0].element = DecorationId::Axis(Side::Left);
        assert!(state.validate().is_err());
    }
    #[cfg(feature = "serde")]
    #[test]
    fn old_documents_default_positions_and_all_models_preserve_edits() {
        for mut value in [
            serde_json::to_value(crate::RadialChart::default()).unwrap(),
            serde_json::to_value(crate::CategoricalChart::default()).unwrap(),
            serde_json::to_value(crate::BoxPlotChart::default()).unwrap(),
        ] {
            value.as_object_mut().unwrap().remove("decorations");
            assert!(value.get("decorations").is_none());
            // Each old chart type fills only the newly added state with defaults.
            if value.get("slices").is_some() {
                assert_eq!(
                    serde_json::from_value::<crate::RadialChart>(value)
                        .unwrap()
                        .decorations,
                    DecorationLayout::default()
                );
            } else if value.get("mode").is_some() {
                assert_eq!(
                    serde_json::from_value::<crate::CategoricalChart>(value)
                        .unwrap()
                        .decorations,
                    DecorationLayout::default()
                );
            } else {
                assert_eq!(
                    serde_json::from_value::<crate::BoxPlotChart>(value)
                        .unwrap()
                        .decorations,
                    DecorationLayout::default()
                );
            }
        }
        let mut state = DecorationLayout::default();
        element(DecorationId::ChartTitle).drag_by(&mut state, 4., 6.);
        state.selected = Some(DecorationId::ChartTitle);
        let mut radial = crate::RadialChart::default();
        radial.decorations = state.clone();
        let mut categorical = crate::CategoricalChart::default();
        categorical.decorations = state.clone();
        let mut boxplot = crate::BoxPlotChart::default();
        boxplot.decorations = state;
        assert_eq!(
            serde_json::from_str::<crate::RadialChart>(&serde_json::to_string(&radial).unwrap())
                .unwrap(),
            radial
        );
        assert_eq!(
            serde_json::from_str::<crate::CategoricalChart>(
                &serde_json::to_string(&categorical).unwrap()
            )
            .unwrap(),
            categorical
        );
        assert_eq!(
            serde_json::from_str::<crate::BoxPlotChart>(&serde_json::to_string(&boxplot).unwrap())
                .unwrap(),
            boxplot
        );
    }
}
