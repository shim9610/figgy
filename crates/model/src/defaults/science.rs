//! Energy-level and Feynman diagrams.

#![allow(unused_imports)]
// Some sources are read only by `Default` (`crate::helpers`).
#![cfg_attr(not(feature = "helpers"), allow(dead_code))]

use super::*;
use crate::energy_levels::*;
use crate::feynman::*;
use crate::format::*;
use crate::layout::*;
use crate::line::*;
use crate::text::*;
use crate::*;

/// The documented default of [`EnergyLevelSeries`]: the value a document that
/// omits it takes.
pub(crate) fn energy_level_series() -> EnergyLevelSeries {
    EnergyLevelSeries {
        levels: Vec::new(),
        limits: Vec::new(),
        transitions: Vec::new(),
        bands: Vec::new(),
        style: energy_level_style(),
        show: level_view(),
        source: None,
    }
}

/// The documented default of [`EnergyLevelStyle`]: the value a document that
/// omits it takes.
pub(crate) fn energy_level_style() -> EnergyLevelStyle {
    EnergyLevelStyle {
        level: level_stroke_style(),
        limit: AnnotationLineStyle {
            color: Color::from_rgb8(60, 60, 140),
            width: 1.5,
            style: crate::line::LineStylePreset::Dash,
        },
        transition: transition_arrow_style(),
        label_side: level_label_side(),
        picked_width_scale: energy_level_picked_width_scale(),
        band: level_band_style(),
        wavelength_labels: None,
    }
}

/// The documented default of [`FeynmanLayout`]: the value a document that
/// omits it takes.
pub(crate) fn feynman_layout() -> FeynmanLayout {
    FeynmanLayout {
        direction: time_direction(),
    }
}

/// The documented default of [`FeynmanPicked`]: the value a document that
/// omits it takes.
pub(crate) fn feynman_picked() -> FeynmanPicked {
    FeynmanPicked {
        color: Color::from_rgb8(255, 215, 0),
        width_extra_px: 2.0,
    }
}

/// The documented default of [`FeynmanSeries`]: the value a document that
/// omits it takes.
pub(crate) fn feynman_series() -> FeynmanSeries {
    FeynmanSeries {
        vertices: Vec::new(),
        propagators: Vec::new(),
        style: feynman_style(),
        layout: feynman_layout(),
    }
}

/// The documented default of [`FeynmanStyle`]: the value a document that
/// omits it takes.
pub(crate) fn feynman_style() -> FeynmanStyle {
    FeynmanStyle {
        color: Color::from_rgb8(30, 30, 30),
        width: 1.5,
        wave: Wiggle {
            amplitude: 4.0,
            wavelength: 12.0,
        },
        coil: Wiggle {
            amplitude: 5.0,
            wavelength: 9.0,
        },
        head_length: 9.0,
        head_width: 7.0,
        dot_radius: 3.0,
        loop_radius: 18.0,
        fan_step: 90.0,
        picked: feynman_picked(),
    }
}

/// The documented default of [`LevelBandStyle`]: the value a document that
/// omits it takes.
pub(crate) fn level_band_style() -> LevelBandStyle {
    LevelBandStyle {
        fill: Color::from_rgba8(34, 52, 88, 40),
        inner_width: 0.8,
    }
}

/// The documented default of [`LevelLabelSide`]: the value a document that
/// omits it takes.
pub(crate) const fn level_label_side() -> LevelLabelSide {
    LevelLabelSide::Right
}

/// The documented default of [`LevelRole`]: the value a document that
/// omits it takes.
pub(crate) const fn level_role() -> LevelRole {
    LevelRole::Level
}

/// The documented default of [`LevelStrokeStyle`]: the value a document that
/// omits it takes.
pub(crate) fn level_stroke_style() -> LevelStrokeStyle {
    LevelStrokeStyle {
        color: Color::from_rgb8(40, 40, 40),
        width: 2.0,
        slot_fraction: 0.7,
    }
}

/// The documented default of [`LevelView`]: the value a document that
/// omits it takes.
pub(crate) const fn level_view() -> LevelView {
    LevelView::Main
}

/// The documented default of [`MomentumSide`]: the value a document that
/// omits it takes.
pub(crate) const fn momentum_side() -> MomentumSide {
    MomentumSide::Left
}

/// The documented default of [`ParticleLine`]: the value a document that
/// omits it takes.
pub(crate) const fn particle_line() -> ParticleLine {
    ParticleLine::Plain
}

/// The documented default of [`TimeDirection`]: the value a document that
/// omits it takes.
pub(crate) const fn time_direction() -> TimeDirection {
    TimeDirection::LeftToRight
}

/// The documented default of [`TransitionArrowStyle`]: the value a document that
/// omits it takes.
pub(crate) fn transition_arrow_style() -> TransitionArrowStyle {
    TransitionArrowStyle {
        color: Color::from_rgb8(200, 120, 0),
        width: 1.5,
        head_length: 9.0,
        head_width: 7.0,
        direction: TransitionDirection::Emission,
        color_by_value: false,
    }
}

/// The documented default of [`TransitionDirection`]: the value a document that
/// omits it takes.
pub(crate) const fn transition_direction() -> TransitionDirection {
    TransitionDirection::Emission
}

/// The documented default of [`VertexRole`]: the value a document that
/// omits it takes.
pub(crate) const fn vertex_role() -> VertexRole {
    VertexRole::Internal
}

/// The documented default of [`WavelengthLabels`]: the value a document that
/// omits it takes.
pub(crate) fn wavelength_labels() -> WavelengthLabels {
    WavelengthLabels {
        format: crate::format::PlainNumberFormat::AUTO,
        unit: None,
        text: RichText::plain("", Color::from_rgb8(40, 40, 40), 11.0, ""),
    }
}

/// [`EnergyLevelStyle::picked_width_scale`] where a document omits it.
pub(crate) fn energy_level_picked_width_scale() -> f32 {
    2.0
}
