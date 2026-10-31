//! Native procedural categorical bars, no image generation or post-render scaling.
use renderer::{Color, RendererDevice, categorical::*, data_render, encode_png};
use std::{path::PathBuf, sync::Arc};
fn main() -> Result<(), Box<dyn std::error::Error>> {
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
    let mut renderer = CategoricalRenderer::new(gpu.clone(), wgpu::TextureFormat::Rgba8Unorm)?;
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
    chart.style.material = CategoryBarMaterial::Matte;
    let save = |r: &mut CategoricalRenderer,
                c: &CategoricalChart,
                name: &str|
     -> Result<(), Box<dyn std::error::Error>> {
        let image = r.export_rgba(c, (900, 620), 1.0)?;
        std::fs::write(directory.join(format!("{name}.png")), encode_png(&image)?)?;
        r.end_frame();
        gpu.device().poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        })?;
        println!("{name}: {}x{}", image.width, image.height);
        Ok(())
    };
    save(&mut renderer, &chart, "grouped-matte")?;
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
    chart.style.material = CategoryBarMaterial::SatinMetal;
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
    save(&mut renderer, &chart, "horizontal-satin")?;
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
    chart.style.material = CategoryBarMaterial::Enamel;
    save(&mut renderer, &chart, "stacked-enamel")?;
    chart.title = "100% Stacked · Outline".into();
    chart.direction = CategoryBarDirection::Horizontal;
    chart.mode = CategoryBarMode::PercentStacked;
    chart.label_format = CategoryBarLabelFormat::Percent;
    chart.value_title = "Share".into();
    chart.style.material = CategoryBarMaterial::Flat;
    chart.style.outline = true;
    save(&mut renderer, &chart, "percent-outline")?;
    chart.title = "Positive and negative values".into();
    chart.mode = CategoryBarMode::Stacked;
    chart.direction = CategoryBarDirection::Vertical;
    chart.label_format = CategoryBarLabelFormat::Value;
    chart.value_title = "Change".into();
    chart.series[1].values = vec![Some(-8.), Some(-6.), Some(-12.)];
    save(&mut renderer, &chart, "signed-stacked")?;
    Ok(())
}
