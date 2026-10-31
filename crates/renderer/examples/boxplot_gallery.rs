//! Native box plot outputs corresponding to the approved visual concepts.
use renderer::{Category, Color, RendererDevice, boxplot::*, data_render, encode_png};
use std::{path::PathBuf, sync::Arc};
fn summary(low: f64, q1: f64, med: f64, q3: f64, high: f64, outlier: f64) -> Option<BoxSummary> {
    let mut v = BoxSummary::new(low, q1, med, q3, high);
    v.outliers = vec![outlier];
    v.sample_count = Some(80);
    v.median_ci = Some([med - 5., med + 5.]);
    v.mean = Some(med + 2.);
    Some(v)
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
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
    let mut r = BoxPlotRenderer::new(gpu.clone(), wgpu::TextureFormat::Rgba8Unorm)?;
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
    let save = |r: &mut BoxPlotRenderer,
                c: &BoxPlotChart,
                name: &str|
     -> Result<(), Box<dyn std::error::Error>> {
        let image = r.export_rgba(c, (960, 640), 1.)?;
        let colored = image
            .rgba
            .chunks_exact(4)
            .filter(|p| p[0].max(p[1]).max(p[2]) - p[0].min(p[1]).min(p[2]) > 40)
            .count();
        if colored < 1000 {
            return Err("missing box plot pixels".into());
        }
        std::fs::write(dir.join(format!("{name}.png")), encode_png(&image)?)?;
        r.end_frame();
        gpu.device().poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        })?;
        println!(
            "{name}: {}x{}, {colored} colored pixels",
            image.width, image.height
        );
        Ok(())
    };
    save(&mut r, &c, "01-classic-matte")?;
    c.title = "Satin finish".into();
    c.style.material = BoxPlotMaterial::SatinMetal;
    c.style.gloss = 0.6;
    save(&mut r, &c, "02-satin")?;
    c.title = "Horizontal comparison".into();
    c.direction = BoxPlotDirection::Horizontal;
    c.style.material = BoxPlotMaterial::Matte;
    save(&mut r, &c, "03-horizontal")?;
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
    save(&mut r, &c, "04-grouped-notch")?;
    c.title = "Mean and individual styling".into();
    c.style.show_mean = true;
    c.style.outline = false;
    c.selected = Some(BoxPlotTarget::new("B", "after"));
    save(&mut r, &c, "05-mean-selection")?;
    Ok(())
}
