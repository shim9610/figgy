//! Polar coordinates for the Cartesian chart: `Config.polar`.
//!
//! With `polar: Some(..)` the chart draws `bottom_x` as the angle (the outer
//! ring) and `left_y` as the radius (the labelled ray); `top_x` and
//! `right_y` stay in the document but are not drawn. Scatter, line and
//! scatter-line series, with their fills, keep their columns, picking and
//! export, and radar series draw their polygons. Histogram, field,
//! contour, error-bar, vector, category bar and box plot series, overlay
//! layers and streamed sources are rejected with an error. `None` (the
//! default, omitted from JSON) draws the rectangular chart.
//!
//! A categorical angle axis (`bottom_x.categories`) puts category `i` at
//! `i / n` of the full turn, or of the sector, from `zero_angle_degrees` in
//! the `clockwise` direction ([`PolarOptions::angle_map`]): its spokes are
//! the categories and its labels the category labels.

use crate::config::{AxisOptions, AxisScale, Config, LabelStyle};

/// The most pieces one segment is subdivided into under
/// [`PolarInterpolation::Arc`].
pub const MAX_POLAR_ARC_PIECES: u32 = 64;

/// How the angle column is read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum AngleUnit {
    /// Degrees, 360 per turn (the default).
    #[default]
    Degrees,
    /// Radians, 2π per turn.
    Radians,
}

impl AngleUnit {
    /// The factor from a value in this unit to radians.
    pub fn radians_per_unit(self) -> f64 {
        match self {
            Self::Degrees => std::f64::consts::PI / 180.0,
            Self::Radians => 1.0,
        }
    }
}

/// How a line segment between two polar samples is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum PolarInterpolation {
    /// A straight segment on screen between the two projected samples.
    #[default]
    Chord,
    /// `pieces` straight segments between points interpolated linearly in
    /// (angle, radius): a segment at constant radius follows the ring. Each
    /// piece is one strip of the line, so dashes and picking walk the same
    /// path. 1 to [`MAX_POLAR_ARC_PIECES`].
    Arc {
        /// Chords per segment.
        pieces: u32,
    },
}

impl PolarInterpolation {
    /// Whether segments are straight chords (the default).
    pub fn is_chord(&self) -> bool {
        matches!(self, Self::Chord)
    }
}

/// The shape of a polar chart's rings: the radius grid and the outer ring.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum GridShape {
    /// Circles (the default).
    #[default]
    Circle,
    /// Regular polygons through the spokes of a categorical angle axis: each
    /// ring joins its points at the category angles with straight chords
    /// (the radar look). Needs a categorical angle axis.
    Polygon,
}

/// How an angle-axis value maps to a screen angle
/// ([`PolarOptions::angle_map`]): the one mapping the GPU transform, the
/// polar grid, the angle labels and the picker share.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AngleMap {
    /// The screen angle of value 0, in radians counter-clockwise from the
    /// right with y up.
    pub zero_radians: f64,
    /// Radians per angle-axis value, negative when values grow clockwise.
    pub radians_per_value: f64,
    /// The visible window `[start, end]` in angle-axis values, `None` for the
    /// full circle.
    pub window: Option<[f64; 2]>,
}

impl AngleMap {
    /// The screen angle of `value`, in radians counter-clockwise from the
    /// right with y up.
    pub fn screen_angle(&self, value: f64) -> f64 {
        self.zero_radians + self.radians_per_value * value
    }

    /// +1 when values grow counter-clockwise, -1 clockwise.
    pub fn direction(&self) -> f64 {
        if self.radians_per_value < 0.0 {
            -1.0
        } else {
            1.0
        }
    }

    /// Angle-axis values per full turn.
    pub fn values_per_turn(&self) -> f64 {
        std::f64::consts::TAU / self.radians_per_value.abs()
    }

    /// Whether `value` is inside the visible window.
    pub fn contains(&self, value: f64) -> bool {
        match self.window {
            None => value.is_finite(),
            Some([start, end]) => value >= start && value <= end,
        }
    }
}

/// The polar coordinate system of a `Config`. Screen angles are measured
/// counter-clockwise from the right, in degrees.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct PolarOptions {
    /// Where the angle value 0 points, in screen degrees: 0 right, 90 up.
    pub zero_angle_degrees: f64,
    /// Whether angle values grow clockwise on screen. `false` (the default)
    /// is the mathematical convention: counter-clockwise.
    pub clockwise: bool,
    /// The unit of the angle column and of `sector`.
    pub angle_unit: AngleUnit,
    /// The visible angular window `[start, end]` in `angle_unit`, at most one
    /// turn; drawing is clipped at its edges. `None` draws the full circle,
    /// where an angle outside the axis range lands where `cos` / `sin` put
    /// it: the renderer neither wraps nor sorts samples.
    pub sector: Option<[f64; 2]>,
    /// The radius of the hole at the centre as a fraction of the outer
    /// radius, `0.0..=0.9`. `left_y.min` maps to this ring.
    pub inner_radius_fraction: f32,
    /// How a segment between two samples is drawn.
    pub interpolation: PolarInterpolation,
    /// The screen direction of the ray the radial tick labels and the radial
    /// axis line sit on, in degrees counter-clockwise from the right.
    /// Dragging the ray (`Element::Axis(Side::Left)`) turns it.
    pub radial_label_angle_degrees: f64,
    /// The shape of the rings. `None` (the default, omitted from JSON)
    /// draws circles; [`GridShape::Polygon`] joins them at the categories
    /// of a categorical angle axis.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub grid_shape: Option<GridShape>,
}

impl Default for PolarOptions {
    fn default() -> Self {
        Self {
            zero_angle_degrees: 0.0,
            clockwise: false,
            angle_unit: AngleUnit::Degrees,
            sector: None,
            inner_radius_fraction: 0.0,
            interpolation: PolarInterpolation::Chord,
            radial_label_angle_degrees: 90.0,
            grid_shape: None,
        }
    }
}

/// The circle a polar chart draws into, in the chart's pixel space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PolarGeometry {
    /// The centre of the data area.
    pub center: [f32; 2],
    /// The outer ring: the largest circle inside the data area that leaves
    /// one band of angular tick labels inside the area.
    pub radius: f32,
    /// The hole: `radius * inner_radius_fraction`.
    pub inner_radius: f32,
}

impl PolarGeometry {
    /// The unit direction from the centre to `at`, or `None` within a pixel
    /// of the centre.
    pub fn spoke_direction(&self, at: [f32; 2]) -> Option<[f32; 2]> {
        let (dx, dy) = (at[0] - self.center[0], at[1] - self.center[1]);
        let len = dx.hypot(dy);
        (len >= 1.0).then(|| [dx / len, dy / len])
    }

    /// The screen angle of `at` around the centre in radians, counter-
    /// clockwise from the right with y up, or `None` within a pixel of the
    /// centre.
    pub fn screen_angle_of(&self, at: [f32; 2]) -> Option<f64> {
        let d = self.spoke_direction(at)?;
        Some(f64::from(-d[1]).atan2(f64::from(d[0])))
    }
}

impl PolarOptions {
    /// The screen unit vector of the radius axis ray, from the centre
    /// outward: `radial_label_angle_degrees` with y down.
    pub fn ray_direction(&self) -> [f32; 2] {
        let a = self.radial_label_angle_degrees.to_radians();
        [a.cos() as f32, -(a.sin() as f32)]
    }

    /// The unit vector across the ray on the side its tick labels sit:
    /// counter-clockwise from the ray on screen.
    pub fn ray_side(&self) -> [f32; 2] {
        let d = self.ray_direction();
        [d[1], -d[0]]
    }

    /// Turn the ray by `degrees` counter-clockwise, keeping the angle in
    /// `0..360`.
    pub fn turn_ray(&mut self, degrees: f64) {
        self.radial_label_angle_degrees =
            (self.radial_label_angle_degrees + degrees).rem_euclid(360.0);
    }

    /// The screen offset of the radius tick labels. In polar mode
    /// `left_y.label_style.label_offset_x` is read along the ray (outward
    /// when positive) and `label_offset_y` across it (toward the label side
    /// when positive), so a dragged offset follows the ray when it turns.
    pub fn radius_label_offset(&self, labels: &LabelStyle) -> [f32; 2] {
        let d = self.ray_direction();
        let s = self.ray_side();
        [
            d[0] * labels.label_offset_x + s[0] * labels.label_offset_y,
            d[1] * labels.label_offset_x + s[1] * labels.label_offset_y,
        ]
    }

    /// The radial offset of the angle tick labels. In polar mode
    /// `bottom_x.label_style.label_offset_y` moves every label along its own
    /// spoke, outward when positive; `label_offset_x` has no effect.
    pub fn angle_label_offset(labels: &LabelStyle) -> f32 {
        labels.label_offset_y
    }

    /// The screen angle of an angle-column `value`, in radians,
    /// counter-clockwise from the right with y up.
    pub fn screen_angle(&self, value: f64) -> f64 {
        let direction = if self.clockwise { -1.0 } else { 1.0 };
        self.zero_angle_degrees.to_radians()
            + direction * value * self.angle_unit.radians_per_unit()
    }

    /// The direction the angle value grows in: +1 counter-clockwise, -1
    /// clockwise.
    pub fn direction(&self) -> f64 {
        if self.clockwise { -1.0 } else { 1.0 }
    }

    /// The sector bounds in radians of the angle column, `None` for the full
    /// circle.
    pub fn sector_radians(&self) -> Option<[f64; 2]> {
        let unit = self.angle_unit.radians_per_unit();
        self.sector.map(|[start, end]| [start * unit, end * unit])
    }

    /// Whether an angle-column `value` is inside the visible window.
    pub fn contains_angle(&self, value: f64) -> bool {
        match self.sector {
            None => value.is_finite(),
            Some([start, end]) => value >= start && value <= end,
        }
    }

    /// The ring shape: `grid_shape`, or circles.
    pub fn grid_shape(&self) -> GridShape {
        self.grid_shape.unwrap_or_default()
    }

    /// How the values of `angle_axis` (the chart's `bottom_x`) map to screen
    /// angles. A numeric axis reads its values in `angle_unit` from
    /// `zero_angle_degrees` in the `clockwise` direction, inside `sector`.
    /// A categorical axis with `n` categories puts category `i` at `i / n`
    /// of the full turn from `zero_angle_degrees`, or at `i / n` of the
    /// sector from its start; its window is then `[0, n]` in category
    /// values.
    pub fn angle_map(&self, angle_axis: &AxisOptions) -> AngleMap {
        let zero = self.zero_angle_degrees.to_radians();
        let unit = self.angle_unit.radians_per_unit();
        let direction = self.direction();
        match angle_axis.category_count() {
            None => AngleMap {
                zero_radians: zero,
                radians_per_value: direction * unit,
                window: self.sector,
            },
            Some(n) => {
                let n = n.max(1) as f64;
                match self.sector {
                    None => AngleMap {
                        zero_radians: zero,
                        radians_per_value: direction * std::f64::consts::TAU / n,
                        window: None,
                    },
                    Some([start, end]) => AngleMap {
                        zero_radians: zero + direction * start * unit,
                        radians_per_value: direction * (end - start) * unit / n,
                        window: Some([0.0, n]),
                    },
                }
            }
        }
    }

    /// Validate against the axes the coordinate system reads.
    pub fn validate(&self, config: &Config) -> Result<(), &'static str> {
        if !self.zero_angle_degrees.is_finite() || !self.radial_label_angle_degrees.is_finite() {
            return Err("polar angles must be finite");
        }
        if !self.inner_radius_fraction.is_finite()
            || !(0.0..=0.9).contains(&self.inner_radius_fraction)
        {
            return Err("polar inner_radius_fraction must be within 0..=0.9");
        }
        if let Some([start, end]) = self.sector {
            if !start.is_finite() || !end.is_finite() || end <= start {
                return Err("polar sector must be a finite [start, end] with start < end");
            }
            if (end - start) * self.angle_unit.radians_per_unit()
                > std::f64::consts::TAU * (1.0 + 1e-9)
            {
                return Err("polar sector cannot exceed one turn");
            }
        }
        if let PolarInterpolation::Arc { pieces } = self.interpolation
            && !(1..=MAX_POLAR_ARC_PIECES).contains(&pieces)
        {
            return Err("polar arc pieces must be within 1..=64");
        }
        if config.bottom_x.scale != AxisScale::Linear {
            return Err("the polar angle axis (bottom_x) must be linear");
        }
        if config.bottom_x.inverted {
            return Err("the polar angle axis (bottom_x) cannot be inverted; use clockwise");
        }
        if self.grid_shape() == GridShape::Polygon && config.bottom_x.categories.is_none() {
            return Err(
                "a polygon polar grid joins its rings at the categories: it needs a \
                 categorical angle axis (bottom_x.categories)",
            );
        }
        if config.left_y.categories.is_some() {
            return Err("the polar radius axis (left_y) cannot be categorical");
        }
        if !config.overlays.is_empty() {
            return Err("polar charts have no overlay layers");
        }
        Ok(())
    }
}

impl Config {
    /// How this polar chart's angle axis maps to screen angles
    /// ([`PolarOptions::angle_map`] of `bottom_x`), `None` for a rectangular
    /// chart.
    pub fn polar_angle_map(&self) -> Option<AngleMap> {
        Some(self.polar.as_ref()?.angle_map(&self.bottom_x))
    }

    /// The polar circle of this chart, when the chart is polar and has a
    /// data area. The GPU transform, the polar grid and axis raster, the CPU
    /// picker and the decoration hit regions and drags all read this circle.
    /// The annotation layer does not: its ratio anchors stay relative to the
    /// data area and its data anchors are skipped.
    pub fn polar_geometry(&self) -> Option<PolarGeometry> {
        self.polar.as_ref()?;
        let area = self.data_area().ok()?.0;
        let center = [
            area.x as f32 + area.width as f32 * 0.5,
            area.y as f32 + area.height as f32 * 0.5,
        ];
        let half = (area.width.min(area.height) as f32) * 0.5;
        let axis = &self.bottom_x;
        let labels = &axis.label_style;
        // One band of angular tick labels fits inside the data area on every
        // side; the tick marks point outward from the ring. Every term of the
        // band is a multiple of a size `Config::scaled` scales (the tick
        // length and the label font size), so a 2x export is the 1x layout
        // doubled. With labels the band holds a line of 1.4 font sizes plus a
        // third of one as clearance (6 px at the default 18 px); without them,
        // a ninth of the font size (2 px at 18 px).
        let font_size = labels.font_size.max(0.0);
        let band = axis.major_tick_length.max(0.0)
            + if labels.visible && labels.label_visible {
                font_size * 1.4 + font_size / 3.0
            } else {
                font_size / 9.0
            };
        let radius = (half - band).max(0.0);
        Some(PolarGeometry {
            center,
            radius,
            inner_radius: radius * self.polar.as_ref()?.inner_radius_fraction,
        })
    }

    /// Validate `polar` against the axes it reads; `None` always passes.
    pub fn validate_polar(&self) -> Result<(), &'static str> {
        match &self.polar {
            Some(polar) => polar.validate(self),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::default::default_config;

    fn polar_config() -> Config {
        let mut config = default_config();
        config.bottom_x.min = 0.0;
        config.bottom_x.max = 360.0;
        config.left_y.min = 0.0;
        config.left_y.max = 10.0;
        config.polar = Some(PolarOptions::default());
        config
    }

    #[test]
    fn screen_angles_follow_zero_direction_and_unit() {
        let mut polar = PolarOptions::default();
        assert!((polar.screen_angle(90.0) - std::f64::consts::FRAC_PI_2).abs() < 1e-12);
        polar.clockwise = true;
        polar.zero_angle_degrees = 90.0;
        assert!((polar.screen_angle(90.0) - 0.0).abs() < 1e-12);
        polar.angle_unit = AngleUnit::Radians;
        assert!(
            (polar.screen_angle(std::f64::consts::FRAC_PI_2) - 0.0).abs() < 1e-12,
            "radians are read as is"
        );
        polar.sector = Some([0.0, 1.0]);
        assert!(polar.contains_angle(0.5));
        assert!(!polar.contains_angle(1.5));
        assert_eq!(polar.sector_radians(), Some([0.0, 1.0]));
    }

    #[test]
    fn validation_rejects_every_unsupported_combination() {
        let config = polar_config();
        assert_eq!(config.validate_polar(), Ok(()));
        let mut c = config.clone();
        c.polar.as_mut().unwrap().inner_radius_fraction = 0.95;
        assert!(c.validate_polar().is_err());
        let mut c = config.clone();
        c.polar.as_mut().unwrap().sector = Some([10.0, 10.0]);
        assert!(c.validate_polar().is_err());
        let mut c = config.clone();
        c.polar.as_mut().unwrap().sector = Some([0.0, 400.0]);
        assert!(c.validate_polar().is_err());
        let mut c = config.clone();
        c.polar.as_mut().unwrap().sector = Some([-90.0, 90.0]);
        assert_eq!(c.validate_polar(), Ok(()));
        let mut c = config.clone();
        c.polar.as_mut().unwrap().interpolation = PolarInterpolation::Arc { pieces: 0 };
        assert!(c.validate_polar().is_err());
        c.polar.as_mut().unwrap().interpolation = PolarInterpolation::Arc { pieces: 65 };
        assert!(c.validate_polar().is_err());
        let mut c = config.clone();
        c.bottom_x.scale = AxisScale::Logarithmic;
        c.bottom_x.min = 1.0;
        assert!(c.validate_polar().is_err());
        let mut c = config.clone();
        c.bottom_x.inverted = true;
        assert!(c.validate_polar().is_err());
        let category = || Some(vec![crate::config::Category::new("a", "A")]);
        let mut c = config.clone();
        c.bottom_x.categories = category();
        assert_eq!(
            c.validate_polar(),
            Ok(()),
            "a categorical angle axis is valid"
        );
        c.polar.as_mut().unwrap().grid_shape = Some(GridShape::Polygon);
        assert_eq!(c.validate_polar(), Ok(()));
        let mut c = config.clone();
        c.polar.as_mut().unwrap().grid_shape = Some(GridShape::Polygon);
        assert!(
            c.validate_polar()
                .unwrap_err()
                .contains("needs a categorical angle axis")
        );
        c.polar.as_mut().unwrap().grid_shape = Some(GridShape::Circle);
        assert_eq!(c.validate_polar(), Ok(()));
        let mut c = config.clone();
        c.left_y.categories = category();
        assert_eq!(
            c.validate_polar(),
            Err("the polar radius axis (left_y) cannot be categorical")
        );
        let mut c = config.clone();
        c.overlays.push(crate::layer::OverlayLayer::default());
        assert!(c.validate_polar().is_err());
        let mut c = config.clone();
        c.polar.as_mut().unwrap().zero_angle_degrees = f64::NAN;
        assert!(c.validate_polar().is_err());
        let mut plain = default_config();
        plain.polar = None;
        plain.bottom_x.inverted = true;
        assert_eq!(plain.validate_polar(), Ok(()));
    }

    #[test]
    fn a_numeric_angle_axis_maps_as_screen_angle_does() {
        let config = polar_config();
        let mut polar = PolarOptions {
            zero_angle_degrees: 30.0,
            clockwise: true,
            sector: Some([-45.0, 200.0]),
            ..PolarOptions::default()
        };
        let map = polar.angle_map(&config.bottom_x);
        for value in [-45.0, 0.0, 12.5, 200.0] {
            assert_eq!(map.screen_angle(value), polar.screen_angle(value));
            assert_eq!(map.contains(value), polar.contains_angle(value));
        }
        assert_eq!(map.window, polar.sector);
        assert_eq!(map.direction(), -1.0);
        assert!((map.values_per_turn() - 360.0).abs() < 1e-9);
        polar.angle_unit = AngleUnit::Radians;
        polar.sector = None;
        let map = polar.angle_map(&config.bottom_x);
        assert_eq!(map.radians_per_value, -1.0);
        assert_eq!(map.window, None);
    }

    #[test]
    fn a_categorical_angle_axis_puts_category_i_at_i_over_n_of_the_turn() {
        let mut config = polar_config();
        config.bottom_x.categories = Some(
            ["a", "b", "c", "d"]
                .map(|id| crate::config::Category::new(id, id))
                .to_vec(),
        );
        let mut polar = PolarOptions {
            zero_angle_degrees: 90.0,
            clockwise: true,
            ..PolarOptions::default()
        };
        let quarter = std::f64::consts::FRAC_PI_2;
        let map = polar.angle_map(&config.bottom_x);
        // Category 0 points up, the others follow clockwise a quarter apart.
        for i in 0..4 {
            assert!((map.screen_angle(i as f64) - (quarter - i as f64 * quarter)).abs() < 1e-12);
        }
        assert_eq!(map.window, None);
        assert!((map.values_per_turn() - 4.0).abs() < 1e-12);
        // Counter-clockwise from the right.
        polar.zero_angle_degrees = 0.0;
        polar.clockwise = false;
        let map = polar.angle_map(&config.bottom_x);
        assert!((map.screen_angle(1.0) - quarter).abs() < 1e-12);
        // A sector of 180 degrees from 90: i / n of the sector from its
        // start, the window [0, n] in category values.
        polar.sector = Some([90.0, 270.0]);
        let map = polar.angle_map(&config.bottom_x);
        assert!((map.screen_angle(0.0) - quarter).abs() < 1e-12);
        assert!((map.screen_angle(2.0) - (quarter + std::f64::consts::FRAC_PI_2)).abs() < 1e-12);
        assert!((map.screen_angle(4.0) - 3.0 * quarter).abs() < 1e-12);
        assert_eq!(map.window, Some([0.0, 4.0]));
        assert!(map.contains(3.0) && !map.contains(4.5));
        // The angle unit reads the sector, not the categories.
        polar.angle_unit = AngleUnit::Radians;
        polar.sector = Some([0.0, std::f64::consts::PI]);
        let map = polar.angle_map(&config.bottom_x);
        assert!((map.radians_per_value - std::f64::consts::PI / 4.0).abs() < 1e-12);
        assert_eq!(
            config.polar_angle_map(),
            Some(PolarOptions::default().angle_map(&config.bottom_x))
        );
    }

    #[test]
    fn geometry_is_the_inscribed_circle_minus_the_label_band() {
        let config = polar_config();
        let area = config.data_area().unwrap().0;
        let geometry = config.polar_geometry().unwrap();
        assert_eq!(
            geometry.center,
            [
                area.x as f32 + area.width as f32 * 0.5,
                area.y as f32 + area.height as f32 * 0.5
            ]
        );
        let half = area.width.min(area.height) as f32 * 0.5;
        assert!(geometry.radius < half && geometry.radius > half * 0.6);
        let mut hidden = config.clone();
        hidden.bottom_x.label_style.visible = false;
        assert!(hidden.polar_geometry().unwrap().radius > geometry.radius);
        let mut plain = config;
        plain.polar = None;
        assert_eq!(plain.polar_geometry(), None);
    }

    #[test]
    fn geometry_at_2x_is_the_1x_geometry_doubled() {
        // `Config::scaled` scales the tick length and the label font size,
        // which are every term of the label band, so the band doubles
        // exactly; the centre and radius also follow the data area, whose
        // edges are floored at each scale.
        for font_size in [18.0, 9.0, 12.0, 16.0] {
            for labels_visible in [true, false] {
                let mut config = polar_config();
                config.bottom_x.label_style.font_size = font_size;
                config.bottom_x.label_style.visible = labels_visible;
                let one = config.polar_geometry().unwrap();
                let scaled = config.scaled(2.0);
                let two = scaled.polar_geometry().unwrap();
                let half = |c: &Config| {
                    let area = c.data_area().unwrap().0;
                    area.width.min(area.height) as f32 * 0.5
                };
                let band_one = half(&config) - one.radius;
                let band_two = half(&scaled) - two.radius;
                assert!(
                    (band_two - 2.0 * band_one).abs() < 1e-4,
                    "font {font_size}, labels {labels_visible}: band {band_two} at 2x, \
                     {band_one} at 1x"
                );
                for (a, b) in [
                    (two.center[0], one.center[0]),
                    (two.center[1], one.center[1]),
                    (two.radius, one.radius),
                ] {
                    assert!(
                        (a - 2.0 * b).abs() <= 1.0,
                        "font {font_size}, labels {labels_visible}: {a} at 2x, {b} at 1x"
                    );
                }
            }
        }
    }

    #[test]
    fn the_default_label_band_is_unchanged() {
        // At the default 18 px label font the band's clearance is the 6 px
        // (2 px without labels) of earlier releases, to the bit.
        let config = polar_config();
        assert_eq!(config.bottom_x.label_style.font_size, 18.0);
        let area = config.data_area().unwrap().0;
        let half = area.width.min(area.height) as f32 * 0.5;
        let tick = config.bottom_x.major_tick_length.max(0.0);
        assert_eq!(
            config.polar_geometry().unwrap().radius,
            half - (tick + (18.0f32 * 1.4 + 6.0))
        );
        let mut hidden = config.clone();
        hidden.bottom_x.label_style.visible = false;
        assert_eq!(hidden.polar_geometry().unwrap().radius, half - (tick + 2.0));
    }

    #[cfg(feature = "serde")]
    #[test]
    fn polar_is_additive_in_json() {
        let plain = default_config();
        let json = serde_json::to_value(&plain).unwrap();
        assert!(json.get("polar").is_none());
        let back: Config = serde_json::from_value(json).unwrap();
        assert_eq!(back, plain);
        let mut polar = polar_config();
        polar.polar = Some(PolarOptions {
            zero_angle_degrees: 90.0,
            clockwise: true,
            angle_unit: AngleUnit::Radians,
            sector: Some([0.0, 3.0]),
            inner_radius_fraction: 0.2,
            interpolation: PolarInterpolation::Arc { pieces: 12 },
            radial_label_angle_degrees: 45.0,
            grid_shape: Some(GridShape::Polygon),
        });
        let text = serde_json::to_string(&polar).unwrap();
        let back: Config = serde_json::from_str(&text).unwrap();
        assert_eq!(back, polar);
        assert!(text.contains(r#""grid_shape":"Polygon""#), "{text}");
        // The default grid shape is omitted, so earlier documents keep
        // their bytes.
        let default_text = serde_json::to_string(&PolarOptions::default()).unwrap();
        assert!(!default_text.contains("grid_shape"), "{default_text}");
        let circle: PolarOptions =
            serde_json::from_value(serde_json::json!({ "grid_shape": "Circle" })).unwrap();
        assert_eq!(circle.grid_shape, Some(GridShape::Circle));
        assert_eq!(circle.grid_shape(), GridShape::Circle);
        let partial: PolarOptions =
            serde_json::from_value(serde_json::json!({ "clockwise": true })).unwrap();
        assert_eq!(
            partial,
            PolarOptions {
                clockwise: true,
                ..Default::default()
            }
        );
    }
}
