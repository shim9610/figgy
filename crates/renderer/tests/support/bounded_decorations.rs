//! The decoration queries the bounded chart tests share over the bounded
//! charts' two hit-map APIs: the `DecorationMap` of the box plot and radar
//! frames, which answers alone, and the radial frame's
//! `HitMap<RadialChart>`, whose elements place their parts from the chart
//! they are asked about.
#![allow(dead_code)]
use ::renderer::{
    BoundedAnnotationFrame, CpuTextMeasure, DecorationId, DecorationLayout, HitId, LayerHit,
    LayerRef, RadarChart, RadarFrame, RadialChart, RadialFrame, SelectionBox,
    layout::{NudgeResult, RectF},
    resize::ResizeHandle,
    select::SelectionGeometry,
    sink::{DrawnPart, Parts},
};

pub trait Decorations {
    type Chart: Clone + PartialEq;
    /// Whether the frame draws data under `p`: the box plot and radar
    /// frames' CPU hit tests, the radial frame's GPU pick.
    async fn has_data(self: &std::sync::Arc<Self>, p: [f32; 2]) -> bool;
    /// The chart's decoration state.
    fn layout(chart: &mut Self::Chart) -> &mut DecorationLayout;
    /// Every decoration the frame draws.
    fn entries(&self, chart: &Self::Chart) -> Vec<(HitId, DecorationId)>;
    /// The number of entries of the frame's hit map.
    fn len(&self) -> usize;
    /// The entry the hit map finds by stable element name.
    fn find(&self, name: &str) -> Option<HitId>;
    fn hit(&self, chart: &Self::Chart, p: [f32; 2]) -> Option<HitId>;
    /// Everything the element draws.
    fn bounds(&self, chart: &Self::Chart, hit: HitId) -> Option<RectF>;
    /// What the element moves when it is dragged: its text, for a radial
    /// label whose leader line keeps its start on the slice; all of it
    /// otherwise.
    fn text_bounds(&self, chart: &Self::Chart, hit: HitId) -> Option<RectF>;
    fn selection(&self, chart: &Self::Chart, hit: HitId) -> Option<SelectionBox>;
    /// The followers a move takes along.
    fn followers(&self, chart: &Self::Chart, hit: HitId) -> Vec<DecorationId>;
    fn drag(&self, chart: &mut Self::Chart, hit: HitId, dx: f32, dy: f32) -> Option<NudgeResult>;
    fn resize(
        &self,
        chart: &mut Self::Chart,
        hit: HitId,
        handle: ResizeHandle,
        dx: f32,
        dy: f32,
    ) -> Option<NudgeResult>;
    fn handle_at(&self, chart: &Self::Chart, hit: HitId, p: [f32; 2]) -> Option<ResizeHandle>;
    /// `contains` of the element itself.
    fn contains(&self, chart: &Self::Chart, hit: HitId, p: [f32; 2]) -> bool;
    /// The annotation layer's pointer route over this frame's decorations.
    fn route(
        &self,
        annotations: &BoundedAnnotationFrame,
        layer: LayerRef,
        chart: &Self::Chart,
        p: [f32; 2],
    ) -> Option<LayerHit>;

    /// The entry of `key`; panics when the frame does not draw it.
    fn id_of(&self, chart: &Self::Chart, key: &DecorationId) -> HitId {
        self.entries(chart)
            .into_iter()
            .find(|(_, k)| k == key)
            .unwrap_or_else(|| panic!("missing {key:?}"))
            .0
    }
    fn key_of(&self, chart: &Self::Chart, hit: HitId) -> Option<DecorationId> {
        self.entries(chart)
            .into_iter()
            .find(|(h, _)| *h == hit)
            .map(|(_, k)| k)
    }
}

macro_rules! decoration_map_frame {
    ($frame:ty, $chart:ty) => {
        impl Decorations for $frame {
            type Chart = $chart;
            async fn has_data(self: &std::sync::Arc<Self>, p: [f32; 2]) -> bool {
                self.hit_test(p).is_some()
            }
            fn layout(chart: &mut $chart) -> &mut DecorationLayout {
                &mut chart.decorations
            }
            fn entries(&self, _: &$chart) -> Vec<(HitId, DecorationId)> {
                self.decorations()
                    .entries()
                    .map(|(id, e)| (id, e.id.clone()))
                    .collect()
            }
            fn len(&self) -> usize {
                self.decorations().hit_map().len()
            }
            fn find(&self, name: &str) -> Option<HitId> {
                self.decorations().hit_map().find(name)
            }
            fn hit(&self, _: &$chart, p: [f32; 2]) -> Option<HitId> {
                self.decorations().hit_test(p)
            }
            fn bounds(&self, _: &$chart, hit: HitId) -> Option<RectF> {
                self.decorations().bounds(hit)
            }
            fn text_bounds(&self, _: &$chart, hit: HitId) -> Option<RectF> {
                self.decorations().bounds(hit)
            }
            fn selection(&self, _: &$chart, hit: HitId) -> Option<SelectionBox> {
                self.decorations().selection_box(hit)
            }
            fn followers(&self, _: &$chart, hit: HitId) -> Vec<DecorationId> {
                self.decorations()
                    .element(hit)
                    .map(|e| e.followers.iter().map(|(f, _)| f.clone()).collect())
                    .unwrap_or_default()
            }
            fn drag(
                &self,
                chart: &mut $chart,
                hit: HitId,
                dx: f32,
                dy: f32,
            ) -> Option<NudgeResult> {
                self.decorations()
                    .drag_by(&mut chart.decorations, hit, dx, dy)
            }
            fn resize(
                &self,
                chart: &mut $chart,
                hit: HitId,
                handle: ResizeHandle,
                dx: f32,
                dy: f32,
            ) -> Option<NudgeResult> {
                self.decorations()
                    .resize_by(&mut chart.decorations, hit, handle, dx, dy)
            }
            fn handle_at(&self, _: &$chart, hit: HitId, p: [f32; 2]) -> Option<ResizeHandle> {
                self.decorations().hit_resize_handle(hit, p)
            }
            fn contains(&self, _: &$chart, hit: HitId, p: [f32; 2]) -> bool {
                let map = self.decorations();
                map.hit_map().get(hit).is_some_and(|e| {
                    e.contains(
                        map.layout(),
                        &CpuTextMeasure::for_style(&Default::default()),
                        p[0],
                        p[1],
                    )
                })
            }
            fn route(
                &self,
                annotations: &BoundedAnnotationFrame,
                layer: LayerRef,
                _: &$chart,
                p: [f32; 2],
            ) -> Option<LayerHit> {
                annotations.hit_test_active_layer(layer, self.decorations(), p)
            }
        }
    };
}
decoration_map_frame!(RadarFrame, RadarChart);

impl Decorations for RadialFrame {
    type Chart = RadialChart;
    async fn has_data(self: &std::sync::Arc<Self>, p: [f32; 2]) -> bool {
        self.pick(p)
            .expect("pick request")
            .resolve()
            .await
            .expect("pick result")
            .is_some()
    }
    fn layout(chart: &mut RadialChart) -> &mut DecorationLayout {
        &mut chart.decorations
    }
    fn entries(&self, chart: &RadialChart) -> Vec<(HitId, DecorationId)> {
        self.decoration_ids(chart).collect()
    }
    fn len(&self) -> usize {
        self.decorations().len()
    }
    fn find(&self, name: &str) -> Option<HitId> {
        self.decorations().find(name)
    }
    fn hit(&self, chart: &RadialChart, p: [f32; 2]) -> Option<HitId> {
        self.decoration_at(chart, p)
    }
    fn bounds(&self, chart: &RadialChart, hit: HitId) -> Option<RectF> {
        self.decoration_bounds(chart, hit)
    }
    fn text_bounds(&self, chart: &RadialChart, hit: HitId) -> Option<RectF> {
        let element = self.decorations().get(hit)?;
        let mut text: Option<RectF> = None;
        element.emit(
            chart,
            &CpuTextMeasure::default(),
            &mut Parts(&mut |part| {
                if let DrawnPart::Text(r) = part {
                    text = Some(text.map_or(r, |u| ::renderer::sink::union(u, r)));
                }
            }),
        );
        text.or_else(|| self.decoration_bounds(chart, hit))
    }
    fn selection(&self, chart: &RadialChart, hit: HitId) -> Option<SelectionBox> {
        self.decoration_selection_box(chart, hit)
    }
    fn followers(&self, _: &RadialChart, _: HitId) -> Vec<DecorationId> {
        Vec::new()
    }
    fn drag(&self, chart: &mut RadialChart, hit: HitId, dx: f32, dy: f32) -> Option<NudgeResult> {
        self.drag_decoration(chart, hit, dx, dy)
    }
    fn resize(
        &self,
        chart: &mut RadialChart,
        hit: HitId,
        handle: ResizeHandle,
        dx: f32,
        dy: f32,
    ) -> Option<NudgeResult> {
        self.resize_decoration(chart, hit, handle, dx, dy)
    }
    fn handle_at(&self, chart: &RadialChart, hit: HitId, p: [f32; 2]) -> Option<ResizeHandle> {
        self.decoration_resize_handle(chart, hit, p)
    }
    fn contains(&self, chart: &RadialChart, hit: HitId, p: [f32; 2]) -> bool {
        self.decorations()
            .get(hit)
            .is_some_and(|e| e.contains(chart, &CpuTextMeasure::default(), p[0], p[1]))
    }
    fn route(
        &self,
        annotations: &BoundedAnnotationFrame,
        layer: LayerRef,
        chart: &RadialChart,
        p: [f32; 2],
    ) -> Option<LayerHit> {
        annotations.route_active_layer(layer, p, || self.decoration_at(chart, p))
    }
}
