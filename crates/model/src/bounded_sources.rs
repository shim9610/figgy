//! The readers of the legacy categorical bar, box plot and radar documents,
//! and the borrowed views their inline values upload through.
//!
//! A [`CategoricalChart`] keeps its values inline (`series[].values`). A
//! categorical bar chart is a Cartesian chart now, whose values live in the
//! renderer's column pool like every other series' data, so the reader turns
//! the document into a [`Config`], its series and one [`SeriesUpload`] per
//! column. An upload borrows the document's values ([`UploadValues`]); the
//! renderer implements its `ColumnSource` for that view, so the values go
//! straight from the document into the pool and are not copied on the way.
//!
//! A [`BoxPlotChart`] keeps a nine-field summary and the outliers of every
//! box inline. A box plot is a Cartesian series kind over a record and a
//! ragged column of the pool, so `Config::from_boxplot` returns, beside the
//! category positions, one record upload and one ragged upload per series;
//! their field, value and offset adapters ([`BoxColumn`]) read the summaries
//! on the fly as they are written.
//!
//! A [`RadarChart`] keeps one value per category inline. A radar chart is a
//! polar chart with a categorical angle axis whose radar series read their
//! values from the pool, so `Config::from_radar` returns the category
//! indices and one value column per series, each borrowing the document's
//! values ([`UploadValues::Dense`]).
//!
//! The three document types are deprecated with their readers' names and
//! are removed in the next milestone.
// The readers take the deprecated document types.
#![allow(deprecated)]

use crate::boxplot::{BoxPlotChart, BoxPlotDirection, BoxSummary};
use crate::categorical::CategoricalChart;
use crate::color::Color;
use crate::config::{
    AxisOptions, AxisTitleOptions, Config, DataSelectionsConfig, GridOptions, PickedDataRef,
    TickVisibility,
};
use crate::data::ColumnId;
use crate::data_config::{
    BarGroups, BoxPlotSeriesLabels, CategoryBarDirection, CategoryBarMode, CategoryBarSeriesLabels,
    DataRenderType, RadarSeriesLabels, SeriesConfig, format_box_plot_labels, format_labels,
    format_radar_labels,
};
use crate::decoration::DecorationId;
use crate::format::{LabelFormat, PlainNumberFormat};
use crate::layout::{Side, axis_mut};
use crate::polar::{AngleUnit, GridShape, PolarInterpolation, PolarOptions};
use crate::radar::{RadarChart, RadarGridShape};
use crate::text::RichText;

/// The id of the column holding the category positions of a converted
/// categorical chart.
pub const CATEGORY_POSITIONS_COLUMN: &str = "categorical.positions";

/// The id of the column holding the values of the converted series
/// `series_id`.
pub fn category_values_column(series_id: &str) -> ColumnId {
    format!("categorical.values.{series_id}")
}

/// The id of the column holding the category positions of a converted box
/// plot document.
pub const BOX_PLOT_POSITIONS_COLUMN: &str = "boxplot.positions";

/// The id of the record holding the box statistics of the converted series
/// `series_id`.
pub fn box_plot_record(series_id: &str) -> ColumnId {
    format!("boxplot.record.{series_id}")
}

/// The id of the ragged column holding the outliers of the converted series
/// `series_id`.
pub fn box_plot_outliers(series_id: &str) -> ColumnId {
    format!("boxplot.outliers.{series_id}")
}

/// The id of the column holding the category indices of a converted radar
/// document.
pub const RADAR_POSITIONS_COLUMN: &str = "radar.positions";

/// The id of the column holding the values of the converted radar series
/// `series_id`.
pub fn radar_values_column(series_id: &str) -> ColumnId {
    format!("radar.values.{series_id}")
}

/// One field of the box plot record, in the order of the renderer's
/// `BOX_PLOT_RECORD` schema.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoxField {
    Median,
    Q1,
    Q3,
    WhiskerLow,
    WhiskerHigh,
    Mean,
    SampleCount,
    CiLow,
    CiHigh,
}

impl BoxField {
    /// Every field, in schema order.
    pub const ALL: [BoxField; 9] = [
        BoxField::Median,
        BoxField::Q1,
        BoxField::Q3,
        BoxField::WhiskerLow,
        BoxField::WhiskerHigh,
        BoxField::Mean,
        BoxField::SampleCount,
        BoxField::CiLow,
        BoxField::CiHigh,
    ];

    /// The field's name in the record schema.
    pub fn name(self) -> &'static str {
        match self {
            BoxField::Median => "median",
            BoxField::Q1 => "q1",
            BoxField::Q3 => "q3",
            BoxField::WhiskerLow => "whisker_low",
            BoxField::WhiskerHigh => "whisker_high",
            BoxField::Mean => "mean",
            BoxField::SampleCount => "sample_count",
            BoxField::CiLow => "ci_low",
            BoxField::CiHigh => "ci_high",
        }
    }

    /// The field of `summary`, NaN when the summary does not hold it.
    pub fn of(self, summary: &BoxSummary) -> f64 {
        match self {
            BoxField::Median => summary.median,
            BoxField::Q1 => summary.q1,
            BoxField::Q3 => summary.q3,
            BoxField::WhiskerLow => summary.whisker_low,
            BoxField::WhiskerHigh => summary.whisker_high,
            BoxField::Mean => summary.mean.unwrap_or(f64::NAN),
            // A count is an integer; up to 2^53 it is exact as a value.
            BoxField::SampleCount => summary.sample_count.map_or(f64::NAN, |n| n as f64),
            BoxField::CiLow => summary.median_ci.map_or(f64::NAN, |ci| ci[0]),
            BoxField::CiHigh => summary.median_ci.map_or(f64::NAN, |ci| ci[1]),
        }
    }
}

/// One column of a box plot document's statistics, read on the fly from the
/// borrowed summaries of one series: a record field, the outliers of every
/// box in category order, or the offsets that split them into one group per
/// category. Nothing is copied: every value is computed from the summaries
/// when it is written.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BoxColumn<'a> {
    /// One record field, one row per category; a missing box is NaN.
    Field(&'a [Option<BoxSummary>], BoxField),
    /// The outlier values, group after group.
    OutlierValues(&'a [Option<BoxSummary>]),
    /// `categories + 1` offsets: group `g` is values
    /// `offsets[g]..offsets[g + 1]`.
    OutlierOffsets(&'a [Option<BoxSummary>]),
}

impl BoxColumn<'_> {
    /// The number of values.
    pub fn len(&self) -> usize {
        match self {
            BoxColumn::Field(summaries, _) => summaries.len(),
            BoxColumn::OutlierValues(summaries) => outlier_count(summaries),
            BoxColumn::OutlierOffsets(summaries) => summaries.len() + 1,
        }
    }

    /// Whether there are no values.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Every value in order, `f(index, value)`, walking the summaries once.
    pub fn for_each(&self, mut f: impl FnMut(usize, f64)) {
        match self {
            BoxColumn::Field(summaries, field) => {
                for (i, summary) in summaries.iter().enumerate() {
                    f(i, summary.as_ref().map_or(f64::NAN, |s| field.of(s)));
                }
            }
            BoxColumn::OutlierValues(summaries) => {
                let mut i = 0;
                for summary in summaries.iter().flatten() {
                    for value in &summary.outliers {
                        f(i, *value);
                        i += 1;
                    }
                }
            }
            BoxColumn::OutlierOffsets(summaries) => {
                let mut offset = 0usize;
                f(0, 0.0);
                for (g, summary) in summaries.iter().enumerate() {
                    offset += summary.as_ref().map_or(0, |s| s.outliers.len());
                    f(g + 1, offset as f64);
                }
            }
        }
    }

    /// Value `index`, NaN past the end.
    pub fn value(&self, index: usize) -> f64 {
        let mut out = f64::NAN;
        self.for_each(|i, v| {
            if i == index {
                out = v;
            }
        });
        out
    }

    /// The smallest finite value, `+inf` when there is none.
    pub fn min(&self) -> f64 {
        let mut min = f64::INFINITY;
        self.for_each(|_, v| {
            if v.is_finite() {
                min = min.min(v);
            }
        });
        min
    }

    /// The largest finite value, `-inf` when there is none.
    pub fn max(&self) -> f64 {
        let mut max = f64::NEG_INFINITY;
        self.for_each(|_, v| {
            if v.is_finite() {
                max = max.max(v);
            }
        });
        max
    }
}

fn outlier_count(summaries: &[Option<BoxSummary>]) -> usize {
    summaries.iter().flatten().map(|s| s.outliers.len()).sum()
}

/// The values one column of a converted document uploads, borrowed from the
/// document.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum UploadValues<'a> {
    /// The category positions `0, 1, ..., n - 1`.
    Positions(usize),
    /// A series' values in category order; a missing value uploads NaN.
    Values(&'a [Option<f64>]),
    /// A radar series' values in category order, as the document holds them:
    /// a non-finite value uploads as it is and breaks the polygon there.
    Dense(&'a [f64]),
    /// The box statistics of one box plot series, one row per category:
    /// not one column but a record (`Renderer::upsert_record` with the box
    /// plot schema), whose fields are [`UploadValues::box_fields`]. As a
    /// single column it holds no values.
    BoxRecord(&'a [Option<BoxSummary>]),
    /// The outliers of one box plot series, one group per category: not one
    /// column but a ragged column (`Renderer::upsert_ragged`), whose values
    /// and offsets are [`UploadValues::box_outliers`]. As a single column it
    /// holds no values.
    BoxOutliers(&'a [Option<BoxSummary>]),
}

impl<'a> UploadValues<'a> {
    /// The field adapters of a [`UploadValues::BoxRecord`], in the order of
    /// the box plot record schema ([`BoxField::ALL`]); `None` for any other
    /// upload.
    pub fn box_fields(&self) -> Option<[BoxColumn<'a>; 9]> {
        match self {
            UploadValues::BoxRecord(summaries) => {
                Some(BoxField::ALL.map(|field| BoxColumn::Field(summaries, field)))
            }
            _ => None,
        }
    }

    /// The values and the offsets of a [`UploadValues::BoxOutliers`];
    /// `None` for any other upload.
    pub fn box_outliers(&self) -> Option<(BoxColumn<'a>, BoxColumn<'a>)> {
        match self {
            UploadValues::BoxOutliers(summaries) => Some((
                BoxColumn::OutlierValues(summaries),
                BoxColumn::OutlierOffsets(summaries),
            )),
            _ => None,
        }
    }
}

impl UploadValues<'_> {
    /// The number of values; none for a record or a ragged column, which
    /// are not one column.
    pub fn len(&self) -> usize {
        match self {
            UploadValues::Positions(n) => *n,
            UploadValues::Values(values) => values.len(),
            UploadValues::Dense(values) => values.len(),
            UploadValues::BoxRecord(_) | UploadValues::BoxOutliers(_) => 0,
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
            UploadValues::Dense(values) => values.get(index).copied().unwrap_or(f64::NAN),
            UploadValues::BoxRecord(_) | UploadValues::BoxOutliers(_) => f64::NAN,
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

/// One upload of a converted document: the id its series read and the
/// values, borrowed from the document. A box plot's record and outliers are
/// uploads too, under their record and ragged ids.
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
    /// `decorations.data_area` and the decoration selection. The data label
    /// offsets are the series' value label offsets (`labels.offsets`). The
    /// chart area is the default placeholder; the host sets it.
    pub fn from_categorical(
        chart: &CategoricalChart,
    ) -> (Config, Vec<SeriesConfig>, Vec<SeriesUpload<'_>>) {
        let horizontal = chart.direction == CategoryBarDirection::Horizontal;
        let mut config = legacy_config(&LegacyChrome {
            horizontal,
            categories: &chart.categories,
            value_title: &chart.value_title,
            category_title: &chart.category_title,
            font_family: &chart.font_family,
            font_size: chart.font_size,
            text_color: chart.text_color,
            grid: chart.grid,
            title: chart.resolved_title(),
            active_layer: chart.active_layer(),
            annotations: &chart.annotations,
        });
        // The category axis line is the zero line of the value axis.
        crate::layout::axis_mut(
            &mut config,
            if horizontal {
                &Side::Left
            } else {
                &Side::Bottom
            },
        )
        .cross_at = Some(0.0);
        let text = |s: &str| {
            RichText::plain(
                s,
                chart.text_color,
                chart.font_size,
                chart.font_family.as_str(),
            )
        };

        config.bar_groups = Some(BarGroups {
            mode: chart.mode,
            direction: chart.direction,
            group_width: chart.group_width,
            bar_gap: chart.bar_gap,
        });

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
                        offsets: Vec::new(),
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

        legacy_legend(
            &mut config,
            &series,
            chart.legend,
            (&chart.font_family, chart.font_size, chart.text_color),
            |id| {
                chart
                    .series
                    .iter()
                    .find(|s| s.id == id)
                    .map(|s| s.label.as_str())
            },
        );
        if let Some(selected) = &chart.selected {
            legacy_selection(
                &mut config,
                &chart.categories,
                &selected.category_id,
                &selected.series_id,
            );
        }
        legacy_offsets(&mut config, &chart.decorations);
        legacy_label_offsets(&mut series, &chart.categories, &chart.decorations);
        (config, series, uploads)
    }
}

/// The chrome of a legacy bounded document with a category axis and a
/// value axis: what [`legacy_config`] maps.
struct LegacyChrome<'a> {
    horizontal: bool,
    categories: &'a [crate::config::Category],
    value_title: &'a str,
    category_title: &'a str,
    font_family: &'a str,
    font_size: f32,
    text_color: Color,
    grid: bool,
    title: crate::BoundedTitleOptions,
    active_layer: crate::layer::LayerRef,
    annotations: &'a crate::AnnotationLayer,
}

/// The Cartesian config of a legacy bounded document: the categorical axis
/// of the direction (`bottom_x` vertical, `left_y` horizontal and top-down,
/// as the legacy charts drew it, so annotations on `Y` land on the same
/// categories), the value axis with the plain label format, both axis
/// titles, fonts and line colours, no ticks, the top and right axes hidden,
/// the value axis' major grid in the bounded grid colour, the chart title,
/// the active layer and the annotations.
fn legacy_config(chrome: &LegacyChrome<'_>) -> Config {
    let horizontal = chrome.horizontal;
    let text =
        |s: &str| RichText::plain(s, chrome.text_color, chrome.font_size, chrome.font_family);
    let style_axis = |axis: &mut AxisOptions, title: &str| {
        axis.label_style.color = chrome.text_color;
        axis.label_style.font_size = chrome.font_size;
        axis.label_style.label_font = chrome.font_family.to_owned();
        axis.tick = TickVisibility::None;
        axis.title_option = AxisTitleOptions {
            text: text(title),
            visible: !title.is_empty(),
            offset_x: 0.0,
            offset_y: 0.0,
        };
        axis.line_color = chrome.text_color;
        axis.line_width = 1.0;
    };

    let mut config = crate::default::default_config();
    let mut category_axis = config.bottom_x.clone();
    style_axis(&mut category_axis, chrome.category_title);
    category_axis.categories = Some(chrome.categories.to_vec());
    let mut value_axis = config.left_y.clone();
    style_axis(&mut value_axis, chrome.value_title);
    value_axis.label_style.format = LabelFormat::Plain(PlainNumberFormat::AUTO);
    category_axis.inverted = horizontal;
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
    *show_major_x = chrome.grid && horizontal;
    *show_major_y = chrome.grid && !horizontal;
    *show_minor_x = false;
    *show_minor_y = false;
    *major_x_color = LEGACY_GRID_COLOR;
    *major_y_color = LEGACY_GRID_COLOR;

    config.chart_title.text = chrome.title.text.clone();
    config.chart_title.visible = chrome.title.visible;
    config.active_layer = chrome.active_layer;
    config.annotations = chrome.annotations.clone();
    config
}

/// The legend of a converted document: visible when `visible`, in the
/// document's `(font, size, colour)`, one entry per series with its label
/// (`label` of a series id).
fn legacy_legend<'a>(
    config: &mut Config,
    series: &[SeriesConfig],
    visible: bool,
    (font, font_size, color): (&str, f32, Color),
    label: impl Fn(&str) -> Option<&'a str>,
) {
    config.legend.visible = visible;
    config.legend.content.font = font.to_owned();
    config.legend.content.font_size = font_size;
    config.legend.content.color = color;
    for item in series {
        crate::legend::append_legend_entry(
            &mut config.legend.content,
            crate::legend::series_symbol_segments(item, None),
            label(&item.series_id).unwrap_or(""),
        );
    }
}

/// The typed selection of the selected `(category, series)` of a converted
/// document: a histogram bin of the series whose index is the category row.
/// A category the document does not hold selects nothing.
fn legacy_selection(
    config: &mut Config,
    categories: &[crate::config::Category],
    category_id: &str,
    series_id: &str,
) {
    if let Some(index) = categories.iter().position(|c| c.id == category_id) {
        let mut picked = DataSelectionsConfig::default();
        picked.refs.push(PickedDataRef::HistogramBin {
            source_id: None,
            series_id: series_id.to_owned(),
            bin_index: index,
        });
        config.picked_data = Some(picked);
    }
}

/// The decoration offsets of a legacy document as the matching Cartesian
/// offsets: the title, the legend, the axis lines (across themselves), tick
/// labels and titles. Every other element's offset is not carried.
fn legacy_offsets(config: &mut Config, decorations: &crate::DecorationLayout) {
    for offset in &decorations.offsets {
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
                axis_mut(config, side).line_offset = match side {
                    Side::Left | Side::Right => dx,
                    Side::Top | Side::Bottom => dy,
                };
            }
            DecorationId::AxisLabel(side) => {
                let axis = axis_mut(config, side);
                axis.label_style.label_offset_x = dx;
                axis.label_style.label_offset_y = dy;
            }
            DecorationId::AxisTitle(side) => {
                let axis = axis_mut(config, side);
                axis.title_option.offset_x = dx;
                axis.title_option.offset_y = dy;
            }
            _ => {}
        }
    }
}

/// The data label offsets of a legacy document as the value label offsets
/// of its series (`labels.offsets`): a [`DecorationId::DataLabel`] names its
/// category and series by id, and its offset is in the same chart-local
/// logical pixels. An offset naming no category or series of the document
/// is dropped.
fn legacy_label_offsets(
    series: &mut [SeriesConfig],
    categories: &[crate::config::Category],
    decorations: &crate::DecorationLayout,
) {
    for offset in &decorations.offsets {
        let DecorationId::DataLabel {
            category_id,
            series_id,
        } = &offset.element
        else {
            continue;
        };
        let Some(index) = categories.iter().position(|c| &c.id == category_id) else {
            continue;
        };
        let Some(item) = series.iter_mut().find(|s| &s.series_id == series_id) else {
            continue;
        };
        let [dx, dy] = offset.offset;
        let _ = item.drag_value_label(index, categories.len(), dx, dy);
    }
}

impl Config {
    /// The Cartesian chart a legacy [`BoxPlotChart`] document draws: its
    /// `Config`, its series and what to upload.
    ///
    /// The chart is a `Config` with a categorical axis (`bottom_x` for
    /// vertical boxes, `left_y` for horizontal ones, top-down as the legacy
    /// chart drew them) holding the document's categories, a grouped
    /// `bar_groups` from `direction`, `group_width` and `box_gap`, and one
    /// [`DataRenderType::BoxPlot`] series per document series in document
    /// order. Each reads the shared [`BOX_PLOT_POSITIONS_COLUMN`] as its
    /// `x_column`, its own [`box_plot_record`] as its `y_column` and, when
    /// any of its boxes has an outlier, its own [`box_plot_outliers`], and
    /// takes its colour, its style (or the chart's), the chart's label
    /// settings, its overrides, and the label text
    /// [`format_box_plot_labels`] writes from the document's medians and
    /// sample counts.
    ///
    /// The uploads are the positions column first, then per series its
    /// record ([`UploadValues::BoxRecord`]: `Renderer::upsert_record` with
    /// the box plot schema and the field adapters of
    /// [`UploadValues::box_fields`]) and its outliers when it has any
    /// ([`UploadValues::BoxOutliers`]: `Renderer::upsert_ragged` with the
    /// adapters of [`UploadValues::box_outliers`]). They borrow the
    /// document's summaries and compute every value from them as it is
    /// written; a missing box is a row of NaN.
    ///
    /// Field map: `value_range` is the value axis range with nice tick
    /// spacing (without it the range is the host's to fit, as the former
    /// chart fitted every supplied value); `value_title` and
    /// `category_title` are the axis titles (in `font_family`, `font_size`
    /// and `text_color`, hidden when empty); `font_family`, `font_size` and
    /// `text_color` are both axes' tick labels and lines; `grid` is the
    /// value axis' major grid in the bounded grid colour; `legend` is the
    /// Cartesian legend's visibility with one entry per series; the
    /// resolved title is `chart_title`; the decoration offsets of the title,
    /// the axis lines, tick labels and titles and the legend are the
    /// matching offsets; `selected` is the typed selection; `active_layer`
    /// and `annotations` carry over. Not carried: `hovered` (transient),
    /// `background` (the host's clear colour), an explicit
    /// `decorations.data_area` and the decoration selection. The data label
    /// offsets are the series' value label offsets (`labels.offsets`). The
    /// chart area is the default placeholder; the host sets it.
    pub fn from_boxplot(
        chart: &BoxPlotChart,
    ) -> (Config, Vec<SeriesConfig>, Vec<SeriesUpload<'_>>) {
        let horizontal = chart.direction == BoxPlotDirection::Horizontal;
        let mut config = legacy_config(&LegacyChrome {
            horizontal,
            categories: &chart.categories,
            value_title: &chart.value_title,
            category_title: &chart.category_title,
            font_family: &chart.font_family,
            font_size: chart.font_size,
            text_color: chart.text_color,
            grid: chart.grid,
            title: chart.resolved_title(),
            active_layer: chart.active_layer(),
            annotations: &chart.annotations,
        });
        if let Some([min, max]) = chart.value_range {
            let value = if horizontal {
                &mut config.bottom_x
            } else {
                &mut config.left_y
            };
            // The fixed range is kept as it is; only the spacing is nice.
            if let Ok(plan) = AxisOptions::compute_nice_ticks(value.scale.clone(), min, max, 5) {
                value.major_spacing = plan.major_spacing;
                value.minor_count = plan.minor_count;
            }
            value.min = min;
            value.max = max;
            let fitted = value.clone();
            let twin = if horizontal {
                &mut config.top_x
            } else {
                &mut config.right_y
            };
            twin.min = fitted.min;
            twin.max = fitted.max;
            twin.major_spacing = fitted.major_spacing;
            twin.minor_count = fitted.minor_count;
        }
        config.bar_groups = Some(BarGroups {
            mode: CategoryBarMode::Grouped,
            direction: if horizontal {
                CategoryBarDirection::Horizontal
            } else {
                CategoryBarDirection::Vertical
            },
            group_width: chart.group_width,
            bar_gap: chart.box_gap,
        });
        let text = |s: &str| {
            RichText::plain(
                s,
                chart.text_color,
                chart.font_size,
                chart.font_family.as_str(),
            )
        };

        let mut series = Vec::with_capacity(chart.series.len());
        let mut uploads = Vec::with_capacity(chart.series.len() * 2 + 1);
        uploads.push(SeriesUpload {
            column: BOX_PLOT_POSITIONS_COLUMN.into(),
            values: UploadValues::Positions(chart.categories.len()),
        });
        for s in &chart.series {
            let overrides: Vec<_> = chart
                .overrides
                .iter()
                .filter(|o| o.target.series_id == s.id)
                .cloned()
                .collect();
            let record = box_plot_record(&s.id);
            let has_outliers = s.values.iter().flatten().any(|v| !v.outliers.is_empty());
            let outliers = has_outliers.then(|| box_plot_outliers(&s.id));
            series.push(SeriesConfig {
                series_id: s.id.clone(),
                source_id: None,
                label: Some(text(&s.label)),
                x_column: BOX_PLOT_POSITIONS_COLUMN.into(),
                y_column: record.clone(),
                render_type: DataRenderType::BoxPlot {
                    color: s.color,
                    style: s.style.clone().unwrap_or_else(|| chart.style.clone()),
                    labels: BoxPlotSeriesLabels {
                        placement: chart.labels,
                        decimals: chart.label_decimals,
                        value_suffix: chart.value_suffix.clone(),
                        text: Vec::new(),
                        offsets: Vec::new(),
                    },
                    outliers: outliers.clone(),
                    overrides: (!overrides.is_empty()).then_some(overrides),
                },
            });
            uploads.push(SeriesUpload {
                column: record,
                values: UploadValues::BoxRecord(&s.values),
            });
            if let Some(column) = outliers {
                uploads.push(SeriesUpload {
                    column,
                    values: UploadValues::BoxOutliers(&s.values),
                });
            }
        }
        format_box_plot_labels(&mut series, &chart.categories, |k, i| {
            let summary = chart.series.get(k)?.values.get(i)?.as_ref()?;
            Some((summary.median, summary.sample_count))
        });

        legacy_legend(
            &mut config,
            &series,
            chart.legend,
            (&chart.font_family, chart.font_size, chart.text_color),
            |id| {
                chart
                    .series
                    .iter()
                    .find(|s| s.id == id)
                    .map(|s| s.label.as_str())
            },
        );
        if let Some(selected) = &chart.selected {
            legacy_selection(
                &mut config,
                &chart.categories,
                &selected.category_id,
                &selected.series_id,
            );
        }
        legacy_offsets(&mut config, &chart.decorations);
        legacy_label_offsets(&mut series, &chart.categories, &chart.decorations);
        (config, series, uploads)
    }
}

impl Config {
    /// The polar chart a legacy [`RadarChart`] document draws: its `Config`,
    /// its series and the columns to upload.
    ///
    /// The chart is a polar `Config` ([`PolarOptions`]) whose angle axis
    /// (`bottom_x`) holds the document's categories, so category `i` sits
    /// at `i / n` of the turn, and one [`DataRenderType::Radar`] series per
    /// document series in document order, reading the shared
    /// [`RADAR_POSITIONS_COLUMN`] (the category indices `0..n`) and its own
    /// [`radar_values_column`]. Each series takes its colour, its style with
    /// every unset field read from the chart style
    /// ([`RadarSeriesStyle::resolved_from`](crate::data_config::RadarSeriesStyle::resolved_from)), the chart's label settings
    /// and the label text [`format_radar_labels`] writes from the document's
    /// values.
    ///
    /// The uploads are the positions column first, then the value columns
    /// in series order, borrowing the document's values
    /// ([`UploadValues::Dense`]); nothing is copied.
    ///
    /// Field map: `start_angle_degrees` (screen degrees with y down) is
    /// `zero_angle_degrees` (with y up, so negated) and the radius labels'
    /// ray, `clockwise` is `clockwise`, `grid_shape` is the polar
    /// `grid_shape`; `value_range` is the radius axis (`left_y`) range with
    /// `ring_spacing` as its major spacing, or nice spacing without one
    /// (without a `value_range` the range is the host's to fit, as the
    /// former chart fitted the supplied values; `ring_spacing` still sets
    /// the spacing); `grid` shows the rings (`left_y` major grid in
    /// `ring_color`, and the outer ring, the angle axis line, in the same
    /// colour) and the spokes (`bottom_x` major grid in `spoke_color`);
    /// `font_family`, `font_size` and `text_color` are the category and
    /// ring labels; `legend` is the legend's visibility with one entry per
    /// series; the resolved title is `chart_title`; the offsets of the
    /// title, the legend and the ring labels are the matching offsets; a
    /// selected vertex is the typed selection (a point whose index is its
    /// category); `active_layer` and `annotations` carry over. Not carried:
    /// `hovered` (transient), a selected whole series (a selection names a
    /// vertex), the style's `emphasis_brightness` (the former renderer
    /// brightened the hovered or selected series; a picked vertex takes the
    /// picked ring instead), `background` (the host's clear colour), an explicit
    /// `decorations.data_area`, the spoke label offsets (the category labels
    /// move together) and the decoration selection; the data label offsets
    /// are the series' value label offsets (`labels.offsets`). The chart area is the default
    /// placeholder; the host sets it.
    pub fn from_radar(chart: &RadarChart) -> (Config, Vec<SeriesConfig>, Vec<SeriesUpload<'_>>) {
        let mut config = crate::default::default_config();
        let text = |s: &str| {
            RichText::plain(
                s,
                chart.text_color,
                chart.font_size,
                chart.font_family.as_str(),
            )
        };
        let style_axis = |axis: &mut AxisOptions| {
            axis.label_style.color = chart.text_color;
            axis.label_style.font_size = chart.font_size;
            axis.label_style.label_font = chart.font_family.clone();
            axis.tick = TickVisibility::None;
            axis.title_option = AxisTitleOptions {
                text: text(""),
                visible: false,
                offset_x: 0.0,
                offset_y: 0.0,
            };
            axis.line_width = 1.0;
        };
        let style = &chart.style;
        // The angle axis: the categories, the outer ring as its line.
        style_axis(&mut config.bottom_x);
        config.bottom_x.categories = Some(chart.categories.to_vec());
        config.bottom_x.line_visible = chart.grid;
        config.bottom_x.line_color = style.ring_color;
        // The radius axis: the ring labels along the first spoke, without a
        // line of its own (the first spoke is drawn there).
        style_axis(&mut config.left_y);
        config.left_y.line_visible = false;
        config.left_y.line_color = chart.text_color;
        config.left_y.label_style.format = LabelFormat::Plain(PlainNumberFormat::AUTO);
        if let Some([min, max]) = chart.value_range {
            // The fixed range is kept as it is; only the spacing is nice.
            if let Ok(plan) =
                AxisOptions::compute_nice_ticks(config.left_y.scale.clone(), min, max, 5)
            {
                config.left_y.major_spacing = plan.major_spacing;
                config.left_y.minor_count = plan.minor_count;
            }
            config.left_y.min = min;
            config.left_y.max = max;
        }
        if let Some(spacing) = chart.ring_spacing {
            config.left_y.major_spacing = spacing;
        }
        for axis in [&mut config.top_x, &mut config.right_y] {
            axis.line_visible = false;
            axis.tick = TickVisibility::None;
        }
        let zero = -f64::from(style.start_angle_degrees);
        config.polar = Some(PolarOptions {
            zero_angle_degrees: zero,
            clockwise: style.clockwise,
            radial_label_angle_degrees: zero.rem_euclid(360.0),
            grid_shape: Some(match style.grid_shape {
                RadarGridShape::Polygon => GridShape::Polygon,
                RadarGridShape::Circle => GridShape::Circle,
            }),
            // A legacy radar is the full circle of the default unit, with
            // straight edges and no hole.
            angle_unit: AngleUnit::Degrees,
            sector: None,
            inner_radius_fraction: 0.0,
            interpolation: PolarInterpolation::Chord,
        });

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
        *show_major_x = chart.grid;
        *show_major_y = chart.grid;
        *show_minor_x = false;
        *show_minor_y = false;
        *major_x_color = style.spoke_color;
        *major_y_color = style.ring_color;

        let title = chart.resolved_title();
        config.chart_title.text = title.text;
        config.chart_title.visible = title.visible;
        config.active_layer = chart.active_layer();
        config.annotations = chart.annotations.clone();

        let mut series = Vec::with_capacity(chart.series.len());
        let mut uploads = Vec::with_capacity(chart.series.len() + 1);
        uploads.push(SeriesUpload {
            column: RADAR_POSITIONS_COLUMN.into(),
            values: UploadValues::Positions(chart.categories.len()),
        });
        for s in &chart.series {
            let column = radar_values_column(&s.id);
            series.push(SeriesConfig {
                series_id: s.id.clone(),
                source_id: None,
                label: Some(text(&s.label)),
                x_column: RADAR_POSITIONS_COLUMN.into(),
                y_column: column.clone(),
                render_type: DataRenderType::Radar {
                    color: s.color,
                    style: s.style.clone().unwrap_or_default().resolved_from(style),
                    labels: RadarSeriesLabels {
                        placement: chart.labels,
                        decimals: chart.label_decimals,
                        value_suffix: chart.value_suffix.clone(),
                        text: Vec::new(),
                        offsets: Vec::new(),
                    },
                },
            });
            uploads.push(SeriesUpload {
                column,
                values: UploadValues::Dense(&s.values),
            });
        }
        format_radar_labels(&mut series, &chart.categories, |k, i| {
            chart.series.get(k)?.values.get(i).copied()
        });

        legacy_legend(
            &mut config,
            &series,
            chart.legend,
            (&chart.font_family, chart.font_size, chart.text_color),
            |id| {
                chart
                    .series
                    .iter()
                    .find(|s| s.id == id)
                    .map(|s| s.label.as_str())
            },
        );
        if let Some(selected) = &chart.selected
            && let Some(category) = selected.category_id.as_deref()
            && let Some(index) = chart.categories.iter().position(|c| c.id == category)
        {
            let mut picked = DataSelectionsConfig::default();
            picked.refs.push(PickedDataRef::Point {
                source_id: None,
                series_id: selected.series_id.clone(),
                point_index: index,
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
                // The ring labels: along and across the ray in a polar
                // chart, as the legacy offset moved them on screen only
                // when the ray points right.
                DecorationId::AxisLabel(Side::Left) => {
                    config.left_y.label_style.label_offset_x = dx;
                    config.left_y.label_style.label_offset_y = dy;
                }
                _ => {}
            }
        }
        legacy_label_offsets(&mut series, &chart.categories, &chart.decorations);
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
        assert!(!config.bottom_x.inverted);
        let mut h = chart();
        h.direction = CategoryBarDirection::Horizontal;
        let (config, series, _) = Config::from_categorical(&h);
        assert_eq!(config.left_y.categories.as_deref(), Some(&h.categories[..]));
        assert_eq!(config.bottom_x.categories, None);
        // Top-down, as the legacy chart drew them.
        assert!(config.left_y.inverted);
        assert_eq!(config.left_y.validate_categories(), Ok(()));
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
        c.chart_title = Some(c.resolved_title());
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
        c.active_layer = LayerRef::Annotations;
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
    fn data_label_offsets_become_the_value_label_offsets() {
        let mut c = chart();
        let label = |category: &str, series: &str| DecorationId::DataLabel {
            category_id: category.into(),
            series_id: series.into(),
        };
        c.decorations.offsets = vec![
            DecorationOffset {
                element: label("b", "t"),
                offset: [4.0, -6.0],
            },
            // Naming no category or no series of the document: dropped.
            DecorationOffset {
                element: label("zz", "t"),
                offset: [1.0, 1.0],
            },
            DecorationOffset {
                element: label("a", "zz"),
                offset: [1.0, 1.0],
            },
        ];
        let (_, series, _) = Config::from_categorical(&c);
        assert_eq!(bars(&series[0]).labels.offsets, Vec::<[f32; 2]>::new());
        assert_eq!(bars(&series[1]).labels.offsets, [[0.0, 0.0], [4.0, -6.0]]);
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

#[cfg(test)]
mod box_tests {
    use super::*;
    use crate::boxplot::{
        BoxOutlierShape, BoxPlotLabels, BoxPlotOverride, BoxPlotSeries, BoxPlotStyle, BoxPlotTarget,
    };
    use crate::config::Category;
    use crate::decoration::DecorationOffset;
    use crate::layer::LayerRef;
    use crate::{BoxPlotRef, Material};

    fn summary(low: f64, q1: f64, med: f64, q3: f64, high: f64) -> BoxSummary {
        BoxSummary::new(low, q1, med, q3, high)
    }

    fn chart() -> BoxPlotChart {
        let mut full = summary(1.0, 2.0, 3.0, 4.0, 5.0);
        full.mean = Some(3.5);
        full.sample_count = Some(42);
        full.median_ci = Some([2.5, 3.25]);
        full.outliers = vec![-1.0, 9.0];
        let mut other = summary(2.0, 3.0, 4.0, 5.0, 6.0);
        other.outliers = vec![8.0];
        BoxPlotChart {
            categories: vec![
                Category::new("a", "Alpha"),
                Category::new("b", "Beta"),
                Category::new("c", "Gamma"),
            ],
            series: vec![
                BoxPlotSeries::new(
                    "s",
                    "Sales",
                    vec![Some(full), None, Some(other)],
                    Color::from_rgb8(200, 0, 0),
                ),
                BoxPlotSeries::new(
                    "t",
                    "Costs",
                    vec![
                        Some(summary(0.0, 1.0, 2.0, 3.0, 4.0)),
                        Some(summary(1.0, 2.0, 2.5, 3.0, 4.0)),
                        None,
                    ],
                    Color::from_rgb8(0, 0, 200),
                ),
            ],
            ..Default::default()
        }
    }

    fn boxes(series: &SeriesConfig) -> BoxPlotRef<'_> {
        series.render_type.box_plot().expect("box plot")
    }

    fn values(column: &BoxColumn<'_>) -> Vec<f64> {
        let mut out = Vec::new();
        column.for_each(|i, v| {
            assert_eq!(i, out.len());
            out.push(v);
        });
        out
    }

    fn same(a: &[f64], b: &[f64]) -> bool {
        a.len() == b.len()
            && a.iter()
                .zip(b)
                .all(|(x, y)| x == y || (x.is_nan() && y.is_nan()))
    }

    #[test]
    fn categories_become_the_categorical_axis_of_the_direction() {
        let c = chart();
        let (config, series, _) = Config::from_boxplot(&c);
        assert_eq!(
            config.bottom_x.categories.as_deref(),
            Some(&c.categories[..])
        );
        assert!(!config.bottom_x.inverted);
        // A box plot's category axis sits at the plot edge, as before.
        assert_eq!(config.bottom_x.cross_at, None);
        assert_eq!(config.validate_box_plots(&series), Ok(()));
        let mut h = chart();
        h.direction = BoxPlotDirection::Horizontal;
        let (config, series, _) = Config::from_boxplot(&h);
        assert_eq!(config.left_y.categories.as_deref(), Some(&h.categories[..]));
        assert_eq!(config.bottom_x.categories, None);
        assert!(config.left_y.inverted);
        assert_eq!(config.validate_box_plots(&series), Ok(()));
        // The roles do not swap with the direction.
        assert_eq!(series[0].x_column, BOX_PLOT_POSITIONS_COLUMN);
        assert_eq!(series[0].y_column, box_plot_record("s"));
    }

    #[test]
    fn direction_group_width_and_gap_are_a_grouped_bar_groups() {
        let mut c = chart();
        c.direction = BoxPlotDirection::Horizontal;
        c.group_width = 0.5;
        c.box_gap = 6.0;
        let (config, _, _) = Config::from_boxplot(&c);
        assert_eq!(
            config.bar_groups,
            Some(BarGroups {
                mode: CategoryBarMode::Grouped,
                direction: CategoryBarDirection::Horizontal,
                group_width: 0.5,
                bar_gap: 6.0,
            })
        );
    }

    #[test]
    fn uploads_are_the_positions_then_each_series_record_and_outliers() {
        let c = chart();
        let (_, series, uploads) = Config::from_boxplot(&c);
        let columns: Vec<_> = uploads.iter().map(|u| u.column.as_str()).collect();
        assert_eq!(
            columns,
            [
                BOX_PLOT_POSITIONS_COLUMN,
                "boxplot.record.s",
                "boxplot.outliers.s",
                "boxplot.record.t",
            ]
        );
        assert_eq!(uploads[0].values, UploadValues::Positions(3));
        assert!(
            matches!(uploads[1].values, UploadValues::BoxRecord(v) if std::ptr::eq(v, &c.series[0].values[..]))
        );
        assert!(
            matches!(uploads[2].values, UploadValues::BoxOutliers(v) if std::ptr::eq(v, &c.series[0].values[..]))
        );
        // A series without outliers names no outlier column.
        assert_eq!(boxes(&series[0]).outliers, Some("boxplot.outliers.s"));
        assert_eq!(boxes(&series[1]).outliers, None);
        // A record or a ragged upload is not one column.
        assert_eq!(uploads[1].values.len(), 0);
        assert!(uploads[1].values.value(0).is_nan());
        assert!(uploads[0].values.box_fields().is_none());
        assert!(uploads[1].values.box_outliers().is_none());
    }

    #[test]
    fn record_fields_read_every_summary_field_and_nan_for_a_missing_box() {
        let c = chart();
        let (_, _, uploads) = Config::from_boxplot(&c);
        let fields = uploads[1].values.box_fields().unwrap();
        let names: Vec<_> = BoxField::ALL.iter().map(|f| f.name()).collect();
        assert_eq!(
            names,
            [
                "median",
                "q1",
                "q3",
                "whisker_low",
                "whisker_high",
                "mean",
                "sample_count",
                "ci_low",
                "ci_high"
            ]
        );
        let nan = f64::NAN;
        let expected: [[f64; 3]; 9] = [
            [3.0, nan, 4.0],
            [2.0, nan, 3.0],
            [4.0, nan, 5.0],
            [1.0, nan, 2.0],
            [5.0, nan, 6.0],
            // An absent optional statistic is NaN in a present box too.
            [3.5, nan, nan],
            [42.0, nan, nan],
            [2.5, nan, nan],
            [3.25, nan, nan],
        ];
        for (k, column) in fields.iter().enumerate() {
            assert_eq!(column.len(), 3);
            assert!(same(&values(column), &expected[k]), "{}", names[k]);
            assert!(same(&[column.value(1)], &[nan]));
        }
        assert_eq!((fields[0].min(), fields[0].max()), (3.0, 4.0));
        assert_eq!(fields[6].min(), 42.0);
    }

    #[test]
    fn outlier_values_and_offsets_group_the_outliers_by_category() {
        let c = chart();
        let (_, _, uploads) = Config::from_boxplot(&c);
        let (values_column, offsets) = uploads[2].values.box_outliers().unwrap();
        assert_eq!(values_column.len(), 3);
        assert_eq!(values(&values_column), [-1.0, 9.0, 8.0]);
        assert_eq!(offsets.len(), 4);
        // The missing box is an empty group.
        assert_eq!(values(&offsets), [0.0, 2.0, 2.0, 3.0]);
        assert_eq!((values_column.min(), values_column.max()), (-1.0, 9.0));
        assert_eq!(offsets.value(3), 3.0);
        assert!(offsets.value(9).is_nan());
    }

    #[test]
    fn series_colour_label_style_and_overrides_carry_over() {
        let mut c = chart();
        c.style.material = Material::Paper;
        c.series[1].style = Some(BoxPlotStyle {
            outlier_shape: BoxOutlierShape::Square,
            ..Default::default()
        });
        let mut o = BoxPlotOverride::new(BoxPlotTarget::new("c", "s"));
        o.color = Some(Color::WHITE);
        c.overrides.push(o.clone());
        let (_, series, _) = Config::from_boxplot(&c);
        assert_eq!(boxes(&series[0]).color, c.series[0].color);
        assert_eq!(boxes(&series[0]).style, &c.style);
        assert_eq!(boxes(&series[1]).style, c.series[1].style.as_ref().unwrap());
        assert_eq!(boxes(&series[0]).overrides, [o]);
        assert!(boxes(&series[1]).overrides.is_empty());
        let label = series[1].label.as_ref().unwrap();
        assert_eq!(
            label.segments.iter().map(|s| s.text).collect::<String>(),
            "Costs"
        );
    }

    #[test]
    fn label_settings_and_text_come_from_the_summaries() {
        let mut c = chart();
        c.labels = BoxPlotLabels::MedianAndCount;
        c.label_decimals = 2;
        c.value_suffix = " ms".into();
        let mut o = BoxPlotOverride::new(BoxPlotTarget::new("a", "t"));
        o.labels = Some(BoxPlotLabels::Median);
        c.overrides.push(o);
        let (_, series, _) = Config::from_boxplot(&c);
        let labels = boxes(&series[0]).labels;
        assert_eq!(labels.placement, BoxPlotLabels::MedianAndCount);
        assert_eq!((labels.decimals, labels.value_suffix.as_str()), (2, " ms"));
        assert_eq!(labels.text, ["3.00 ms (n=42)", "", "4.00 ms (n=—)"]);
        assert_eq!(
            boxes(&series[1]).labels.text,
            ["2.00 ms", "2.50 ms (n=—)", ""]
        );
        c.labels = BoxPlotLabels::None;
        c.overrides.clear();
        let (_, series, _) = Config::from_boxplot(&c);
        assert_eq!(boxes(&series[0]).labels.text, ["", "", ""]);
    }

    #[test]
    fn the_value_range_fixes_the_value_axis() {
        let mut c = chart();
        let (default, _, _) = Config::from_boxplot(&c);
        c.value_range = Some([-5.0, 12.0]);
        let (config, _, _) = Config::from_boxplot(&c);
        assert_eq!((config.left_y.min, config.left_y.max), (-5.0, 12.0));
        assert_eq!((config.right_y.min, config.right_y.max), (-5.0, 12.0));
        assert_ne!(config.left_y.major_spacing, 0.0);
        assert_eq!(config.bottom_x, default.bottom_x);
        c.direction = BoxPlotDirection::Horizontal;
        let (config, _, _) = Config::from_boxplot(&c);
        assert_eq!((config.bottom_x.min, config.bottom_x.max), (-5.0, 12.0));
        assert_eq!((config.top_x.min, config.top_x.max), (-5.0, 12.0));
    }

    #[test]
    fn titles_fonts_grid_and_legend_map_as_for_categorical_documents() {
        let mut c = chart();
        c.value_title = "Latency".into();
        c.category_title = "Condition".into();
        c.font_family = "Serif".into();
        c.font_size = 18.0;
        c.text_color = Color::from_rgb8(10, 20, 30);
        let (config, _, _) = Config::from_boxplot(&c);
        let text = |t: &RichText| t.segments.iter().map(|s| s.text).collect::<String>();
        assert_eq!(text(&config.left_y.title_option.text), "Latency");
        assert_eq!(text(&config.bottom_x.title_option.text), "Condition");
        for axis in [&config.bottom_x, &config.left_y] {
            assert_eq!(axis.label_style.label_font, "Serif");
            assert_eq!(axis.label_style.font_size, 18.0);
            assert_eq!(axis.line_color, c.text_color);
            assert_eq!(axis.tick, TickVisibility::None);
        }
        assert!(config.grid.show_major_y && !config.grid.show_major_x);
        assert_eq!(config.grid.major_y_color, LEGACY_GRID_COLOR);
        assert!(config.legend.visible);
        let legend: String = config
            .legend
            .content
            .segments
            .iter()
            .map(|s| s.text)
            .collect();
        assert!(
            legend.contains("Sales") && legend.contains("Costs"),
            "{legend}"
        );
        assert_eq!(config.legend.content.font, "Serif");
        c.grid = false;
        c.legend = false;
        c.value_title.clear();
        let (config, _, _) = Config::from_boxplot(&c);
        assert!(!config.grid.show_major_y && !config.legend.visible);
        assert!(!config.left_y.title_option.visible);
    }

    #[test]
    fn title_selection_layer_annotations_and_offsets_carry_over() {
        let mut c = chart();
        c.title = "Spread".into();
        c.hovered = Some(BoxPlotTarget::new("a", "s"));
        let (config, _, _) = Config::from_boxplot(&c);
        assert_eq!(config.chart_title.text, c.resolved_title().text);
        assert_eq!(config.picked_data, None);
        c.selected = Some(BoxPlotTarget::new("b", "t"));
        c.active_layer = LayerRef::Annotations;
        c.annotations.visible = false;
        let offset = |element, offset| DecorationOffset { element, offset };
        c.decorations.offsets = vec![
            offset(DecorationId::ChartTitle, [3.0, 4.0]),
            offset(DecorationId::Legend, [-5.0, 6.0]),
            offset(DecorationId::Axis(Side::Bottom), [99.0, 8.0]),
            offset(DecorationId::AxisLabel(Side::Left), [1.0, 2.0]),
            offset(DecorationId::AxisTitle(Side::Bottom), [-1.0, -2.0]),
        ];
        let (config, _, _) = Config::from_boxplot(&c);
        assert_eq!(
            config.picked_data.unwrap().refs,
            [PickedDataRef::HistogramBin {
                source_id: None,
                series_id: "t".into(),
                bin_index: 1,
            }]
        );
        assert_eq!(config.active_layer, LayerRef::Annotations);
        assert_eq!(config.annotations, c.annotations);
        assert_eq!(
            (config.chart_title.offset_x, config.chart_title.offset_y),
            (3.0, 4.0)
        );
        assert_eq!(
            (config.legend.offset_x, config.legend.offset_y),
            (-5.0, 6.0)
        );
        assert_eq!(config.bottom_x.line_offset, 8.0);
        let label = &config.left_y.label_style;
        assert_eq!((label.label_offset_x, label.label_offset_y), (1.0, 2.0));
        let title = &config.bottom_x.title_option;
        assert_eq!((title.offset_x, title.offset_y), (-1.0, -2.0));
    }

    #[test]
    fn background_and_the_explicit_data_area_are_not_carried() {
        let mut c = chart();
        let (before, _, _) = Config::from_boxplot(&c);
        c.background = Color::BLACK;
        c.decorations.data_area = Some(crate::layout::RectF {
            x: 100.0,
            y: 100.0,
            width: 300.0,
            height: 200.0,
        });
        let (after, _, _) = Config::from_boxplot(&c);
        assert_eq!(after, before);
    }
}

#[cfg(test)]
mod radar_tests {
    use super::*;
    use crate::config::Category;
    use crate::data_config::{RadarLabels, RadarMarker, RadarRef, RadarSeriesStyle};
    use crate::decoration::DecorationOffset;
    use crate::layer::LayerRef;
    use crate::radar::{RadarSeries, RadarTarget};

    fn chart() -> RadarChart {
        RadarChart {
            categories: vec![
                Category::new("speed", "Speed"),
                Category::new("range", "Range"),
                Category::new("cost", "Cost"),
            ],
            series: vec![
                RadarSeries::new(
                    "a",
                    "Model A",
                    vec![4.0, f64::NAN, 2.5],
                    Color::from_rgb8(200, 0, 0),
                ),
                RadarSeries::new(
                    "b",
                    "Model B",
                    vec![1.0, 2.0, 3.0],
                    Color::from_rgb8(0, 0, 200),
                ),
            ],
            ..Default::default()
        }
    }

    fn radar(series: &SeriesConfig) -> RadarRef<'_> {
        series.render_type.radar().expect("radar")
    }

    #[test]
    fn categories_become_the_categorical_angle_axis_of_a_polar_chart() {
        let c = chart();
        let (config, series, _) = Config::from_radar(&c);
        assert_eq!(
            config.bottom_x.categories.as_deref(),
            Some(&c.categories[..])
        );
        assert_eq!(config.left_y.categories, None);
        assert!(config.polar.is_some());
        assert_eq!(config.validate_polar(), Ok(()));
        assert_eq!(config.bottom_x.validate_categories(), Ok(()));
        assert_eq!(config.validate_radar(&series), Ok(()));
        assert_eq!(series[0].x_column, RADAR_POSITIONS_COLUMN);
        assert_eq!(series[0].y_column, radar_values_column("a"));
    }

    #[test]
    fn values_become_borrowed_uploads_in_series_order() {
        let c = chart();
        let (_, series, uploads) = Config::from_radar(&c);
        let columns: Vec<_> = uploads.iter().map(|u| u.column.as_str()).collect();
        assert_eq!(
            columns,
            [RADAR_POSITIONS_COLUMN, "radar.values.a", "radar.values.b"]
        );
        assert_eq!(uploads[0].values, UploadValues::Positions(3));
        assert!(
            matches!(uploads[1].values, UploadValues::Dense(v) if std::ptr::eq(v, &c.series[0].values[..]))
        );
        assert_eq!(uploads[1].values.len(), 3);
        assert_eq!(uploads[1].values.value(0), 4.0);
        assert!(uploads[1].values.value(1).is_nan());
        assert!(uploads[1].values.value(3).is_nan());
        assert_eq!(
            (uploads[1].values.min(), uploads[1].values.max()),
            (2.5, 4.0)
        );
        assert_eq!(
            series
                .iter()
                .map(|s| s.series_id.as_str())
                .collect::<Vec<_>>(),
            ["a", "b"]
        );
    }

    #[test]
    fn the_spoke_layout_maps_to_the_polar_options() {
        let mut c = chart();
        let (config, _, _) = Config::from_radar(&c);
        let polar = config.polar.as_ref().unwrap();
        // -90 screen degrees with y down is up: 90 with y up.
        assert_eq!(polar.zero_angle_degrees, 90.0);
        assert!(polar.clockwise);
        assert_eq!(polar.grid_shape, Some(GridShape::Polygon));
        assert_eq!(polar.radial_label_angle_degrees, 90.0);
        assert!(polar.interpolation.is_chord());
        assert_eq!(polar.sector, None);
        // The first category points up and the second follows clockwise.
        let map = config.polar_angle_map().unwrap();
        assert!((map.screen_angle(0.0) - std::f64::consts::FRAC_PI_2).abs() < 1e-12);
        assert!(map.screen_angle(1.0) < map.screen_angle(0.0));
        c.style.start_angle_degrees = 30.0;
        c.style.clockwise = false;
        c.style.grid_shape = RadarGridShape::Circle;
        let (config, _, _) = Config::from_radar(&c);
        let polar = config.polar.as_ref().unwrap();
        assert_eq!(polar.zero_angle_degrees, -30.0);
        assert_eq!(polar.radial_label_angle_degrees, 330.0);
        assert!(!polar.clockwise);
        assert_eq!(polar.grid_shape, Some(GridShape::Circle));
    }

    #[test]
    fn the_value_range_and_ring_spacing_are_the_radius_axis() {
        let mut c = chart();
        let (default, _, _) = Config::from_radar(&c);
        c.value_range = Some([0.0, 10.0]);
        let (config, _, _) = Config::from_radar(&c);
        assert_eq!((config.left_y.min, config.left_y.max), (0.0, 10.0));
        assert_eq!(config.left_y.major_spacing, 2.0);
        c.ring_spacing = Some(2.5);
        let (config, _, _) = Config::from_radar(&c);
        assert_eq!((config.left_y.min, config.left_y.max), (0.0, 10.0));
        assert_eq!(config.left_y.major_spacing, 2.5);
        // Without a range the host fits it; the spacing still applies.
        c.value_range = None;
        let (config, _, _) = Config::from_radar(&c);
        assert_eq!(
            (config.left_y.min, config.left_y.max),
            (default.left_y.min, default.left_y.max)
        );
        assert_eq!(config.left_y.major_spacing, 2.5);
        assert_eq!(
            config.left_y.label_style.format,
            LabelFormat::Plain(PlainNumberFormat::AUTO)
        );
    }

    #[test]
    fn series_colour_label_and_resolved_style_carry_over() {
        let mut c = chart();
        c.style.line_width_px = 3.0;
        c.style.fill_opacity = 0.4;
        c.style.marker = RadarMarker::Square;
        c.style.marker_size_px = 9.0;
        c.style.emphasis_brightness = 0.2;
        c.series[1].style = Some(RadarSeriesStyle {
            line_width_px: Some(1.5),
            fill: Some(Color::new(0.0, 0.0, 0.0, 0.0)),
            marker: Some(RadarMarker::None),
            ..Default::default()
        });
        let (_, series, _) = Config::from_radar(&c);
        let a = radar(&series[0]);
        assert_eq!(a.color, c.series[0].color);
        assert_eq!(
            a.style,
            &RadarSeriesStyle {
                line_width_px: Some(3.0),
                fill: None,
                marker: Some(RadarMarker::Square),
                marker_size_px: Some(9.0),
                fill_opacity: Some(0.4),
            }
        );
        assert_eq!(a.style.fill_color(a.color).a, 0.4);
        let b = radar(&series[1]);
        assert_eq!(b.style.line_width(), 1.5);
        assert_eq!(b.style.marker(), RadarMarker::None);
        assert_eq!(b.style.fill_color(b.color).a, 0.0);
        assert!(b.scatter_style().is_none());
        assert!(
            b.line_style().fill.is_none(),
            "a transparent fill draws none"
        );
        let label = series[0].label.as_ref().unwrap();
        assert_eq!(
            label.segments.iter().map(|s| s.text).collect::<String>(),
            "Model A"
        );
    }

    #[test]
    fn label_settings_go_to_every_series_and_write_the_text() {
        let mut c = chart();
        let (_, series, _) = Config::from_radar(&c);
        assert_eq!(radar(&series[0]).labels.text, ["", "", ""]);
        c.labels = RadarLabels::Values;
        c.label_decimals = 2;
        c.value_suffix = " pt".into();
        let (_, series, _) = Config::from_radar(&c);
        let labels = radar(&series[0]).labels;
        assert_eq!(labels.placement, RadarLabels::Values);
        assert_eq!((labels.decimals, labels.value_suffix.as_str()), (2, " pt"));
        // A non-finite value has no label.
        assert_eq!(labels.text, ["4.00 pt", "", "2.50 pt"]);
        assert_eq!(
            radar(&series[1]).labels.text,
            ["1.00 pt", "2.00 pt", "3.00 pt"]
        );
    }

    #[test]
    fn the_grid_is_the_rings_spokes_and_outer_ring() {
        let mut c = chart();
        let (config, _, _) = Config::from_radar(&c);
        assert!(config.grid.show_major_x && config.grid.show_major_y);
        assert!(!config.grid.show_minor_x && !config.grid.show_minor_y);
        assert_eq!(config.grid.major_x_color, c.style.spoke_color);
        assert_eq!(config.grid.major_y_color, c.style.ring_color);
        assert!(config.bottom_x.line_visible);
        assert_eq!(config.bottom_x.line_color, c.style.ring_color);
        assert!(!config.left_y.line_visible);
        c.grid = false;
        let (config, _, _) = Config::from_radar(&c);
        assert!(!config.grid.show_major_x && !config.grid.show_major_y);
        assert!(!config.bottom_x.line_visible);
    }

    #[test]
    fn fonts_title_legend_layer_and_annotations_carry_over() {
        let mut c = chart();
        c.font_family = "Serif".into();
        c.font_size = 18.0;
        c.text_color = Color::from_rgb8(10, 20, 30);
        c.title = "Quality".into();
        c.active_layer = LayerRef::Annotations;
        c.annotations.visible = false;
        let (config, _, _) = Config::from_radar(&c);
        for axis in [&config.bottom_x, &config.left_y] {
            assert_eq!(axis.label_style.label_font, "Serif");
            assert_eq!(axis.label_style.font_size, 18.0);
            assert_eq!(axis.label_style.color, c.text_color);
            assert_eq!(axis.tick, TickVisibility::None);
            assert!(!axis.title_option.visible);
        }
        assert_eq!(config.chart_title.text, c.resolved_title().text);
        assert!(config.chart_title.visible);
        assert_eq!(config.legend.content.font, "Serif");
        let legend: String = config
            .legend
            .content
            .segments
            .iter()
            .map(|s| s.text)
            .collect();
        assert!(
            legend.contains("Model A") && legend.contains("Model B"),
            "{legend}"
        );
        assert!(config.legend.visible);
        assert_eq!(config.active_layer, LayerRef::Annotations);
        assert_eq!(config.annotations, c.annotations);
        c.legend = false;
        assert!(!Config::from_radar(&c).0.legend.visible);
    }

    #[test]
    fn a_selected_vertex_is_the_typed_selection_and_hover_is_dropped() {
        let mut c = chart();
        c.hovered = Some(RadarTarget::vertex("a", "speed"));
        let (config, _, _) = Config::from_radar(&c);
        assert_eq!(config.picked_data, None);
        c.selected = Some(RadarTarget::series("b"));
        let (config, _, _) = Config::from_radar(&c);
        assert_eq!(config.picked_data, None, "a whole series is not a vertex");
        c.selected = Some(RadarTarget::vertex("b", "cost"));
        let (config, _, _) = Config::from_radar(&c);
        assert_eq!(
            config.picked_data.unwrap().refs,
            [PickedDataRef::Point {
                source_id: None,
                series_id: "b".into(),
                point_index: 2,
            }]
        );
    }

    #[test]
    fn title_legend_and_ring_label_offsets_carry_over() {
        let mut c = chart();
        let offset = |element, offset| DecorationOffset { element, offset };
        c.decorations.offsets = vec![
            offset(DecorationId::ChartTitle, [3.0, 4.0]),
            offset(DecorationId::Legend, [-5.0, 6.0]),
            offset(DecorationId::AxisLabel(Side::Left), [1.0, 2.0]),
            offset(
                DecorationId::SpokeLabel {
                    category_id: "speed".into(),
                },
                [9.0, 9.0],
            ),
        ];
        let (config, _, _) = Config::from_radar(&c);
        assert_eq!(
            (config.chart_title.offset_x, config.chart_title.offset_y),
            (3.0, 4.0)
        );
        assert_eq!(
            (config.legend.offset_x, config.legend.offset_y),
            (-5.0, 6.0)
        );
        let label = &config.left_y.label_style;
        assert_eq!((label.label_offset_x, label.label_offset_y), (1.0, 2.0));
        let spokes = &config.bottom_x.label_style;
        assert_eq!((spokes.label_offset_x, spokes.label_offset_y), (0.0, 0.0));
    }

    #[test]
    fn background_and_the_explicit_data_area_are_not_carried() {
        let mut c = chart();
        let (before, _, _) = Config::from_radar(&c);
        c.background = Color::BLACK;
        c.decorations.data_area = Some(crate::layout::RectF {
            x: 100.0,
            y: 100.0,
            width: 300.0,
            height: 200.0,
        });
        let (after, _, _) = Config::from_radar(&c);
        assert_eq!(after, before);
    }
}
