//! Minimal native line chart: upload two columns and write a PNG.
//! cargo run --locked -p figgy-renderer --example line
use renderer::theme::Theme;
use renderer::{Chart, Color, ExportOptions, ExportSource, Renderer, SeriesConfig};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut renderer = Renderer::headless_blocking()?;
    let x: Vec<f64> = (0..=100).map(|i| f64::from(i) / 10.0).collect();
    let f = 0.2;
    let y: Vec<f64> = x
        .iter()
        .map(|t| (std::f64::consts::TAU * f * t).sin())
        .collect();
    renderer.add_column("x", &x)?;
    renderer.add_column("y", &y)?;

    let mut series = [SeriesConfig::line("sine", "x", "y")];
    let mut chart = Chart::sized(960, 600)
        .with_title("A 0.2 Hz sine wave")
        .with_x_title("Time (s)")
        // A `$…$` span in a title is TeX.
        .with_y_title(r"$\sin(2\pi f t)$");
    chart.set_x_range(0.0, 10.0);
    chart.set_y_range(-1.2, 1.2);
    // The documented look, with a blue line a little heavier. A theme is a
    // value: edit any field before applying it. Applying refits the margins
    // to the labels.
    let mut theme = Theme::default();
    theme.series.palette = vec![Color::from_rgb8(34, 104, 198)];
    theme.series.lines.line_width = 2.5;
    chart.apply_custom_theme(&theme, &mut series)?;

    let output = std::env::args_os()
        .nth(1)
        .unwrap_or_else(|| "line.png".into());
    let options = ExportOptions::panel(1.0).with_clear(Color::WHITE);
    renderer.save_blocking(ExportSource::chart(&chart, &series), options, &output)?;
    println!("{}", std::path::Path::new(&output).display());
    Ok(())
}
