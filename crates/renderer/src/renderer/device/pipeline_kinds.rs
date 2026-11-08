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
    match kind {
        TernaryPipeline::Line => data_render::create_line_ternary_pipeline_with_sample_count(
            device,
            &shaders.line,
            transform_bgl,
            style_bgl,
            surface_format,
            sample_count,
        ),
        TernaryPipeline::Scatter | TernaryPipeline::Ring => {
            let (vs, fs, label) = if kind == TernaryPipeline::Scatter {
                ("vs_ternary", "fs_main", "figgy scatter ternary pipeline")
            } else {
                (
                    "vs_pick_ring_ternary",
                    "fs_pick_ring",
                    "figgy picked point ternary ring pipeline",
                )
            };
            data_render::create_scatter_ternary_pipeline(
                device,
                &shaders.scatter,
                transform_bgl,
                style_bgl,
                surface_format,
                sample_count,
                vs,
                fs,
                label,
            )
        }
        TernaryPipeline::ScatterMapped | TernaryPipeline::RingMapped => {
            let (vs, fs, label) = if kind == TernaryPipeline::ScatterMapped {
                (
                    "vs_ternary_mapped",
                    "fs_mapped",
                    "figgy scatter ternary mapped pipeline",
                )
            } else {
                (
                    "vs_pick_ring_ternary_mapped",
                    "fs_pick_ring",
                    "figgy picked point ternary mapped ring pipeline",
                )
            };
            data_render::create_scatter_ternary_mapped_pipeline(
                device,
                &shaders.scatter,
                transform_bgl,
                style_bgl,
                style_map_bgl,
                value_map_bgl,
                surface_format,
                sample_count,
                vs,
                fs,
                label,
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
            data_render::create_line_view3d_pipeline_with_sample_count(
                device,
                &shaders.line,
                transform_bgl,
                style_bgl,
                surface_bgl,
                surface_format,
                sample_count,
                pass,
            )
        }
        _ if !kind.mapped() => {
            let (vs, fs, label) = match kind {
                View3dPipeline::ScatterPick => {
                    ("vs_3d_pick", "fs_3d_pick", "figgy scatter 3D pick pipeline")
                }
                View3dPipeline::Ring => (
                    "vs_pick_ring_3d",
                    "fs_pick_ring",
                    "figgy picked point 3D ring pipeline",
                ),
                _ => ("vs_3d", "fs_main", "figgy scatter 3D pipeline"),
            };
            data_render::create_scatter_view3d_pipeline(
                device,
                &shaders.scatter,
                transform_bgl,
                style_bgl,
                surface_bgl,
                surface_format,
                sample_count,
                pass,
                vs,
                fs,
                label,
            )
        }
        _ => {
            let (vs, fs, label) = match kind {
                View3dPipeline::ScatterMappedPick => (
                    "vs_3d_mapped_pick",
                    "fs_3d_mapped_pick",
                    "figgy scatter 3D mapped pick pipeline",
                ),
                View3dPipeline::RingMapped => (
                    "vs_pick_ring_3d_mapped",
                    "fs_pick_ring",
                    "figgy picked point 3D mapped ring pipeline",
                ),
                _ => (
                    "vs_3d_mapped",
                    "fs_mapped",
                    "figgy scatter 3D mapped pipeline",
                ),
            };
            data_render::create_scatter_view3d_mapped_pipeline(
                device,
                &shaders.scatter,
                transform_bgl,
                style_bgl,
                style_map_bgl,
                view3d_value_map_bgl,
                surface_format,
                sample_count,
                pass,
                vs,
                fs,
                label,
            )
        }
    }
}
