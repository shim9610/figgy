# Categorical bars

A categorical bar chart is a Cartesian chart: one `DataRenderType::CategoryBars`
series per bar series, drawn over a categorical axis
(`AxisOptions::categories`) by the Cartesian `Renderer`. Hover, click
selection, `step_selection`, the picked highlight, decoration drag, plot-area
drag and resize, the annotation layer, PNG export, overlays and subplots are
the Cartesian ones. `CategoricalChart` is the document format releases up to
0.12 wrote; `Config::from_categorical` reads it.

## Series and bar groups

- The categorical axis runs along the bar direction's category axis:
  `bottom_x` for vertical bars, `left_y` for horizontal ones. It has 1 to 64
  categories (`AxisOptions::validate_categories`).
- Each series reads two columns of one value per category. With
  `BarGroups::direction` `Vertical`, `x_column` holds the category positions
  `0, 1, …, n - 1` and `y_column` the values; `Horizontal` swaps them, like a
  histogram's orientation. A NaN value is a missing bar; a zero draws no bar
  and stays a value.
- The `CategoryBars` series of one layer are its bar group, at most 16
  (`MAX_BAR_SERIES`) and at most 512 bars (`MAX_CATEGORY_BARS`).
  `Config.bar_groups` lays them out: `mode` `Grouped` (side by side in
  series order), `Stacked` (positive and negative values stack away from
  zero independently) or `PercentStacked` (shares of the category total of
  positive values, `0..100`; negative values are rejected), `group_width`
  (the share of a slot the group covers, 0.1 to 0.95) and `bar_gap`
  (logical pixels between grouped bars, 0 to 24). `None` reads as the
  default: grouped, vertical, 0.7, 3 px.
- The value axis is linear. `Chart::auto_fit_layer_union` fits it to what
  the bars draw: zero and each column's range for grouped bars, `0..100`
  for percent bars, and for stacked bars the stack extent the column pool
  measured while the group's value columns were uploaded as one batch
  (`add_columns` with exactly those columns). The renderer keeps no value
  of a bar on the CPU.
- `AxisOptions::cross_at` puts the category axis line at a value of the
  value axis, usually zero.

```rust,no_run
use renderer::config::Category;
use renderer::data_config::{BarGroups, CategoryBarMode, CategoryBarStyle};
use renderer::{Chart, Color, ColumnSource, DataRenderType, LayerRef, Renderer, SeriesConfig};
# fn build(r: &mut Renderer, positions: &dyn ColumnSource, before: &dyn ColumnSource,
#     after: &dyn ColumnSource) -> renderer::Result<()> {
r.add_column("pos", positions)?;
// The value columns together, so the pool measures their stack.
r.add_columns(&[("before", before), ("after", after)])?;
let mut config = renderer::default::default_config();
config.bottom_x.categories = Some(vec![Category::new("a", "A"), Category::new("b", "B")]);
config.bottom_x.cross_at = Some(0.0);
config.bar_groups = Some(BarGroups {
    mode: CategoryBarMode::Stacked,
    ..BarGroups::default()
});
let bars = |id: &str, color: Color| SeriesConfig {
    series_id: id.into(),
    source_id: None,
    label: None,
    x_column: "pos".into(),
    y_column: id.into(),
    render_type: DataRenderType::CategoryBars {
        color,
        style: CategoryBarStyle::default(),
        labels: Default::default(),
        label_format: Default::default(),
        overrides: None,
    },
};
let series = [bars("before", Color::from_rgb8(70, 130, 205)), bars("after", Color::from_rgb8(255, 166, 105))];
let mut chart = Chart::new(config);
chart.auto_fit_layer_union(r.pool(), LayerRef::Base, &series, 0.0)?;
# Ok(())
# }
```

## Style and per-bar overrides

`style: CategoryBarStyle` is the series' look; an override in `overrides`
names one bar by category id and series id (`CategoryBarTarget`) and sets
its colour, replaces its style whole, or sets its label placement and
format. At most one override per bar.

| Field | Default | Range and behaviour |
|---|---|---|
| `material` | `Flat` | Any [`Material`](RADIAL.md#materials) except Toon and Pearl; a bar is shaded like a flat pie top |
| `corner_radius` | 5 | Logical pixels, 0 to 32, at most half the bar's thickness and length; only the exposed end of a stack is rounded |
| `outline` | false | Draw the outline |
| `outline_width` | 1 | Logical pixels, 0 to 8, drawn inside the bar |
| `outline_color` | RGB(40, 55, 75) | Channels 0 to 1, alpha allowed |
| `texture_strength` | 0.2 | 0 to 1; 0 turns the texture off |
| `texture_scale` | 1 | 0.1 to 8; higher is finer |
| `gloss` | 0.45 | 0 to 1; the sheen of SatinMetal and BrushedMetal |
| `emphasis_brightness` | 0.08 | 0 to 0.5; the brightness a picked bar gains |

The series colour is opaque. A bar's data end never moves with its
material, outline or highlight.

## Value labels

`labels: CategoryBarSeriesLabels` holds the placement (`None`, `Inside`,
`Outside`, `Auto`), the decimals (0 to 6), the value suffix and `text`, the
drawn string of every bar. `format_labels` writes `text` at edit time from
the values the host holds and `label_format` (`Value`, `Percent`,
`ValuePercent`, `CategoryValue`; a percent of a category with a negative
value is `—`). The renderer validates the text and does not draw it yet:
value labels arrive with the label module of #40 item 13h.

## Selection and picking

`Renderer::pick_chart_data` answers a bar as
`PickedData::HistogramBin { series_id, bin_index, .. }`, `bin_index` the
category's row. A selected bar (`PickedDataRef::HistogramBin` in
`Config.picked_data`) is outlined inside its drawn shape, overrides
included, and gains its emphasis brightness. The browser wrapper's
`step_selection` walks the categories and the series of the bar group, and
`selection_details` reports the category id and label and the value read
back from the GPU column.

## Opening a legacy document

```rust,no_run
use renderer::{CategoricalChart, Chart, ColumnSource, LayerRef, Renderer};
# fn open(r: &mut Renderer, json: &str) -> Result<(), Box<dyn std::error::Error>> {
let document: CategoricalChart = serde_json::from_str(json)?;
document.validate()?;
let (mut config, series, uploads) = renderer::config::Config::from_categorical(&document);
config.chart_area = renderer::layout::ChartArea(renderer::layout::Rect {
    x: 0, y: 0, width: 900, height: 620,
});
// The positions first, then the value columns as one batch. The uploads
// borrow the document's values.
let (positions, values) = uploads.split_first().expect("positions");
r.add_column(positions.column.clone(), &positions.values)?;
let columns: Vec<(&str, &dyn ColumnSource)> = values
    .iter()
    .map(|u| (u.column.as_str(), &u.values as &dyn ColumnSource))
    .collect();
r.add_columns(&columns)?;
let mut chart = Chart::new(config);
chart.auto_fit_layer_union(r.pool(), LayerRef::Base, &series, 0.0)?;
# Ok(())
# }
```

The reader maps the categories to the categorical axis of the direction,
the value and category titles, the grid (major lines on the value axis), the
legend, the font, the text colour, the chart title, the annotations, the
active layer, the selection and the decoration offsets onto their Cartesian
options, and writes each series' label text. It does not carry the hover
state, the background colour, the explicit `decorations.data_area` or the
data label offsets. Horizontal categories run bottom-up, as on every
categorical axis. In the browser, `FiggyChart.set_categorical_document(json)`
does all of this.

`CategoricalChart::validate` still holds a document to the rules of the
releases that wrote it: 1 to 64 categories, 1 to 16 series, at most 512
bars, unique ids, one value per category, no negative value in a percent
stack, and finite stack totals.
