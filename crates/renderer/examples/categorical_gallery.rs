//! Categorical bar documents drawn as Cartesian category bars: each
//! `CategoricalChart` is opened with `Config::from_categorical`, its columns
//! uploaded from the borrowed values, and exported by the Cartesian renderer
//! at 900 x 620 without post-render scaling, in the `document` theme.
//! Each case shows one finish: it varies the theme's bar material, and
//! where the case has colours of its own its palette, before applying it.
// Legacy documents are read through their deprecated reader types.
#![allow(deprecated)]
#[allow(dead_code)]
#[path = "support/gallery.rs"]
mod gallery;
use gallery::Result;
use renderer::{
    Chart, Color, ColumnSource, ExportOptions, ExportSource, LayerRef, Material, Renderer,
    categorical::*,
    config::Config,
    theme::{ChartKind, SubplotScope, Theme},
};
use std::path::Path;

/// `document` as a Cartesian chart of category bars in `theme`, exported
/// to `name.png`.
fn save(
    r: &mut Renderer,
    document: &CategoricalChart,
    theme: &Theme,
    directory: &Path,
    name: &str,
) -> Result {
    document.validate()?;
    let horizontal = document.direction == CategoryBarDirection::Horizontal;
    let (config, mut series, uploads) = Config::from_categorical(document);
    // The previous document's columns go; the positions come first, then
    // the value columns as one batch.
    for upload in &uploads {
        r.remove_column(&upload.column)?;
    }
    let (positions, values) = uploads.split_first().ok_or("no positions column")?;
    r.add_column(positions.column.clone(), &positions.values)?;
    let columns: Vec<(&str, &dyn ColumnSource)> = values
        .iter()
        .map(|u| (u.column.as_str(), &u.values as &dyn ColumnSource))
        .collect();
    r.add_columns(&columns)?;
    // Fit the value axis to the bars; the fit resets the label format, so
    // the document's is put back before the ticks are rounded.
    let value_format = |c: &Config| {
        if horizontal {
            c.bottom_x.label_style.format.clone()
        } else {
            c.left_y.label_style.format.clone()
        }
    };
    let format = value_format(&config);
    // Outside value labels need room past the longest bar, as the former
    // chart left them: 12 % of the value range on the sides the bars grow
    // to. A percent stack ends at 100 and keeps its labels inside.
    let outside = document.mode != CategoryBarMode::PercentStacked
        && (document.labels == CategoryBarLabels::Outside
            || (document.labels == CategoryBarLabels::Auto
                && document.mode == CategoryBarMode::Grouped));
    let mut chart = Chart::new(config).with_area(0, 0, 900, 620);
    chart.auto_fit_layer_union(r.pool(), LayerRef::Base, &series, 0.0)?;
    let config = chart.config_mut();
    let value = if horizontal {
        &mut config.bottom_x
    } else {
        &mut config.left_y
    };
    value.label_style.format = format;
    let (mut min, mut max) = (value.min, value.max);
    let span = max - min;
    if outside && max > 0.0 {
        max += 0.12 * span;
    }
    if outside && min < 0.0 {
        min -= 0.12 * span;
    }
    value
        .auto_ticks(min, max, 5)
        .map_err(|e| format!("value axis ticks: {e:?}"))?;
    // The theme last: it fits the layout to the axes' final labels.
    chart.apply_custom_theme(theme, &mut series, SubplotScope::All)?;
    // The grid marks the value axis, which runs along x for horizontal
    // bars.
    let grid = &mut chart.config_mut().grid;
    if horizontal {
        (grid.show_major_x, grid.show_major_y) = (grid.show_major_y, grid.show_major_x);
    }
    chart.fit_layout();
    let path = directory.join(format!("{name}.png"));
    r.save_blocking(
        ExportSource::chart(&chart, &series),
        ExportOptions::panel(1.0),
        &path,
    )?;
    println!("{}", path.display());
    Ok(())
}

fn main() -> Result {
    let directory = gallery::output_directory("target/categorical-gallery")?;
    gallery::with_renderer(|renderer| render(renderer, &directory))
}

fn render(renderer: &mut Renderer, directory: &Path) -> Result {
    let document = gallery::theme_named(ChartKind::CategoryBars, "document")?;
    let mut chart = CategoricalChart {
        title: "Grouped · Matte".into(),
        categories: ["A", "B", "C", "D"]
            .into_iter()
            .map(|x| Category::new(x, x))
            .collect(),
        series: vec![
            BarSeries::new(
                "before",
                "Before",
                [12., 18., 9., 15.].map(Some).into(),
                Color::BLACK,
            ),
            BarSeries::new(
                "after",
                "After",
                [18., 24., 14., 21.].map(Some).into(),
                Color::BLACK,
            ),
        ],
        value_title: "Value".into(),
        ..Default::default()
    };
    save(renderer, &chart, &document, directory, "grouped-matte")?;
    chart.title = "Horizontal · Satin".into();
    chart.direction = CategoryBarDirection::Horizontal;
    chart.legend = false;
    chart.categories = ["Alpha", "Beta", "Gamma", "Delta"]
        .into_iter()
        .map(|x| Category::new(x, x))
        .collect();
    chart.series = vec![BarSeries::new(
        "share",
        "Share",
        [72., 58., 43., 29.].map(Some).into(),
        Color::BLACK,
    )];
    // One series: the theme colours each category, here down a ramp from
    // blue to teal.
    let mut satin = document.clone();
    satin.series.palette = vec![
        Color::from_rgb8(61, 125, 205),
        Color::from_rgb8(67, 153, 211),
        Color::from_rgb8(48, 171, 174),
        Color::from_rgb8(98, 194, 192),
    ];
    satin.series.bars.material = Material::SatinMetal;
    satin.series.bars.gloss = 0.65;
    save(renderer, &chart, &satin, directory, "horizontal-satin")?;
    chart = CategoricalChart {
        title: "Stacked · Enamel".into(),
        categories: ["A", "B", "C"]
            .into_iter()
            .map(|x| Category::new(x, x))
            .collect(),
        series: vec![
            BarSeries::new(
                "core",
                "Core",
                [12., 18., 9.].map(Some).into(),
                Color::BLACK,
            ),
            BarSeries::new(
                "extra",
                "Extra",
                [8., 6., 12.].map(Some).into(),
                Color::BLACK,
            ),
            BarSeries::new(
                "other",
                "Other",
                [5., 6., 4.].map(Some).into(),
                Color::BLACK,
            ),
        ],
        mode: CategoryBarMode::Stacked,
        value_title: "Value".into(),
        ..Default::default()
    };
    // The stacks' three parts: blue, teal and orange.
    let stack = vec![
        Color::from_rgb8(66, 133, 208),
        Color::from_rgb8(63, 169, 167),
        Color::from_rgb8(255, 170, 113),
    ];
    let mut enamel = document.clone();
    enamel.series.palette.clone_from(&stack);
    enamel.series.bars.material = Material::Enamel;
    save(renderer, &chart, &enamel, directory, "stacked-enamel")?;
    chart.title = "100% Stacked · Outline".into();
    chart.direction = CategoryBarDirection::Horizontal;
    chart.mode = CategoryBarMode::PercentStacked;
    chart.label_format = CategoryBarLabelFormat::Percent;
    chart.value_title = "Share".into();
    let mut outlined = document.clone();
    outlined.series.palette = stack;
    outlined.series.bars.material = Material::Flat;
    outlined.series.bars.outline = true;
    save(renderer, &chart, &outlined, directory, "percent-outline")?;
    chart.title = "Positive and negative values".into();
    chart.mode = CategoryBarMode::Stacked;
    chart.direction = CategoryBarDirection::Vertical;
    chart.label_format = CategoryBarLabelFormat::Value;
    chart.value_title = "Change".into();
    chart.series[1].values = vec![Some(-8.), Some(-6.), Some(-12.)];
    save(renderer, &chart, &outlined, directory, "signed-stacked")?;
    Ok(())
}
