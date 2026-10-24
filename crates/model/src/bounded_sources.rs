//! The readers of the legacy bounded chart documents, and the borrowed views
//! their inline values upload through.
//!
//! A [`CategoricalChart`] keeps its values inline (`series[].values`). A
//! categorical bar chart is a Cartesian chart now, whose values live in the
//! renderer's column pool like every other series' data, so the reader turns
//! the document into a [`Config`], its series and one [`SeriesUpload`] per
//! column. An upload borrows the document's values ([`UploadValues`]); the
//! renderer implements its `ColumnSource` for that view, so the values go
//! straight from the document into the pool and are not copied on the way.

use crate::categorical::CategoricalChart;
use crate::color::Color;
use crate::config::{
    AxisOptions, AxisTitleOptions, Config, DataSelectionsConfig, GridOptions, PickedDataRef,
    TickVisibility,
};
use crate::data::ColumnId;
use crate::data_config::{
    BarGroups, CategoryBarDirection, CategoryBarSeriesLabels, DataRenderType, SeriesConfig,
    format_labels,
};
use crate::decoration::DecorationId;
use crate::format::{LabelFormat, PlainNumberFormat};
use crate::layout::{Side, axis_mut};
use crate::text::RichText;

/// The id of the column holding the category positions of a converted
/// categorical chart.
pub const CATEGORY_POSITIONS_COLUMN: &str = "categorical.positions";

/// The id of the column holding the values of the converted series
/// `series_id`.
pub fn category_values_column(series_id: &str) -> ColumnId {
    format!("categorical.values.{series_id}")
}

/// The values one column of a converted document uploads, borrowed from the
/// document.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum UploadValues<'a> {
    /// The category positions `0, 1, ..., n - 1`.
    Positions(usize),
    /// A series' values in category order; a missing value uploads NaN.
    Values(&'a [Option<f64>]),
}

impl UploadValues<'_> {
    /// The number of values.
    pub fn len(&self) -> usize {
        match self {
            UploadValues::Positions(n) => *n,
            UploadValues::Values(values) => values.len(),
        }
    }

    /// Whether there are no values.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Value `index`, NaN for a missing value or an index past the end.
    pub fn value(&self, index: usize) -> f64 {
        match self {
            UploadValues::Positions(n) if index < *n => index as f64,
            UploadValues::Positions(_) => f64::NAN,
            UploadValues::Values(values) => {
                values.get(index).copied().flatten().unwrap_or(f64::NAN)
            }
        }
    }

    /// The smallest finite value, `+inf` when there is none.
    pub fn min(&self) -> f64 {
        (0..self.len())
            .map(|i| self.value(i))
            .filter(|v| v.is_finite())
            .fold(f64::INFINITY, f64::min)
    }

    /// The largest finite value, `-inf` when there is none.
    pub fn max(&self) -> f64 {
        (0..self.len())
            .map(|i| self.value(i))
            .filter(|v| v.is_finite())
            .fold(f64::NEG_INFINITY, f64::max)
    }
}

/// One column a converted document uploads: the id its series read and the
/// values, borrowed from the document.
#[derive(Clone, Debug, PartialEq)]
pub struct SeriesUpload<'a> {
    pub column: ColumnId,
    pub values: UploadValues<'a>,
}

/// The grid colour of a legacy bounded value axis.
const LEGACY_GRID_COLOR: Color = Color::from_rgb8(224, 229, 235);

impl Config {
    /// The Cartesian chart a legacy [`CategoricalChart`] document draws:
    /// its `Config`, its series and the columns to upload.
    ///
    /// The chart is a `Config` with a categorical axis (`bottom_x` for
    /// vertical bars, `left_y` for horizontal ones) holding the document's
    /// categories, `bar_groups` from `mode`, `direction`, `group_width` and
    /// `bar_gap`, and one [`DataRenderType::CategoryBars`] series per
    /// document series in document order, reading the shared
    /// [`CATEGORY_POSITIONS_COLUMN`] and its own [`category_values_column`].
    /// Each series takes its colour, its style (or the chart's), the chart's
    /// label settings, its overrides, and the label text [`format_labels`]
    /// writes from the document's values.
    ///
    /// The uploads are the positions column first, then the value columns
    /// in series order; upload the value columns together
    /// (`Renderer::add_columns`), which is what measures a stacked group's
    /// extent for the value-axis fit. The value axis range is the host's to
    /// fit; it starts at the default placeholder.
    ///
    /// Field map: `value_title` and `category_title` are the axis titles
    /// (in `font_family`, `font_size` and `text_color`, hidden when empty);
    /// `font_family`, `font_size` and `text_color` are both axes' tick
    /// labels and lines; `grid` is the value axis' major grid in the bounded
    /// grid colour; `legend` is the Cartesian legend's visibility with one
    /// entry per series; the resolved title is `chart_title`; the decoration
    /// offsets of the title, the axis lines, tick labels and titles and the
    /// legend are the matching offsets; the category axis line crosses the
    /// value axis at zero (`cross_at`); `selected` is the typed selection;
    /// `active_layer` and `annotations` carry over. Not carried: `hovered`
    /// (transient), `background` (the host's clear colour), an explicit
    /// `decorations.data_area` and the data label offsets (the value labels
    /// are not drawn yet), and the decoration selection. The chart area is
    /// the default placeholder; the host sets it.
    pub fn from_categorical(
        chart: &CategoricalChart,
    ) -> (Config, Vec<SeriesConfig>, Vec<SeriesUpload<'_>>) {
        let horizontal = chart.direction == CategoryBarDirection::Horizontal;
        let font = chart.font_family.as_str();
        let text = |s: &str| RichText::plain(s, chart.text_color, chart.font_size, font);
        let style_axis = |axis: &mut AxisOptions, title: &str| {
            axis.label_style.color = chart.text_color;
            axis.label_style.font_size = chart.font_size;
            axis.label_style.label_font = chart.font_family.clone();
            axis.tick = TickVisibility::None;
            axis.title_option = AxisTitleOptions {
                text: text(title),
                visible: !title.is_empty(),
                offset_x: 0.0,
                offset_y: 0.0,
            };
            axis.line_color = chart.text_color;
            axis.line_width = 1.0;
        };

        let mut config = crate::default::default_config();
        let mut category_axis = config.bottom_x.clone();
        style_axis(&mut category_axis, &chart.category_title);
        category_axis.categories = Some(chart.categories.clone());
        let mut value_axis = config.left_y.clone();
        style_axis(&mut value_axis, &chart.value_title);
        value_axis.label_style.format = LabelFormat::Plain(PlainNumberFormat::AUTO);
        // The category axis line is the zero line of the value axis.
        category_axis.cross_at = Some(0.0);
        if horizontal {
            config.left_y = category_axis;
            config.bottom_x = value_axis;
        } else {
            config.bottom_x = category_axis;
            config.left_y = value_axis;
        }
        for axis in [&mut config.top_x, &mut config.right_y] {
            axis.line_visible = false;
            axis.tick = TickVisibility::None;
        }

        let GridOptions {
            show_major_x,
            major_x_color,
            show_major_y,
            major_y_color,
            show_minor_x,
            show_minor_y,
            // The legacy grid has one line style: the default widths and
            // dashes, and no minor lines whose look would matter.
            major_x_width: _,
            major_x_style: _,
            major_y_width: _,
            major_y_style: _,
            minor_x_color: _,
            minor_x_width: _,
            minor_x_style: _,
            minor_y_color: _,
            minor_y_width: _,
            minor_y_style: _,
        } = &mut config.grid;
        *show_major_x = chart.grid && horizontal;
        *show_major_y = chart.grid && !horizontal;
        *show_minor_x = false;
        *show_minor_y = false;
        *major_x_color = LEGACY_GRID_COLOR;
        *major_y_color = LEGACY_GRID_COLOR;

        let title = chart.resolved_title();
        config.chart_title.text = title.text;
        config.chart_title.visible = title.visible;

        config.bar_groups = Some(BarGroups {
            mode: chart.mode,
            direction: chart.direction,
            group_width: chart.group_width,
            bar_gap: chart.bar_gap,
        });
        config.active_layer = chart.active_layer();
        config.annotations = chart.annotations.clone();

        let mut series = Vec::with_capacity(chart.series.len());
        let mut uploads = Vec::with_capacity(chart.series.len() + 1);
        uploads.push(SeriesUpload {
            column: CATEGORY_POSITIONS_COLUMN.into(),
            values: UploadValues::Positions(chart.categories.len()),
        });
        for s in &chart.series {
            let overrides: Vec<_> = chart
                .overrides
                .iter()
                .filter(|o| o.target.series_id == s.id)
                .cloned()
                .collect();
            let mut item = SeriesConfig {
                series_id: s.id.clone(),
                source_id: None,
                label: Some(text(&s.label)),
                x_column: ColumnId::new(),
                y_column: ColumnId::new(),
                render_type: DataRenderType::CategoryBars {
                    color: s.color,
                    style: s.style.clone().unwrap_or_else(|| chart.style.clone()),
                    labels: CategoryBarSeriesLabels {
                        placement: chart.labels,
                        decimals: chart.label_decimals,
                        value_suffix: chart.value_suffix.clone(),
                        text: Vec::new(),
                    },
                    label_format: chart.label_format,
                    overrides: (!overrides.is_empty()).then_some(overrides),
                },
            };
            let column = category_values_column(&s.id);
            // The direction decides the roles, like a histogram's orientation.
            if horizontal {
                item.y_column = CATEGORY_POSITIONS_COLUMN.into();
                item.x_column = column.clone();
            } else {
                item.x_column = CATEGORY_POSITIONS_COLUMN.into();
                item.y_column = column.clone();
            }
            series.push(item);
            uploads.push(SeriesUpload {
                column,
                values: UploadValues::Values(&s.values),
            });
        }
        format_labels(&mut series, &chart.categories, |k, i| {
            chart.series.get(k)?.values.get(i).copied().flatten()
        });

        config.legend.visible = chart.legend;
        config.legend.content.font = chart.font_family.clone();
        config.legend.content.font_size = chart.font_size;
        config.legend.content.color = chart.text_color;
        for item in &series {
            crate::legend::append_legend_entry(
                &mut config.legend.content,
                crate::legend::series_symbol_segments(item, None),
                chart
                    .series
                    .iter()
                    .find(|s| s.id == item.series_id)
                    .map_or("", |s| s.label.as_str()),
            );
        }

        if let Some(selected) = &chart.selected
            && let Some(index) = chart
                .categories
                .iter()
                .position(|c| c.id == selected.category_id)
        {
            let mut picked = DataSelectionsConfig::default();
            picked.refs.push(PickedDataRef::HistogramBin {
                source_id: None,
                series_id: selected.series_id.clone(),
                bin_index: index,
            });
            config.picked_data = Some(picked);
        }

        for offset in &chart.decorations.offsets {
            let [dx, dy] = offset.offset;
            match &offset.element {
                DecorationId::ChartTitle => {
                    config.chart_title.offset_x = dx;
                    config.chart_title.offset_y = dy;
                }
                DecorationId::Legend => {
                    config.legend.offset_x = dx;
                    config.legend.offset_y = dy;
                }
                DecorationId::Axis(side) => {
                    // A detached axis moves across itself.
                    axis_mut(&mut config, side).line_offset = match side {
                        Side::Left | Side::Right => dx,
                        Side::Top | Side::Bottom => dy,
                    };
                }
                DecorationId::AxisLabel(side) => {
                    let axis = axis_mut(&mut config, side);
                    axis.label_style.label_offset_x = dx;
                    axis.label_style.label_offset_y = dy;
                }
                DecorationId::AxisTitle(side) => {
                    let axis = axis_mut(&mut config, side);
                    axis.title_option.offset_x = dx;
                    axis.title_option.offset_y = dy;
                }
                _ => {}
            }
        }
        (config, series, uploads)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::categorical::{BarSeries, CategoryBarMode, CategoryBarOverride, CategoryBarTarget};
    use crate::config::Category;
    use crate::data_config::{CategoryBarLabelFormat, CategoryBarLabels, CategoryBarStyle};
    use crate::decoration::DecorationOffset;
    use crate::layer::LayerRef;
    use crate::{CategoricalChart, Material};

    fn chart() -> CategoricalChart {
        CategoricalChart {
            categories: vec![Category::new("a", "Alpha"), Category::new("b", "Beta")],
            series: vec![
                BarSeries::new(
                    "s",
                    "Sales",
                    vec![Some(2.0), None],
                    Color::from_rgb8(200, 0, 0),
                ),
                BarSeries::new(
                    "t",
                    "Costs",
                    vec![Some(3.0), Some(1.5)],
                    Color::from_rgb8(0, 0, 200),
                ),
            ],
            ..Default::default()
        }
    }

    fn bars(series: &SeriesConfig) -> crate::data_config::CategoryBarsRef<'_> {
        series.render_type.category_bars().expect("category bars")
    }

    #[test]
    fn categories_become_the_categorical_axis_of_the_direction() {
        let c = chart();
        let (config, _, _) = Config::from_categorical(&c);
        assert_eq!(
            config.bottom_x.categories.as_deref(),
            Some(&c.categories[..])
        );
        assert_eq!(config.left_y.categories, None);
        let mut h = chart();
        h.direction = CategoryBarDirection::Horizontal;
        let (config, series, _) = Config::from_categorical(&h);
        assert_eq!(config.left_y.categories.as_deref(), Some(&h.categories[..]));
        assert_eq!(config.bottom_x.categories, None);
        // A categorical axis is never inverted: the categories run
        // bottom-up, where the legacy chart drew them top-down.
        assert!(!config.left_y.inverted);
        assert_eq!(series[0].y_column, CATEGORY_POSITIONS_COLUMN);
        assert_eq!(series[0].x_column, category_values_column("s"));
        assert_eq!(config.validate_category_bars(&series), Ok(()));
    }

    #[test]
    fn series_values_become_borrowed_uploads_in_series_order() {
        let c = chart();
        let (config, series, uploads) = Config::from_categorical(&c);
        assert_eq!(uploads.len(), 3);
        assert_eq!(uploads[0].column, CATEGORY_POSITIONS_COLUMN);
        assert_eq!(uploads[0].values, UploadValues::Positions(2));
        assert_eq!(uploads[1].column, category_values_column("s"));
        assert!(
            matches!(uploads[1].values, UploadValues::Values(v) if std::ptr::eq(v, &c.series[0].values[..]))
        );
        assert_eq!(
            (
                uploads[1].values.value(0),
                uploads[1].values.value(1).is_nan()
            ),
            (2.0, true)
        );
        assert_eq!(
            (uploads[2].values.min(), uploads[2].values.max()),
            (1.5, 3.0)
        );
        assert_eq!(
            (uploads[0].values.min(), uploads[0].values.max()),
            (0.0, 1.0)
        );
        assert_eq!(
            series
                .iter()
                .map(|s| s.series_id.as_str())
                .collect::<Vec<_>>(),
            ["s", "t"]
        );
        assert_eq!(series[0].x_column, CATEGORY_POSITIONS_COLUMN);
        assert_eq!(series[1].y_column, category_values_column("t"));
        assert_eq!(config.validate_category_bars(&series), Ok(()));
    }

    #[test]
    fn series_colour_label_and_style_carry_over() {
        let mut c = chart();
        c.style.material = Material::Paper;
        c.series[1].style = Some(CategoryBarStyle {
            corner_radius: 0.0,
            ..Default::default()
        });
        let (_, series, _) = Config::from_categorical(&c);
        assert_eq!(bars(&series[0]).color, c.series[0].color);
        assert_eq!(bars(&series[0]).style, &c.style);
        assert_eq!(bars(&series[1]).style, c.series[1].style.as_ref().unwrap());
        let label = series[0].label.as_ref().unwrap();
        assert_eq!(
            label.segments.iter().map(|s| s.text).collect::<String>(),
            "Sales"
        );
        assert_eq!((label.font.as_str(), label.font_size), ("sans-serif", 14.0));
    }

    #[test]
    fn mode_direction_group_width_and_gap_are_the_bar_groups() {
        let mut c = chart();
        c.mode = CategoryBarMode::Stacked;
        c.direction = CategoryBarDirection::Horizontal;
        c.group_width = 0.5;
        c.bar_gap = 6.0;
        let (config, _, _) = Config::from_categorical(&c);
        assert_eq!(
            config.bar_groups,
            Some(BarGroups {
                mode: CategoryBarMode::Stacked,
                direction: CategoryBarDirection::Horizontal,
                group_width: 0.5,
                bar_gap: 6.0,
            })
        );
    }

    #[test]
    fn label_settings_go_to_every_series_and_write_the_text() {
        let mut c = chart();
        c.labels = CategoryBarLabels::Inside;
        c.label_format = CategoryBarLabelFormat::ValuePercent;
        c.label_decimals = 2;
        c.value_suffix = "%pt".into();
        let (_, series, _) = Config::from_categorical(&c);
        for s in &series {
            let b = bars(s);
            assert_eq!(b.labels.placement, CategoryBarLabels::Inside);
            assert_eq!(b.labels.decimals, 2);
            assert_eq!(b.labels.value_suffix, "%pt");
            assert_eq!(b.label_format, CategoryBarLabelFormat::ValuePercent);
        }
        assert_eq!(bars(&series[0]).labels.text, ["2%pt (40%)", ""]);
        assert_eq!(
            bars(&series[1]).labels.text,
            ["3%pt (60%)", "1.5%pt (100%)"]
        );
    }

    #[test]
    fn overrides_go_to_their_series() {
        let mut c = chart();
        let mut o = CategoryBarOverride::new(CategoryBarTarget::new("b", "t"));
        o.color = Some(Color::WHITE);
        o.label_format = Some(CategoryBarLabelFormat::Percent);
        c.overrides.push(o.clone());
        let (_, series, _) = Config::from_categorical(&c);
        assert!(bars(&series[0]).overrides.is_empty());
        assert_eq!(bars(&series[1]).overrides, [o]);
        assert_eq!(bars(&series[1]).labels.text[1], "100%");
    }

    #[test]
    fn titles_become_the_axis_titles_and_hide_when_empty() {
        let mut c = chart();
        let (config, _, _) = Config::from_categorical(&c);
        assert!(!config.bottom_x.title_option.visible);
        assert!(!config.left_y.title_option.visible);
        c.value_title = "Revenue".into();
        c.category_title = "Region".into();
        let (config, _, _) = Config::from_categorical(&c);
        let text = |t: &RichText| t.segments.iter().map(|s| s.text).collect::<String>();
        assert!(config.left_y.title_option.visible);
        assert_eq!(text(&config.left_y.title_option.text), "Revenue");
        assert_eq!(text(&config.bottom_x.title_option.text), "Region");
        c.direction = CategoryBarDirection::Horizontal;
        let (config, _, _) = Config::from_categorical(&c);
        assert_eq!(text(&config.bottom_x.title_option.text), "Revenue");
        assert_eq!(text(&config.left_y.title_option.text), "Region");
    }

    #[test]
    fn the_chart_title_is_the_resolved_title() {
        let mut c = chart();
        c.title = "Quarter".into();
        let (config, _, _) = Config::from_categorical(&c);
        assert_eq!(config.chart_title.text, c.resolved_title().text);
        assert!(config.chart_title.visible);
        c.normalize_title();
        c.chart_title.as_mut().unwrap().visible = false;
        let (config, _, _) = Config::from_categorical(&c);
        assert!(!config.chart_title.visible);
    }

    #[test]
    fn the_font_and_text_colour_style_both_axes_and_the_legend() {
        let mut c = chart();
        c.font_family = "Serif".into();
        c.font_size = 18.0;
        c.text_color = Color::from_rgb8(10, 20, 30);
        let (config, _, _) = Config::from_categorical(&c);
        for axis in [&config.bottom_x, &config.left_y] {
            assert_eq!(axis.label_style.label_font, "Serif");
            assert_eq!(axis.label_style.font_size, 18.0);
            assert_eq!(axis.label_style.color, c.text_color);
            assert_eq!(axis.line_color, c.text_color);
            assert_eq!(axis.title_option.text.font, "Serif");
            assert_eq!(axis.tick, TickVisibility::None);
        }
        assert_eq!(config.legend.content.font, "Serif");
        assert_eq!(config.legend.content.font_size, 18.0);
        assert_eq!(config.legend.content.color, c.text_color);
        assert!(!config.top_x.line_visible && !config.right_y.line_visible);
        assert_eq!(
            config.left_y.label_style.format,
            LabelFormat::Plain(PlainNumberFormat::AUTO)
        );
    }

    #[test]
    fn the_grid_is_the_value_axis_major_grid() {
        let mut c = chart();
        let (config, _, _) = Config::from_categorical(&c);
        assert!(config.grid.show_major_y && !config.grid.show_major_x);
        assert_eq!(config.grid.major_y_color, LEGACY_GRID_COLOR);
        c.direction = CategoryBarDirection::Horizontal;
        let (config, _, _) = Config::from_categorical(&c);
        assert!(config.grid.show_major_x && !config.grid.show_major_y);
        c.grid = false;
        let (config, _, _) = Config::from_categorical(&c);
        assert!(!config.grid.show_major_x && !config.grid.show_major_y);
    }

    #[test]
    fn the_legend_lists_every_series() {
        let mut c = chart();
        let (config, _, _) = Config::from_categorical(&c);
        assert!(config.legend.visible);
        let text: String = config
            .legend
            .content
            .segments
            .iter()
            .map(|s| s.text)
            .collect();
        assert!(text.contains("Sales") && text.contains("Costs"), "{text}");
        c.legend = false;
        assert!(!Config::from_categorical(&c).0.legend.visible);
    }

    #[test]
    fn the_category_axis_line_crosses_the_value_axis_at_zero() {
        let mut c = chart();
        let (config, _, _) = Config::from_categorical(&c);
        assert_eq!(config.bottom_x.cross_at, Some(0.0));
        assert_eq!(config.left_y.cross_at, None);
        c.direction = CategoryBarDirection::Horizontal;
        let (config, _, _) = Config::from_categorical(&c);
        assert_eq!(config.left_y.cross_at, Some(0.0));
        assert_eq!(config.bottom_x.cross_at, None);
    }

    #[test]
    fn the_selection_is_the_typed_selection_and_hover_is_dropped() {
        let mut c = chart();
        c.hovered = Some(CategoryBarTarget::new("a", "s"));
        let (config, _, _) = Config::from_categorical(&c);
        assert_eq!(config.picked_data, None);
        c.selected = Some(CategoryBarTarget::new("b", "t"));
        let (config, _, _) = Config::from_categorical(&c);
        assert_eq!(
            config.picked_data.unwrap().refs,
            [PickedDataRef::HistogramBin {
                source_id: None,
                series_id: "t".into(),
                bin_index: 1,
            }]
        );
    }

    #[test]
    fn the_active_layer_and_annotations_carry_over() {
        let mut c = chart();
        c.set_active_layer(LayerRef::Annotations);
        c.annotations.visible = false;
        let (config, _, _) = Config::from_categorical(&c);
        assert_eq!(config.active_layer, LayerRef::Annotations);
        assert_eq!(config.annotations, c.annotations);
    }

    #[test]
    fn decoration_offsets_become_the_matching_offsets() {
        let mut c = chart();
        let offset = |element, offset| DecorationOffset { element, offset };
        c.decorations.offsets = vec![
            offset(DecorationId::ChartTitle, [3.0, 4.0]),
            offset(DecorationId::Legend, [-5.0, 6.0]),
            offset(DecorationId::Axis(Side::Left), [7.0, 99.0]),
            offset(DecorationId::Axis(Side::Bottom), [99.0, 8.0]),
            offset(DecorationId::AxisLabel(Side::Bottom), [1.0, 2.0]),
            offset(DecorationId::AxisTitle(Side::Left), [-1.0, -2.0]),
            offset(DecorationId::DataArea, [50.0, 50.0]),
        ];
        let (config, _, _) = Config::from_categorical(&c);
        assert_eq!(
            (config.chart_title.offset_x, config.chart_title.offset_y),
            (3.0, 4.0)
        );
        assert_eq!(
            (config.legend.offset_x, config.legend.offset_y),
            (-5.0, 6.0)
        );
        assert_eq!(config.left_y.line_offset, 7.0);
        assert_eq!(config.bottom_x.line_offset, 8.0);
        let label = &config.bottom_x.label_style;
        assert_eq!((label.label_offset_x, label.label_offset_y), (1.0, 2.0));
        let title = &config.left_y.title_option;
        assert_eq!((title.offset_x, title.offset_y), (-1.0, -2.0));
    }

    #[test]
    fn background_and_the_explicit_data_area_are_not_carried() {
        let mut c = chart();
        let (before, _, _) = Config::from_categorical(&c);
        c.background = Color::BLACK;
        c.decorations.data_area = Some(crate::layout::RectF {
            x: 100.0,
            y: 100.0,
            width: 300.0,
            height: 200.0,
        });
        let (after, _, _) = Config::from_categorical(&c);
        assert_eq!(after, before);
    }
}
