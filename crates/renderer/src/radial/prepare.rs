//! Preparing frames and the annotation layer, with the frame cache keys.

use super::*;

impl RadialRenderer {
    /// The annotation layer of `chart` for the frame [`Self::prepare`]
    /// returns for the same arguments: its texture and its hit map, cached
    /// like the frame. Draw it over the frame (the frame, then
    /// `draw_overlay`). The radial chart has no value axes, so it draws no
    /// span: every item is drawn above the slices.
    pub fn prepare_annotations(
        &mut self,
        chart: &RadialChart,
        size: (u32, u32),
        scale: f32,
    ) -> Result<Arc<crate::annotation::BoundedAnnotationFrame>, RadialError> {
        let frame = self.prepare(chart, size, scale)?;
        self.prepare_annotations_for(&frame, chart, scale)
    }
    pub(super) fn prepare_annotations_for(
        &mut self,
        frame: &RadialFrame,
        chart: &RadialChart,
        scale: f32,
    ) -> Result<Arc<crate::annotation::BoundedAnnotationFrame>, RadialError> {
        let selected = crate::annotation::selected_index(chart.decorations.selected.as_ref());
        let space = frame.annotation_space();
        if self.annotation_layer.needs_raster(
            &chart.annotations,
            space,
            frame.size,
            scale,
            selected,
        ) {
            self.admit(
                self.annotation_layer
                    .bytes_needed(&chart.annotations, frame.size),
            )?;
        }
        self.annotation_layer
            .prepare(
                &self.gpu,
                self.gpu.ledger(),
                &chart.annotations,
                space,
                frame.size,
                scale,
                selected,
            )
            .map_err(|error| RadialError::Gpu(error.to_string()))
    }
}

impl RadialRenderer {
    /// `size` is logical pixels. `scale` (at least 0.5) increases actual geometry AND
    /// glyph resolution without changing the layout. Invalid edits do not evict
    /// the last valid snapshot. All chart/model fields participate in reuse.
    /// Each logical dimension must be at least 160; physical output must fit the
    /// device texture limit. Model edits need no manual cache invalidation.
    pub fn prepare(
        &mut self,
        chart: &RadialChart,
        size: (u32, u32),
        scale: f32,
    ) -> Result<Arc<RadialFrame>, RadialError> {
        // A lost device refuses here, before any buffer is created: wgpu's
        // `create_buffer_init` panics on a destroyed device.
        self.gpu.health().check().map_err(radial_gpu_error)?;
        let (width, height) = self.frame_size(chart, size, scale)?;
        let fonts = crate::text_render::font_generation();
        if let Some((_, _, _, _, frame)) =
            self.cached.as_ref().filter(|(c, s, k, generation, _)| {
                c.matches(chart) && *s == size && *k == scale && *generation == fonts
            })
        {
            return Ok(Arc::clone(frame));
        }
        let count = chart.node_count();
        let key = annotation_key(chart);
        let reused = self
            .annotations_cached
            .as_ref()
            .filter(|(c, s, k, f, _)| c == &key && *s == size && *k == scale && *f == fonts)
            .map(|(_, _, _, _, t)| Arc::clone(t));
        self.admit(
            if reused.is_some() {
                0
            } else {
                u64::from(width) * u64::from(height) * 4
            } + 112
                + count as u64 * std::mem::size_of::<Sector>() as u64,
        )?;
        let labels = if reused.is_none() {
            layout::Labels::Raster
        } else {
            layout::Labels::None
        };
        let l = layout::layout(chart, size, scale, labels)?;
        let device = self.gpu.device();
        let annotations = if let Some(texture) = reused {
            texture
        } else {
            // gpu-alloc: PanelTexture
            let annotations = crate::renderer::create_texture_checked(
                device,
                &wgpu::TextureDescriptor {
                    label: Some("radial labels at output resolution"),
                    size: wgpu::Extent3d {
                        width,
                        height,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                },
                "radial label texture",
            )
            .map_err(radial_gpu_error)?;
            let annotations = TrackedTexture::new(
                self.gpu.ledger(),
                GpuResourceKind::PanelTexture,
                annotations,
            );
            self.gpu.queue().write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &annotations,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &l.annotations,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(width * 4),
                    rows_per_image: Some(height),
                },
                wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
            );
            Arc::new(annotations)
        };
        let globals = frame_globals(chart, &l, (width, height), self.format);
        let (uniform, sectors, bind_group) = self.bind_frame(&globals, &l, &annotations)?;
        let frame = Arc::new(RadialFrame {
            decorations: l.decorations,
            gpu: self.gpu.clone(),
            size: (width, height),
            pipeline: self.pipeline.clone(),
            bind_group,
            pick_pipeline: self.pick_pipeline.clone(),
            pick_layout: self.pick_layout.clone(),
            _uniform: uniform,
            globals,
            sectors,
            _annotations: Arc::clone(&annotations),
            #[cfg(feature = "cpu-twin")]
            geometry: l.sectors,
            targets: l.targets,
            scale,
            space: l.space,
        });
        self.annotations_cached = Some((key, size, scale, fonts, annotations));
        self.cached = Some((FrameKey::of(chart), size, scale, fonts, Arc::clone(&frame)));
        Ok(frame)
    }
}

// Only properties read by annotation/layout generation survive. Appearance and
// ordinary data selection do not invalidate the rasterized label texture.
// Edited/selected inside labels are rasterized at their current lifted position.
pub(super) fn raster_moves_inside_labels(chart: &RadialChart) -> bool {
    let has_inside = (0..chart.group_count())
        .flat_map(|group| {
            (0..chart.group_len(group)).map(move |index| RadialTarget { group, index })
        })
        .any(|t| {
            chart.slice(t).is_some_and(|s| {
                s.value > 0.0 && layout::placement(chart, t, s) == RadialLabels::Inside
            })
        });
    has_inside
        && (matches!(
            chart.decorations.selected,
            Some(model::DecorationId::RadialLabel(_))
        ) || chart
            .decorations
            .offsets
            .iter()
            .any(|o| matches!(o.element, model::DecorationId::RadialLabel(_))))
}

/// What a radial frame's pixels, sectors and hit map are built from: the
/// chart's fields that reach them, named one by one (#66). [`Self::of`] and
/// [`Self::matches`] destructure `RadialChart` without a rest pattern, so a
/// new chart field does not compile until it is classified there: taken
/// into the key, or named as not reaching the frame, with the reason. The
/// per-field mutation test (`frame_key_tests`) holds each classification.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct FrameKey {
    data_area: Option<crate::layout::RectF>,
    offsets: Vec<model::DecorationOffset>,
    /// The decoration selection the frame highlights: an annotation's is
    /// drawn on the annotation overlay instead.
    selected: Option<model::DecorationId>,
    chart_title: Option<model::config::ChartTitleOptions>,
    slices: Vec<RadialSlice>,
    kind: RadialKind,
    start_angle_degrees: f32,
    split: Option<RadialSplit>,
    style: RadialStyle,
    labels: RadialLabels,
    interaction: RadialInteraction,
    font_family: String,
    font_size: f32,
    label_color: crate::Color,
    background: crate::Color,
}
/// A decoration selection as the frame sees it.
fn frame_selection(selected: &Option<model::DecorationId>) -> Option<&model::DecorationId> {
    selected
        .as_ref()
        .filter(|s| crate::annotation::selected_index(Some(s)).is_none())
}
impl FrameKey {
    fn of(chart: &RadialChart) -> Self {
        let RadialChart {
            decorations,
            chart_title,
            // Routes pointer interaction only; no pixel depends on it.
            active_layer: _,
            // Drawn by its own pass (`RadialAnnotationCache`).
            annotations: _,
            slices,
            kind,
            start_angle_degrees,
            split,
            style,
            labels,
            // The frame draws each slice's `text`; these only write it, at
            // edit time (`RadialChart::format_labels`).
            label_format: _,
            label_decimals: _,
            value_suffix: _,
            interaction,
            font_family,
            font_size,
            label_color,
            background,
        } = chart;
        let model::DecorationLayout {
            data_area,
            offsets,
            selected,
        } = decorations;
        Self {
            data_area: *data_area,
            offsets: offsets.clone(),
            selected: frame_selection(selected).cloned(),
            chart_title: chart_title.clone(),
            slices: slices.clone(),
            kind: *kind,
            start_angle_degrees: *start_angle_degrees,
            split: split.clone(),
            style: style.clone(),
            labels: *labels,
            interaction: interaction.clone(),
            font_family: font_family.clone(),
            font_size: *font_size,
            label_color: *label_color,
            background: *background,
        }
    }
    /// `*self == Self::of(chart)` without building the key: the per-frame
    /// cache check.
    fn matches(&self, chart: &RadialChart) -> bool {
        let RadialChart {
            decorations,
            chart_title,
            active_layer: _,
            annotations: _,
            slices,
            kind,
            start_angle_degrees,
            split,
            style,
            labels,
            label_format: _,
            label_decimals: _,
            value_suffix: _,
            interaction,
            font_family,
            font_size,
            label_color,
            background,
        } = chart;
        let model::DecorationLayout {
            data_area,
            offsets,
            selected,
        } = decorations;
        self.data_area == *data_area
            && self.offsets == *offsets
            && self.selected.as_ref() == frame_selection(selected)
            && self.chart_title == *chart_title
            && self.slices == *slices
            && self.kind == *kind
            && self.start_angle_degrees == *start_angle_degrees
            && self.split == *split
            && self.style == *style
            && self.labels == *labels
            && self.interaction == *interaction
            && self.font_family == *font_family
            && self.font_size == *font_size
            && self.label_color == *label_color
            && self.background == *background
    }
}
/// The key of a frame's label raster: its [`FrameKey`] with what only the
/// GPU draws put to fixed values, so appearance edits and ordinary hover
/// reuse the raster.
fn annotation_key(chart: &RadialChart) -> FrameKey {
    let mut c = FrameKey::of(chart);
    c.style = c.style.label_raster_key();
    c.interaction = c
        .interaction
        .label_raster_key(raster_moves_inside_labels(chart));
    c.background = crate::Color::WHITE;
    for slice in c
        .slices
        .iter_mut()
        .chain(c.split.iter_mut().flat_map(|s| s.children.iter_mut()))
    {
        slice.style = slice
            .style
            .as_ref()
            .map(RadialStyle::label_raster_key)
            .filter(|s| s != &c.style);
        if slice.labels.unwrap_or(c.labels) != RadialLabels::Inside || slice.label_color.is_some() {
            slice.color = crate::Color::WHITE;
        }
    }
    c
}

#[cfg(test)]
mod frame_key_tests {
    use super::*;
    use model::{DecorationId, DecorationOffset, LayerRef};
    fn base() -> RadialChart {
        let mut chart = RadialChart {
            slices: vec![
                RadialSlice::new("a", 1., crate::Color::WHITE),
                RadialSlice::new("b", 2., crate::Color::BLACK),
            ],
            ..Default::default()
        };
        chart.format_labels();
        chart
    }
    /// Every field of `RadialChart`, mutated once: the frame key moves
    /// exactly for the fields it names, and `matches` agrees with `of`.
    #[test]
    fn every_chart_field_is_in_the_frame_key_or_named_out_of_it() {
        // Exhaustive, like `FrameKey::of`: a new field must get a row here.
        let RadialChart {
            decorations: _,
            chart_title: _,
            active_layer: _,
            annotations: _,
            slices: _,
            kind: _,
            start_angle_degrees: _,
            split: _,
            style: _,
            labels: _,
            label_format: _,
            label_decimals: _,
            value_suffix: _,
            interaction: _,
            font_family: _,
            font_size: _,
            label_color: _,
            background: _,
        } = base();
        type Edit = fn(&mut RadialChart);
        let reaching: &[(&str, Edit)] = &[
            ("decorations.data_area", |c| {
                c.decorations.data_area = Some(crate::layout::RectF {
                    x: 30.,
                    y: 80.,
                    width: 200.,
                    height: 150.,
                })
            }),
            ("decorations.offsets", |c| {
                c.decorations.offsets.push(DecorationOffset {
                    element: DecorationId::ChartTitle,
                    offset: [1., 2.],
                })
            }),
            ("decorations.selected", |c| {
                c.decorations.selected = Some(DecorationId::ChartTitle)
            }),
            ("chart_title", |c| c.set_title_text("T")),
            ("slices", |c| c.slices[0].explode = 0.1),
            ("slices.text", |c| c.slices[0].text = Some("x".into())),
            ("kind", |c| c.kind = RadialKind::Donut { inner_radius: 0.5 }),
            ("start_angle_degrees", |c| c.start_angle_degrees = 0.),
            ("split", |c| {
                c.split = Some(RadialSplit {
                    slice_index: 1,
                    kind: RadialKind::Pie,
                    children: vec![RadialSlice::new("c", 2., crate::Color::WHITE)],
                })
            }),
            ("style", |c| c.style.depth = 0.2),
            ("labels", |c| c.labels = RadialLabels::Inside),
            ("interaction", |c| {
                c.interaction.selected = Some(RadialTarget::main(0))
            }),
            ("font_family", |c| c.font_family = "Comic Neue".into()),
            ("font_size", |c| c.font_size = 20.),
            ("label_color", |c| c.label_color = crate::Color::BLACK),
            ("background", |c| c.background = crate::Color::BLACK),
        ];
        let outside: &[(&str, Edit)] = &[
            ("active_layer", |c| c.active_layer = LayerRef::Annotations),
            ("annotations", |c| c.annotations.visible = false),
            ("decorations.selected (an annotation)", |c| {
                c.decorations.selected = Some(DecorationId::Annotation(0))
            }),
            ("label_format", |c| {
                c.label_format = RadialLabelFormat::Value
            }),
            ("label_decimals", |c| c.label_decimals = 3),
            ("value_suffix", |c| c.value_suffix = " kg".into()),
        ];
        // A slice is keyed whole, as the one field `slices`: its name and
        // its own label format ride with it although the frame reads them
        // only through the `text` `format_labels` writes.
        let key = FrameKey::of(&base());
        for (name, edit) in reaching {
            let mut c = base();
            edit(&mut c);
            assert_ne!(FrameKey::of(&c), key, "{name} must reach the frame key");
            assert!(!key.matches(&c), "{name}: matches disagrees with of");
            assert!(FrameKey::of(&c).matches(&c), "{name}");
        }
        for (name, edit) in outside {
            let mut c = base();
            edit(&mut c);
            assert_eq!(FrameKey::of(&c), key, "{name} must not reach the frame key");
            assert!(key.matches(&c), "{name}: matches disagrees with of");
        }
    }
    #[test]
    fn the_label_raster_key_ignores_what_only_the_gpu_draws() {
        let key = annotation_key(&base());
        for edit in [
            |c: &mut RadialChart| c.style.material = crate::Material::Wood,
            |c: &mut RadialChart| c.background = crate::Color::BLACK,
            |c: &mut RadialChart| {
                c.interaction.hovered = Some(RadialTarget::main(0));
                c.interaction.hover_progress = 0.5;
            },
            |c: &mut RadialChart| c.active_layer = LayerRef::Annotations,
        ] {
            let mut c = base();
            edit(&mut c);
            assert_eq!(annotation_key(&c), key);
        }
        let mut c = base();
        c.slices[0].text = Some("x".into());
        assert_ne!(annotation_key(&c), key);
    }
}

#[cfg(test)]
mod layer_cache_key_tests {
    use super::*;
    #[test]
    fn frame_and_annotation_keys_ignore_layer_fields_only() {
        let base = RadialChart {
            slices: vec![RadialSlice::new("a", 1., crate::Color::WHITE)],
            ..Default::default()
        };
        let mut layered = base.clone();
        layered.active_layer = model::LayerRef::Annotations;
        layered.annotations.visible = false;
        assert_eq!(FrameKey::of(&base), FrameKey::of(&layered));
        assert!(FrameKey::of(&base).matches(&layered));
        assert_eq!(annotation_key(&base), annotation_key(&layered));
        let mut edited = base.clone();
        edited.set_title_text("x");
        assert_ne!(FrameKey::of(&base), FrameKey::of(&edited));
        assert!(!FrameKey::of(&base).matches(&edited));
        assert_ne!(annotation_key(&base), annotation_key(&edited));
    }
}
