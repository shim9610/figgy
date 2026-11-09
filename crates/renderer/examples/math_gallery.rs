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
    AxisPreset, Chart, Color, ExportOptions, ExportSource, Renderer,
    annotation::{Anchor, Annotation, AnnotationKind},
    config::DrawStyle,
    text::RichText,
};
use std::{error::Error, path::PathBuf};

/// The page.
const SIZE: (u32, u32) = (1200, 1600);
/// The formulas, in reading order (two columns).
const FORMULAS: &[&str] = &[
    r"$\frac{a}{b}$ and $\frac{1}{1+\frac{1}{x}}$",
    r"$\binom{n}{k} = \frac{n!}{k!\,(n-k)!}$",
    r"$\sqrt{x^2+y^2}$ and $\sqrt[3]{x}$",
    r"$\sum_{i=1}^{n} i = \frac{n(n+1)}{2}$",
    r"$$\sum_{i=1}^{n} i = \frac{n(n+1)}{2}$$",
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
    let mut renderer = Renderer::headless_blocking()?;
    for (name, sketch) in [("math-gallery", false), ("math-gallery-sketch", true)] {
        let mut chart = gallery(sketch)?;
        chart.fit_layout();
        for (suffix, scale) in [("", 1.0), ("@2x", 2.0)] {
            let path = output.join(format!("{name}{suffix}.png"));
            let options = ExportOptions::document(scale).with_clear(Color::WHITE);
            renderer.save_blocking(ExportSource::chart(&chart, &[]), options, &path)?;
            eprintln!("wrote {}", path.display());
        }
    }
    Ok(())
}

/// The page: no axes, no grid, a math title and one annotation pair (the
/// formula, its source) per formula.
fn gallery(sketch: bool) -> Result<Chart, Box<dyn Error>> {
    let ink = Color::from_rgb8(24, 39, 63);
    let source = Color::from_rgb8(110, 118, 130);
    let mut chart = Chart::sized(SIZE.0, SIZE.1);
    let cfg = chart.config_mut();
    cfg.legend.visible = false;
    // No axis lines, ticks, tick labels or grid.
    cfg.apply_axis_preset(AxisPreset::Hidden);
    for axis in [
        &mut cfg.bottom_x,
        &mut cfg.top_x,
        &mut cfg.left_y,
        &mut cfg.right_y,
    ] {
        axis.title_option.visible = false;
    }
    cfg.chart_title.text = RichText::from_tex(
        r"Math in figgy: $\sum_{k} \frac{x^k}{k!}$, from TeX",
        ink,
        26.0,
        "",
    )?;
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
            .push(text(plain, x, y + 0.68 / rows as f32));
    }
    Ok(chart)
}

fn text(text: RichText, x: f32, y: f32) -> Annotation {
    Annotation::new(AnnotationKind::Text {
        anchor: Anchor::Ratio { x, y },
        text,
        box_style: None,
        align: Default::default(),
    })
}
