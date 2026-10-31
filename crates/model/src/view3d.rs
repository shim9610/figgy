//! 3D charts: `Config.view_3d` and the surface look of a heatmap.
//!
//! With `view_3d: Some(..)` the chart draws a box in its data area, seen
//! through a camera kept in the SSOT. A `Heatmap` series draws as a surface:
//! its grid's x and y on `bottom_x` and `left_y`, the cell value as the
//! height on `right_y`, coloured through the colourbar. A `Contour` series
//! draws its isolines on the box floor, and a `HeatmapContour` both. The
//! data type is the heatmap's: switching a heatmap between 2D and 3D is one
//! configuration change. [`SurfaceStyle`], an optional field of the heatmap
//! series, holds the surface's look; absent, every option takes its default.
//!
//! The renderer projects, depth-tests, maps colour and, for a lit material,
//! derives shading normals from the neighbouring heights on every draw:
//! appearance only (`CLAUDE.md` rule 1), never stored. Hover and selection
//! report the grid sample and its values read back from the pool.
//!
//! [`Config::view_matrix`] is the one camera matrix: the GPU uniform, every
//! 3D decoration, the hit tests and the drags read it.

use crate::config::{AxisOptions, AxisScale, Config};
use crate::data_config::{
    DataRenderType, FillMode, GridLayout, LineInterpolation, MatrixOrientation, MatrixRef,
    SeriesConfig,
};
use crate::material::Material;
use crate::{Color, ColumnId};

/// Degrees of camera turn per pixel of a data-area drag on a 3D chart.
pub const ORBIT_DEGREES_PER_PX: f64 = 0.5;

/// The eye distance the box is fitted at, in half-diagonals of the box:
/// [`View3d::distance`] below it magnifies, above it shrinks.
pub const REFERENCE_DISTANCE: f64 = 3.0;

/// The allowed eye distances, in half-diagonals of the box.
pub const DISTANCE_RANGE: std::ops::RangeInclusive<f64> = 1.2..=100.0;

/// The allowed relative box lengths.
pub const BOX_ASPECT_RANGE: std::ops::RangeInclusive<f64> = 0.05..=20.0;

/// The box's relative x : y : z lengths when [`View3d::box_aspect`] is
/// `None`.
pub const DEFAULT_BOX_ASPECT: [f64; 3] = [1.0, 1.0, 0.75];

/// How the box is projected onto the screen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Projection3d {
    /// Parallel projection: a distance change scales the view.
    #[default]
    Orthographic,
    /// A pinhole at [`View3d::distance`]: nearer parts are larger, and a
    /// shorter distance foreshortens more. The box is fitted to the data
    /// area at [`REFERENCE_DISTANCE`], so the distance alone sets the
    /// perspective.
    Perspective,
}

/// The camera of a 3D chart (`Config.view_3d`). The box's axes are x
/// (`bottom_x`), y (`left_y`) and z (`right_y`, up).
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct View3d {
    /// The camera's turn around the z axis, in degrees from the +x axis
    /// towards +y. Any finite value; it is read modulo 360.
    pub azimuth_degrees: f64,
    /// The camera's height above the floor plane, in degrees, `-90..=90`.
    pub elevation_degrees: f64,
    /// The eye distance in half-diagonals of the box,
    /// [`DISTANCE_RANGE`]; [`REFERENCE_DISTANCE`] (the default) fits the
    /// box in the data area.
    pub distance: f64,
    pub projection: Projection3d,
    /// The relative x : y : z box lengths, each in [`BOX_ASPECT_RANGE`].
    /// `None` (the default, omitted from JSON) is [`DEFAULT_BOX_ASPECT`].
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub box_aspect: Option<[f64; 3]>,
}

impl Default for View3d {
    fn default() -> Self {
        Self {
            azimuth_degrees: -60.0,
            elevation_degrees: 30.0,
            distance: REFERENCE_DISTANCE,
            projection: Projection3d::Orthographic,
            box_aspect: None,
        }
    }
}

/// The grid lines a surface draws over its fill, along the grid's rows and
/// columns only.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SurfaceWireframe {
    pub color: Color,
    /// Logical pixels, constant on screen.
    pub width: f32,
}

/// The look of a heatmap drawn as a surface on a 3D chart. The fill's
/// shading and opacity are the heatmap's own `fill`.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(default)
)]
pub struct SurfaceStyle {
    /// A second grid of the same shape to colour the surface by; `None`
    /// (the default, omitted from JSON) colours by the height.
    #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
    pub color_matrix: Option<MatrixRef>,
    /// `None` (the default, omitted from JSON) draws no wireframe.
    #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
    pub wireframe: Option<SurfaceWireframe>,
    /// `Flat` (the default) is unlit.
    pub material: Material,
    pub gloss: f32,
    pub texture_strength: f32,
    pub texture_scale: f32,
    /// The brightness the picked sample's patch gains.
    pub emphasis_brightness: f32,
}

impl Default for SurfaceStyle {
    fn default() -> Self {
        Self {
            color_matrix: None,
            wireframe: None,
            material: Material::Flat,
            gloss: 0.45,
            texture_strength: 0.2,
            texture_scale: 1.0,
            emphasis_brightness: 0.12,
        }
    }
}

/// The camera of a 3D chart as one projective map from box coordinates
/// `t` (each axis' fraction, `0..=1`) to the chart's pixel space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewMatrix {
    /// Row-major: `[x·w, y·w, depth·w, w] = m · [tx, ty, tz, 1]`, `x` and
    /// `y` in chart pixels (y down), `depth` in `0..=1` (0 nearest).
    pub m: [[f64; 4]; 4],
}

impl ViewMatrix {
    /// The pixel position and depth of box point `t`, or `None` behind the
    /// eye.
    pub fn project(&self, t: [f64; 3]) -> Option<[f64; 3]> {
        let row = |r: &[f64; 4]| r[0] * t[0] + r[1] * t[1] + r[2] * t[2] + r[3];
        let w = row(&self.m[3]);
        (w > 1e-9).then(|| {
            [
                row(&self.m[0]) / w,
                row(&self.m[1]) / w,
                row(&self.m[2]) / w,
            ]
        })
    }

    /// The matrix as `f32` columns, the layout of a WGSL `mat4x4<f32>`.
    pub fn columns_f32(&self) -> [[f32; 4]; 4] {
        let mut c = [[0.0f32; 4]; 4];
        for (j, col) in c.iter_mut().enumerate() {
            for (i, v) in col.iter_mut().enumerate() {
                *v = self.m[i][j] as f32;
            }
        }
        c
    }
}

impl View3d {
    /// The box's relative lengths, normalized so the half-diagonal is 1.
    pub fn half_extents(&self) -> [f64; 3] {
        let a = self.box_aspect.unwrap_or(DEFAULT_BOX_ASPECT);
        let len = (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt().max(1e-12);
        [a[0] / len, a[1] / len, a[2] / len]
    }

    /// The unit vector from the box centre towards the eye.
    pub fn eye_direction(&self) -> [f64; 3] {
        let (a, e) = (
            self.azimuth_degrees.to_radians(),
            self.elevation_degrees.to_radians(),
        );
        [e.cos() * a.cos(), e.cos() * a.sin(), e.sin()]
    }

    /// The screen's right and up directions in box space.
    fn screen_axes(&self) -> ([f64; 3], [f64; 3]) {
        let a = self.azimuth_degrees.to_radians();
        let d = self.eye_direction();
        let r = [-a.sin(), a.cos(), 0.0];
        // u = r × (−d)
        let u = [
            -(r[1] * d[2] - r[2] * d[1]),
            -(r[2] * d[0] - r[0] * d[2]),
            -(r[0] * d[1] - r[1] * d[0]),
        ];
        (r, u)
    }

    /// The camera map from box coordinates `t` to unscaled screen
    /// coordinates (x right, y up) and depth, with the eye at `distance`:
    /// rows of `[sx·w, sy·w, depth·w, w]`.
    fn camera(&self, distance: f64) -> [[f64; 4]; 4] {
        let h = self.half_extents();
        let d = self.eye_direction();
        let (r, u) = self.screen_axes();
        // p = (2t − 1)·h, so a linear form v·p is Σ 2·v_i·h_i·t_i − v_i·h_i.
        let affine = |v: [f64; 3], scale: f64, offset: f64| -> [f64; 4] {
            [
                2.0 * v[0] * h[0] * scale,
                2.0 * v[1] * h[1] * scale,
                2.0 * v[2] * h[2] * scale,
                -(v[0] * h[0] + v[1] * h[1] + v[2] * h[2]) * scale + offset,
            ]
        };
        match self.projection {
            // w = distance / 3: a parallel view scaled by 3 / distance.
            Projection3d::Orthographic => {
                let w = distance / REFERENCE_DISTANCE;
                let depth = affine(d, -0.5 * w, 0.5 * w);
                [
                    affine(r, 1.0, 0.0),
                    affine(u, 1.0, 0.0),
                    depth,
                    [0.0, 0.0, 0.0, w],
                ]
            }
            // w = (distance − d·p) / 3, the depth along the view; the box
            // lies between near = distance − 1 and far = distance + 1, and
            // depth = far / (far − near) · (1 − near / w).
            Projection3d::Perspective => {
                let w = affine(d, -1.0 / REFERENCE_DISTANCE, distance / REFERENCE_DISTANCE);
                let (near, far) = (
                    (distance - 1.0) / REFERENCE_DISTANCE,
                    (distance + 1.0) / REFERENCE_DISTANCE,
                );
                let k = far / (far - near);
                let depth = [w[0] * k, w[1] * k, w[2] * k, (w[3] - near) * k];
                [affine(r, 1.0, 0.0), affine(u, 1.0, 0.0), depth, w]
            }
        }
    }

    /// The eight corners of the box in box coordinates.
    pub fn corners() -> [[f64; 3]; 8] {
        let mut out = [[0.0; 3]; 8];
        for (i, c) in out.iter_mut().enumerate() {
            *c = [(i & 1) as f64, ((i >> 1) & 1) as f64, ((i >> 2) & 1) as f64];
        }
        out
    }

    /// Validate the camera's values.
    pub fn validate(&self) -> Result<(), &'static str> {
        if !self.azimuth_degrees.is_finite() {
            return Err("view_3d.azimuth_degrees must be finite");
        }
        if !(self.elevation_degrees.is_finite() && (-90.0..=90.0).contains(&self.elevation_degrees))
        {
            return Err("view_3d.elevation_degrees must be in -90..=90");
        }
        if !(self.distance.is_finite() && DISTANCE_RANGE.contains(&self.distance)) {
            return Err("view_3d.distance must be in 1.2..=100");
        }
        if let Some(a) = self.box_aspect
            && a.iter()
                .any(|v| !(v.is_finite() && BOX_ASPECT_RANGE.contains(v)))
        {
            return Err("view_3d.box_aspect lengths must be in 0.05..=20");
        }
        Ok(())
    }
}

/// The band outside the box for an axis' ticks, labels and title, in
/// pixels; every term scales with `Config::scaled`.
fn axis_band(axis: &AxisOptions) -> f32 {
    let labels = &axis.label_style;
    let font = labels.font_size.max(0.0);
    let tick = axis.major_tick_length.max(0.0);
    let text = if labels.visible && labels.label_visible {
        font * 1.4 + font / 3.0
    } else {
        font / 9.0
    };
    let title = &axis.title_option;
    let title = if title.visible && !title.text.segments.is_empty() {
        title.text.font_size.max(0.0) * 1.4
    } else {
        0.0
    };
    tick + text + title
}

/// The edge of the box an axis is drawn on: its two box points, their pixel
/// positions (moved out by the axis' `line_offset`), and the unit screen
/// direction away from the box, along which the ticks, labels and title
/// sit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoxEdge {
    /// Box points of the axis' 0 and 1 ends.
    pub t0: [f64; 3],
    pub t1: [f64; 3],
    /// Their pixel positions, `line_offset` applied.
    pub a: [f32; 2],
    pub b: [f32; 2],
    /// Unit screen direction away from the projected box centre,
    /// perpendicular to the edge.
    pub normal: [f32; 2],
}

impl BoxEdge {
    /// The pixel point at axis fraction `f`, moved `offset` pixels along
    /// the normal.
    pub fn point(&self, f: f32, offset: f32) -> [f32; 2] {
        [
            self.a[0] + (self.b[0] - self.a[0]) * f + self.normal[0] * offset,
            self.a[1] + (self.b[1] - self.a[1]) * f + self.normal[1] * offset,
        ]
    }

    /// The unit screen direction of the edge, from its 0 to its 1 end.
    pub fn direction(&self) -> [f32; 2] {
        let (dx, dy) = (self.b[0] - self.a[0], self.b[1] - self.a[1]);
        let len = dx.hypot(dy).max(f32::EPSILON);
        [dx / len, dy / len]
    }
}

/// The labels' clearance band outside an edge, without the title: the
/// tick and a line of labels.
fn label_band(axis: &AxisOptions) -> f32 {
    let labels = &axis.label_style;
    let font = labels.font_size.max(0.0);
    let tick = axis.major_tick_length.max(0.0);
    let text = if labels.visible && labels.label_visible {
        font * 1.4 + font / 3.0
    } else {
        font / 9.0
    };
    tick + text
}

/// The axis of box direction `k`: 0 x `bottom_x`, 1 y `left_y`, 2 z
/// `right_y`.
pub fn box_axis(config: &Config, k: usize) -> &AxisOptions {
    match k {
        0 => &config.bottom_x,
        1 => &config.left_y,
        _ => &config.right_y,
    }
}

impl Config {
    /// The camera matrix of this 3D chart, when it is one and has a data
    /// area: box coordinates to chart pixels and depth. The box is fitted
    /// at [`REFERENCE_DISTANCE`] so its eight corners sit inside the data
    /// area less a band for the axes' ticks, labels and titles, centred; the
    /// camera's distance then zooms.
    pub fn view_matrix(&self) -> Option<ViewMatrix> {
        let view = self.view_3d.as_ref()?;
        let area = self.data_area().ok()?.0;
        let band = f64::from(
            (0..3)
                .map(|k| axis_band(box_axis(self, k)))
                .fold(0.0f32, f32::max),
        );
        let fit = view.camera(REFERENCE_DISTANCE);
        let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for t in View3d::corners() {
            let row = |r: &[f64; 4]| r[0] * t[0] + r[1] * t[1] + r[2] * t[2] + r[3];
            let w = row(&fit[3]);
            let (sx, sy) = (row(&fit[0]) / w, row(&fit[1]) / w);
            x0 = x0.min(sx);
            x1 = x1.max(sx);
            y0 = y0.min(sy);
            y1 = y1.max(sy);
        }
        let width = (f64::from(area.width) - 2.0 * band).max(1.0);
        let height = (f64::from(area.height) - 2.0 * band).max(1.0);
        let s = (width / (x1 - x0).max(1e-9)).min(height / (y1 - y0).max(1e-9));
        let (bx, by) = ((x0 + x1) * 0.5, (y0 + y1) * 0.5);
        let cx = f64::from(area.x) + f64::from(area.width) * 0.5;
        let cy = f64::from(area.y) + f64::from(area.height) * 0.5;
        // px = cx + s·(sx − bx), py = cy − s·(sy − by), each times w.
        let c = view.camera(view.distance);
        let combine = |a: &[f64; 4], ka: f64, kw: f64| -> [f64; 4] {
            [
                a[0] * ka + c[3][0] * kw,
                a[1] * ka + c[3][1] * kw,
                a[2] * ka + c[3][2] * kw,
                a[3] * ka + c[3][3] * kw,
            ]
        };
        Some(ViewMatrix {
            m: [
                combine(&c[0], s, cx - s * bx),
                combine(&c[1], -s, cy + s * by),
                c[2],
                c[3],
            ],
        })
    }

    /// The box edge axis `k` (0 x, 1 y, 2 z) is drawn on for the current
    /// camera: x and y on the floor edges nearest the eye, z on the
    /// vertical edge left-most on screen.
    pub fn box_axis_edge(&self, k: usize) -> Option<BoxEdge> {
        let view = self.view_3d.as_ref()?;
        let m = self.view_matrix()?;
        let d = view.eye_direction();
        let front = |i: usize| if d[i] >= 0.0 { 1.0 } else { 0.0 };
        let (t0, t1) = match k {
            0 => ([0.0, front(1), 0.0], [1.0, front(1), 0.0]),
            1 => ([front(0), 0.0, 0.0], [front(0), 1.0, 0.0]),
            _ => {
                let corner = [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]]
                    .into_iter()
                    .filter_map(|c: [f64; 2]| Some((m.project([c[0], c[1], 0.0])?[0], c)))
                    .fold(None, |best: Option<(f64, [f64; 2])>, cand| match best {
                        Some(b) if b.0 <= cand.0 => Some(b),
                        _ => Some(cand),
                    })?
                    .1;
                ([corner[0], corner[1], 0.0], [corner[0], corner[1], 1.0])
            }
        };
        let pa = m.project(t0)?;
        let pb = m.project(t1)?;
        let centre = m.project([0.5, 0.5, 0.5])?;
        let (dx, dy) = ((pb[0] - pa[0]) as f32, (pb[1] - pa[1]) as f32);
        let len = dx.hypot(dy).max(f32::EPSILON);
        let mut normal = [dy / len, -dx / len];
        let mid = [
            ((pa[0] + pb[0]) * 0.5) as f32,
            ((pa[1] + pb[1]) * 0.5) as f32,
        ];
        let away = [mid[0] - centre[0] as f32, mid[1] - centre[1] as f32];
        if away[0] * normal[0] + away[1] * normal[1] < 0.0 {
            normal = [-normal[0], -normal[1]];
        }
        let offset = box_axis(self, k).line_offset;
        let shift = |p: [f64; 3]| {
            [
                p[0] as f32 + normal[0] * offset,
                p[1] as f32 + normal[1] * offset,
            ]
        };
        Some(BoxEdge {
            t0,
            t1,
            a: shift(pa),
            b: shift(pb),
            normal,
        })
    }

    /// Where the title of box axis `k` sits: centred past its edge's tick
    /// and label band along the edge's normal, parallel to the edge and
    /// reading left to right, moved by the title offsets in screen space.
    pub fn box_title_placement(
        &self,
        k: usize,
        offset: (f32, f32),
        extents: crate::text::TextExtents,
    ) -> Option<crate::layout::TitlePlacement> {
        let edge = self.box_axis_edge(k)?;
        let reach = label_band(box_axis(self, k)) + extents.height() * 0.5;
        let mid = edge.point(0.5, reach);
        let center = (mid[0] + offset.0, mid[1] + offset.1);
        let mut dir = edge.direction();
        if dir[0] < 0.0 || (dir[0] == 0.0 && dir[1] > 0.0) {
            dir = [-dir[0], -dir[1]];
        }
        Some(crate::layout::TitlePlacement {
            origin: (
                center.0 - extents.width * 0.5,
                center.1 + (extents.ascent - extents.descent) * 0.5,
            ),
            rotation_center: center,
            rotation_degrees: dir[1].atan2(dir[0]).to_degrees(),
        })
    }

    /// The box's three back panes for the current camera, the faces whose
    /// outward normal points away from the eye: for each direction `k`,
    /// the coordinate (0 or 1) of the pane across it. The grid is drawn on
    /// them.
    pub fn box_back_panes(&self) -> Option<[f64; 3]> {
        let d = self.view_3d.as_ref()?.eye_direction();
        Some([0, 1, 2].map(|i| if d[i] >= 0.0 { 0.0 } else { 1.0 }))
    }

    /// The pixel position and depth of box point `t` on this 3D chart
    /// ([`Self::view_matrix`]).
    pub fn project_3d(&self, t: [f64; 3]) -> Option<[f64; 3]> {
        self.view_matrix()?.project(t)
    }

    /// Orbit the camera of a 3D chart by a screen drag: the azimuth turns
    /// with `dx` and the elevation with `dy`, clamped to the poles.
    /// Returns whether anything changed.
    pub fn orbit_view(&mut self, dx: f32, dy: f32) -> bool {
        let Some(view) = self.view_3d.as_mut() else {
            return false;
        };
        let before = (view.azimuth_degrees, view.elevation_degrees);
        view.azimuth_degrees -= f64::from(dx) * ORBIT_DEGREES_PER_PX;
        view.elevation_degrees =
            (view.elevation_degrees + f64::from(dy) * ORBIT_DEGREES_PER_PX).clamp(-90.0, 90.0);
        (view.azimuth_degrees, view.elevation_degrees) != before
    }

    /// Zoom the camera of a 3D chart: the eye distance is divided by
    /// `factor` and clamped to [`DISTANCE_RANGE`]. Returns whether it
    /// changed.
    pub fn zoom_view(&mut self, factor: f64) -> bool {
        let Some(view) = self.view_3d.as_mut() else {
            return false;
        };
        if !(factor.is_finite() && factor > 0.0) {
            return false;
        }
        let before = view.distance;
        view.distance =
            (view.distance / factor).clamp(*DISTANCE_RANGE.start(), *DISTANCE_RANGE.end());
        view.distance != before
    }

    /// Validate `view_3d` against the axes it reads; `None` always passes.
    pub fn validate_view_3d(&self) -> Result<(), &'static str> {
        let Some(view) = &self.view_3d else {
            return Ok(());
        };
        view.validate()?;
        for k in 0..3 {
            let axis = box_axis(self, k);
            if axis.categories.is_some() {
                return Err("3D chart axes (bottom_x, left_y, right_y) cannot be categorical");
            }
            if !matches!(axis.scale, AxisScale::Linear | AxisScale::Logarithmic) {
                return Err("3D chart axes must be linear or logarithmic");
            }
        }
        if self.polar.is_some() || self.ternary.is_some() {
            return Err("a 3D chart is neither polar nor ternary");
        }
        if !self.overlays.is_empty() {
            return Err("3D charts have no overlay layers");
        }
        if self
            .line_offsets
            .as_ref()
            .is_some_and(|offsets| !offsets.series.is_empty())
        {
            return Err("3D charts have no line offsets");
        }
        if !self.draw_style.is_precise() {
            return Err("3D charts are drawn in the Precise style only");
        }
        Ok(())
    }

    /// The rules the series of a chart are held to with a 3D view: a 3D
    /// chart draws heatmaps (as surfaces), contours (on the floor), heatmaps
    /// with contours, and scatter, line and scatter-line series that name a
    /// `z_column` (#132); a surface's colour grid matches its height grid,
    /// its style is in range, its fill is continuous, a floor contour has no
    /// labels, and a 3D line is linear without an area fill. A heatmap's
    /// `surface` style and a series' `z_column` on a flat chart are kept
    /// and unused, so a chart switches between 2D and 3D by its view alone.
    pub fn validate_view3d_series(&self, series: &[SeriesConfig]) -> Result<(), &'static str> {
        for item in series {
            let (surface, matrix, fill, contour) = match &item.render_type {
                DataRenderType::Scatter { .. }
                | DataRenderType::Line { .. }
                | DataRenderType::ScatterLine { .. }
                    if self.view_3d.is_some() =>
                {
                    if item.z_column.as_deref().is_none_or(str::is_empty) {
                        return Err(
                            "a scatter or line series on a 3D chart names its height column \
                             (z_column)",
                        );
                    }
                    let line = match &item.render_type {
                        DataRenderType::Line { line }
                        | DataRenderType::ScatterLine { line, .. } => Some(line),
                        _ => None,
                    };
                    if let Some(line) = line {
                        if line.interpolation != LineInterpolation::Linear {
                            return Err("a line on a 3D chart is linear: no step interpolation");
                        }
                        if line.fill.is_some() {
                            return Err("a line on a 3D chart has no area fill");
                        }
                    }
                    continue;
                }
                DataRenderType::RgbImage { .. } if self.view_3d.is_some() => {
                    return Err("an image draws flat: not on a 3D chart");
                }
                DataRenderType::Parallel { .. } if self.view_3d.is_some() => {
                    return Err("a parallel series draws flat: not on a 3D chart");
                }
                DataRenderType::Heatmap {
                    matrix,
                    fill,
                    surface,
                } => (surface, matrix, Some(fill), None),
                DataRenderType::HeatmapContour {
                    matrix,
                    fill,
                    contour,
                    surface,
                } => (surface, matrix, Some(fill), Some(contour)),
                DataRenderType::Contour { matrix, contour } => (&None, matrix, None, Some(contour)),
                _ if self.view_3d.is_some() => {
                    return Err(
                        "a 3D chart draws only heatmaps, contours, heatmap contours and the \
                         scatter and line series that name a z_column",
                    );
                }
                _ => continue,
            };
            if let Some(style) = surface {
                validate_surface_style(style, matrix)?;
            }
            if self.view_3d.is_none() {
                continue;
            }
            if matrix.grid_layout == GridLayout::Hexagonal {
                return Err("a hexagonal grid draws flat: not on a 3D chart");
            }
            if fill.is_some_and(|f| f.mode == FillMode::Bands) {
                return Err("a surface's fill is continuous: banded fills are not drawn in 3D");
            }
            if contour.is_some_and(|c| c.labels.is_some()) {
                return Err("contours on a 3D chart's floor have no labels");
            }
        }
        Ok(())
    }
}

fn finite_in(v: f32, lo: f32, hi: f32) -> bool {
    v.is_finite() && (lo..=hi).contains(&v)
}

/// A surface style in range, its colour grid the shape of its height grid.
fn validate_surface_style(style: &SurfaceStyle, matrix: &MatrixRef) -> Result<(), &'static str> {
    if let Some(color) = &style.color_matrix {
        let same_shape = color.columns.len() == matrix.columns.len()
            && same_orientation(&color.orientation, &matrix.orientation)
            && same_layout(&color.grid_layout, &matrix.grid_layout);
        if !same_shape {
            return Err(
                "a surface's color_matrix has the column count, orientation and layout of its \
                 matrix",
            );
        }
        if color.columns.iter().any(|c: &ColumnId| c.is_empty()) {
            return Err("a surface's color_matrix names its columns");
        }
    }
    if let Some(wire) = &style.wireframe
        && (!finite_in(wire.width, 0.0, 24.0)
            || ![wire.color.r, wire.color.g, wire.color.b, wire.color.a]
                .into_iter()
                .all(|v| finite_in(v, 0.0, 1.0)))
    {
        return Err("a surface wireframe has a width in 0..=24 and a colour in 0..=1");
    }
    if !finite_in(style.gloss, 0.0, 1.0)
        || !finite_in(style.texture_strength, 0.0, 1.0)
        || !(style.texture_scale.is_finite() && style.texture_scale > 0.0)
        || !finite_in(style.emphasis_brightness, 0.0, 1.0)
    {
        return Err(
            "a surface's gloss, texture_strength and emphasis_brightness are in 0..=1 and its \
             texture_scale is positive",
        );
    }
    Ok(())
}

fn same_orientation(a: &MatrixOrientation, b: &MatrixOrientation) -> bool {
    std::mem::discriminant(a) == std::mem::discriminant(b)
}

fn same_layout(a: &GridLayout, b: &GridLayout) -> bool {
    std::mem::discriminant(a) == std::mem::discriminant(b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data_config::{
        ContourConfig, ContourLabelConfig, DataScatterStyleConfig, FieldFillConfig, Shading,
    };
    use crate::{ScatterShape, layout::Element, layout::NudgeResult};

    fn chart() -> Config {
        let mut config = crate::default::default_config();
        config.view_3d = Some(View3d::default());
        config
    }

    fn matrix(n: usize) -> MatrixRef {
        MatrixRef {
            columns: (0..n).map(|i| format!("z{i}")).collect(),
            orientation: MatrixOrientation::ColumnsAreX,
            grid_layout: GridLayout::Centers,
        }
    }

    fn heatmap(surface: Option<SurfaceStyle>) -> SeriesConfig {
        SeriesConfig {
            series_id: "s".into(),
            source_id: None,
            label: None,
            x_column: "x".into(),
            y_column: "y".into(),
            z_column: None,
            render_type: DataRenderType::Heatmap {
                matrix: matrix(3),
                fill: FieldFillConfig {
                    mode: FillMode::Continuous,
                    shading: Shading::Interpolated,
                    opacity: 1.0,
                },
                surface,
            },
        }
    }

    #[test]
    fn the_box_fits_the_data_area_and_the_nearest_corner_is_nearest() {
        for projection in [Projection3d::Orthographic, Projection3d::Perspective] {
            let mut config = chart();
            config.view_3d.as_mut().unwrap().projection = projection;
            let area = config.data_area().unwrap().0;
            let view = config.view_3d.clone().unwrap();
            let d = view.eye_direction();
            let h = view.half_extents();
            let mut nearest = (f64::MIN, 0.0);
            let mut farthest = (f64::MAX, 0.0);
            for t in View3d::corners() {
                let p = config.project_3d(t).unwrap();
                assert!(
                    p[0] >= f64::from(area.x) - 0.5
                        && p[0] <= f64::from(area.x + area.width) + 0.5
                        && p[1] >= f64::from(area.y) - 0.5
                        && p[1] <= f64::from(area.y + area.height) + 0.5,
                    "{projection:?}: {t:?} at {p:?} outside {area:?}"
                );
                assert!(
                    (0.0..=1.0).contains(&p[2]),
                    "{projection:?}: depth {}",
                    p[2]
                );
                let toward: f64 = (0..3).map(|i| d[i] * (2.0 * t[i] - 1.0) * h[i]).sum();
                if toward > nearest.0 {
                    nearest = (toward, p[2]);
                }
                if toward < farthest.0 {
                    farthest = (toward, p[2]);
                }
            }
            assert!(
                nearest.1 < farthest.1,
                "{projection:?}: depth grows away from the eye"
            );
        }
    }

    #[test]
    fn the_matrix_columns_match_the_projection() {
        let config = chart();
        let m = config.view_matrix().unwrap();
        let c = m.columns_f32();
        let t = [0.25, 0.5, 0.75];
        let v = [t[0], t[1], t[2], 1.0];
        let mut out = [0.0f64; 4];
        for (i, o) in out.iter_mut().enumerate() {
            *o = (0..4).map(|j| f64::from(c[j][i]) * v[j]).sum();
        }
        let p = m.project(t).unwrap();
        assert!((out[0] / out[3] - p[0]).abs() < 1e-3 && (out[1] / out[3] - p[1]).abs() < 1e-3);
    }

    #[test]
    fn zoom_and_orbit_move_the_camera_and_clamp() {
        let mut config = chart();
        let before = config.project_3d([1.0, 1.0, 1.0]).unwrap();
        assert!(config.zoom_view(2.0));
        let after = config.project_3d([1.0, 1.0, 1.0]).unwrap();
        let centre = config.project_3d([0.5, 0.5, 0.5]).unwrap();
        let reach = |p: [f64; 3]| (p[0] - centre[0]).hypot(p[1] - centre[1]);
        assert!(reach(after) > reach(before) * 1.5, "zooming in magnifies");
        assert!(config.zoom_view(1000.0));
        assert_eq!(config.view_3d.as_ref().unwrap().distance, 1.2);
        assert!(!config.zoom_view(1000.0), "clamped: nothing changes");
        assert!(!config.zoom_view(0.0));

        let measure = crate::select::tests::FixedMeasure;
        let mut orbit = chart();
        assert_eq!(
            orbit.nudge(Element::DataArea, &measure, 10.0, 0.0),
            NudgeResult::Moved
        );
        let view = orbit.view_3d.as_ref().unwrap();
        assert_eq!(view.azimuth_degrees, -65.0);
        assert!(orbit.orbit_view(0.0, 1000.0));
        assert_eq!(orbit.view_3d.as_ref().unwrap().elevation_degrees, 90.0);
        assert!(!orbit.orbit_view(0.0, 10.0), "clamped at the pole");
        // The view stays well defined at the pole.
        assert!(orbit.project_3d([1.0, 0.0, 0.0]).is_some());
        // A flat chart's data area still moves.
        let mut flat = crate::default::default_config();
        assert!(!flat.orbit_view(1.0, 1.0));
        assert!(!flat.zoom_view(2.0));
    }

    #[test]
    fn validation_names_each_rejection() {
        assert_eq!(chart().validate_view_3d(), Ok(()));
        let reject = |edit: &dyn Fn(&mut Config)| {
            let mut config = chart();
            edit(&mut config);
            config.validate_view_3d().unwrap_err()
        };
        assert!(
            reject(&|c| c.view_3d.as_mut().unwrap().elevation_degrees = 91.0).contains("elevation")
        );
        assert!(
            reject(&|c| c.view_3d.as_mut().unwrap().azimuth_degrees = f64::NAN).contains("azimuth")
        );
        assert!(reject(&|c| c.view_3d.as_mut().unwrap().distance = 1.0).contains("distance"));
        assert!(
            reject(&|c| c.view_3d.as_mut().unwrap().box_aspect = Some([1.0, 0.0, 1.0]))
                .contains("box_aspect")
        );
        assert!(
            reject(&|c| c.polar = Some(crate::polar::PolarOptions::default())).contains("polar")
        );
        assert!(
            reject(&|c| c.ternary = Some(crate::ternary::TernaryOptions {})).contains("ternary")
        );
        assert!(
            reject(&|c| c.overlays.push(crate::layer::OverlayLayer::default())).contains("overlay")
        );
        assert!(
            reject(&|c| c.draw_style = crate::config::DrawStyle::Sketch(Default::default()))
                .contains("Precise")
        );
        assert!(
            reject(&|c| c.bottom_x.categories = Some(vec![crate::config::Category::new("a", "A")]))
                .contains("categorical")
        );
        let mut log = chart();
        log.right_y.scale = AxisScale::Logarithmic;
        log.right_y.min = 1.0;
        assert_eq!(
            log.validate_view_3d(),
            Ok(()),
            "a logarithmic axis is drawn"
        );
    }

    #[test]
    fn series_rules_are_checked() {
        let config = chart();
        assert_eq!(config.validate_view3d_series(&[heatmap(None)]), Ok(()));
        let err = |s: SeriesConfig| config.validate_view3d_series(&[s]).unwrap_err();
        let scatter = SeriesConfig {
            render_type: DataRenderType::Scatter {
                scatter: DataScatterStyleConfig {
                    point_color: Color::BLACK,
                    point_shape: ScatterShape::CircleFilled,
                    point_size: 3.0,
                    point_style_table: None,
                    point_style_index_column: None,
                    point_style_overrides: None,
                    point_color_map: None,
                    point_size_map: None,
                },
            },
            ..heatmap(None)
        };
        assert!(err(scatter.clone()).contains("z_column"));
        // With a height column the scatter draws in 3D (#132); a line too,
        // when linear and without a fill; an error bar series never.
        let mut scatter_3d = scatter.clone();
        scatter_3d.z_column = Some("z".into());
        assert_eq!(config.validate_view3d_series(&[scatter_3d.clone()]), Ok(()));
        let line = |line: crate::data_config::DataLineStyleConfig| SeriesConfig {
            render_type: DataRenderType::Line { line },
            ..scatter_3d.clone()
        };
        let plain = crate::data_config::DataLineStyleConfig::default();
        assert_eq!(
            config.validate_view3d_series(&[line(plain.clone())]),
            Ok(())
        );
        let stepped = crate::data_config::DataLineStyleConfig {
            interpolation: LineInterpolation::StepPost,
            ..plain.clone()
        };
        assert!(err(line(stepped)).contains("linear"));
        let filled = crate::data_config::DataLineStyleConfig {
            fill: Some(crate::data_config::LineFill::ToBaseline(0.0)),
            ..plain
        };
        assert!(err(line(filled)).contains("area fill"));
        let bars = SeriesConfig {
            render_type: DataRenderType::ScatterErrorbarY {
                scatter: match &scatter.render_type {
                    DataRenderType::Scatter { scatter } => scatter.clone(),
                    _ => unreachable!(),
                },
                err_y: crate::data_config::ErrorRef::Symmetric { column: "e".into() },
                err_style: crate::data_config::DataErrorBarStyleConfig {
                    error_bar_color: Color::BLACK,
                    error_bar_width: 1.0,
                    error_bar_cap_size: 3.0,
                    cap_width: 1.0,
                    error_bar_style_table: None,
                    error_bar_style_index_column: None,
                    error_bar_style_overrides: None,
                },
            },
            ..scatter_3d.clone()
        };
        assert!(err(bars).contains("only heatmaps"));
        // A flat chart draws the scatter and keeps a heatmap's surface style
        // and a series' z column.
        let flat = crate::default::default_config();
        assert_eq!(
            flat.validate_view3d_series(&[
                scatter,
                scatter_3d,
                heatmap(Some(SurfaceStyle::default()))
            ]),
            Ok(())
        );
        let mut banded = heatmap(None);
        if let DataRenderType::Heatmap { fill, .. } = &mut banded.render_type {
            fill.mode = FillMode::Bands;
        }
        assert!(err(banded).contains("continuous"));
        let wrong = SurfaceStyle {
            color_matrix: Some(matrix(2)),
            ..SurfaceStyle::default()
        };
        assert!(err(heatmap(Some(wrong))).contains("color_matrix"));
        let bright = SurfaceStyle {
            gloss: 2.0,
            ..SurfaceStyle::default()
        };
        assert!(err(heatmap(Some(bright))).contains("gloss"));
        let contour = ContourConfig {
            levels: vec![1.0],
            line: crate::data_config::DataLineStyleConfig {
                line_style: crate::line::LineStylePreset::Solid,
                line_color: Color::BLACK,
                line_width: 1.0,
                fill: None,
                fill_color: None,
                interpolation: crate::data_config::LineInterpolation::Linear,
            },
            per_level_color: None,
            labels: Some(ContourLabelConfig {
                visible: true,
                font_size: 11.0,
                color: Color::BLACK,
                format: crate::format::LabelFormat::Decimal,
                significant_digits: 3,
                spacing_px: 120.0,
                anchors: Vec::new(),
                bg_color: None,
                bg_padding_px: 0.0,
            }),
        };
        let labelled = SeriesConfig {
            render_type: DataRenderType::Contour {
                matrix: matrix(3),
                contour,
            },
            ..heatmap(None)
        };
        assert!(err(labelled).contains("no labels"));
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serialization_round_trips_and_omits_unset_keys() {
        let config = chart();
        let json = serde_json::to_string(&config).unwrap();
        assert!(json.contains("\"view_3d\":{"), "{json}");
        assert!(!json.contains("box_aspect"), "{json}");
        let back: Config = serde_json::from_str(&json).unwrap();
        assert_eq!(back.view_3d, config.view_3d);
        let flat = crate::default::default_config();
        assert!(!serde_json::to_string(&flat).unwrap().contains("view_3d"));
        let plain = heatmap(None);
        let json = serde_json::to_string(&plain).unwrap();
        assert!(
            !json.contains("surface"),
            "an older heatmap stays as it was: {json}"
        );
        let styled = heatmap(Some(SurfaceStyle {
            wireframe: Some(SurfaceWireframe {
                color: Color::BLACK,
                width: 1.0,
            }),
            ..SurfaceStyle::default()
        }));
        let json = serde_json::to_string(&styled).unwrap();
        assert!(
            json.contains("\"wireframe\"") && !json.contains("color_matrix"),
            "{json}"
        );
        let back: SeriesConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back, styled);
        let partial: View3d = serde_json::from_str("{\"elevation_degrees\": 10.0}").unwrap();
        assert_eq!(partial.azimuth_degrees, -60.0);
    }
}
