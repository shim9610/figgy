//! Categorical bar documents drawn as Cartesian category bars: each
//! `CategoricalChart` is opened with `Config::from_categorical`, its columns
//! uploaded from the borrowed values, and exported by the Cartesian renderer
//! at 900 x 620 without post-render scaling.
// Legacy documents are read through their deprecated reader types.
#![allow(deprecated)]
use renderer::{
    Chart, Color, ColumnSource, LayerRef, Material, Renderer, RendererDevice, categorical::*,
    config::Config, data_render, encode_png,
};
use std::{path::Path, path::PathBuf, sync::Arc};

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

/// `document` as a Cartesian chart of category bars, exported to `name.png`.
fn save(r: &mut Renderer, document: &CategoricalChart, directory: &Path, name: &str) -> Result {
    document.validate()?;
    let (mut config, series, uploads) = Config::from_categorical(document);
    config.chart_area = renderer::layout::ChartArea(renderer::layout::Rect {
        x: 0,
        y: 0,
        width: 900,
        height: 620,
    });
    // The previous document's columns go; the positions come first, then
    // the value columns as one batch, so the pool measures their stack.
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
    let horizontal = document.direction == CategoryBarDirection::Horizontal;
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
    let mut chart = Chart::new(config);
    chart.auto_fit_layer_union(r.pool(), LayerRef::Base, &series, 0.0)?;
    let mut config = chart.config().clone();
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
    let image = r.export_panel_rgba(&Chart::new(config), &series, 1.0)?;
    std::fs::write(directory.join(format!("{name}.png")), encode_png(&image)?)?;
    println!("{name}: {}x{}", image.width, image.height);
    Ok(())
}

fn main() -> Result {
    let directory = PathBuf::from(
        std::env::args()
            .nth(1)
            .unwrap_or_else(|| "target/categorical-gallery".into()),
    );
    std::fs::create_dir_all(&directory)?;
    let instance = data_render::create_instance();
    let adapter = data_render::request_adapter(&instance)?;
    let (device, queue) = data_render::request_device(&adapter)?;
    let gpu = RendererDevice::new(Arc::new(device), Arc::new(queue));
    let mut renderer = Renderer::try_new(gpu, wgpu::TextureFormat::Rgba8Unorm, 4 << 20)?;
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
                Color::from_rgb8(85, 143, 211),
            ),
            BarSeries::new(
                "after",
                "After",
                [18., 24., 14., 21.].map(Some).into(),
                Color::from_rgb8(255, 166, 105),
            ),
        ],
        value_title: "Value".into(),
        font_size: 18.0,
        ..Default::default()
    };
    chart.style.material = Material::Matte;
    save(&mut renderer, &chart, &directory, "grouped-matte")?;
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
        Color::from_rgb8(61, 125, 205),
    )];
    chart.style.material = Material::SatinMetal;
    chart.style.gloss = 0.65;
    for (id, color) in [
        ("Beta", Color::from_rgb8(67, 153, 211)),
        ("Gamma", Color::from_rgb8(48, 171, 174)),
        ("Delta", Color::from_rgb8(98, 194, 192)),
    ] {
        let mut edit = CategoryBarOverride::new(CategoryBarTarget::new(id, "share"));
        edit.color = Some(color);
        chart.overrides.push(edit);
    }
    save(&mut renderer, &chart, &directory, "horizontal-satin")?;
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
                Color::from_rgb8(66, 133, 208),
            ),
            BarSeries::new(
                "extra",
                "Extra",
                [8., 6., 12.].map(Some).into(),
                Color::from_rgb8(63, 169, 167),
            ),
            BarSeries::new(
                "other",
                "Other",
                [5., 6., 4.].map(Some).into(),
                Color::from_rgb8(255, 170, 113),
            ),
        ],
        mode: CategoryBarMode::Stacked,
        value_title: "Value".into(),
        font_size: 18.,
        ..Default::default()
    };
    chart.style.material = Material::Enamel;
    save(&mut renderer, &chart, &directory, "stacked-enamel")?;
    chart.title = "100% Stacked · Outline".into();
    chart.direction = CategoryBarDirection::Horizontal;
    chart.mode = CategoryBarMode::PercentStacked;
    chart.label_format = CategoryBarLabelFormat::Percent;
    chart.value_title = "Share".into();
    chart.style.material = Material::Flat;
    chart.style.outline = true;
    save(&mut renderer, &chart, &directory, "percent-outline")?;
    chart.title = "Positive and negative values".into();
    chart.mode = CategoryBarMode::Stacked;
    chart.direction = CategoryBarDirection::Vertical;
    chart.label_format = CategoryBarLabelFormat::Value;
    chart.value_title = "Change".into();
    chart.series[1].values = vec![Some(-8.), Some(-6.), Some(-12.)];
    save(&mut renderer, &chart, &directory, "signed-stacked")?;
    Ok(())
}
