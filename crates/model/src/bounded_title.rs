//! The rich title of the legacy categorical, box plot and radar documents
//! (`BoundedTitleOptions`), and the wire form of the radial chart's title.
//! The chart's `DecorationLayout` owns the title's position.
use crate::{
    Color,
    text::{RichSegment, RichText},
};

#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct BoundedTitleOptions {
    pub visible: bool,
    pub text: RichText,
}

/// The text sizes of a categorical, box plot or radar document by role, in
/// logical pixels (#357): the chart title, the axis titles, the tick labels
/// (a radar's spoke and ring labels) and the legend.
///
/// A document without them (`text_sizes` absent, every document written
/// before the field) draws every text at its one `font_size` and the title
/// half as large again ([`Self::legacy`]). The documents' Rust `Default`
/// sets [`Self::CARTESIAN`], the sizes of the documented Cartesian chart,
/// so a document built in Rust draws its text as a Cartesian chart does.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct DocumentTextSizes {
    pub title: f32,
    pub axis_title: f32,
    pub tick_label: f32,
    pub legend: f32,
}

impl DocumentTextSizes {
    /// The documented Cartesian chart's sizes: a 28 px title, 22 px axis
    /// titles, 18 px tick labels and a 14 px legend.
    pub const CARTESIAN: Self = Self {
        title: 28.0,
        axis_title: 22.0,
        tick_label: 18.0,
        legend: 14.0,
    };

    /// The smallest and largest size a role takes, in logical pixels.
    pub const RANGE: std::ops::RangeInclusive<f32> = 8.0..=64.0;

    /// The sizes of a document without `text_sizes`: every role at
    /// `font_size`, the title at one and a half times it.
    pub fn legacy(font_size: f32) -> Self {
        Self {
            title: font_size * 1.5,
            axis_title: font_size,
            tick_label: font_size,
            legend: font_size,
        }
    }

    /// The sizes a document draws with: `sizes`, or [`Self::legacy`] of
    /// `font_size` without them.
    pub fn resolved(sizes: Option<&Self>, font_size: f32) -> Self {
        sizes.copied().unwrap_or_else(|| Self::legacy(font_size))
    }

    /// Whether every size is finite and within [`Self::RANGE`].
    pub fn is_valid(&self) -> bool {
        [self.title, self.axis_title, self.tick_label, self.legend]
            .into_iter()
            .all(|size| Self::RANGE.contains(&size))
    }
}

impl BoundedTitleOptions {
    /// Snapshot the legacy plain title once before editing label styles.
    /// Once installed as `chart_title`, this document is authoritative; the
    /// legacy `title` string is ignored by rendering.
    pub fn from_legacy(title: &str, font: &str, label_size: f32, color: Color) -> Self {
        Self::from_plain(title, font, label_size * 1.5, color)
    }

    /// The plain title `title` at `size`.
    fn from_plain(title: &str, font: &str, size: f32, color: Color) -> Self {
        Self {
            visible: true,
            text: RichText {
                segments: title.chars().map(RichSegment::plain).collect(),
                color,
                font_size: size,
                font: font.into(),
                align: crate::text::TextAlign::Left,
            },
        }
    }

    /// The stored `chart_title`, or the snapshot [`Self::from_legacy`]
    /// takes of the legacy plain title while there is none: at the title
    /// size of `sizes` when the document has them (#357).
    pub(crate) fn resolved(
        chart_title: Option<&Self>,
        title: &str,
        font: &str,
        label_size: f32,
        sizes: Option<&DocumentTextSizes>,
        color: Color,
    ) -> Self {
        chart_title.cloned().unwrap_or_else(|| match sizes {
            Some(sizes) => Self::from_plain(title, font, sizes.title, color),
            None => Self::from_legacy(title, font, label_size, color),
        })
    }

    pub fn validate(&self) -> Result<(), crate::RuleKind> {
        self.text
            .validate_math()
            .map_err(|error| crate::RuleKind::InvalidMath { error })?;
        if !self.text.within_limits() {
            return Err(crate::RuleKind::BoundedTitle);
        }
        Ok(())
    }
}

/// The wire form of the radial chart's `chart_title`, held as a
/// [`ChartTitleOptions`](crate::config::ChartTitleOptions): the
/// `{"visible", "text"}` object a [`BoundedTitleOptions`] writes, in that
/// order, so documents keep their bytes. The chart keeps the title's position
/// in its `decorations`, so the offsets and `top_margin` are not written and
/// read as 0; the chart's validation rejects other values.
#[cfg(feature = "serde")]
pub(crate) mod bounded_title_wire {
    use crate::config::ChartTitleOptions;
    use crate::text::RichText;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    #[derive(Serialize)]
    struct WireRef<'a> {
        visible: bool,
        text: &'a RichText,
    }

    #[derive(Deserialize)]
    struct Wire {
        visible: bool,
        text: RichText,
    }

    pub(crate) fn serialize<S: Serializer>(
        title: &Option<ChartTitleOptions>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        match title {
            Some(title) => serializer.serialize_some(&WireRef {
                visible: title.visible,
                text: &title.text,
            }),
            None => serializer.serialize_none(),
        }
    }

    pub(crate) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<ChartTitleOptions>, D::Error> {
        Ok(
            Option::<Wire>::deserialize(deserializer)?.map(|wire| ChartTitleOptions {
                text: wire.text,
                visible: wire.visible,
                offset_x: 0.0,
                offset_y: 0.0,
                top_margin: 0.0,
            }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validates_independent_base_and_segment_styles() {
        let base = BoundedTitleOptions::from_legacy("Title", "", 16., Color::BLACK);
        base.validate().unwrap();
        for size in [f32::NAN, f32::INFINITY, -1., 0., 161.] {
            let mut title = base.clone();
            title.text.font_size = size;
            assert!(title.validate().is_err());
            title = base.clone();
            title.text.segments[0].font_size = Some(size);
            assert!(title.validate().is_err());
        }
        let mut title = base.clone();
        title.text.segments[0].field_em = Some(f32::INFINITY);
        assert!(title.validate().is_err());
        title = base.clone();
        title.text.segments[0].color = Some(Color::new(1., 0., 0., f32::NAN));
        assert!(title.validate().is_err());
    }
    /// A radial document written before 0.19 with a plain `title`: the
    /// title is read in the document's font, keeps that look when the font
    /// changes, and only `chart_title` is written (#160).
    #[cfg(feature = "serde")]
    #[test]
    fn the_legacy_radial_title_is_read_in_its_style_and_round_trips() {
        let mut old = serde_json::to_value(crate::RadialChart {
            font_size: 16.,
            ..crate::defaults::radial_chart()
        })
        .unwrap();
        assert!(old.get("title").is_none() && old.get("chart_title").is_none());
        old["title"] = "A²".into();
        let mut chart: crate::RadialChart = serde_json::from_value(old).unwrap();
        let expected = crate::config::ChartTitleOptions::from_legacy_plain(
            "A²",
            "sans-serif",
            16.,
            chart.label_color,
        );
        assert_eq!(chart.chart_title.as_ref(), Some(&expected));
        chart.font_size = 28.;
        chart.label_color = Color::WHITE;
        chart.font_family = "Another font".into();
        assert_eq!(chart.resolved_title(), expected);
        chart.chart_title.as_mut().unwrap().text.align = crate::text::TextAlign::Right;
        chart.chart_title.as_mut().unwrap().visible = false;
        let json = serde_json::to_value(&chart).unwrap();
        assert!(json.get("title").is_none(), "{json}");
        let copy: crate::RadialChart = serde_json::from_value(json).unwrap();
        assert_eq!(copy, chart);
        // A rich title wins over a legacy one, as it was drawn.
        let mut both = serde_json::to_value(&chart).unwrap();
        both["title"] = "ignored".into();
        let read: crate::RadialChart = serde_json::from_value(both).unwrap();
        assert_eq!(read.chart_title, chart.chart_title);
    }
}
