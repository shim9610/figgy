//! Native radar chart outputs covering the grid shapes, markers, labels and
//! selection emphasis.
use renderer::{Category, Color, RendererDevice, data_render, encode_png, radar::*};
use std::{path::PathBuf, sync::Arc};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = PathBuf::from(
        std::env::args()
            .nth(1)
            .unwrap_or_else(|| "target/radar-gallery".into()),
    );
    std::fs::create_dir_all(&dir)?;
    let instance = data_render::create_instance();
    let adapter = data_render::request_adapter(&instance)?;
    eprintln!("Radar adapter: {:?}", adapter.get_info());
    let (d, q) = data_render::request_device(&adapter)?;
    let gpu = RendererDevice::new(Arc::new(d), Arc::new(q));
    let mut r = RadarRenderer::new(gpu.clone(), wgpu::TextureFormat::Rgba8Unorm)?;
    let mut c = RadarChart {
        title: "Build quality".into(),
        categories: ["Speed", "Safety", "Size", "Docs", "Tests", "Ergonomics"]
            .into_iter()
            .map(|s| Category::new(s.to_lowercase(), s))
            .collect(),
        series: vec![
            RadarSeries::new(
                "v1",
                "Release 1",
                vec![6., 8., 4., 5., 7., 6.5],
                Color::from_rgb8(65, 144, 208),
            ),
            RadarSeries::new(
                "v2",
                "Release 2",
                vec![8., 7., 6., 8., 9., 7.],
                Color::from_rgb8(230, 120, 64),
            ),
        ],
        font_size: 18.,
        ..Default::default()
    };
    let save = |r: &mut RadarRenderer,
                c: &RadarChart,
                name: &str|
     -> Result<(), Box<dyn std::error::Error>> {
        let image = r.export_rgba(c, (960, 640), 1.)?;
        let colored = image
            .rgba
            .chunks_exact(4)
            .filter(|p| p[0].max(p[1]).max(p[2]) - p[0].min(p[1]).min(p[2]) > 40)
            .count();
        if colored < 1000 {
            return Err("missing radar pixels".into());
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
    save(&mut r, &c, "01-spider-web")?;
    c.title = "Circular grid, value labels".into();
    c.style.grid_shape = RadarGridShape::Circle;
    c.labels = RadarLabels::Values;
    c.value_suffix = " pt".into();
    c.label_decimals = 0;
    save(&mut r, &c, "02-circle-values")?;
    c.title = "Fixed 0 to 10 axis, square markers".into();
    c.style.grid_shape = RadarGridShape::Polygon;
    c.labels = RadarLabels::None;
    c.value_range = Some([0., 10.]);
    c.ring_spacing = Some(2.5);
    c.style.marker = RadarMarker::Square;
    c.style.fill_opacity = 0.15;
    save(&mut r, &c, "03-fixed-axis")?;
    c.title = "Four teams, counter-clockwise from the right".into();
    c.style.start_angle_degrees = 0.;
    c.style.clockwise = false;
    c.style.marker = RadarMarker::None;
    c.style.line_width_px = 3.;
    c.value_range = None;
    c.ring_spacing = None;
    c.series.push(RadarSeries::new(
        "v3",
        "Release 3",
        vec![9., 8.5, 8., 9., 9.5, 8.],
        Color::from_rgb8(42, 168, 120),
    ));
    let mut muted = RadarSeries::new(
        "baseline",
        "Baseline",
        vec![5., 5., 5., 5., 5., 5.],
        Color::from_rgb8(120, 120, 130),
    );
    muted.style = Some(RadarSeriesStyle {
        line_width_px: Some(1.5),
        fill: Some(Color::new(0., 0., 0., 0.)),
        marker: Some(RadarMarker::None),
        marker_size_px: None,
    });
    c.series.push(muted);
    save(&mut r, &c, "04-four-series")?;
    c.title = "Selected vertex with emphasized labels".into();
    c.style.start_angle_degrees = -90.;
    c.style.clockwise = true;
    c.style.marker = RadarMarker::Circle;
    c.series.truncate(2);
    c.labels = RadarLabels::Emphasized;
    c.selected = Some(RadarTarget::vertex("v2", "docs"));
    c.hovered = Some(RadarTarget::series("v1"));
    save(&mut r, &c, "05-selection")?;
    Ok(())
}
