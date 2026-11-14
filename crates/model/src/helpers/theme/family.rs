//! Theme families (#295): one name that themes every kind of chart.
//!
//! A [`ThemeFamily`] is a look with a member [`Theme`] for every
//! [`ChartKind`] and a member [`RadialTheme`]. [`Theme::apply_named`]
//! applies a family by name to a document: every chart in scope takes its
//! own kind's member, so a dashboard that mixes kinds is themed in one
//! call. The family names are the only theme names; any other look is a
//! custom [`Theme`] value.

use super::{
    ChartKind, Charts, RadialTheme, SubplotScope, Theme, ThemeError, catalog, chart_layers,
    charts_in_scope, check_scope, drawn_on,
};
use crate::config::{Config, DrawStyle};
use crate::data_config::{DataRenderType, SeriesConfig};
use crate::layer::LayerRef;

/// A named look with a member theme for every [`ChartKind`] and a member
/// [`RadialTheme`]. Every member draws in the Precise style.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ThemeFamily {
    /// White paper, navy ink, a slate frame and a pale grid: the gallery's
    /// look.
    Gallery,
    /// A light dashboard: slate ink on a cool off-white.
    CoolSlate,
    /// A dark screen: pale ink on midnight navy.
    Midnight,
    /// Warm cream paper and brown ink, for reports and print.
    WarmPaper,
}

impl ThemeFamily {
    /// Every family, in the order of [`Self::NAMES`].
    pub const ALL: [Self; 4] = [
        Self::Gallery,
        Self::CoolSlate,
        Self::Midnight,
        Self::WarmPaper,
    ];

    /// The families' names: the names `Chart::apply_theme`, the web's
    /// `apply_theme` and [`crate::radial::RadialChart::apply_theme`] take.
    pub const NAMES: &'static [&'static str] = &["gallery", "cool-slate", "midnight", "warm-paper"];

    /// The family's name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Gallery => "gallery",
            Self::CoolSlate => "cool-slate",
            Self::Midnight => "midnight",
            Self::WarmPaper => "warm-paper",
        }
    }

    /// The family `name`, or `None`.
    pub fn named(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|family| family.name() == name)
    }

    /// The family `name`, or [`ThemeError::UnknownName`] listing the
    /// family names.
    pub fn parse(name: &str) -> Result<Self, ThemeError> {
        Self::named(name).ok_or_else(|| ThemeError::UnknownName {
            name: name.to_owned(),
            names: Self::NAMES,
        })
    }

    /// The member the family gives the chart `layer` names: its member for
    /// the chart's kind, the line member while the chart draws no series.
    fn member_for(self, config: &Config, series: &[SeriesConfig], layer: LayerRef) -> Theme {
        self.theme(ChartKind::of(config, series, layer).unwrap_or(ChartKind::Line))
    }

    /// The family's member for `kind`: a theme to apply as it is or to edit
    /// first (`Chart::apply_custom_theme`).
    pub fn theme(self, kind: ChartKind) -> Theme {
        catalog::member(self, kind)
    }

    /// The family's member for the radial chart.
    pub fn radial(self) -> RadialTheme {
        catalog::radial_member(self)
    }
}

impl std::fmt::Display for ThemeFamily {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

impl Theme {
    /// Apply the theme family `name` ([`ThemeFamily::NAMES`]) to a chart
    /// and its series, each chart in scope taking the family's member for
    /// its own kind ([`ChartKind::of`]): the base chart and the subplots
    /// `subplots` names, with their overlays and the series drawn there, as
    /// [`Self::apply`] writes them. A dashboard whose subplots mix kinds is
    /// themed in one call; a chart that draws no series yet takes the
    /// line member.
    ///
    /// Refused, changing nothing, for a name that is not a family's
    /// ([`ThemeError::UnknownName`]) and for a scope that names a subplot
    /// the chart does not have ([`ThemeError::SubplotIndex`]). Every member
    /// draws in the Precise style, which draws every series.
    pub fn apply_named(
        name: &str,
        config: &mut Config,
        series: &mut [SeriesConfig],
        subplots: SubplotScope,
    ) -> Result<(), ThemeError> {
        let family = ThemeFamily::parse(name)?;
        check_scope(config, subplots)?;
        let scope = Charts::scope(subplots);
        // Styling keeps every chart's kind, so each chart takes the same
        // member in both passes. All series are styled before any chrome
        // is written: a legend shows the marks of the series it names.
        for layer in chart_layers(config.subplots.len(), scope) {
            let theme = family.member_for(config, series, layer);
            theme.style_charts(config, series, Charts::one(&layer));
        }
        for layer in chart_layers(config.subplots.len(), scope) {
            let theme = family.member_for(config, series, layer);
            theme.finish_charts(config, series, Charts::one(&layer));
        }
        Ok(())
    }

    /// Apply the theme to the charts of its kind: the base chart and every
    /// subplot that is of [`Self::kind`], or that draws no series yet, with
    /// their overlays and the series drawn there, as [`Self::apply`] writes
    /// them. Charts of another kind are left as they are. Refused, changing
    /// nothing, when no chart takes the theme ([`ThemeError::WrongKind`],
    /// naming the base chart or the first subplot of another kind), or
    /// when the theme's draw style cannot draw a series on a chart it
    /// themes ([`ThemeError::UnsupportedDrawStyle`]; `draws` as in
    /// [`Self::apply_named`]).
    pub fn apply_to_its_kind(
        &self,
        config: &mut Config,
        series: &mut [SeriesConfig],
        draws: impl Fn(&DrawStyle, &DataRenderType) -> bool,
    ) -> Result<(), ThemeError> {
        let all = Charts::scope(SubplotScope::All);
        let takes = |config: &Config, series: &[SeriesConfig], layer: LayerRef| {
            ChartKind::of(config, series, layer).is_none_or(|kind| kind == self.kind)
        };
        let mut themed = false;
        let mut refused = None;
        for layer in charts_in_scope(config, all) {
            if !takes(config, series, layer) {
                if let (None, Some(chart)) = (&refused, ChartKind::of(config, series, layer)) {
                    refused = Some((chart, layer));
                }
                continue;
            }
            themed = true;
            let style = &self.chrome.draw_style;
            if let Some(item) = series.iter().find(|item| {
                drawn_on(config, layer, &item.series_id) && !draws(style, &item.render_type)
            }) {
                return Err(ThemeError::UnsupportedDrawStyle {
                    series_id: item.series_id.clone(),
                });
            }
        }
        if let (false, Some((chart, layer))) = (themed, refused) {
            return Err(ThemeError::WrongKind {
                theme: self.kind,
                chart,
                layer,
            });
        }
        // As in `apply_named`: every series first, then the chrome.
        for layer in chart_layers(config.subplots.len(), all) {
            if takes(config, series, layer) {
                self.style_charts(config, series, Charts::one(&layer));
            }
        }
        for layer in chart_layers(config.subplots.len(), all) {
            if takes(config, series, layer) {
                self.finish_charts(config, series, Charts::one(&layer));
            }
        }
        Ok(())
    }

    /// The theme the family `name` gives the chart `layer` names, the base
    /// chart or a subplot, as [`Self::apply_named`] gives it: the family's
    /// member for the chart's kind (the line member while it draws no
    /// series), a value to edit and apply with [`Self::apply`] or
    /// [`Self::apply_to_its_kind`]. Refused for a name that is not a
    /// family's ([`ThemeError::UnknownName`]).
    pub fn for_chart(
        name: &str,
        config: &Config,
        series: &[SeriesConfig],
        layer: LayerRef,
    ) -> Result<Self, ThemeError> {
        Ok(ThemeFamily::parse(name)?.member_for(config, series, layer))
    }
}

impl crate::radial::RadialChart {
    /// Apply the theme family `name`'s radial member
    /// ([`ThemeFamily::radial`]), as [`RadialTheme::apply`] writes it.
    /// Refused, changing nothing, for a name that is not a family's
    /// ([`ThemeError::UnknownName`]).
    pub fn apply_theme(&mut self, name: &str) -> Result<(), ThemeError> {
        ThemeFamily::parse(name)?.radial().apply(self);
        Ok(())
    }
}
