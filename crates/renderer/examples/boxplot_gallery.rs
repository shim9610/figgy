//! Box plot documents drawn as Cartesian box plot series: each
//! `BoxPlotChart` is opened with `Config::from_boxplot`, its positions,
//! records and outliers uploaded from the borrowed summaries, and exported by
//! the Cartesian renderer at 960 x 640 without post-render scaling.
use renderer::{
    BOX_PLOT_RECORD, Category, Chart, Color, ColumnSource, LayerRef, Material, RaggedSource,
    RecordSource, Renderer, RendererDevice, UploadValues, boxplot::*, config::Config, data_render,
    encode_png,
};
use std::{path::Path, path::PathBuf, sync::Arc};

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn summary(low: f64, q1: f64, med: f64, q3: f64, high: f64, outlier: f64) -> Option<BoxSummary> {
    let mut v = BoxSummary::new(low, q1, med, q3, high);
    v.outliers = vec![outlier];
    v.sample_count = Some(80);
    v.median_ci = Some([med - 5., med + 5.]);
    v.mean = Some(med + 2.);
    Some(v)
}

/// `document` as a Cartesian chart of box plot series, exported to
/// `name.png`.
fn save(r: &mut Renderer, document: &BoxPlotChart, directory: &Path, name: &str) -> Result {
    document.validate()?;
    let (mut config, series, uploads) = Config::from_boxplot(document);
    config.chart_area = renderer::layout::ChartArea(renderer::layout::Rect {
        x: 0,
        y: 0,
        width: 960,
        height: 640,
    });
    for upload in &uploads {
        match &upload.values {
            UploadValues::BoxRecord(_) => {
                let fields = upload.values.box_fields().ok_or("record fields")?;
                let pairs: [(&str, &dyn ColumnSource); 9] = std::array::from_fn(|k| {
                    (BOX_PLOT_RECORD[k].0, &fields[k] as &dyn ColumnSource)
                });
                r.upsert_record(
                    upload.column.clone(),
                    &RecordSource {
                        schema: BOX_PLOT_RECORD,
                        fields: &pairs,
                    },
                )?;
            }
            UploadValues::BoxOutliers(_) => {
                let (values, offsets) = upload.values.box_outliers().ok_or("outliers")?;
                r.upsert_ragged(
                    upload.column.clone(),
                    &RaggedSource {
                        values: &values,
                        offsets: &offsets,
                    },
                )?;
            }
            values => {
                r.remove_column(&upload.column)?;
                r.add_column(upload.column.clone(), values)?;
            }
        }
    }
    // The fixed `value_range` of the documents is the value axis; a
    // document without one fits the records' and outliers' statistics.
    let chart = if document.value_range.is_some() {
        Chart::new(config)
    } else {
        let mut chart = Chart::new(config);
        chart.auto_fit_layer_union(r.pool(), LayerRef::Base, &series, 0.1)?;
        chart
    };
    let image = r.export_panel_rgba(&chart, &series, 1.0)?;
    let colored = image
        .rgba
        .chunks_exact(4)
        .filter(|p| p[0].max(p[1]).max(p[2]) - p[0].min(p[1]).min(p[2]) > 40)
        .count();
    if colored < 1000 {
        return Err("missing box plot pixels".into());
    }
    std::fs::write(directory.join(format!("{name}.png")), encode_png(&image)?)?;
    println!(
        "{name}: {}x{}, {colored} colored pixels",
        image.width, image.height
    );
    Ok(())
}

fn main() -> Result {
    let dir = PathBuf::from(
        std::env::args()
            .nth(1)
            .unwrap_or_else(|| "target/boxplot-gallery".into()),
    );
    std::fs::create_dir_all(&dir)?;
    let instance = data_render::create_instance();
    let adapter = data_render::request_adapter(&instance)?;
    eprintln!("Boxplot adapter: {:?}", adapter.get_info());
    let (d, q) = data_render::request_device(&adapter)?;
    let gpu = RendererDevice::new(Arc::new(d), Arc::new(q));
    let mut r = Renderer::try_new(gpu, wgpu::TextureFormat::Rgba8Unorm, 8 << 20)?;
    let mut c = BoxPlotChart {
        title: "Classic matte".into(),
        categories: ["Control", "Low", "High"]
            .into_iter()
            .map(|s| Category::new(s, s))
            .collect(),
        series: vec![BoxPlotSeries::new(
            "response",
            "Response",
            vec![
                summary(12., 30., 42., 55., 78., 90.),
                summary(20., 42., 55., 68., 85., 96.),
                summary(8., 24., 36., 49., 67., 82.),
            ],
            Color::from_rgb8(65, 144, 208),
        )],
        value_range: Some([0., 100.]),
        value_title: "Response".into(),
        font_size: 18.,
        legend: false,
        ..Default::default()
    };
    for (id, color) in [
        ("Low", Color::from_rgb8(42, 168, 162)),
        ("High", Color::from_rgb8(230, 169, 64)),
    ] {
        let mut o = BoxPlotOverride::new(BoxPlotTarget::new(id, "response"));
        o.color = Some(color);
        c.overrides.push(o);
    }
    save(&mut r, &c, &dir, "01-classic-matte")?;
    c.title = "Satin finish".into();
    c.style.material = Material::SatinMetal;
    c.style.gloss = 0.6;
    save(&mut r, &c, &dir, "02-satin")?;
    c.title = "Horizontal comparison".into();
    c.direction = BoxPlotDirection::Horizontal;
    c.style.material = Material::Matte;
    save(&mut r, &c, &dir, "03-horizontal")?;
    c.title = "Grouped + notch".into();
    c.direction = BoxPlotDirection::Vertical;
    c.style.notched = true;
    c.legend = true;
    c.categories = ["A", "B", "C"]
        .into_iter()
        .map(|s| Category::new(s, s))
        .collect();
    c.overrides.clear();
    c.series[0].id = "before".into();
    c.series[0].label = "Before".into();
    c.series.push(BoxPlotSeries::new(
        "after",
        "After",
        vec![
            summary(18., 37., 50., 62., 82., 96.),
            summary(12., 30., 44., 58., 74., 88.),
            summary(9., 27., 40., 52., 69., 80.),
        ],
        Color::from_rgb8(42, 168, 162),
    ));
    save(&mut r, &c, &dir, "04-grouped-notch")?;
    c.title = "Mean and individual styling".into();
    c.style.show_mean = true;
    c.style.outline = false;
    c.selected = Some(BoxPlotTarget::new("B", "after"));
    save(&mut r, &c, &dir, "05-mean-selection")?;
    Ok(())
}
