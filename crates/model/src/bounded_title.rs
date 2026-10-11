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
            },
        }
    }
    pub fn has_text(&self) -> bool {
        self.visible && !self.text.segments.is_empty()
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        fn range(v: f32, lo: f32, hi: f32) -> bool {
            v.is_finite() && (lo..=hi).contains(&v)
        }
        fn color(c: Color) -> bool {
            [c.r, c.g, c.b, c.a].into_iter().all(|v| range(v, 0., 1.))
        }
        let t = &self.text;
        if !range(t.font_size, 6., 160.)
            || !color(t.color)
            || t.font.chars().count() > 160
            || t.font.chars().any(char::is_control)
            || t.segments.len() > 512
            || t.segments.iter().filter(|s| s.text == '\n').count() > 7
            || t.segments.iter().any(|s| {
                (s.text.is_control() && !matches!(s.text, '\n' | '\t'))
                    || s.font_size.is_some_and(|v| !range(v, 6., 160.))
                    || s.color.is_some_and(|v| !color(v))
                    || s.field_em.is_some_and(|v| !range(v, 0., 32.))
                    || s.rule_dash
                        .as_ref()
                        .is_some_and(|d| d.len() > 32 || d.iter().any(|v| !range(*v, 0.01, 32.)))
            })
        {
            return Err(
                "invalid bounded title: size 6..160, at most 512 characters / 8 lines, finite styles",
            );
        }
        Ok(())
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
                let copy: $chart =
                    serde_json::from_str(&serde_json::to_string(&chart).unwrap()).unwrap();
                assert_eq!(copy, chart);
            }};
        }
        check!(crate::RadialChart, label_color);
        check!(crate::CategoricalChart, text_color);
        check!(crate::BoxPlotChart, text_color);
    }
}
