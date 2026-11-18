// ── Surface look (group 3, binding 5) ───────────────────────────────────────
//
// The look of a surface on a 3D chart (`Config.view_3d`). The heatmap grid
// (`surface_input_grid.wgsl`) and the triangle mesh
// (`surface_input_mesh.wgsl`, #210) read one `SurfaceParams`, so the two
// inputs share the entries of `surface_entry.wgsl`.
//
// CPU twin: `data_render/surface_mesh.rs::SurfaceParamsGpu` (`#[repr(C)]`,
// 80 B).
struct SurfaceParams {
    /// Where the colour grid's columns start in `grid`, when
    /// `SURFACE_COLOR_GRID` is set (a grid surface).
    color_base: u32,
    flags: u32,
    /// The picked sample's constituent column and row (a grid surface), or
    /// the picked triangle and 0 (a mesh surface); `SURFACE_NO_PICK` for
    /// none.
    picked_c: u32,
    picked_r: u32,
    wire_width_px: f32,
    /// What the ID pick writes for this series: its pick slot plus one.
    pick_id: u32,
    _pad0: u32,
    _pad1: u32,
    /// Premultiplied.
    wire_color: vec4<f32>,
    material: Material,
};

const SURFACE_COLOR_GRID: u32 = 1u;
const SURFACE_WIREFRAME: u32 = 2u;
const SURFACE_LIT: u32 = 4u;
/// No picked sample or triangle (`surface_mesh.rs::SURFACE_NO_PICK`).
const SURFACE_NO_PICK: u32 = 0xffffffffu;

@group(3) @binding(5) var<uniform> surface: SurfaceParams;

fn surface_flag(bit: u32) -> bool {
    return (surface.flags & bit) != 0u;
}

/// What an input's `surface_patch` answers for a fragment of the picked
/// patch's instances: whether the fragment lies in the patch, and its
/// distance in pixels to the patch's border (the outline is drawn inside
/// 1.5 px of it).
struct SurfacePatch {
    inside: bool,
    edge_px: f32,
};
