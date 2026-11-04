//! Grotrian diagrams of four atoms from NIST data (#205), each trimmed to
//! the group of levels a paper on it would show:
//!
//! - **He I**: singlets beside triplets, n ≤ 4 labelled, the visible lines.
//! - **Ar I**: the 3p⁵ 4s and 4p manifolds with 5s, 3d and 5p, the two
//!   ion cores (²P°₃/₂ and ²P°₁/₂) side by side, the 4p → 4s lines used in
//!   plasma diagnostics, and Paschen labels on 4s and 4p.
//! - **Cu I**: the doublets and the 3d⁹4s4p quartets below 7 eV, with the
//!   324.8 and 327.4 nm resonance lines and the 510.6 and 578.2 nm lines of
//!   the copper vapour laser.
//! - **U I**: the selected levels of the NIST Handbook (Blaise and Wyart),
//!   in cm⁻¹, by J and parity, with lines computed by the host from level
//!   pairs that E1 selection allows (ΔJ ≤ 1, parity change), among them the
//!   591.5 nm first step of laser isotope separation.
//!
//! The host chooses the columns, the labels and the lines; the renderer
//! draws them. Run from the workspace root:
//! cargo run --locked -p figgy-renderer --example nist_grotrian_gallery -- target/nist_grotrian_gallery

#[allow(dead_code)]
#[path = "support/gallery.rs"]
mod gallery;
#[allow(dead_code)]
#[path = "support/nist_csv.rs"]
mod nist_csv;
use gallery::{Result, chart, output_directory, with_renderer};
use nist_csv::{LevelRow, Levels, LineRow};
use renderer::config::{
    AxisBreak, AxisBreakStyle, AxisBreaks, BreakMark, Category, CategoryGroup, CategoryGroups,
    TickVisibility,
};
use renderer::energy_levels::{
    EnergyLevel, EnergyLevelSeries, IonizationLimit, LevelRole, Transition,
};
use renderer::text::RichText;
use renderer::{Chart, Color, DataRenderType, Renderer, SeriesConfig, encode_png};
use std::path::Path;

const LEVEL: Color = Color::from_rgb8(34, 52, 88);
const ARROW: Color = Color::from_rgb8(214, 120, 20);
const LABEL: Color = Color::from_rgb8(70, 76, 90);
const WAVELENGTH: Color = Color::from_rgb8(150, 90, 0);
const HEADER: Color = Color::from_rgb8(24, 39, 63);

fn data(name: &str) -> Result<String> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("examples/data/nist")
        .join(name);
    Ok(std::fs::read_to_string(path)?)
}

fn tex(text: &str, size: f32, color: Color) -> Result<RichText> {
    Ok(RichText::from_tex(text, color, size, "")?)
}

fn level(row: &LevelRow, column: &str, label: Option<RichText>) -> EnergyLevel {
    EnergyLevel {
        id: row.id(),
        column: column.into(),
        configuration: row.configuration.clone(),
        term: row.term.clone(),
        j: row.j.clone(),
        g: row.g,
        prefix: String::new(),
        energy: row.energy,
        suffix: String::new(),
        uncertainty: row.uncertainty,
        reference: row.reference.clone(),
        role: LevelRole::Level,
        label,
        label_offset: [0.0; 2],
    }
}

fn transition(id: &str, lower: &str, upper: &str, nm: f64, aki: Option<f64>) -> Transition {
    Transition {
        id: id.into(),
        lower: lower.into(),
        upper: upper.into(),
        label: Some(RichText::plain(&format!("{nm:.1}"), WAVELENGTH, 11.0, "")),
        label_offset: [0.0; 2],
        value: aki,
        x_offset: 0.0,
        observed_wavelength: Some(nm),
        observed_uncertainty: None,
        ritz_wavelength: None,
        ritz_uncertainty: None,
        intensity: String::new(),
        aki,
        accuracy: String::new(),
        line_type: String::new(),
        tp_reference: String::new(),
        line_reference: String::new(),
    }
}

fn limit(row: &LevelRow, text: &str) -> IonizationLimit {
    IonizationLimit {
        id: row.configuration.clone(),
        energy: row.energy,
        label: Some(RichText::plain(
            text,
            Color::from_rgb8(40, 50, 120),
            12.0,
            "",
        )),
        label_offset: [0.0; 2],
        uncertainty: row.uncertainty,
        reference: row.reference.clone(),
    }
}

/// The strongest `count` lines between drawn levels, by Aki, one per pair.
fn strongest(
    lines: &[LineRow],
    series: &EnergyLevelSeries,
    keep: impl Fn(&LineRow) -> bool,
    count: usize,
) -> Vec<Transition> {
    let drawn = |id: &str| series.levels.iter().any(|l| l.id == id);
    let mut picked: Vec<Transition> = Vec::new();
    for line in lines {
        let Some(nm) = line.wavelength() else {
            continue;
        };
        if !keep(line) || !drawn(&line.lower) || !drawn(&line.upper) {
            continue;
        }
        if let Some(old) = picked
            .iter_mut()
            .find(|t| t.lower == line.lower && t.upper == line.upper)
        {
            if line.aki > old.aki {
                *old = transition(&format!("{nm:.2}"), &line.lower, &line.upper, nm, line.aki);
            }
            continue;
        }
        picked.push(transition(
            &format!("{nm:.2}"),
            &line.lower,
            &line.upper,
            nm,
            line.aki,
        ));
    }
    picked.sort_by(|a, b| {
        b.aki
            .partial_cmp(&a.aki)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    picked.truncate(count);
    picked
}

/// A Grotrian chart: no grid, no mirrored ticks, the term columns named.
fn grotrian(title: &str, y_title: &str, categories: Vec<Category>) -> Chart {
    let mut c = chart(title, "", y_title, (1100, 780));
    let config = c.config_mut();
    config.grid.show_major_x = false;
    config.grid.show_major_y = false;
    config.grid.show_minor_y = false;
    config.top_x.tick = TickVisibility::None;
    config.right_y.tick = TickVisibility::None;
    config.bottom_x.categories = Some(categories);
    config.bottom_x.out_margin = 84.0;
    c
}

fn groups(c: &mut Chart, groups: &[(&str, &str, &str)]) {
    c.config_mut().bottom_x.category_groups = Some(CategoryGroups {
        groups: groups
            .iter()
            .map(|(label, first, last)| CategoryGroup {
                label: RichText::plain(label, HEADER, 15.0, ""),
                first: (*first).into(),
                last: (*last).into(),
                bracket: true,
            })
            .collect(),
        offset: [0.0; 2],
    });
}

fn break_below(c: &mut Chart, min: f64, max: f64, from: f64, to: f64) {
    break_sized(
        c,
        min,
        max,
        from,
        to,
        renderer::config::PieceSizing::Weights(vec![1.0, 9.0]),
    );
}

fn break_sized(
    c: &mut Chart,
    min: f64,
    max: f64,
    from: f64,
    to: f64,
    sizing: renderer::config::PieceSizing,
) {
    c.set_y_range(min, max);
    c.config_mut().left_y.minor_count = 1;
    c.config_mut().left_y.breaks = Some(AxisBreaks {
        ranges: vec![AxisBreak::new(from, to)],
        style: AxisBreakStyle::Continuous,
        gap: 0.04,
        mark: BreakMark::default(),
        frame_each_panel: false,
        sizing,
    });
}

fn spec(id: &str, series: EnergyLevelSeries) -> SeriesConfig {
    SeriesConfig {
        series_id: id.into(),
        source_id: None,
        label: None,
        x_column: String::new(),
        y_column: String::new(),
        z_column: None,
        render_type: DataRenderType::EnergyLevels(series),
    }
}

fn styled(mut series: EnergyLevelSeries) -> EnergyLevelSeries {
    series.style.level.color = LEVEL;
    series.style.level.width = 2.2;
    series.style.transition.color = ARROW;
    series.style.transition.width = 1.3;
    series
}

fn save(r: &mut Renderer, c: &Chart, series: &[SeriesConfig], out: &Path, name: &str) -> Result {
    // The host fits the axis bands to their labels and titles.
    let measure = renderer::CpuTextMeasure::for_style(&c.config().draw_style);
    let c = &Chart::new(renderer::layout::auto_layout(c.config(), &measure));
    for scale in [1.0, 2.0] {
        let image = r.export_rgba_blocking(
            renderer::ExportSource::chart(c, series),
            renderer::ExportOptions::document(scale).with_clear(Color::WHITE),
        )?;
        let path = out.join(format!("{name}-{scale}x.png"));
        std::fs::write(&path, encode_png(&image)?)?;
        println!("{}: {}x{}", path.display(), image.width, image.height);
    }
    Ok(())
}

// He I -----------------------------------------------------------------

fn helium(r: &mut Renderer, out: &Path) -> Result {
    let Levels { levels, limits, .. } = nist_csv::levels(&data("he_i_levels.csv")?)?;
    let lines = nist_csv::lines(&data("he_i_lines.csv")?)?;
    let columns = ["1S", "1P*", "1D", "1F*", "gap", "3S", "3P*", "3D", "3F*"];
    let mut series = EnergyLevelSeries::default();
    for row in &levels {
        if !columns.contains(&row.term.as_str()) {
            continue;
        }
        let outer = row.outer();
        let n: u32 = outer
            .chars()
            .take_while(char::is_ascii_digit)
            .collect::<String>()
            .parse()
            .unwrap_or(1);
        // A triplet keeps one labelled row per term: J = 1 (³S has one).
        let shown = !row.term.starts_with('3') || row.term == "3S" || row.j == "1";
        let label = (n <= 4 && shown).then(|| RichText::plain(outer, LABEL, 11.0, ""));
        series.levels.push(level(row, &row.term, label));
    }
    for l in &limits {
        series
            .limits
            .push(limit(l, &format!("He II  {:.3} eV", l.energy)));
    }
    // Resolved multiplets: the host adds each one's g-weighted centroid,
    // which the main view draws, and the J rows become its components.
    let mut centroids: Vec<EnergyLevel> = Vec::new();
    for level in &series.levels {
        let key = (&level.configuration, &level.term);
        if centroids.iter().any(|c| (&c.configuration, &c.term) == key) {
            continue;
        }
        let members: Vec<&EnergyLevel> = series
            .levels
            .iter()
            .filter(|l| (&l.configuration, &l.term) == key)
            .collect();
        if members.len() < 2 {
            continue;
        }
        let weight: f64 = members.iter().map(|l| l.g.unwrap_or(1.0)).sum();
        let energy = members
            .iter()
            .map(|l| l.g.unwrap_or(1.0) * l.energy)
            .sum::<f64>()
            / weight;
        let label = members.iter().find_map(|l| l.label.clone());
        centroids.push(EnergyLevel {
            id: format!("{} {}", level.configuration, level.term),
            j: String::new(),
            g: Some(weight),
            energy,
            uncertainty: None,
            role: LevelRole::Centroid,
            label,
            ..level.clone()
        });
    }
    for level in &mut series.levels {
        let id = format!("{} {}", level.configuration, level.term);
        if centroids.iter().any(|c| c.id == id) {
            level.role = LevelRole::Component { multiplet: id };
            level.label = None;
        }
    }
    series.levels.extend(centroids);
    // Lines: each end taken to its multiplet's centroid, one arrow per pair
    // of drawn levels, the strongest visible same-spin lines.
    let to_drawn = |id: &str| -> String {
        let multiplet = id.rsplit_once(' ').map_or(id, |(m, _)| m).to_string();
        if series.levels.iter().any(|l| l.id == multiplet) {
            multiplet
        } else {
            id.to_string()
        }
    };
    let lines: Vec<LineRow> = lines
        .into_iter()
        .map(|mut l| {
            l.lower = to_drawn(&l.lower);
            l.upper = to_drawn(&l.upper);
            l
        })
        .collect();
    let spin = |id: &str| id.split(' ').nth(1).and_then(|t| t.chars().next());
    series.transitions = strongest(
        &lines,
        &series,
        |l| {
            l.wavelength()
                .is_some_and(|nm| (380.0..=720.0).contains(&nm))
                && spin(&l.lower) == spin(&l.upper)
        },
        12,
    );
    let mut categories = Vec::new();
    for id in columns {
        let mut category = Category::new(id, "");
        if id != "gap" {
            let (m, rest) = id.split_at(1);
            let (l, odd) = rest.split_at(1);
            category.label = id.into();
            category.rich_label = Some(tex(
                &format!(
                    "$^{m}\\mathrm{{{l}}}{}$",
                    if odd == "*" { "^o" } else { "" }
                ),
                15.0,
                Color::from_rgb8(30, 30, 30),
            )?);
        }
        categories.push(category);
    }
    let mut c = grotrian("He I", "Energy (eV)", categories);
    groups(
        &mut c,
        &[
            ("Singlets (S = 0)", "1S", "1F*"),
            ("Triplets (S = 1)", "3S", "3F*"),
        ],
    );
    break_below(&mut c, -0.15, 25.2, 0.25, 19.6);
    let series = styled(series);
    series.write_continuum(
        &mut c.config_mut().annotations,
        "he",
        25.2,
        Color::from_rgba8(80, 100, 220, 36),
    )?;
    save(r, &c, &[spec("he", series)], out, "he-i")
}

// Ar I -----------------------------------------------------------------

fn argon(r: &mut Renderer, out: &Path) -> Result {
    let Levels { levels, limits, .. } = nist_csv::levels(&data("ar_i_levels.csv")?)?;
    let lines = nist_csv::lines(&data("ar_i_lines.csv")?)?;
    let manifolds = ["4s", "4p", "5s", "3d", "5p"];
    let column = |row: &LevelRow| -> Option<String> {
        if row.configuration == "3s2.3p6" {
            return Some("ground".into());
        }
        let primed = row.configuration.contains("<1/2>");
        manifolds
            .contains(&row.outer())
            .then(|| format!("{}{}", row.outer(), if primed { "'" } else { "" }))
    };
    // Paschen numbers: 1s5..1s2 and 2p10..2p1 count down in energy.
    let paschen = |row: &LevelRow| -> Option<String> {
        let (shell, top) = match row.outer() {
            "4s" => ("1s", 5),
            "4p" => ("2p", 10),
            _ => return None,
        };
        let below = levels
            .iter()
            .filter(|l| l.outer() == row.outer() && l.energy < row.energy)
            .count();
        Some(format!("{shell}{}", top - below))
    };
    let mut series = EnergyLevelSeries::default();
    for row in &levels {
        let Some(col) = column(row) else {
            continue;
        };
        let label = match paschen(row) {
            Some(p) => Some(RichText::plain(&p, LABEL, 10.0, "")),
            None if col == "ground" => Some(tex("$3p^6$", 11.0, LABEL)?),
            None => {
                let k = row.term.trim_start_matches('2').trim_end_matches('*');
                Some(tex(&format!("${k}_{{{}}}$", row.j), 10.0, LABEL)?)
            }
        };
        series.levels.push(level(row, &col, label));
    }
    for l in &limits {
        series
            .limits
            .push(limit(l, &format!("Ar II ²P°₃/₂  {:.3} eV", l.energy)));
    }
    series.transitions = strongest(
        &lines,
        &series,
        |l| l.lower.contains(".4s ") && l.upper.contains(".4p "),
        14,
    );
    let order = [
        "ground", "4s", "4p", "5s", "3d", "5p", "gap", "4s'", "4p'", "5s'", "3d'", "5p'",
    ];
    let categories = order
        .iter()
        .map(|id| {
            let mut c = Category::new(*id, "");
            if *id != "gap" {
                c.label = (*id).into();
                c.rich_label = tex(
                    &match *id {
                        "ground" => "$3p^6$".to_string(),
                        id => format!("$\\mathrm{{{}}}$", id.replace('\'', "'")),
                    },
                    14.0,
                    Color::from_rgb8(30, 30, 30),
                )
                .ok();
            }
            c
        })
        .collect();
    let mut c = grotrian("Ar I", "Energy (eV)", categories);
    groups(
        &mut c,
        &[
            ("3p⁵ ²P°₃/₂ core", "4s", "5p"),
            ("3p⁵ ²P°₁/₂ core", "4s'", "5p'"),
        ],
    );
    break_below(&mut c, -0.2, 16.0, 0.3, 11.3);
    let series = styled(series);
    series.write_continuum(
        &mut c.config_mut().annotations,
        "ar",
        16.0,
        Color::from_rgba8(80, 100, 220, 36),
    )?;
    save(r, &c, &[spec("ar", series)], out, "ar-i")
}

// Cu I -----------------------------------------------------------------

fn copper(r: &mut Renderer, out: &Path) -> Result {
    let Levels { levels, limits, .. } = nist_csv::levels(&data("cu_i_levels.csv")?)?;
    let lines = nist_csv::lines(&data("cu_i_lines.csv")?)?;
    let order = ["2S", "2P*", "2D", "2F*", "gap", "4P*", "4D*", "4F*"];
    let mut series = EnergyLevelSeries::default();
    for row in &levels {
        if !order.contains(&row.term.as_str()) {
            continue;
        }
        let shell = row
            .configuration
            .trim_start_matches("3d10.")
            .trim_start_matches("3d9.(2D).")
            .trim_start_matches("3d9.")
            .replace(".(3P*)", "")
            .replace(".(1P*)", "")
            .replace('.', "");
        let label = tex(&format!("$\\mathrm{{{shell}}}_{{{}}}$", row.j), 10.0, LABEL)?;
        series.levels.push(level(row, &row.term, Some(label)));
    }
    for l in &limits {
        series
            .limits
            .push(limit(l, &format!("Cu II 3d¹⁰ ¹S  {:.3} eV", l.energy)));
    }
    let classic = [324.754, 327.396, 510.554, 578.213];
    let mut transitions = strongest(&lines, &series, |_| true, 10);
    for t in strongest(
        &lines,
        &series,
        |l| {
            l.wavelength()
                .is_some_and(|nm| classic.iter().any(|c| (nm - c).abs() < 0.05))
        },
        4,
    ) {
        if !transitions.iter().any(|old| old.id == t.id) {
            transitions.push(t);
        }
    }
    series.transitions = transitions;
    let categories = order
        .iter()
        .map(|id| {
            let mut c = Category::new(*id, "");
            if *id != "gap" {
                let (m, rest) = id.split_at(1);
                let (l, odd) = rest.split_at(1);
                c.label = (*id).into();
                c.rich_label = tex(
                    &format!(
                        "$^{m}\\mathrm{{{l}}}{}$",
                        if odd == "*" { "^o" } else { "" }
                    ),
                    15.0,
                    Color::from_rgb8(30, 30, 30),
                )
                .ok();
            }
            c
        })
        .collect();
    let mut c = grotrian("Cu I", "Energy (eV)", categories);
    groups(
        &mut c,
        &[
            ("Doublets", "2S", "2F*"),
            ("Quartets (3d⁹4s4p)", "4P*", "4F*"),
        ],
    );
    c.set_y_range(-0.2, 8.2);
    c.config_mut().left_y.minor_count = 1;
    let series = styled(series);
    series.write_continuum(
        &mut c.config_mut().annotations,
        "cu",
        8.2,
        Color::from_rgba8(80, 100, 220, 36),
    )?;
    save(r, &c, &[spec("cu", series)], out, "cu-i")
}

// U I ------------------------------------------------------------------

fn uranium(r: &mut Renderer, out: &Path) -> Result {
    let Levels { levels, limits, .. } = nist_csv::levels(&data("u_i_levels.csv")?)?;
    // Unassigned levels above 15 000 cm⁻¹ are the even upper levels of the
    // Handbook table; below, a blank term is odd (`*`).
    let odd = |row: &LevelRow| row.odd() || (row.term.is_empty() && row.energy < 15_000.0);
    let column = |row: &LevelRow| format!("{}{}", if odd(row) { "o" } else { "e" }, row.j);
    let mut series = EnergyLevelSeries::default();
    for row in &levels {
        let label = (!row.term.is_empty() && row.term != "*").then(|| {
            let t = row.term.trim_end_matches('*');
            let (m, l) = t.split_at(1);
            tex(
                &format!(
                    "$^{m}\\mathrm{{{l}}}{}_{{{}}}$",
                    if row.odd() { "^o" } else { "" },
                    row.j
                ),
                10.0,
                LABEL,
            )
        });
        let label = label.transpose()?;
        series.levels.push(level(row, &column(row), label));
    }
    for l in &limits {
        series
            .limits
            .push(limit(l, &format!("U II ⁴I°₉/₂  {:.1} cm⁻¹", l.energy)));
    }
    // Lines computed from level pairs: the ground J = 6 and the 620 cm⁻¹
    // J = 5 levels to the even levels E1 selection allows.
    let mut transitions = Vec::new();
    for lower in levels.iter().filter(|l| l.energy < 700.0) {
        let jl: i32 = lower.j.parse().unwrap_or(0);
        for upper in levels.iter().filter(|u| !odd(u) && u.energy > 15_000.0) {
            let ju: i32 = upper.j.parse().unwrap_or(-9);
            if (ju - jl).abs() > 1 {
                continue;
            }
            let nm = 1.0e7 / (upper.energy - lower.energy) / 1.000_277;
            transitions.push(transition(
                &format!("{nm:.2}"),
                &lower.id(),
                &upper.id(),
                nm,
                None,
            ));
        }
    }
    transitions.retain(|t| {
        let nm = t.observed_wavelength.unwrap_or(0.0);
        [591.54, 436.2, 415.4, 394.4, 424.2, 404.3]
            .iter()
            .any(|c| (nm - c).abs() < 0.6)
    });
    series.transitions = transitions;
    let mut order: Vec<String> = Vec::new();
    for p in ["o", "e"] {
        let mut js: Vec<i32> = series
            .levels
            .iter()
            .filter(|l| l.column.starts_with(p))
            .filter_map(|l| l.column[1..].parse().ok())
            .collect();
        js.sort();
        js.dedup();
        order.extend(js.into_iter().map(|j| format!("{p}{j}")));
        if p == "o" {
            order.push("gap".into());
        }
    }
    let categories = order
        .iter()
        .map(|id| {
            let mut c = Category::new(id.as_str(), "");
            if id != "gap" {
                c.label = id[1..].into();
            }
            c
        })
        .collect();
    let first_odd = order.first().cloned().unwrap_or_default();
    let last_odd = order
        .iter()
        .take_while(|c| *c != "gap")
        .last()
        .cloned()
        .unwrap_or_default();
    let first_even = order
        .iter()
        .skip_while(|c| *c != "gap")
        .nth(1)
        .cloned()
        .unwrap_or_default();
    let last_even = order.last().cloned().unwrap_or_default();
    let mut c = grotrian("U I", "Energy (cm⁻¹)", categories);
    c.config_mut().bottom_x.title_option.visible = true;
    c.config_mut().bottom_x.title_option.text = RichText::plain("J", HEADER, 15.0, "");
    groups(
        &mut c,
        &[
            ("Odd parity", &first_odd, &last_odd),
            ("Even parity", &first_even, &last_even),
        ],
    );
    // The limit sits far above the levels: a thin piece above the break.
    break_sized(
        &mut c,
        -500.0,
        51_500.0,
        33_000.0,
        49_000.0,
        renderer::config::PieceSizing::Weights(vec![9.0, 1.0]),
    );
    let series = styled(series);
    series.write_continuum(
        &mut c.config_mut().annotations,
        "u",
        52_000.0,
        Color::from_rgba8(80, 100, 220, 36),
    )?;
    save(r, &c, &[spec("u", series)], out, "u-i")
}

fn main() -> Result {
    let out = output_directory("target/nist_grotrian_gallery")?;
    with_renderer(|r| {
        helium(r, &out)?;
        argon(r, &out)?;
        copper(r, &out)?;
        uranium(r, &out)
    })
}
