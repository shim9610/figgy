// ── Surface input: the heatmap grid as a mesh (group 2 and group 3) ─────────
//
// A heatmap on a 3D chart draws as a surface over its sample lattice: one
// instance per quad between four neighbouring samples, six vertices, no
// vertex buffers. Each vertex reads its sample's x, y and height from the
// pool through the field group 2 (`field_grid.wgsl`), the same table the
// flat heatmap reads, so the surface and the heatmap of one grid sample the
// same values. Nothing is stored per sample: positions, colours and, for a
// lit material, normals are computed here on every draw (rule 1, rule 2).
//
// CPU twin: `data_render/surface_mesh.rs::SurfaceParamsGpu` (`#[repr(C)]`,
// 80 B).
struct SurfaceParams {
    /// Where the colour grid's columns start in `grid`, when
    /// `SURFACE_COLOR_GRID` is set.
    color_base: u32,
    flags: u32,
    /// The picked sample's constituent column and row, `u32::MAX` for none.
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

@group(3) @binding(5) var<uniform> surface: SurfaceParams;

fn surface_flag(bit: u32) -> bool {
    return (surface.flags & bit) != 0u;
}

/// The lattice size: samples along x and along y.
fn surface_samples() -> vec2<u32> {
    let columns_are_y = field_flag(FIELD_COLUMNS_ARE_Y);
    return vec2<u32>(
        select(field.cols, field.rows, columns_are_y),
        select(field.rows, field.cols, columns_are_y),
    );
}

/// Quads along x and y between the samples.
fn surface_quads() -> vec2<u32> {
    let n = surface_samples();
    return vec2<u32>(max(n.x, 1u) - 1u, max(n.y, 1u) - 1u);
}

/// The grid address `(c, r)` of the sample `(i, j)` along x and y.
fn surface_cell(ij: vec2<u32>) -> vec2<u32> {
    let columns_are_y = field_flag(FIELD_COLUMNS_ARE_Y);
    return vec2<u32>(select(ij.x, ij.y, columns_are_y), select(ij.y, ij.x, columns_are_y));
}

/// The height sample at `(i, j)`.
fn surface_height(ij: vec2<u32>) -> GridValue {
    let cr = surface_cell(ij);
    return grid_value(cr.x, cr.y);
}

/// The colour sample at `(i, j)`: the colour grid's when the series has
/// one, else the height.
fn surface_color_value(ij: vec2<u32>) -> GridValue {
    if (!surface_flag(SURFACE_COLOR_GRID)) {
        return surface_height(ij);
    }
    let cr = surface_cell(ij);
    var out: GridValue;
    out.pair = vec2<f32>(0.0, 0.0);
    out.valid = false;
    if (cr.x >= field.cols) {
        return out;
    }
    let column = grid[surface.color_base + cr.x];
    if (cr.y >= column.len) {
        return out;
    }
    let j = column.base + cr.y * 2u;
    out.pair = vec2<f32>(field_pool[j], field_pool[j + 1u]);
    out.valid = true;
    return out;
}

/// Box point of sample `(i, j)` and whether it can be placed: its x, y
/// and height finite.
struct SurfacePoint {
    t: vec3<f32>,
    valid: bool,
};

fn surface_point(ij: vec2<u32>) -> SurfacePoint {
    var out: SurfacePoint;
    let n = surface_samples();
    let x = view3d_axis_t(sample_point_pair(field.x_base, field.x_len, ij.x), 0u);
    let y = view3d_axis_t(sample_point_pair(field.y_base, field.y_len, ij.y), 1u);
    let h = surface_height(ij);
    let z = view3d_z_t(h.pair);
    out.t = vec3<f32>(x, y, z);
    out.valid = ij.x < n.x && ij.y < n.y && h.valid && vec2_f32_is_finite(h.pair)
        && f32_is_finite(x) && f32_is_finite(y) && f32_is_finite(z);
    return out;
}

/// The box-space normal at sample `(i, j)` from central differences of its
/// neighbours (one-sided at the lattice's edges): a shading value computed
/// on every draw, never stored.
fn surface_normal(ij: vec2<u32>) -> vec3<f32> {
    let n = surface_samples();
    let i0 = max(ij.x, 1u) - 1u;
    let i1 = min(ij.x + 1u, n.x - 1u);
    let j0 = max(ij.y, 1u) - 1u;
    let j1 = min(ij.y + 1u, n.y - 1u);
    let a = surface_point(vec2<u32>(i0, ij.y));
    let b = surface_point(vec2<u32>(i1, ij.y));
    let c = surface_point(vec2<u32>(ij.x, j0));
    let d = surface_point(vec2<u32>(ij.x, j1));
    if (!a.valid || !b.valid || !c.valid || !d.valid) {
        return vec3<f32>(0.0, 0.0, 1.0);
    }
    let du = box_point(b.t) - box_point(a.t);
    let dv = box_point(d.t) - box_point(c.t);
    let nrm = cross(du, dv);
    let len = length(nrm);
    if (len <= 0.0 || !f32_is_finite(len)) {
        return vec3<f32>(0.0, 0.0, 1.0);
    }
    return nrm / len;
}

/// The six corners of a quad as sample offsets: two triangles.
fn surface_corner(vertex: u32) -> vec2<u32> {
    var corners = array<vec2<u32>, 6>(
        vec2<u32>(0u, 0u),
        vec2<u32>(1u, 0u),
        vec2<u32>(0u, 1u),
        vec2<u32>(1u, 0u),
        vec2<u32>(1u, 1u),
        vec2<u32>(0u, 1u),
    );
    return corners[vertex];
}

/// One vertex of the mesh: its quad, its corner, its box point and normal.
/// `valid` is false when any of the quad's four samples cannot be placed:
/// the quad's two triangles are dropped, as a NaN breaks a line.
struct SurfaceVertex {
    quad: vec2<u32>,
    corner: vec2<u32>,
    t: vec3<f32>,
    normal: vec3<f32>,
    valid: bool,
};

fn surface_vertex(quad_index: u32, vertex: u32) -> SurfaceVertex {
    var out: SurfaceVertex;
    let q = surface_quads();
    out.valid = false;
    if (q.x == 0u || q.y == 0u) {
        return out;
    }
    out.quad = vec2<u32>(quad_index % q.x, quad_index / q.x);
    out.corner = surface_corner(vertex);
    if (out.quad.y >= q.y) {
        return out;
    }
    let base = out.quad;
    let p00 = surface_point(base);
    let p10 = surface_point(base + vec2<u32>(1u, 0u));
    let p01 = surface_point(base + vec2<u32>(0u, 1u));
    let p11 = surface_point(base + vec2<u32>(1u, 1u));
    if (!p00.valid || !p10.valid || !p01.valid || !p11.valid) {
        return out;
    }
    let here = surface_point(base + out.corner);
    out.t = here.t;
    out.normal = vec3<f32>(0.0, 0.0, 1.0);
    if (surface_flag(SURFACE_LIT)) {
        out.normal = surface_normal(base + out.corner);
    }
    out.valid = true;
    return out;
}

/// The premultiplied fill colour at in-quad fractions `f` of `quad`: the
/// heatmap's own `field_fill_color`, bilinear for interpolated shading and
/// the quad's mean for flat shading (matplotlib's `plot_surface`).
fn surface_fill_color(quad: vec2<u32>, f: vec2<f32>) -> vec4<f32> {
    let z00 = surface_color_value(quad);
    let z10 = surface_color_value(quad + vec2<u32>(1u, 0u));
    let z01 = surface_color_value(quad + vec2<u32>(0u, 1u));
    let z11 = surface_color_value(quad + vec2<u32>(1u, 1u));
    if (field_flag(FIELD_INTERPOLATED)) {
        return field_fill_color(z00, z10, z01, z11, f.x, f.y, style.color_premul);
    }
    var mean: GridValue;
    mean.valid = z00.valid && z10.valid && z01.valid && z11.valid;
    mean.pair = scale_grid_pair(
        add_grid_pairs(add_grid_pairs(z00.pair, z10.pair), add_grid_pairs(z01.pair, z11.pair)),
        0.25,
    );
    return field_fill_color(mean, mean, mean, mean, 0.0, 0.0, style.color_premul);
}
