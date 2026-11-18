//! Which series draw which 3D and ternary pipelines, and those pipelines' kinds and builders.

use super::*;

/// Whether a series draws a surface on a 3D chart: a heatmap kind.
pub(in crate::renderer) fn draws_surface(rt: &crate::data_config::DataRenderType) -> bool {
    matches!(
        rt,
        crate::data_config::DataRenderType::Field(crate::data_config::FieldSeries {
            fill: Some(_),
            ..
        })
    )
}

/// Whether a series draws contours on a 3D chart's floor.
pub(in crate::renderer) fn draws_floor(rt: &crate::data_config::DataRenderType) -> bool {
    matches!(
        rt,
        crate::data_config::DataRenderType::Field(crate::data_config::FieldSeries {
            contour: Some(_),
            ..
        })
    )
}

/// Whether a series draws as a mesh surface on a 3D chart (#210): a
/// triangle mesh with a fill.
pub(in crate::renderer) fn draws_mesh_surface(rt: &crate::data_config::DataRenderType) -> bool {
    matches!(
        rt,
        crate::data_config::DataRenderType::TriMesh(crate::tri_mesh::TriMeshSeries {
            fill: Some(_),
            ..
        })
    )
}

/// Whether a series draws contours on a 3D chart's floor through the mesh
/// floor entries (#210): a triangle mesh with contours.
pub(in crate::renderer) fn draws_mesh_floor(rt: &crate::data_config::DataRenderType) -> bool {
    matches!(
        rt,
        crate::data_config::DataRenderType::TriMesh(crate::tri_mesh::TriMeshSeries {
            contour: Some(_),
            ..
        })
    )
}

/// The ternary pipelines (`TargetPipelines::ternary`).
pub(in crate::renderer) const TERNARY_PIPELINES: usize = 5;

/// One of a ternary chart's pipelines (`TargetPipelines::ternary`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::renderer) enum TernaryPipeline {
    Line = 0,
    Scatter = 1,
    ScatterMapped = 2,
    Ring = 3,
    RingMapped = 4,
}

// `TargetPipelines::ternary` is indexed by `kind as usize`: the last
// discriminant is the last slot (#152).
const _: () = assert!(TernaryPipeline::RingMapped as usize + 1 == TERNARY_PIPELINES);

impl TernaryPipeline {
    pub(in crate::renderer) const ALL: [Self; TERNARY_PIPELINES] = [
        Self::Line,
        Self::Scatter,
        Self::ScatterMapped,
        Self::Ring,
        Self::RingMapped,
    ];

    pub(super) fn stage(self) -> &'static str {
        match self {
            Self::Line => "ternary line",
            Self::Scatter => "ternary scatter",
            Self::ScatterMapped => "mapped ternary scatter",
            Self::Ring => "ternary picked point ring",
            Self::RingMapped => "mapped ternary picked point ring",
        }
    }

    /// The label of the pipeline [`create_ternary_pipeline`] builds, which
    /// the browser's background compile is keyed by (#299).
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Line => "figgy line ternary pipeline",
            Self::Scatter => "figgy scatter ternary pipeline",
            Self::ScatterMapped => "figgy scatter ternary mapped pipeline",
            Self::Ring => "figgy picked point ternary ring pipeline",
            Self::RingMapped => "figgy picked point ternary mapped ring pipeline",
        }
    }
}

/// The ternary pipeline `kind` over the layouts `[transform, style, style
/// map, value map]` (the plain ones read the first two).
pub(super) fn create_ternary_pipeline(
    kind: TernaryPipeline,
    device: &wgpu::Device,
    shaders: &data_render::ShaderModules,
    [transform_bgl, style_bgl, style_map_bgl, value_map_bgl]: [&wgpu::BindGroupLayout; 4],
    surface_format: wgpu::TextureFormat,
    sample_count: u32,
) -> wgpu::RenderPipeline {
    let label = kind.label();
    match kind {
        TernaryPipeline::Line => data_render::create_line_pipeline(
            device,
            &shaders.line,
            &[Some(transform_bgl), Some(style_bgl)],
            data_render::LINE_MAPPED_SLOTS,
            surface_format,
            sample_count,
            ("vs_ternary", "fs_main", label),
            wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING,
            wgpu::PrimitiveTopology::TriangleStrip,
            None,
        ),
        TernaryPipeline::Scatter | TernaryPipeline::Ring => {
            let (vs, fs) = if kind == TernaryPipeline::Scatter {
                ("vs_ternary", "fs_main")
            } else {
                ("vs_pick_ring_ternary", "fs_pick_ring")
            };
            data_render::create_scatter_pipeline(
                device,
                &shaders.scatter,
                [transform_bgl, style_bgl],
                surface_format,
                sample_count,
                (vs, fs, label),
                None,
                true,
                None,
            )
        }
        TernaryPipeline::ScatterMapped | TernaryPipeline::RingMapped => {
            let (vs, fs) = if kind == TernaryPipeline::ScatterMapped {
                ("vs_ternary_mapped", "fs_mapped")
            } else {
                ("vs_pick_ring_ternary_mapped", "fs_pick_ring")
            };
            data_render::create_scatter_mapped_pipeline(
                device,
                &shaders.scatter,
                [transform_bgl, style_bgl, style_map_bgl, value_map_bgl],
                surface_format,
                sample_count,
                (vs, fs, label),
                true,
                None,
            )
        }
    }
}

/// The 3D point pipelines (`TargetPipelines::view3d`, #132).
pub(in crate::renderer) const VIEW3D_PIPELINES: usize = 10;

/// One of a 3D chart's scatter and line pipelines (`TargetPipelines::view3d`,
/// #132): the markers (`vs_3d`, `vs_3d_mapped`) and the stroke (`vs_3d`)
/// in the panel composite with the depth test, their 1×1 ID picks
/// (`vs_3d_pick`, `vs_3d_mapped_pick`, the line's `vs_3d_pick`) into the
/// integer pick target, and the picked ring in the live slot
/// (`vs_pick_ring_3d`, `vs_pick_ring_3d_mapped`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::renderer) enum View3dPipeline {
    Scatter = 0,
    ScatterMapped = 1,
    ScatterPick = 2,
    ScatterMappedPick = 3,
    Ring = 4,
    RingMapped = 5,
    Line = 6,
    LinePick = 7,
    /// The arrows of a 3D vector series (#211).
    Vector = 8,
    /// Their ID pick.
    VectorPick = 9,
}

// `TargetPipelines::view3d` is indexed by `kind as usize`: the last
// discriminant is the last slot (#152).
const _: () = assert!(View3dPipeline::VectorPick as usize + 1 == VIEW3D_PIPELINES);

impl View3dPipeline {
    pub(in crate::renderer) const ALL: [Self; VIEW3D_PIPELINES] = [
        Self::Scatter,
        Self::ScatterMapped,
        Self::ScatterPick,
        Self::ScatterMappedPick,
        Self::Ring,
        Self::RingMapped,
        Self::Line,
        Self::LinePick,
        Self::Vector,
        Self::VectorPick,
    ];

    pub(super) fn stage(self) -> &'static str {
        match self {
            Self::Scatter => "3D scatter",
            Self::ScatterMapped => "mapped 3D scatter",
            Self::ScatterPick => "3D scatter pick",
            Self::ScatterMappedPick => "mapped 3D scatter pick",
            Self::Ring => "3D picked point ring",
            Self::RingMapped => "mapped 3D picked point ring",
            Self::Line => "3D line",
            Self::LinePick => "3D line pick",
            Self::Vector => "3D vector",
            Self::VectorPick => "3D vector pick",
        }
    }

    /// The label of the pipeline this kind builds, which the browser's
    /// background compile is keyed by (#299).
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Scatter => "figgy scatter 3D pipeline",
            Self::ScatterMapped => "figgy scatter 3D mapped pipeline",
            Self::ScatterPick => "figgy scatter 3D pick pipeline",
            Self::ScatterMappedPick => "figgy scatter 3D mapped pick pipeline",
            Self::Ring => "figgy picked point 3D ring pipeline",
            Self::RingMapped => "figgy picked point 3D mapped ring pipeline",
            Self::Line => "figgy line 3D pipeline",
            Self::LinePick => "figgy line 3D pick pipeline",
            Self::Vector => "figgy vector 3D pipeline",
            Self::VectorPick => "figgy vector 3D pick pipeline",
        }
    }

    fn pass(self) -> data_render::View3dPass {
        match self {
            Self::Scatter | Self::ScatterMapped | Self::Line | Self::Vector => {
                data_render::View3dPass::Draw
            }
            Self::ScatterPick | Self::ScatterMappedPick | Self::LinePick | Self::VectorPick => {
                data_render::View3dPass::Pick
            }
            Self::Ring | Self::RingMapped => data_render::View3dPass::Live,
        }
    }

    fn mapped(self) -> bool {
        matches!(
            self,
            Self::ScatterMapped | Self::ScatterMappedPick | Self::RingMapped
        )
    }
}

/// The 3D point pipeline `kind` over the layouts `[transform, style, style
/// map, camera, value map and camera, vector data]`: the precise entries
/// bind the camera group (the surface layout) as group 3, the mapped ones
/// the combined value map and camera group, the arrows (#211) their vector
/// data as group 2 and the camera as group 3. The pick kinds render one sample
/// into the ID target; the others `sample_count` into `surface_format`.
pub(super) fn create_view3d_pipeline(
    kind: View3dPipeline,
    device: &wgpu::Device,
    shaders: &data_render::ShaderModules,
    [
        transform_bgl,
        style_bgl,
        style_map_bgl,
        surface_bgl,
        view3d_value_map_bgl,
        vector_bgl,
    ]: [&wgpu::BindGroupLayout; 6],
    surface_format: wgpu::TextureFormat,
    sample_count: u32,
) -> wgpu::RenderPipeline {
    let pass = kind.pass();
    let sample_count = if pass == data_render::View3dPass::Pick {
        1
    } else {
        sample_count
    };
    match kind {
        View3dPipeline::Vector | View3dPipeline::VectorPick => {
            data_render::vector::create_vector_view3d_pipeline(
                device,
                &shaders.vector_points_3d,
                transform_bgl,
                style_bgl,
                vector_bgl,
                surface_bgl,
                surface_format,
                sample_count,
                pass,
            )
        }
        View3dPipeline::Line | View3dPipeline::LinePick => {
            let (vs, fs) = match pass {
                data_render::View3dPass::Pick => ("vs_3d_pick", "fs_3d_pick"),
                _ => ("vs_3d", "fs_main"),
            };
            data_render::create_line_pipeline(
                device,
                &shaders.line,
                &[
                    Some(transform_bgl),
                    Some(style_bgl),
                    None,
                    Some(surface_bgl),
                ],
                data_render::LINE_MAPPED_SLOTS,
                surface_format,
                sample_count,
                (vs, fs, kind.label()),
                wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING,
                wgpu::PrimitiveTopology::TriangleStrip,
                Some(pass),
            )
        }
        _ if !kind.mapped() => {
            let (vs, fs) = match kind {
                View3dPipeline::ScatterPick => ("vs_3d_pick", "fs_3d_pick"),
                View3dPipeline::Ring => ("vs_pick_ring_3d", "fs_pick_ring"),
                _ => ("vs_3d", "fs_main"),
            };
            data_render::create_scatter_pipeline(
                device,
                &shaders.scatter,
                [transform_bgl, style_bgl],
                surface_format,
                sample_count,
                (vs, fs, kind.label()),
                None,
                true,
                Some((surface_bgl, pass)),
            )
        }
        _ => {
            let (vs, fs) = match kind {
                View3dPipeline::ScatterMappedPick => ("vs_3d_mapped_pick", "fs_3d_mapped_pick"),
                View3dPipeline::RingMapped => ("vs_pick_ring_3d_mapped", "fs_pick_ring"),
                _ => ("vs_3d_mapped", "fs_mapped"),
            };
            data_render::create_scatter_mapped_pipeline(
                device,
                &shaders.scatter,
                [
                    transform_bgl,
                    style_bgl,
                    style_map_bgl,
                    view3d_value_map_bgl,
                ],
                surface_format,
                sample_count,
                (vs, fs, kind.label()),
                true,
                Some(pass),
            )
        }
    }
}

/// The precise pipelines (`TargetPipelines::precise`, #341).
pub(in crate::renderer) const PRECISE_PIPELINES: usize = 37;

/// One of a target's precise (unstyled) pipelines (`TargetPipelines::precise`,
/// #341): the one table that frame preparation, the prewarm and the format
/// rebuild create them from ([`create_precise_pipeline`]), in the order the
/// prewarm creates them. The ternary and 3D entries, the triangle mesh
/// passes and the histogram envelope keep their own kinds and slots.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::renderer) enum PrecisePipeline {
    Line = 0,
    /// The colour-mapped stroke of a streamline series with a colour column.
    LineMapped = 1,
    /// The fill under a line series, in every draw style.
    Area = 2,
    /// The rows of a parallel coordinates series (#141).
    LineParallel = 3,
    /// The picked row's redraw.
    LineParallelSelection = 4,
    Scatter = 5,
    Errorbar = 6,
    /// The precise stem draw (#206).
    ErrorbarStem = 7,
    ScatterMapped = 8,
    PickRing = 9,
    PickRingMapped = 10,
    ErrorbarMapped = 11,
    Bar = 12,
    BarMapped = 13,
    BarSelection = 14,
    /// The selected bin's outline over the mapped bar layout.
    BarSelectionMapped = 15,
    CategoryBar = 16,
    CategoryBarSelection = 17,
    BoxPlot = 18,
    BoxPlotSelection = 19,
    RangeBar = 20,
    RangeBarSelection = 21,
    Violin = 22,
    ViolinSelection = 23,
    BoxPoints = 24,
    BoxPointSelection = 25,
    /// The value labels of category bar, box plot and radar series.
    ValueLabel = 26,
    ValueLabelSelection = 27,
    Field = 28,
    VectorPoints = 29,
    /// The streamline arrowheads, drawn indirect over the arc slots.
    VectorPath = 30,
    VectorGrid = 31,
    /// The field quad through `fs_contour`.
    Contour = 32,
    /// The contour pass with group 3 bound to the selected label anchors.
    ContourLabelled = 33,
    FieldSelection = 34,
    /// The selected contour level's highlight for a labelled contour.
    FieldSelectionLabelled = 35,
    /// The contour label draw, from the renderer's label engine.
    ContourLabel = 36,
}

// `TargetPipelines::precise` is indexed by `kind as usize`: the last
// discriminant is the last slot (#152).
const _: () = assert!(PrecisePipeline::ContourLabel as usize + 1 == PRECISE_PIPELINES);

impl PrecisePipeline {
    /// Every kind, in the order the prewarm creates them.
    pub(in crate::renderer) const ALL: [Self; PRECISE_PIPELINES] = [
        Self::Line,
        Self::LineMapped,
        Self::Area,
        Self::LineParallel,
        Self::LineParallelSelection,
        Self::Scatter,
        Self::Errorbar,
        Self::ErrorbarStem,
        Self::ScatterMapped,
        Self::PickRing,
        Self::PickRingMapped,
        Self::ErrorbarMapped,
        Self::Bar,
        Self::BarMapped,
        Self::BarSelection,
        Self::BarSelectionMapped,
        Self::CategoryBar,
        Self::CategoryBarSelection,
        Self::BoxPlot,
        Self::BoxPlotSelection,
        Self::RangeBar,
        Self::RangeBarSelection,
        Self::Violin,
        Self::ViolinSelection,
        Self::BoxPoints,
        Self::BoxPointSelection,
        Self::ValueLabel,
        Self::ValueLabelSelection,
        Self::Field,
        Self::VectorPoints,
        Self::VectorPath,
        Self::VectorGrid,
        Self::Contour,
        Self::ContourLabelled,
        Self::FieldSelection,
        Self::FieldSelectionLabelled,
        Self::ContourLabel,
    ];

    /// The prewarm's progress stage for this kind (#299).
    pub(super) fn stage(self) -> &'static str {
        match self {
            Self::Line => "precise line",
            Self::LineMapped => "colour-mapped line",
            Self::Area => "line area fill",
            Self::LineParallel => "parallel rows",
            Self::LineParallelSelection => "selected parallel row",
            Self::Scatter => "precise scatter",
            Self::Errorbar => "precise errorbar",
            Self::ErrorbarStem => "precise stem",
            Self::ScatterMapped => "mapped scatter",
            Self::PickRing => "picked point ring",
            Self::PickRingMapped => "mapped picked point ring",
            Self::ErrorbarMapped => "mapped errorbar",
            Self::Bar => "histogram bars",
            Self::BarMapped => "mapped histogram bars",
            Self::BarSelection => "selected histogram bin",
            Self::BarSelectionMapped => "mapped selected histogram bin",
            Self::CategoryBar => "category bars",
            Self::CategoryBarSelection => "selected category bar",
            Self::BoxPlot => "box plots",
            Self::BoxPlotSelection => "selected box",
            Self::RangeBar => "range bars",
            Self::RangeBarSelection => "selected range bar",
            Self::Violin => "violins",
            Self::ViolinSelection => "selected violin",
            Self::BoxPoints => "box points",
            Self::BoxPointSelection => "selected box point",
            Self::ValueLabel => "value labels",
            Self::ValueLabelSelection => "selected value label",
            Self::Field => "heatmap field",
            Self::VectorPoints => "vector points",
            Self::VectorPath => "vector path",
            Self::VectorGrid => "vector grid",
            Self::Contour => "contour field",
            Self::ContourLabelled => "label-gapped contour field",
            Self::FieldSelection => "selected field data",
            Self::FieldSelectionLabelled => "selected labelled field data",
            Self::ContourLabel => "contour labels",
        }
    }

    /// The label of the pipeline [`create_precise_pipeline`] builds, which
    /// the browser's background compile is keyed by (#299).
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Line => "figgy line columnar pipeline",
            Self::LineMapped => "figgy line mapped pipeline",
            Self::Area => "figgy line area pipeline",
            Self::LineParallel => "figgy line parallel pipeline",
            Self::LineParallelSelection => "figgy selected parallel row pipeline",
            Self::Scatter => "figgy scatter columnar pipeline",
            Self::Errorbar => "figgy errorbar columnar pipeline",
            Self::ErrorbarStem => "figgy stem pipeline",
            Self::ScatterMapped => "figgy scatter mapped pipeline",
            Self::PickRing => "figgy picked point ring pipeline",
            Self::PickRingMapped => "figgy picked point mapped ring pipeline",
            Self::ErrorbarMapped => "figgy errorbar mapped pipeline",
            Self::Bar => "figgy bar columnar pipeline",
            Self::BarMapped => "figgy mapped bar columnar pipeline",
            Self::BarSelection => "figgy selected histogram bin pipeline",
            Self::BarSelectionMapped => "figgy selected mapped histogram bin pipeline",
            Self::CategoryBar => "figgy category bar columnar pipeline",
            Self::CategoryBarSelection => "figgy selected category bar columnar pipeline",
            Self::BoxPlot => "figgy box plot columnar pipeline",
            Self::BoxPlotSelection => "figgy selected box plot columnar pipeline",
            Self::RangeBar => "figgy range bar columnar pipeline",
            Self::RangeBarSelection => "figgy selected range bar columnar pipeline",
            Self::Violin => "figgy violin columnar pipeline",
            Self::ViolinSelection => "figgy selected violin columnar pipeline",
            Self::BoxPoints => "figgy box points columnar pipeline",
            Self::BoxPointSelection => "figgy selected box point columnar pipeline",
            Self::ValueLabel => "figgy value label pipeline",
            Self::ValueLabelSelection => "figgy value label selection pipeline",
            Self::Field => "figgy field columnar pipeline",
            Self::VectorPoints => "figgy vector points pipeline",
            Self::VectorPath => "figgy vector path pipeline",
            Self::VectorGrid => "figgy vector grid pipeline",
            Self::Contour => "figgy field contour pipeline",
            Self::ContourLabelled => "figgy labelled field contour pipeline",
            Self::FieldSelection => "figgy selected field data pipeline",
            Self::FieldSelectionLabelled => "figgy selected labelled field data pipeline",
            Self::ContourLabel => "figgy contour label pipeline",
        }
    }

    /// The key [`PreparationFeature::includes_field`] groups this kind by.
    pub(in crate::renderer) fn field(self) -> &'static str {
        match self {
            Self::Line => "line",
            Self::LineMapped => "line_mapped",
            Self::Area => "area",
            Self::LineParallel => "line_parallel",
            Self::LineParallelSelection => "line_parallel_selection",
            Self::Scatter => "scatter",
            Self::Errorbar => "errorbar",
            Self::ErrorbarStem => "errorbar_stem",
            Self::ScatterMapped => "scatter_mapped",
            Self::PickRing => "pick_ring",
            Self::PickRingMapped => "pick_ring_mapped",
            Self::ErrorbarMapped => "errorbar_mapped",
            Self::Bar => "bar",
            Self::BarMapped => "bar_mapped",
            Self::BarSelection => "bar_selection",
            Self::BarSelectionMapped => "bar_selection_mapped",
            Self::CategoryBar | Self::CategoryBarSelection => "bar_category",
            Self::BoxPlot | Self::BoxPlotSelection => "bar_box_plot",
            Self::RangeBar | Self::RangeBarSelection => "bar_range",
            Self::Violin | Self::ViolinSelection => "bar_violin",
            Self::BoxPoints | Self::BoxPointSelection => "bar_box_points",
            Self::ValueLabel | Self::ValueLabelSelection => "bar_value_label",
            Self::Field => "field",
            Self::VectorPoints => "vector_points",
            Self::VectorPath => "vector_path",
            Self::VectorGrid => "vector_grid",
            Self::Contour => "contour",
            Self::ContourLabelled => "contour_labelled",
            Self::FieldSelection => "field_selection",
            Self::FieldSelectionLabelled => "field_selection_labelled",
            Self::ContourLabel => "contour_label",
        }
    }

    /// Whether this kind is built over the renderer's contour label engine,
    /// and so is created only while the engine exists.
    pub(super) fn needs_label_engine(self) -> bool {
        matches!(
            self,
            Self::ContourLabelled | Self::FieldSelectionLabelled | Self::ContourLabel
        )
    }

    /// Whether a change of the target's format or sample count recreates
    /// this kind when it was live (`Renderer::ensure_target`); the others
    /// are compiled again by the next frame that needs them.
    pub(super) fn rebuilt_with_target(self) -> bool {
        matches!(
            self,
            Self::Line
                | Self::LineMapped
                | Self::Area
                | Self::LineParallel
                | Self::LineParallelSelection
                | Self::Scatter
                | Self::Errorbar
                | Self::ErrorbarStem
                | Self::ScatterMapped
                | Self::PickRing
                | Self::PickRingMapped
                | Self::ErrorbarMapped
                | Self::Bar
                | Self::BarMapped
        )
    }
}

/// The device, layouts and target format the precise pipelines build over
/// (#341): what [`create_precise_pipeline`], the ternary entries and the
/// triangle mesh passes read.
#[derive(Clone, Copy)]
pub(in crate::renderer) struct PreciseLayouts<'a> {
    pub(in crate::renderer) device: &'a wgpu::Device,
    pub(in crate::renderer) transform_bgl: &'a wgpu::BindGroupLayout,
    pub(in crate::renderer) style_bgl: &'a wgpu::BindGroupLayout,
    pub(in crate::renderer) per_point_style_map_bgl: &'a wgpu::BindGroupLayout,
    pub(in crate::renderer) point_value_map_bgl: &'a wgpu::BindGroupLayout,
    pub(in crate::renderer) data_selection_bgl: &'a wgpu::BindGroupLayout,
    pub(in crate::renderer) field_bgl: &'a wgpu::BindGroupLayout,
    pub(in crate::renderer) vector_bgl: &'a wgpu::BindGroupLayout,
    pub(in crate::renderer) category_bar_data_bgl: &'a wgpu::BindGroupLayout,
    pub(in crate::renderer) parallel_bgl: &'a wgpu::BindGroupLayout,
    pub(in crate::renderer) star_data_bgl: &'a wgpu::BindGroupLayout,
    /// The contour label engine, which the labelled kinds need.
    pub(in crate::renderer) label_pipelines: Option<&'a crate::gpu_contour::ContourLabelPipelines>,
    pub(in crate::renderer) surface_format: wgpu::TextureFormat,
}

impl PreciseLayouts<'_> {
    /// Whether [`create_precise_pipeline`] can build `kind` from these
    /// layouts: every kind but those that need a missing label engine.
    pub(super) fn can_create(&self, kind: PrecisePipeline) -> bool {
        !kind.needs_label_engine() || self.label_pipelines.is_some()
    }

    /// The four layouts of a ternary entry ([`create_ternary_pipeline`]).
    pub(super) fn ternary(&self) -> [&wgpu::BindGroupLayout; 4] {
        [
            self.transform_bgl,
            self.style_bgl,
            self.per_point_style_map_bgl,
            self.point_value_map_bgl,
        ]
    }

    /// The three layouts of a triangle mesh pass: the selection binds the
    /// data selection uniform as group 1 in place of the style.
    pub(super) fn tri_mesh(
        &self,
        pass: data_render::tri_mesh::TriMeshPass,
    ) -> [&wgpu::BindGroupLayout; 3] {
        let group1 = if pass == data_render::tri_mesh::TriMeshPass::Selection {
            self.data_selection_bgl
        } else {
            self.style_bgl
        };
        [self.transform_bgl, group1, self.field_bgl]
    }
}

/// The precise pipeline `kind` over `layouts`, from `shaders` at
/// `sample_count` (#341); `None` only for a kind that needs the contour
/// label engine when `layouts` has none ([`PreciseLayouts::can_create`]).
pub(super) fn create_precise_pipeline(
    kind: PrecisePipeline,
    layouts: &PreciseLayouts<'_>,
    shaders: &data_render::ShaderModules,
    sample_count: u32,
) -> Option<wgpu::RenderPipeline> {
    use PrecisePipeline as K;
    let PreciseLayouts {
        device,
        transform_bgl,
        style_bgl,
        per_point_style_map_bgl,
        point_value_map_bgl,
        data_selection_bgl,
        field_bgl,
        vector_bgl,
        category_bar_data_bgl,
        parallel_bgl,
        star_data_bgl,
        label_pipelines,
        surface_format,
    } = *layouts;
    let label = kind.label();
    let premultiplied = wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING;
    let category_axis = |entries: (&str, &str)| {
        data_render::create_category_axis_pipeline(
            device,
            &shaders.bar,
            [
                transform_bgl,
                style_bgl,
                per_point_style_map_bgl,
                category_bar_data_bgl,
            ],
            surface_format,
            sample_count,
            (entries.0, entries.1, label),
        )
    };
    let field = |group1: &wgpu::BindGroupLayout,
                 group3: Option<&wgpu::BindGroupLayout>,
                 fragment_entry: &str| {
        data_render::create_field_render_pipeline(
            device,
            &shaders.field,
            [transform_bgl, group1, field_bgl],
            group3,
            surface_format,
            sample_count,
            "vs_main",
            fragment_entry,
            label,
        )
    };
    let bar = |group2: Option<&wgpu::BindGroupLayout>, vertex_entry: &str| {
        data_render::create_bar_pipeline(
            device,
            &shaders.bar,
            transform_bgl,
            style_bgl,
            group2,
            surface_format,
            sample_count,
            (vertex_entry, label),
        )
    };
    let errorbar = |entries: (&str, &str)| {
        data_render::create_errorbar_pipeline(
            device,
            &shaders.errorbar,
            [transform_bgl, style_bgl],
            surface_format,
            sample_count,
            (entries.0, entries.1, label),
            premultiplied,
        )
    };
    Some(match kind {
        K::Line => data_render::create_line_pipeline(
            device,
            &shaders.line,
            &[Some(transform_bgl), Some(style_bgl)],
            data_render::LINE_SLOTS,
            surface_format,
            sample_count,
            ("vs_main", "fs_main", label),
            premultiplied,
            wgpu::PrimitiveTopology::TriangleStrip,
            None,
        ),
        K::LineMapped => data_render::create_line_pipeline(
            device,
            &shaders.line,
            &[
                Some(transform_bgl),
                Some(style_bgl),
                Some(point_value_map_bgl),
                Some(point_value_map_bgl),
            ],
            data_render::LINE_MAPPED_SLOTS,
            surface_format,
            sample_count,
            ("vs_main_mapped", "fs_main_mapped", label),
            premultiplied,
            wgpu::PrimitiveTopology::TriangleStrip,
            None,
        ),
        K::Area => data_render::create_line_area_pipeline_with_sample_count(
            device,
            &shaders.line,
            transform_bgl,
            style_bgl,
            surface_format,
            sample_count,
        ),
        K::LineParallel | K::LineParallelSelection => {
            let vs = if kind == K::LineParallel {
                "vs_parallel"
            } else {
                "vs_parallel_selected"
            };
            data_render::create_line_pipeline(
                device,
                &shaders.line,
                &[
                    Some(transform_bgl),
                    Some(style_bgl),
                    None,
                    Some(parallel_bgl),
                ],
                0,
                surface_format,
                sample_count,
                (vs, "fs_parallel", label),
                premultiplied,
                wgpu::PrimitiveTopology::TriangleStrip,
                None,
            )
        }
        K::Scatter | K::PickRing => {
            let entries = if kind == K::Scatter {
                ("vs_main", "fs_main")
            } else {
                ("vs_pick_ring", "fs_pick_ring")
            };
            data_render::create_scatter_pipeline(
                device,
                &shaders.scatter,
                [transform_bgl, style_bgl],
                surface_format,
                sample_count,
                (entries.0, entries.1, label),
                None,
                false,
                None,
            )
        }
        K::ScatterMapped | K::PickRingMapped => {
            let (vs, fs) = if kind == K::ScatterMapped {
                ("vs_mapped", "fs_mapped")
            } else {
                ("vs_pick_ring_mapped", "fs_pick_ring")
            };
            data_render::create_scatter_mapped_pipeline(
                device,
                &shaders.scatter,
                [
                    transform_bgl,
                    style_bgl,
                    per_point_style_map_bgl,
                    point_value_map_bgl,
                ],
                surface_format,
                sample_count,
                (vs, fs, label),
                false,
                None,
            )
        }
        K::Errorbar => errorbar(("vs_main", "fs_main")),
        K::ErrorbarStem => errorbar(("vs_stem", "fs_stem")),
        K::ErrorbarMapped => data_render::create_errorbar_mapped_pipeline(
            device,
            &shaders.errorbar,
            transform_bgl,
            style_bgl,
            per_point_style_map_bgl,
            surface_format,
            sample_count,
        ),
        K::Bar => bar(None, "vs_envelope_bars"),
        K::BarMapped => bar(Some(per_point_style_map_bgl), "vs_envelope_mapped_bars"),
        K::BarSelection => bar(None, "vs_bar_selection"),
        K::BarSelectionMapped => bar(Some(per_point_style_map_bgl), "vs_bar_selection_mapped"),
        K::CategoryBar => category_axis(("vs_category_bars", "fs_category_bars")),
        K::CategoryBarSelection => {
            category_axis(("vs_category_bar_selection", "fs_category_bar_selection"))
        }
        K::BoxPlot => category_axis(("vs_box_plot", "fs_box_plot")),
        K::BoxPlotSelection => category_axis(("vs_box_plot_selection", "fs_box_plot_selection")),
        K::RangeBar => category_axis(("vs_range_bars", "fs_range_bars")),
        K::RangeBarSelection => category_axis(("vs_range_bar_selection", "fs_range_bar_selection")),
        K::Violin => category_axis(("vs_violin", "fs_violin")),
        K::ViolinSelection => category_axis(("vs_violin_selection", "fs_violin_selection")),
        K::BoxPoints => category_axis(("vs_box_points", "fs_box_points")),
        K::BoxPointSelection => category_axis(("vs_box_point_selection", "fs_box_point_selection")),
        K::ValueLabel | K::ValueLabelSelection => {
            // Group 1 is the series' value label group: an equal layout to
            // the renderer's own (wgpu compares layouts by their entries),
            // so its bind groups fit.
            let value_label_bgl = data_render::create_value_label_bind_group_layout(device);
            let (vs, fs) = if kind == K::ValueLabel {
                ("vs_value_label", "fs_value_label")
            } else {
                ("vs_value_label_selection", "fs_value_label_selection")
            };
            data_render::create_category_axis_pipeline(
                device,
                &shaders.bar,
                [
                    transform_bgl,
                    &value_label_bgl,
                    per_point_style_map_bgl,
                    category_bar_data_bgl,
                ],
                surface_format,
                sample_count,
                (vs, fs, label),
            )
        }
        K::Field => field(style_bgl, None, "fs_main"),
        K::Contour => field(style_bgl, None, "fs_contour"),
        K::ContourLabelled => field(
            style_bgl,
            Some(label_pipelines?.label_gap_bgl()),
            "fs_contour_labelled",
        ),
        K::FieldSelection => field(data_selection_bgl, None, "fs_data_selection"),
        K::FieldSelectionLabelled => field(
            data_selection_bgl,
            Some(label_pipelines?.label_gap_bgl()),
            "fs_data_selection_labelled",
        ),
        K::VectorPoints | K::VectorPath | K::VectorGrid => {
            let input = match kind {
                K::VectorPoints => data_render::vector::VectorInput::Points,
                K::VectorPath => data_render::vector::VectorInput::Path,
                _ => data_render::vector::VectorInput::Grid,
            };
            data_render::vector::create_vector_pipeline_with_sample_count(
                device,
                shaders.vector(input),
                transform_bgl,
                style_bgl,
                vector_bgl,
                star_data_bgl,
                surface_format,
                sample_count,
                input,
            )
        }
        K::ContourLabel => {
            label_pipelines?.render_pipeline(device, transform_bgl, surface_format, sample_count)
        }
    })
}
