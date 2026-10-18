//! Polar coordinates for the Cartesian chart: `Config.polar`.
//!
//! With `polar: Some(..)` the document keeps its four axes and every series,
//! column and interaction contract, and draws `bottom_x` as the angle and
//! `left_y` as the radius. `None` (the default, omitted from JSON) is the
//! rectangular chart every document had before the field.

use crate::config::{AxisScale, Config, LabelStyle};

/// The most pieces one segment is subdivided into under
/// [`PolarInterpolation::Arc`].
pub const MAX_POLAR_ARC_PIECES: u32 = 64;

/// How the angle column is read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum AngleUnit {
    #[default]
    Degrees,
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
    Arc { pieces: u32 },
}

impl PolarInterpolation {
    /// The pieces one segment is drawn as; a chord is one piece.
    pub fn piece_count(self) -> u32 {
        match self {
            Self::Chord => 1,
            Self::Arc { pieces } => pieces.max(1),
        }
    }

    pub fn is_chord(&self) -> bool {
        matches!(self, Self::Chord)
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
        if !config.overlays.is_empty() {
            return Err("polar charts have no overlay layers");
        }
        Ok(())
    }
}

impl Config {
    /// The polar circle of this chart, when the chart is polar and has a
    /// data area. Same arithmetic on every consumer: the GPU transform, the
    /// grid raster, the CPU picker and the annotation space.
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
        // side; the tick marks point outward from the ring.
        let band = axis.major_tick_length.max(0.0)
            + if labels.visible && labels.label_visible {
                labels.font_size.max(0.0) * 1.4 + 6.0
            } else {
                2.0
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
        assert_eq!(PolarInterpolation::Chord.piece_count(), 1);
        assert_eq!(PolarInterpolation::Arc { pieces: 8 }.piece_count(), 8);
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
        });
        let text = serde_json::to_string(&polar).unwrap();
        let back: Config = serde_json::from_str(&text).unwrap();
        assert_eq!(back, polar);
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
