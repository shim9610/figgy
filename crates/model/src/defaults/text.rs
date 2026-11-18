//! Text, number and date formats, colour maps, materials and line styles.

#![allow(unused_imports)]
// Some sources are read only by `Default` (`crate::helpers`).
#![cfg_attr(not(feature = "helpers"), allow(dead_code))]

use super::*;
use crate::colormap::*;
use crate::format::*;
use crate::format::*;
use crate::layout::*;
use crate::line::*;
use crate::line::*;
use crate::material::*;
use crate::text::math::*;
use crate::text::*;
use crate::text::*;
use crate::*;

/// The documented default of [`ColorMap`]: the value a document that
/// omits it takes.
pub(crate) const fn color_map() -> ColorMap {
    ColorMap::Viridis
}

/// The documented default of [`FractionalSecondDigits`]: the value a document that
/// omits it takes.
pub(crate) const fn fractional_second_digits() -> FractionalSecondDigits {
    FractionalSecondDigits::Auto
}

/// The documented default of [`LineStylePreset`]: the value a document that
/// omits it takes.
pub(crate) const fn line_style_preset() -> LineStylePreset {
    LineStylePreset::Solid
}

/// The documented default of [`Material`]: the value a document that
/// omits it takes.
pub(crate) const fn material() -> Material {
    Material::Flat
}

/// The documented default of [`MathStyle`]: the value a document that
/// omits it takes.
pub(crate) const fn math_style() -> MathStyle {
    MathStyle::Text
}

/// The documented default of [`PlainNumberFormat`]: the value a document that
/// omits it takes.
pub(crate) fn plain_number_format() -> PlainNumberFormat {
    PlainNumberFormat {
        decimals: None,
        trim_zeros: false,
    }
}

/// The documented default of [`TextAlign`]: the value a document that
/// omits it takes.
pub(crate) const fn text_align() -> TextAlign {
    TextAlign::Left
}

/// The documented default of [`TimestampLabelFormat`]: the value a document that
/// omits it takes.
pub(crate) fn timestamp_label_format() -> TimestampLabelFormat {
    TimestampLabelFormat {
        unit: timestamp_unit(),
        timezone: timestamp_zone(),
        label: timestamp_label_mode(),
        fractional: fractional_second_digits(),
        tick_policy: timestamp_tick_policy(),
    }
}

/// The documented default of [`TimestampLabelMode`]: the value a document that
/// omits it takes.
pub(crate) const fn timestamp_label_mode() -> TimestampLabelMode {
    TimestampLabelMode::Auto
}

/// The documented default of [`TimestampTickPolicy`]: the value a document that
/// omits it takes.
pub(crate) const fn timestamp_tick_policy() -> TimestampTickPolicy {
    TimestampTickPolicy::AutoCalendar
}

/// The documented default of [`TimestampUnit`]: the value a document that
/// omits it takes.
pub(crate) const fn timestamp_unit() -> TimestampUnit {
    TimestampUnit::Seconds
}

/// The documented default of [`TimestampZone`]: the value a document that
/// omits it takes.
pub(crate) const fn timestamp_zone() -> TimestampZone {
    TimestampZone::Utc
}
