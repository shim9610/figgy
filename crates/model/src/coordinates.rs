//! The coordinate system a chart draws in (#160): rectangular, polar,
//! ternary, a 3D box or parallel axes, exactly one of them.
//!
//! Documents write it as they always have: at most one of the keys
//! `polar`, `ternary`, `view_3d` and `parallel`, at their place among the
//! chart's keys, and none for a rectangular chart. A document naming two is
//! refused when it is read.

use crate::parallel::ParallelOptions;
use crate::polar::PolarOptions;
use crate::ternary::TernaryOptions;
use crate::view3d::View3d;

/// The coordinate system of a [`Config`](crate::config::Config).
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Coordinates {
    /// The rectangular chart: `bottom_x` / `top_x` across, `left_y` /
    /// `right_y` up.
    #[default]
    Cartesian,
    /// `bottom_x` as the angle axis and `left_y` as the radius axis. See
    /// [`crate::polar`].
    Polar(PolarOptions),
    /// A triangle whose corners are the 100 % of the three components read
    /// on `bottom_x`, `left_y` and `right_y`. See [`crate::ternary`].
    Ternary(TernaryOptions),
    /// A box seen through this camera, with x on `bottom_x`, y on `left_y`
    /// and z on `right_y`. See [`crate::view3d`].
    View3d(View3d),
    /// One vertical axis per category of `bottom_x`, each with its own
    /// range (#141). See [`crate::parallel`].
    Parallel(ParallelOptions),
}

impl Coordinates {
    /// The polar options, on a polar chart.
    pub fn polar(&self) -> Option<&PolarOptions> {
        match self {
            Self::Polar(polar) => Some(polar),
            _ => None,
        }
    }

    /// The ternary options, on a ternary chart.
    pub fn ternary(&self) -> Option<&TernaryOptions> {
        match self {
            Self::Ternary(ternary) => Some(ternary),
            _ => None,
        }
    }

    /// The 3D view, on a 3D chart.
    pub fn view_3d(&self) -> Option<&View3d> {
        match self {
            Self::View3d(view) => Some(view),
            _ => None,
        }
    }

    /// The parallel options, on a parallel chart.
    pub fn parallel(&self) -> Option<&ParallelOptions> {
        match self {
            Self::Parallel(parallel) => Some(parallel),
            _ => None,
        }
    }

    /// The system named by the first of the four options that is set, or
    /// the rectangular chart: the form documents are written in.
    pub fn from_options(
        polar: Option<PolarOptions>,
        ternary: Option<TernaryOptions>,
        view_3d: Option<View3d>,
        parallel: Option<ParallelOptions>,
    ) -> Self {
        match (polar, ternary, view_3d, parallel) {
            (Some(polar), ..) => Self::Polar(polar),
            (_, Some(ternary), ..) => Self::Ternary(ternary),
            (_, _, Some(view), _) => Self::View3d(view),
            (_, _, _, Some(parallel)) => Self::Parallel(parallel),
            _ => Self::Cartesian,
        }
    }

    /// Whether this is the rectangular chart.
    pub fn is_cartesian(&self) -> bool {
        matches!(self, Self::Cartesian)
    }
}

impl crate::config::Config {
    /// The polar options, on a polar chart.
    pub fn polar(&self) -> Option<&PolarOptions> {
        self.coordinates.polar()
    }

    /// The polar options to edit, on a polar chart.
    pub fn polar_mut(&mut self) -> Option<&mut PolarOptions> {
        match &mut self.coordinates {
            Coordinates::Polar(polar) => Some(polar),
            _ => None,
        }
    }

    /// Make this a polar chart with `polar`, or, with `None`, a rectangular
    /// chart if it is a polar one (another system stays).
    pub fn set_polar(&mut self, polar: Option<PolarOptions>) {
        match polar {
            Some(polar) => self.coordinates = Coordinates::Polar(polar),
            None if self.polar().is_some() => self.coordinates = Coordinates::Cartesian,
            None => {}
        }
    }

    /// The ternary options, on a ternary chart.
    pub fn ternary(&self) -> Option<&TernaryOptions> {
        self.coordinates.ternary()
    }

    /// The ternary options to edit, on a ternary chart.
    pub fn ternary_mut(&mut self) -> Option<&mut TernaryOptions> {
        match &mut self.coordinates {
            Coordinates::Ternary(ternary) => Some(ternary),
            _ => None,
        }
    }

    /// Make this a ternary chart with `ternary`, or, with `None`, a
    /// rectangular chart if it is a ternary one.
    pub fn set_ternary(&mut self, ternary: Option<TernaryOptions>) {
        match ternary {
            Some(ternary) => self.coordinates = Coordinates::Ternary(ternary),
            None if self.ternary().is_some() => self.coordinates = Coordinates::Cartesian,
            None => {}
        }
    }

    /// The 3D view, on a 3D chart.
    pub fn view_3d(&self) -> Option<&View3d> {
        self.coordinates.view_3d()
    }

    /// The 3D view to edit, on a 3D chart.
    pub fn view_3d_mut(&mut self) -> Option<&mut View3d> {
        match &mut self.coordinates {
            Coordinates::View3d(view) => Some(view),
            _ => None,
        }
    }

    /// Make this a 3D chart with `view`, or, with `None`, a rectangular
    /// chart if it is a 3D one.
    pub fn set_view_3d(&mut self, view: Option<View3d>) {
        match view {
            Some(view) => self.coordinates = Coordinates::View3d(view),
            None if self.view_3d().is_some() => self.coordinates = Coordinates::Cartesian,
            None => {}
        }
    }

    /// The parallel options, on a parallel chart.
    pub fn parallel(&self) -> Option<&ParallelOptions> {
        self.coordinates.parallel()
    }

    /// The parallel options to edit, on a parallel chart.
    pub fn parallel_mut(&mut self) -> Option<&mut ParallelOptions> {
        match &mut self.coordinates {
            Coordinates::Parallel(parallel) => Some(parallel),
            _ => None,
        }
    }

    /// Make this a parallel chart with `parallel`, or, with `None`, a
    /// rectangular chart if it is a parallel one.
    pub fn set_parallel(&mut self, parallel: Option<ParallelOptions>) {
        match parallel {
            Some(parallel) => self.coordinates = Coordinates::Parallel(parallel),
            None if self.parallel().is_some() => self.coordinates = Coordinates::Cartesian,
            None => {}
        }
    }
}

#[cfg(feature = "serde")]
mod wire {
    use super::*;

    /// The keys a document writes the coordinate system with.
    #[derive(serde::Serialize)]
    struct WireRef<'a> {
        #[serde(skip_serializing_if = "Option::is_none")]
        polar: Option<&'a PolarOptions>,
        #[serde(skip_serializing_if = "Option::is_none")]
        ternary: Option<&'a TernaryOptions>,
        #[serde(skip_serializing_if = "Option::is_none")]
        view_3d: Option<&'a View3d>,
        #[serde(skip_serializing_if = "Option::is_none")]
        parallel: Option<&'a ParallelOptions>,
    }

    #[derive(serde::Deserialize)]
    struct Wire {
        #[serde(default)]
        polar: Option<PolarOptions>,
        #[serde(default)]
        ternary: Option<TernaryOptions>,
        #[serde(default)]
        view_3d: Option<View3d>,
        #[serde(default)]
        parallel: Option<ParallelOptions>,
    }

    impl serde::Serialize for Coordinates {
        fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            WireRef {
                polar: self.polar(),
                ternary: self.ternary(),
                view_3d: self.view_3d(),
                parallel: self.parallel(),
            }
            .serialize(serializer)
        }
    }

    impl<'de> serde::Deserialize<'de> for Coordinates {
        fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
            let wire = Wire::deserialize(deserializer)?;
            let named = [
                wire.polar.is_some(),
                wire.ternary.is_some(),
                wire.view_3d.is_some(),
                wire.parallel.is_some(),
            ]
            .into_iter()
            .filter(|named| *named)
            .count();
            if named > 1 {
                return Err(serde::de::Error::custom(
                    "a chart has one coordinate system: name at most one of polar, ternary, \
                     view_3d and parallel",
                ));
            }
            Ok(
                match (wire.polar, wire.ternary, wire.view_3d, wire.parallel) {
                    (Some(polar), ..) => Coordinates::Polar(polar),
                    (_, Some(ternary), ..) => Coordinates::Ternary(ternary),
                    (_, _, Some(view), _) => Coordinates::View3d(view),
                    (_, _, _, Some(parallel)) => Coordinates::Parallel(parallel),
                    _ => Coordinates::Cartesian,
                },
            )
        }
    }
}
