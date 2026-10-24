//! Independent bounded-chart title. DecorationLayout owns its position.
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

impl BoundedTitleOptions {
    /// Snapshot the legacy plain title once before editing label styles.
    /// Once installed as `chart_title`, this document is authoritative; the
    /// legacy `title` string is ignored by rendering.
    pub fn from_legacy(title: &str, font: &str, label_size: f32, color: Color) -> Self {
        Self {
            visible: true,
            text: RichText {
                segments: title.chars().map(RichSegment::plain).collect(),
                color,
                font_size: label_size * 1.5,
                font: font.into(),
                align: crate::text::TextAlign::Left,
            },
        }
    }
    pub fn has_text(&self) -> bool {
        self.visible && !self.text.segments.is_empty()
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        if !self.text.within_limits() {
            return Err(
                "invalid bounded title: size 6..160, at most 512 characters / 8 lines, finite styles",
            );
        }
        Ok(())
    }
}

/// The wire form of a bounded chart's `chart_title` held as a
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
    #[cfg(feature = "serde")]
    #[test]
    fn all_legacy_families_freeze_title_style_and_round_trip() {
        macro_rules! check {
            ($chart:ty, $color:ident) => {{
                let mut chart = <$chart>::default();
                chart.title = "A²".into();
                chart.font_size = 16.;
                let old = serde_json::to_value(&chart).unwrap();
                assert!(old.get("chart_title").is_none());
                chart = serde_json::from_value(old).unwrap();
                let expected = chart.resolved_title();
                chart.normalize_title();
                chart.font_size = 28.;
                chart.$color = Color::WHITE;
                chart.font_family = "Another font".into();
                chart.normalize_title();
                assert_eq!(chart.resolved_title(), expected);
                chart.chart_title.as_mut().unwrap().text.align = crate::text::TextAlign::Right;
                chart.chart_title.as_mut().unwrap().visible = false;
                let copy: $chart =
                    serde_json::from_str(&serde_json::to_string(&chart).unwrap()).unwrap();
                assert_eq!(copy, chart);
            }};
        }
        check!(crate::RadialChart, label_color);
        check!(crate::CategoricalChart, text_color);
        check!(crate::BoxPlotChart, text_color);
        check!(crate::RadarChart, text_color);
    }
}
