//! Theme families (#295): one name that themes every kind of chart.
//!
//! A [`ThemeFamily`] is a look with a member [`Theme`] for every
//! [`ChartKind`] and a member [`RadialTheme`]. [`Theme::apply_named`]
//! applies a name to a document: a family name gives every chart in scope
//! its own kind's member, so a dashboard that mixes kinds is themed in one
//! call, and a name of one kind's catalogue applies to charts of that kind
//! only.

use super::{
    ChartKind, Charts, RadialTheme, SubplotScope, Theme, ThemeError, catalog, chart_layers,
    charts_in_scope, drawn_on,
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

    /// The families' names, the first names of every kind's list
    /// ([`Theme::names`]) and of the radial list ([`RadialTheme::names`]).
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
    /// Apply the theme `name` to a chart and its series, each chart in
    /// scope taking the theme of its own kind ([`ChartKind::of`]): the base
    /// chart and the subplots `subplots` names, with their overlays and the
    /// series drawn there, as [`Self::apply`] writes them.
    ///
    /// - A family name ([`ThemeFamily::NAMES`]) gives every chart its
    ///   kind's member, so the base chart and subplots of other kinds are
    ///   themed in one call.
    /// - A name of a kind's own catalogue ([`Theme::names`]) applies to the
    ///   charts of a kind that has it; a chart of another kind refuses it
    ///   ([`ThemeError::Kind`]).
    /// - Any other name is refused ([`ThemeError::Unknown`], with the names
    ///   the chart takes).
    /// - A chart that draws no series yet takes the theme of the first kind
    ///   ([`ChartKind::ALL`]) that has the name: a family's line member.
    ///
    /// `draws` tells whether a draw style draws a series
    /// (`renderer::draw_style_supports`, which the model cannot answer); a
    /// series in scope that its chart's theme cannot draw is refused
    /// ([`ThemeError::DrawStyle`]). Every chart is checked before any is
    /// written, so a refusal changes nothing.
    pub fn apply_named(
        name: &str,
        config: &mut Config,
        series: &mut [SeriesConfig],
        subplots: SubplotScope,
        draws: impl Fn(&DrawStyle, &DataRenderType) -> bool,
    ) -> Result<(), ThemeError> {
        let scope = Charts::scope(subplots);
        for layer in charts_in_scope(config, scope) {
            let theme = Self::for_chart(name, config, series, layer)?;
            let style = &theme.chrome.draw_style;
            if let Some(item) = series.iter().find(|item| {
                drawn_on(config, layer, &item.series_id) && !draws(style, &item.render_type)
            }) {
                return Err(ThemeError::DrawStyle {
                    series_id: item.series_id.clone(),
                });
            }
        }
        // Styling keeps every chart's kind, so each chart resolves to the
        // theme it was checked with. All series are styled before any
        // chrome is written: a legend shows the marks of the series it
        // names.
        for layer in chart_layers(config.subplots.len(), scope) {
            let theme = Self::for_chart(name, config, series, layer)?;
            theme.style_charts(config, series, Charts::one(&layer));
        }
        for layer in chart_layers(config.subplots.len(), scope) {
            let theme = Self::for_chart(name, config, series, layer)?;
            theme.finish_charts(config, series, Charts::one(&layer));
        }
        Ok(())
    }

    /// The theme `name` resolves to on the chart `layer` names, the base
    /// chart or a subplot, as [`Self::apply_named`] resolves it: a family's
    /// member for the chart's kind or the kind's own theme of that name, a
    /// value to edit and apply with [`Self::apply`]. Refused with
    /// [`ThemeError::Kind`] for a kind without the name and with
    /// [`ThemeError::Unknown`] for a name no kind has.
    pub fn for_chart(
        name: &str,
        config: &Config,
        series: &[SeriesConfig],
        layer: LayerRef,
    ) -> Result<Self, ThemeError> {
        let kind = ChartKind::of(config, series, layer);
        // The first kind whose names have `name`: the kind a chart without
        // series takes it as, and the kind a refusal names.
        let first = || {
            ChartKind::ALL
                .into_iter()
                .find(|&kind| catalog::names(kind).contains(&name))
        };
        match kind.or_else(first).and_then(|kind| Self::named(kind, name)) {
            Some(theme) => Ok(theme),
            None => Err(match (kind, first()) {
                (Some(chart), Some(theme)) => ThemeError::Kind {
                    name: Some(name.to_owned()),
                    theme,
                    chart,
                    layer,
                },
                (Some(chart), None) => ThemeError::Unknown {
                    name: name.to_owned(),
                    kind,
                    names: catalog::names(chart),
                },
                (None, _) => ThemeError::Unknown {
                    name: name.to_owned(),
                    kind,
                    names: ThemeFamily::NAMES,
                },
            }),
        }
    }
}

impl crate::radial::RadialChart {
    /// Apply the radial theme `name` ([`RadialTheme::names`]): a family's
    /// radial member ([`ThemeFamily::radial`]) or a theme of the radial
    /// catalogue, as [`RadialTheme::apply`] writes it. An unknown name is
    /// refused with [`ThemeError::Unknown`], which lists the radial names,
    /// and changes nothing.
    pub fn apply_theme(&mut self, name: &str) -> Result<(), ThemeError> {
        let theme = RadialTheme::named(name).ok_or_else(|| ThemeError::Unknown {
            name: name.to_owned(),
            kind: None,
            names: RadialTheme::names(),
        })?;
        theme.apply(self);
        Ok(())
    }
}
