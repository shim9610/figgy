//! Grotrian diagrams of four atoms from the bundled NIST exports (#205),
//! each trimmed to the group of levels a paper on it would show. The
//! builders return the chart and its series; the `nist_grotrian_gallery`
//! example saves them and the playground shows them as scenes.
//!
//! - **He I**: singlets beside triplets, n ≤ 4 labelled, the visible lines.
//! - **Ar I**: the 3p⁵ 4s and 4p manifolds with 5s, 3d and 5p, the two
//!   ion cores (²P°₃/₂ and ²P°₁/₂) side by side, the 4p → 4s lines used in
//!   plasma diagnostics, and Paschen labels on 4s and 4p. The full export
//!   draws every term column, the Rydberg members from n = 8 as bands.
//! - **Cu I**: the doublets and the 3d⁹4s4p quartets below 7 eV, with the
//!   324.8 and 327.4 nm resonance lines and the 510.6 and 578.2 nm lines of
//!   the copper vapour laser.
//! - **U I**: the selected levels of the NIST Handbook (Blaise and Wyart),
//!   in cm⁻¹, by J and parity, with lines computed by the host from level
//!   pairs that E1 selection allows (ΔJ ≤ 1, parity change), among them the
//!   591.5 nm first step of laser isotope separation.
//!
//! The host chooses the columns, the labels and the lines; the renderer
//! draws them. The including module declares `gallery` (for `chart` and
//! `Result`) and `nist_csv` beside this one.

pub use super::gallery::Result;
use super::gallery::chart;
use super::nist_csv::{self, LevelRow, Levels, LineRow};
use renderer::config::{
    AxisBreak, AxisBreakStyle, AxisBreaks, BreakMark, Category, CategoryGroup, CategoryGroups,
};
use renderer::energy_levels::{
    EnergyLevel, EnergyLevelSeries, IonizationLimit, LevelBand, LevelRole, Transition,
    WavelengthLabels,
};
use renderer::format::PlainNumberFormat;
use renderer::text::RichText;
use renderer::{Chart, Color, DataRenderType, SeriesConfig};

const LEVEL: Color = Color::from_rgb8(34, 52, 88);
const ARROW: Color = Color::from_rgb8(214, 120, 20);
const LABEL: Color = Color::from_rgb8(70, 76, 90);
const WAVELENGTH: Color = Color::from_rgb8(150, 90, 0);
const HEADER: Color = Color::from_rgb8(24, 39, 63);

/// A bundled NIST export, embedded so every crate that includes this
/// module (the gallery, the playground, the tests) reads the same bytes.
fn data(name: &str) -> Result<String> {
    Ok(match name {
        "he_i_levels.csv" => include_str!("../data/nist/he_i_levels.csv"),
        "he_i_lines.csv" => include_str!("../data/nist/he_i_lines.csv"),
        "ar_i_levels.csv" => include_str!("../data/nist/ar_i_levels.csv"),
        "ar_i_levels_all.csv" => include_str!("../data/nist/ar_i_levels_all.csv"),
        "ar_i_lines.csv" => include_str!("../data/nist/ar_i_lines.csv"),
        "cu_i_levels.csv" => include_str!("../data/nist/cu_i_levels.csv"),
        "cu_i_lines.csv" => include_str!("../data/nist/cu_i_lines.csv"),
        "u_i_levels.csv" => include_str!("../data/nist/u_i_levels.csv"),
        other => return Err(format!("no bundled export {other}").into()),
    }
    .to_string())
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
        // The renderer writes the label from the wavelength (#258).
        label: None,
        label_offset: [0.0; 2],
        value: aki,
        x_offset: 0.0,
        wavelength: Some(nm),
        wavelength_uncertainty: None,
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
            16.0,
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

/// A band over the levels of `column` between `low` and `high` (all of
/// them when the range is open): the host groups a dense run (#205).
fn band(series: &mut EnergyLevelSeries, column: &str, low: f64, high: f64, label: &str) {
    let mut run: Vec<&EnergyLevel> = series
        .levels
        .iter()
        .filter(|l| l.column == column && l.energy >= low && l.energy <= high)
        .collect();
    run.sort_by(|a, b| a.energy.total_cmp(&b.energy));
    let (Some(first), Some(last)) = (run.first(), run.last()) else {
        return;
    };
    if run.len() < 2 {
        return;
    }
    let (first, last) = (first.id.clone(), last.id.clone());
    let id = format!("band {column} {}", series.bands.len());
    series.bands.push(LevelBand {
        id,
        column: column.into(),
        first,
        last,
        label: Some(RichText::plain(label, LABEL, 16.0, "")),
        label_offset: [0.0; 2],
    });
}

/// A Grotrian chart: no grid, no mirrored ticks, the term columns named.
fn grotrian(title: &str, y_title: &str, categories: Vec<Category>) -> Chart {
    let mut c = chart(title, "", y_title, (1100, 780));
    let config = c.config_mut();
    // Read at a column's width, as in a paper: ticks at 18, titles at 20.
    for axis in [
        &mut config.bottom_x,
        &mut config.left_y,
        &mut config.top_x,
        &mut config.right_y,
    ] {
        axis.label_style.font_size = 18.0;
        axis.title_option.text.font_size = 20.0;
    }
    // The Grotrian preset (#205): no grid, no mirrored ticks. Each diagram
    // adds and sizes its own break afterwards.
    config.apply_axis_preset(renderer::AxisPreset::Grotrian);
    config.bottom_x.categories = Some(categories);
    c
}

fn groups(c: &mut Chart, groups: &[(&str, &str, &str)]) {
    c.config_mut().bottom_x.category_groups = Some(CategoryGroups {
        groups: groups
            .iter()
            .map(|(label, first, last)| CategoryGroup {
                label: RichText::plain(label, HEADER, 19.0, ""),
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
        gap_px: 12.0,
        mark: BreakMark::default(),
        frame_each_panel: true,
        sizing,
    });
}

fn spec(id: &str, series: EnergyLevelSeries) -> SeriesConfig {
    if let Err(reason) = series.validate() {
        panic!("{id}: {reason}");
    }
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

/// The diagrams' look. The lines are labelled with their wavelengths in nm
/// to one decimal, written by the renderer from each transition's
/// `wavelength`; the unit is shown once, in a corner note (#258).
fn styled(mut series: EnergyLevelSeries) -> EnergyLevelSeries {
    series.style.level.color = LEVEL;
    series.style.level.width = 2.2;
    series.style.transition.color = ARROW;
    series.style.transition.width = 1.3;
    series.style.wavelength_labels = Some(WavelengthLabels {
        format: PlainNumberFormat {
            decimals: Some(1),
            trim_zeros: false,
        },
        unit: None,
        text: RichText::plain("", WAVELENGTH, 15.0, ""),
    });
    series
}

/// The wavelengths' unit at the plot's `corner`, ten pixels in from it, in
/// the wavelength labels' colour: a text note that owns its text (#258).
fn unit_note(c: &mut Chart, corner: renderer::NoteAlign) {
    let plot = c.config().data_area().map(|area| area.0);
    let config = c.config_mut();
    let text = RichText::plain("Wavelengths in nm", WAVELENGTH, 15.0, "");
    let index = config.annotations.add_corner_note(text, corner);
    if let renderer::AnnotationKind::Text { anchor, .. } = &mut config.annotations.items[index].kind
    {
        // Off the plot's frame: the corner's ratio moved in by 10 px.
        if let (renderer::Anchor::Ratio { x, y }, Ok(plot)) = (anchor, plot) {
            let inward = |f: &mut f32, len: u32| {
                let step = 10.0 / len.max(1) as f32;
                if *f < 0.5 {
                    *f += step;
                } else if *f > 0.5 {
                    *f -= step;
                }
            };
            inward(x, plot.width);
            inward(y, plot.height);
        }
    }
}

// He I -----------------------------------------------------------------

pub fn helium() -> Result<(Chart, Vec<SeriesConfig>)> {
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
        let label = (n <= 4 && shown).then(|| RichText::plain(outer, LABEL, 16.0, ""));
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
                20.0,
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
    unit_note(&mut c, renderer::NoteAlign::BottomRight);
    Ok((c, vec![spec("he", series)]))
}

// Ar I -----------------------------------------------------------------

pub fn argon() -> Result<(Chart, Vec<SeriesConfig>)> {
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
            Some(p) => Some(RichText::plain(&p, LABEL, 16.0, "")),
            None if col == "ground" => Some(tex("$3p^6$", 16.0, LABEL)?),
            None => {
                let k = row.term.trim_start_matches('2').trim_end_matches('*');
                Some(tex(&format!("${k}_{{{}}}$", row.j), 16.0, LABEL)?)
            }
        };
        series.levels.push(level(row, &col, label));
    }
    // The 3d and 5p manifolds as bands: a plasma paper labels 4s and 4p.
    for column in ["3d", "3d'", "5p", "5p'"] {
        let count = series.levels.iter().filter(|l| l.column == column).count();
        let name = column.replace('\'', "′");
        // Below the first limit: the core-excited 3s3p⁶ levels of the same
        // outer electron lie far above it.
        let limit = limits.first().map_or(f64::MAX, |l| l.energy);
        let count = series
            .levels
            .iter()
            .filter(|l| l.column == column && l.energy < limit)
            .count()
            .min(count);
        band(
            &mut series,
            column,
            f64::MIN,
            limit,
            &format!("{name} ({count} levels)"),
        );
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
                    20.0,
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
    unit_note(&mut c, renderer::NoteAlign::BottomRight);
    Ok((c, vec![spec("ar", series)]))
}

// Ar I, every level --------------------------------------------------

/// All 503 Ar I levels of the NIST export by jK term and core, every one
/// labelled: a host that declares no bands. The renderer stacks the labels
/// it can and collapses the Rydberg runs it cannot into brackets (#205).
pub fn argon_all() -> Result<(Chart, Vec<SeriesConfig>)> {
    let Levels { levels, limits, .. } = nist_csv::levels(&data("ar_i_levels_all.csv")?)?;
    let mut series = EnergyLevelSeries::default();
    let mut columns: Vec<String> = Vec::new();
    for row in &levels {
        let core = if row.configuration.contains("<1/2>") {
            "'"
        } else {
            ""
        };
        let column = if row.configuration == "3s2.3p6" {
            "ground".to_string()
        } else {
            format!("{}{core}", row.term)
        };
        if !columns.contains(&column) {
            columns.push(column.clone());
        }
        let label = RichText::plain(row.outer(), LABEL, 15.0, "");
        let mut l = level(row, &column, Some(label));
        l.id = format!("{} {}", row.id(), row.energy);
        series.levels.push(l);
    }
    for l in &limits {
        series
            .limits
            .push(limit(l, &format!("Ar II ²P°₃/₂  {:.3} eV", l.energy)));
    }
    // As a paper draws them: the Rydberg members from n = 8 up to the
    // limit, one band per column, and the low members labelled (#205).
    let ceiling = limits.first().map_or(f64::MAX, |l| l.energy);
    let principal = |outer: &str| -> Option<u32> {
        outer
            .chars()
            .take_while(char::is_ascii_digit)
            .collect::<String>()
            .parse()
            .ok()
    };
    for column in &columns {
        let (low, high) = levels
            .iter()
            .zip(&series.levels)
            .filter(|(row, l)| {
                &l.column == column
                    && row.energy < ceiling
                    && principal(row.outer()).is_some_and(|n| n >= 8)
            })
            .fold((f64::MAX, f64::MIN), |(a, b), (row, _)| {
                (a.min(row.energy), b.max(row.energy))
            });
        if low < high {
            band(&mut series, column, low, high, "n ≥ 8");
        }
    }
    columns.sort();
    let categories = columns
        .iter()
        .map(|id| Category::new(id.as_str(), id.as_str()))
        .collect();
    let mut c = grotrian("Ar I, all levels", "Energy (eV)", categories);
    // Twenty-five term columns: drawn wide, as a full term diagram is.
    c.config_mut().chart_area.0.width = 2000;
    c.config_mut().chart_area.0.height = 900;
    break_below(&mut c, -0.2, 16.3, 0.3, 11.3);
    let mut series = styled(series);
    // Narrow slots leave lanes for the labels of fourteen columns.
    series.style.level.slot_fraction = 0.45;
    Ok((c, vec![spec("ar-all", series)]))
}

// Cu I -----------------------------------------------------------------

pub fn copper() -> Result<(Chart, Vec<SeriesConfig>)> {
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
        let label = tex(&format!("$\\mathrm{{{shell}}}_{{{}}}$", row.j), 16.0, LABEL)?;
        series.levels.push(level(row, &row.term, Some(label)));
    }
    // The 3d⁹4s4p quartets as bands, one per term.
    for (column, name) in [("4P*", "⁴P°"), ("4D*", "⁴D°"), ("4F*", "⁴F°")] {
        band(
            &mut series,
            column,
            f64::MIN,
            f64::MAX,
            &format!("3d⁹4s4p {name}"),
        );
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
                    20.0,
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
    unit_note(&mut c, renderer::NoteAlign::BottomRight);
    Ok((c, vec![spec("cu", series)]))
}

// U I ------------------------------------------------------------------

pub fn uranium() -> Result<(Chart, Vec<SeriesConfig>)> {
    let Levels { levels, limits, .. } = nist_csv::levels(&data("u_i_levels.csv")?)?;
    // Unassigned levels above 15 000 cm⁻¹ are the even upper levels of the
    // Handbook table; below, a blank term is odd (`*`).
    let odd = |row: &LevelRow| row.odd() || (row.term.is_empty() && row.energy < 15_000.0);
    let column = |row: &LevelRow| format!("{}{}", if odd(row) { "o" } else { "e" }, row.j);
    // Unassigned levels share configuration, term and J: the host's ids
    // carry the energy too.
    let uid = |row: &LevelRow| format!("{} {}", row.id(), row.energy);
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
                16.0,
                LABEL,
            )
        });
        let label = label.transpose()?;
        let mut l = level(row, &column(row), label);
        l.id = uid(row);
        series.levels.push(l);
    }
    // The unassigned even J = 6 levels above 23 000 cm⁻¹ as one band.
    band(&mut series, "e6", 23_000.0, 27_000.0, "6 even levels");
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
                &uid(lower),
                &uid(upper),
                nm,
                None,
            ));
        }
    }
    transitions.retain(|t| {
        let nm = t.wavelength.unwrap_or(0.0);
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
    c.config_mut().bottom_x.title_option.text = RichText::plain("J", HEADER, 20.0, "");
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
    unit_note(&mut c, renderer::NoteAlign::BottomRight);
    Ok((c, vec![spec("u", series)]))
}
