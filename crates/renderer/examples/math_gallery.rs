//! Every construction of the math layout (#45), written with
//! `RichText::from_tex` and drawn by the renderer as annotation texts of an
//! empty chart: each formula with its TeX source beneath it in plain text.
//! The chart title is a formula too. The same page is written in the sketch
//! draw style, whose handwritten text face leaves math in the math face (D6).
//!
//! cargo run --locked -p figgy-renderer --example math_gallery -- target/math-gallery
//!
//! Writes `math-gallery.png` (1200 × 1600), `math-gallery@2x.png`, and the
//! sketch pair `math-gallery-sketch.png` and `math-gallery-sketch@2x.png`.
use renderer::{
    Chart, Color, Config, Renderer, RendererDevice,
    annotation::{Anchor, Annotation, AnnotationKind},
    config::{DrawStyle, TickVisibility},
    data_render::{create_instance, request_adapter, request_device},
    default, encode_png,
    layout::{ChartArea, Rect},
    text::RichText,
};
use std::{error::Error, path::PathBuf, sync::Arc};

/// The page.
const SIZE: (u32, u32) = (1200, 1600);
/// The formulas, in reading order (two columns).
const FORMULAS: &[&str] = &[
    r"$\frac{a}{b}$ and $\frac{1}{1+\frac{1}{x}}$",
    r"$\binom{n}{k} = \frac{n!}{k!\,(n-k)!}$",
    r"$\sqrt{x^2+y^2}$ and $\sqrt[3]{x}$",
    r"$\sum_{i=1}^{n} i = \frac{n(n+1)}{2}$",
    r"$\sum\limits_{i=1}^{n} i = \dfrac{n(n+1)}{2}$",
    r"$\int_0^\infty e^{-x^2}\,dx = \frac{\sqrt{\pi}}{2}$",
    r"$\lim_{x\to 0}\frac{\sin x}{x} = 1$",
    r"$\lim\limits_{x\to 0}\frac{\sin x}{x} = 1$",
    r"$\prod_{k=1}^{n} k = n!$",
    r"$\bigcup_{i\in I} A_i \cap \bigcap\limits_{j} B_j$",
    r"$\left(\frac{a}{b}\right)^{2}$",
    r"$\left[\ \frac{x}{2}\ \middle|\ x\in\mathbb{Z}\ \right]$",
    r"$\Bigg(\bigg(\Big(\big(x\big)\Big)\bigg)\Bigg)$",
    r"$\hat{x}\ \bar{y}\ \vec{v}\ \tilde{z}\ \dot{q}\ \ddot{r}$",
    r"$\widehat{abc}\ \widetilde{xyz}\ \overrightarrow{AB}$",
    r"$\overline{AB}\ \underline{cd}$",
    r"$\overbrace{a+b+c}^{n}$",
    r"$\underbrace{x\cdots x}_{k}$",
    r"$a \overset{?}{=} b \quad A \stackrel{\text{def}}{=} B$",
    r"$X \xrightarrow{f} Y \xleftarrow[g]{h} Z$",
    r"$E = mc^2$",
    r"$e^{i\pi} + 1 = 0$",
    r"$\alpha\beta\gamma\ \Gamma\Delta\Omega$",
    r"$\nabla \cdot \vec{E} = \frac{\rho}{\varepsilon_0}$",
    r"$\frac{\partial u}{\partial t} = \alpha \nabla^2 u$",
    r"$a_{n+1} = a_n + d$",
    r"$x^{y^{z}}$ and $f'(x) + g''(x)$",
    r"$\mathbf{A}\mathbf{x} = \mathbf{b}$",
    r"$\text{speed} = 3\,\text{m/s}$",
    r"$\operatorname{tr}(A) \le \|A\|\,\sqrt{n}$",
    r"$x \in \mathbb{R},\ \mathcal{L}(\theta) \propto \exp(-\theta^2)$",
    r"$\max_{x \ge 0} f(x) \ne \min_{y} g(y)$",
];

fn main() -> Result<(), Box<dyn Error>> {
    let output = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/math-gallery"));
    std::fs::create_dir_all(&output)?;
    let instance = create_instance();
    let adapter = request_adapter(&instance)?;
    eprintln!("Adapter: {:?}", adapter.get_info());
    let (device, queue) = request_device(&adapter)?;
    let gpu = RendererDevice::new(Arc::new(device), Arc::new(queue));
    let mut renderer = Renderer::try_new(gpu, wgpu::TextureFormat::Rgba8Unorm, 16 << 20)?;
    for (name, sketch) in [("math-gallery", false), ("math-gallery-sketch", true)] {
        let chart = Chart::new(gallery(sketch)?);
        for (suffix, scale) in [("", 1.0), ("@2x", 2.0)] {
            let image = pollster::block_on(renderer.export_document_rgba_with_clear_async(
                &chart,
                &[],
                scale,
                Color::WHITE,
            ))?;
            let path = output.join(format!("{name}{suffix}.png"));
            std::fs::write(&path, encode_png(&image)?)?;
            eprintln!(
                "wrote {} ({}x{})",
                path.display(),
                image.width,
                image.height
            );
        }
    }
    Ok(())
}

/// The page: no axes, no grid, a math title and one annotation pair (the
/// formula, its source) per formula.
fn gallery(sketch: bool) -> Result<Config, Box<dyn Error>> {
    let ink = Color::from_rgb8(24, 39, 63);
    let source = Color::from_rgb8(110, 118, 130);
    let mut cfg = default::default_config();
    cfg.chart_area = ChartArea(Rect {
        x: 0,
        y: 0,
        width: SIZE.0,
        height: SIZE.1,
    });
    cfg.legend.visible = false;
    for axis in [
        &mut cfg.bottom_x,
        &mut cfg.top_x,
        &mut cfg.left_y,
        &mut cfg.right_y,
    ] {
        axis.line_visible = false;
        axis.label_style.label_visible = false;
        axis.label_style.visible = false;
        axis.tick = TickVisibility::None;
        axis.title_option.visible = false;
        axis.out_margin = 24.0;
    }
    cfg.grid.show_major_x = false;
    cfg.grid.show_major_y = false;
    cfg.grid.show_minor_x = false;
    cfg.grid.show_minor_y = false;
    cfg.chart_title.text = RichText::from_tex(
        r"Math in figgy: $\sum_{k} \frac{x^k}{k!}$, from TeX",
        ink,
        26.0,
        "",
    )?;
    cfg.chart_title.top_margin = 64.0;
    if sketch {
        cfg.draw_style = DrawStyle::Sketch(Default::default());
    }
    let rows = FORMULAS.len().div_ceil(2);
    for (k, tex) in FORMULAS.iter().enumerate() {
        let (column, row) = ((k / rows) as f32, (k % rows) as f32);
        let x = 0.02 + column * 0.5;
        let y = 0.005 + row / rows as f32;
        let formula = RichText::from_tex(tex, ink, 22.0, "")?;
        cfg.annotations.items.push(text(formula, x, y));
        let plain = RichText::plain(tex, source, 11.0, "");
        cfg.annotations
            .items
            .push(text(plain, x, y + 0.6 / rows as f32));
    }
    Ok(cfg)
}

fn text(text: RichText, x: f32, y: f32) -> Annotation {
    Annotation::new(AnnotationKind::Text {
        anchor: Anchor::Ratio { x, y },
        text,
        box_style: None,
    })
}
