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
    /// The plot layout region; registered behind labels and axes.
    DataArea,
    /// Mutation target used by the common resize policy, not a separate hit.
    DataAreaEdge(Side),
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
    /// A radar chart's category label at the end of its spoke, which has no
    /// side to belong to.
    SpokeLabel {
        category_id: String,
    },
    /// An element of an overlay layer, addressed by the layer index and the
    /// element's identity inside that layer. Overlay expansion registers the
    /// active overlay's elements under this wrapper so drag policies reach the
    /// right layer.
    Overlay {
        layer: usize,
        inner: Box<DecorationId>,
    },
    /// An item of the annotation layer by index into its item list.
    Annotation(usize),
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
            Self::DataArea => "data_area".into(),
            Self::DataAreaEdge(s) => format!("data_area_edge_{}", side(s)),
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
            Self::SpokeLabel { category_id } => {
                format!("spoke_label_{}:{}", category_id.len(), category_id)
            }
            Self::Overlay { layer, inner } => format!("overlay_{layer}:{}", inner.element_id()),
            Self::Annotation(index) => format!("annotation_{index}"),
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
    /// Explicit plot region in logical pixels. None uses the measured automatic
    /// region; a chosen region must fit inside it to retain space for labels.
    pub data_area: Option<RectF>,
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
        if *id == DecorationId::DataArea {
            self.data_area = None;
        }
        self.offsets.retain(|o| &o.element != id);
    }
    pub fn resolve_data_area(&self, automatic: RectF) -> Result<RectF, &'static str> {
        let r = self.data_area.unwrap_or(automatic);
        if ![r.x, r.y, r.width, r.height, r.x + r.width, r.y + r.height]
            .into_iter()
            .all(f32::is_finite)
            || r.x < 0.
            || r.y < 0.
            || r.width < 80.
            || r.height < 60.
            || r.x < automatic.x
            || r.y < automatic.y
            || r.x + r.width > automatic.x + automatic.width
            || r.y + r.height > automatic.y + automatic.height
        {
            return Err(
                "data area must fit the measured plot region and be at least 80 by 60 logical pixels",
            );
        }
        Ok(r)
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        if let Some(r) = self.data_area {
            self.resolve_data_area(r)?;
        }
        if self.offsets.len() > 1024 {
            return Err("too many decoration offsets");
        }
        for (i, o) in self.offsets.iter().enumerate() {
            if o.offset.iter().any(|x| !x.is_finite())
                || self.offsets[..i].iter().any(|p| p.element == o.element)
                || matches!(
                    o.element,
                    DecorationId::DataArea | DecorationId::DataAreaEdge(_)
                )
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
        match self.id {
            DecorationId::DataArea => return cfg.resolve_data_area(self.base_bounds).ok(),
            DecorationId::DataAreaEdge(_) => return None,
            _ => {}
        }
        let p = cfg.effective_offset(&self.id);
        Some(self.base_bounds.translated(p[0], p[1]))
    }
    fn as_draggable(&self) -> Option<&dyn Draggable<DecorationLayout>> {
        Some(self)
    }
    fn as_resizable(&self) -> Option<&dyn crate::resize::Resizable<DecorationLayout>> {
        (self.id == DecorationId::DataArea).then_some(self)
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
impl crate::resize::Resizable<DecorationLayout> for DecorationElement {
    fn resize_targets(&self, handle: crate::resize::ResizeHandle) -> (Option<Self>, Option<Self>) {
        use crate::resize::ResizeHandle::*;
        let edge = |side| {
            let mut e = self.clone();
            e.id = DecorationId::DataAreaEdge(side);
            e
        };
        let x = match handle {
            NW | W | SW => Some(edge(Side::Left)),
            NE | E | SE => Some(edge(Side::Right)),
            N | S => None,
        };
        let y = match handle {
            NW | N | NE => Some(edge(Side::Top)),
            SW | S | SE => Some(edge(Side::Bottom)),
            E | W => None,
        };
        (x, y)
    }
    fn resize_by(
        &self,
        cfg: &mut DecorationLayout,
        handle: crate::resize::ResizeHandle,
        dx: f32,
        dy: f32,
    ) -> NudgeResult {
        let reject = NudgeResult::Rejected(NudgeReject::OutOfBounds);
        if self.id != DecorationId::DataArea || !dx.is_finite() || !dy.is_finite() {
            return reject;
        }
        let (x, y) = self.resize_targets(handle);
        let mut trial = cfg.clone();
        for (element, dx, dy) in [(x, dx, 0.), (y, 0., dy)] {
            if let Some(element) = element {
                if trial.nudge(element, dx, dy) != NudgeResult::Moved {
                    return reject;
                }
            }
        }
        *cfg = trial;
        NudgeResult::Moved
    }
}
impl DragConfig for DecorationLayout {
    type Element = DecorationElement;
    fn nudge(&mut self, element: DecorationElement, mut dx: f32, mut dy: f32) -> NudgeResult {
        let reject = NudgeResult::Rejected(NudgeReject::OutOfBounds);
        if !dx.is_finite() || !dy.is_finite() {
            return reject;
        }
        if matches!(
            element.id,
            DecorationId::DataArea | DecorationId::DataAreaEdge(_)
        ) {
            let Ok(mut r) = self.resolve_data_area(element.base_bounds) else {
                return reject;
            };
            match element.id {
                DecorationId::DataArea => {
                    r.x += dx;
                    r.y += dy;
                }
                DecorationId::DataAreaEdge(Side::Left) => {
                    r.x += dx;
                    r.width -= dx;
                }
                DecorationId::DataAreaEdge(Side::Right) => r.width += dx,
                DecorationId::DataAreaEdge(Side::Top) => {
                    r.y += dy;
                    r.height -= dy;
                }
                DecorationId::DataAreaEdge(Side::Bottom) => r.height += dy,
                _ => unreachable!(),
            }
            let mut trial = self.clone();
            trial.data_area = Some(r);
            if trial.resolve_data_area(element.base_bounds).is_err() {
                return reject;
            }
            self.data_area = Some(r);
            return NudgeResult::Moved;
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

    // Layer-addressed identities get stable element-id strings that embed the
    // wrapped element's own id, so hosts keying UI on these strings can route
    // by layer without parsing the enum.
    #[test]
    fn layer_decoration_ids_have_stable_element_ids() {
        let axis = DecorationId::Overlay {
            layer: 1,
            inner: Box::new(DecorationId::Axis(Side::Right)),
        };
        assert_eq!(axis.element_id(), "overlay_1:axis_right");
        let nested_label = DecorationId::Overlay {
            layer: 0,
            inner: Box::new(DecorationId::AxisLabel(Side::Left)),
        };
        assert_eq!(nested_label.element_id(), "overlay_0:tick_labels_left");
        assert_eq!(DecorationId::Annotation(3).element_id(), "annotation_3");
        // Base-layer ids are unchanged by the addition.
        assert_eq!(DecorationId::Axis(Side::Right).element_id(), "axis_right");
    }

    #[cfg(feature = "serde")]
    #[test]
    fn layer_decoration_ids_round_trip() {
        for id in [
            DecorationId::Overlay {
                layer: 2,
                inner: Box::new(DecorationId::Legend),
            },
            DecorationId::Annotation(0),
        ] {
            let json = serde_json::to_string(&id).unwrap();
            let back: DecorationId = serde_json::from_str(&json).unwrap();
            assert_eq!(back, id, "{json}");
        }
        let mut layout = DecorationLayout::default();
        layout.selected = Some(DecorationId::Annotation(4));
        let json = serde_json::to_string(&layout).unwrap();
        let back: DecorationLayout = serde_json::from_str(&json).unwrap();
        assert_eq!(back, layout);
    }

    #[test]
    fn area_handles_move_only_their_edges_and_reject_atomically() {
        use crate::resize::{Resizable, ResizeHandle::*};
        let e = DecorationElement {
            id: DecorationId::DataArea,
            base_bounds: RectF {
                x: 20.,
                y: 20.,
                width: 240.,
                height: 160.,
            },
            canvas_size: (300, 200),
            followers: vec![],
        };
        let original = RectF {
            x: 50.,
            y: 50.,
            width: 150.,
            height: 90.,
        };
        for (handle, left, right, top, bottom) in [
            (NW, true, false, true, false),
            (N, false, false, true, false),
            (NE, false, true, true, false),
            (E, false, true, false, false),
            (SE, false, true, false, true),
            (S, false, false, false, true),
            (SW, true, false, false, true),
            (W, true, false, false, false),
        ] {
            let mut state = DecorationLayout {
                data_area: Some(original),
                ..Default::default()
            };
            assert_eq!(e.resize_by(&mut state, handle, 7., 9.), NudgeResult::Moved);
            let r = state.data_area.unwrap();
            assert_eq!(r.x, original.x + if left { 7. } else { 0. });
            assert_eq!(r.y, original.y + if top { 9. } else { 0. });
            assert_eq!(
                r.x + r.width,
                original.x + original.width + if right { 7. } else { 0. }
            );
            assert_eq!(
                r.y + r.height,
                original.y + original.height + if bottom { 9. } else { 0. }
            );
        }
        let mut state = DecorationLayout {
            data_area: Some(original),
            ..Default::default()
        };
        let before = state.clone();
        for (dx, dy) in [
            (7., -1000.),
            (f32::NAN, 0.),
            (0., f32::INFINITY),
            (100., 0.),
        ] {
            assert!(matches!(
                e.resize_by(&mut state, NW, dx, dy),
                NudgeResult::Rejected(_)
            ));
            assert_eq!(state, before);
        }
        state.reset(&DecorationId::DataArea);
        assert_eq!(
            state.resolve_data_area(e.base_bounds).unwrap(),
            e.base_bounds
        );
    }
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
            data_area: None,
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
            serde_json::to_value(crate::RadarChart::default()).unwrap(),
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
            } else if value.get("direction").is_some() {
                assert_eq!(
                    serde_json::from_value::<crate::BoxPlotChart>(value)
                        .unwrap()
                        .decorations,
                    DecorationLayout::default()
                );
            } else {
                assert_eq!(
                    serde_json::from_value::<crate::RadarChart>(value)
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
        boxplot.decorations = state.clone();
        let mut radar = crate::RadarChart::default();
        radar.decorations = state;
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
        assert_eq!(
            serde_json::from_str::<crate::RadarChart>(&serde_json::to_string(&radar).unwrap())
                .unwrap(),
            radar
        );
    }
}
