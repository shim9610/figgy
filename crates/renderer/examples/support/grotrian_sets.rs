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
//! draws them. The host also writes them in physics notation (#357): a
//! term symbol such as NIST's `1P*` as `$^{1}$P°`, a configuration such as
//! `1s2` as `1s$^{2}$`, TeX script spans drawn as script segments (no
//! Unicode script characters), and it finds where its levels leave the
//! energy axis empty and
//! writes that range as the axis break ([`derived_break`]). Every colour,
//! width and text size is the gallery family's energy-level member's
//! (#295): a builder gives each text its content and applies the theme
//! last. The including module declares `gallery` (for
//! `titled` and `Result`) and `nist_csv` beside this one.

pub use super::gallery::Result;
use super::gallery::titled;
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
use renderer::theme::{ChartKind, Theme, ThemeFamily};
use renderer::{Chart, Color, DataRenderType, SeriesConfig};

/// The diagrams' look: the gallery family's energy-level member.
fn theme() -> Result<Theme> {
    Ok(ThemeFamily::Gallery.theme(ChartKind::EnergyLevels))
}

/// A text of the diagram: its content, in the look the theme gives it.
/// `$…$` spans are TeX, so a script is written `$^{2}$` or `$_{3/2}$` and
/// drawn as a script segment: the examples write no Unicode script
/// characters (#357), which figgy maps only for hosts that send them.
fn plain(text: &str) -> RichText {
    if text.contains('$') {
        RichText::from_label(text, Color::BLACK, 16.0, "")
    } else {
        RichText::plain(text, Color::BLACK, 16.0, "")
    }
}

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

/// `digits` raised: a TeX superscript span ([`plain`] draws it as a
/// superscript segment).
fn superscript(digits: &str) -> String {
    if digits.is_empty() {
        return String::new();
    }
    format!("$^{{{digits}}}$")
}

/// `digits` lowered: a TeX subscript span, so a J value such as `3/2`
/// reads as a subscript 3/2.
pub fn subscript(digits: &str) -> String {
    if digits.is_empty() {
        return String::new();
    }
    format!("$_{{{digits}}}$")
}

/// A NIST term in physics notation: the leading multiplicity raised and
/// the odd-parity `*` written as °, so `1P*` reads as P° with a raised 1
/// and the jK term `2[3/2]*` as [3/2]° with a raised 2. The host writes
/// the notation as TeX script spans; the renderer draws the segments.
pub fn term_symbol(term: &str) -> String {
    let digits = term.chars().take_while(char::is_ascii_digit).count();
    let (multiplicity, rest) = term.split_at(digits);
    let (rest, odd) = match rest.strip_suffix('*') {
        Some(rest) => (rest, "°"),
        None => (rest, ""),
    };
    format!("{}{rest}{odd}", superscript(multiplicity))
}

/// A NIST configuration in physics notation: each subshell's occupancy
/// raised, so `1s2` reads as 1s with a raised 2 and `3d10.4s` as 3d with
/// a raised 10, then 4s. A subshell
/// is its principal number, its letter and its occupancy; parenthesized
/// parent terms are left as written.
pub fn configuration(text: &str) -> String {
    let mut out = String::new();
    for part in text.split('.') {
        let n = part.chars().take_while(char::is_ascii_digit).count();
        let letters = part[n..]
            .chars()
            .take_while(char::is_ascii_lowercase)
            .count();
        let occupancy = &part[n + letters..];
        if n > 0 && letters == 1 && occupancy.chars().all(|c| c.is_ascii_digit()) {
            out.push_str(&part[..n + letters]);
            out.push_str(&superscript(occupancy));
        } else {
            out.push_str(part);
        }
    }
    out
}

/// The axis break a host derives from its own levels (#357 item 13): the
/// widest empty range between the drawn levels and limits in `min..=max`,
/// cut out when it spans more than 30 % of the range, kept 1.5 % of the
/// range clear of the strokes on each side. The piece with fewer levels
/// is drawn thin (`1 : 9`), whichever side of the break it lies on. The
/// renderer computes no break: the host writes this into the axis.
pub fn derived_break(series: &EnergyLevelSeries, min: f64, max: f64) -> Option<AxisBreaks> {
    let mut energies: Vec<f64> = (series.levels.iter().map(|l| l.energy))
        .chain(series.limits.iter().map(|l| l.energy))
        .filter(|e| (min..=max).contains(e))
        .collect();
    energies.sort_by(f64::total_cmp);
    let (low, high) = energies
        .windows(2)
        .map(|pair| (pair[0], pair[1]))
        .max_by(|a, b| (a.1 - a.0).total_cmp(&(b.1 - b.0)))?;
    let span = max - min;
    if high - low <= 0.3 * span {
        return None;
    }
    let pad = 0.015 * span;
    let below = energies.iter().filter(|e| **e <= low).count();
    let above = energies.len() - below;
    let weights = if below <= above {
        vec![1.0, 9.0]
    } else {
        vec![9.0, 1.0]
    };
    Some(AxisBreaks {
        ranges: vec![AxisBreak::new(low + pad, high - pad)],
        style: AxisBreakStyle::Continuous,
        gap_px: 12.0,
        mark: BreakMark::default(),
        // The Grotrian frame is open: the slashes on the energy axis mark
        // the gap, with no panel lines across the columns.
        frame_each_panel: false,
        sizing: renderer::config::PieceSizing::Weights(weights),
    })
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
        label: Some(plain(text)),
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
        label: Some(plain(label)),
        label_offset: [0.0; 2],
    });
}

/// A term column: its NIST id (`1P*`), named by its term symbol as the
/// rich label the theme styles as a header, scripts as segments (#357),
/// and by the symbol's characters as its plain label. The `gap` spacer
/// has no name.
fn column(id: &str) -> Category {
    if id == "gap" {
        return Category::new(id, "");
    }
    named(id, &term_symbol(id))
}

/// Column `id` named `text` (TeX script spans as segments): the rich label
/// the theme styles as a header, and the segments' characters as the
/// plain label.
fn named(id: &str, text: &str) -> Category {
    let rich = plain(text);
    let name: String = rich.segments.iter().map(|s| s.text).collect();
    let mut category = Category::new(id, name.as_str());
    category.rich_label = Some(rich);
    category
}

/// A Grotrian chart with its term columns named. Each diagram derives its
/// own break from its levels ([`energy_axis`]); the theme gives it the
/// Grotrian frame (#205, #357).
fn grotrian(title: &str, y_title: &str, categories: Vec<Category>) -> Chart {
    let mut c = titled(title, "", y_title, (1100, 780));
    c.config_mut().bottom_x.categories = Some(categories);
    c
}

/// The chart and its series in `theme`: the last step of every builder.
fn themed(
    mut c: Chart,
    mut series: Vec<SeriesConfig>,
    theme: &Theme,
) -> Result<(Chart, Vec<SeriesConfig>)> {
    c.apply_custom_theme(theme, &mut series)?;
    Ok((c, series))
}

fn groups(c: &mut Chart, groups: &[(&str, &str, &str)]) {
    c.config_mut().bottom_x.category_groups = Some(CategoryGroups {
        groups: groups
            .iter()
            .map(|(label, first, last)| CategoryGroup {
                label: plain(label),
                first: (*first).into(),
                last: (*last).into(),
                bracket: true,
            })
            .collect(),
        offset: [0.0; 2],
    });
}

/// The energy range `min..=max` and the break the host derives from the
/// series' own levels ([`derived_break`]), if they leave a wide gap.
fn energy_axis(c: &mut Chart, series: &EnergyLevelSeries, min: f64, max: f64) {
    c.set_y_range(min, max);
    c.config_mut().left_y.minor_count = 1;
    c.config_mut().left_y.breaks = derived_break(series, min, max);
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

/// The lines labelled with their wavelengths in nm to one decimal, written
/// by the renderer from each transition's `wavelength`; the unit is shown
/// once, in a corner note (#258).
fn labelled(mut series: EnergyLevelSeries) -> EnergyLevelSeries {
    series.style.wavelength_labels = Some(WavelengthLabels {
        format: PlainNumberFormat {
            decimals: Some(1),
            trim_zeros: false,
        },
        unit: None,
        text: plain(""),
    });
    series
}

/// The continuum above the series' limits up to `top`: annotations, which
/// the theme fills.
fn continuum(c: &mut Chart, series: &EnergyLevelSeries, id: &str, top: f64) -> Result {
    let fill = Color::from_rgba8(128, 128, 128, 36);
    series.write_continuum(&mut c.config_mut().annotations, id, top, fill)?;
    Ok(())
}

/// The wavelengths' unit at the plot's `corner`, ten pixels in from it, in
/// the look the theme gives the wavelength labels: a text note that owns
/// its text (#258).
fn unit_note(c: &mut Chart, corner: renderer::NoteAlign, theme: &Theme) {
    let plot = c.config().data_area().map(|area| area.0);
    let config = c.config_mut();
    let mut text = plain("Wavelengths in nm");
    if let Some(look) = theme.series.energy_levels.transition_labels {
        (text.color, text.font_size) = (look.color, look.size);
    }
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
        let label = (n <= 4 && shown).then(|| plain(&configuration(outer)));
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
    let categories = columns.iter().map(|id| column(id)).collect();
    let mut c = grotrian("He I", "Energy (eV)", categories);
    groups(
        &mut c,
        &[
            ("Singlets (S = 0)", "1S", "1F*"),
            ("Triplets (S = 1)", "3S", "3F*"),
        ],
    );
    energy_axis(&mut c, &series, -0.15, 25.2);
    let series = labelled(series);
    let theme = theme()?;
    continuum(&mut c, &series, "he", 25.2)?;
    unit_note(&mut c, renderer::NoteAlign::BottomRight, &theme);
    themed(c, vec![spec("he", series)], &theme)
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
            Some(p) => p,
            None if col == "ground" => configuration("3p6"),
            None => {
                let k = row.term.trim_start_matches('2').trim_end_matches('*');
                format!("{k}{}", subscript(&row.j))
            }
        };
        let label = Some(plain(&label));
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
        series.limits.push(limit(
            l,
            &format!("Ar II $^{{2}}$P°$_{{3/2}}$  {:.3} eV", l.energy),
        ));
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
    // The columns named by their outer subshell, primed for the ²P°₁/₂
    // core, and the ground state by its configuration.
    let categories = order
        .iter()
        .map(|id| {
            let name = match *id {
                "gap" => return Category::new(*id, ""),
                "ground" => configuration("3p6"),
                id => id.replace('\'', "′"),
            };
            named(id, &name)
        })
        .collect();
    let mut c = grotrian("Ar I", "Energy (eV)", categories);
    groups(
        &mut c,
        &[
            ("3p$^{5}$ $^{2}$P°$_{3/2}$ core", "4s", "5p"),
            ("3p$^{5}$ $^{2}$P°$_{1/2}$ core", "4s'", "5p'"),
        ],
    );
    energy_axis(&mut c, &series, -0.2, 16.0);
    let series = labelled(series);
    let theme = theme()?;
    continuum(&mut c, &series, "ar", 16.0)?;
    unit_note(&mut c, renderer::NoteAlign::BottomRight, &theme);
    themed(c, vec![spec("ar", series)], &theme)
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
        let mut l = level(row, &column, Some(plain(&configuration(row.outer()))));
        l.id = format!("{} {}", row.id(), row.energy);
        series.levels.push(l);
    }
    for l in &limits {
        series.limits.push(limit(
            l,
            &format!("Ar II $^{{2}}$P°$_{{3/2}}$  {:.3} eV", l.energy),
        ));
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
    // Each column named by its jK term symbol, primed for the ²P°₁/₂ core,
    // its multiplicity raised as a script segment. The twenty-five columns
    // are named at the energy axis' 20 px tick size, a size each segment
    // states, rather than at the column headings' size.
    let categories = columns
        .iter()
        .map(|id| {
            let label = match id.strip_suffix('\'') {
                _ if id == "ground" => configuration("3p6"),
                Some(term) => format!("{}′", term_symbol(term)),
                None => term_symbol(id),
            };
            let mut column = named(id, &label);
            if let Some(rich) = column.rich_label.as_mut() {
                for segment in &mut rich.segments {
                    segment.font_size = Some(20.0);
                }
            }
            column
        })
        .collect();
    let mut c = grotrian("Ar I, all levels", "Energy (eV)", categories);
    // Twenty-five term columns: drawn wide, as a full term diagram is.
    c.config_mut().chart_area.0.width = 2000;
    c.config_mut().chart_area.0.height = 900;
    energy_axis(&mut c, &series, -0.2, 16.3);
    let mut series = labelled(series);
    // Narrow slots leave lanes for the labels of fourteen columns.
    series.style.level.slot_fraction = 0.45;
    // Five hundred labels: a size smaller than the theme's.
    let mut theme = theme()?;
    if let Some(look) = &mut theme.series.energy_levels.level_labels {
        look.size = 16.0;
    }
    themed(c, vec![spec("ar-all", series)], &theme)
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
        // The outer subshells in physics notation, J as a subscript.
        let shell = configuration(
            &row.configuration
                .trim_start_matches("3d10.")
                .trim_start_matches("3d9.(2D).")
                .trim_start_matches("3d9.")
                .replace(".(3P*)", "")
                .replace(".(1P*)", ""),
        );
        let label = plain(&format!("{shell}{}", subscript(&row.j)));
        series.levels.push(level(row, &row.term, Some(label)));
    }
    // The 3d⁹4s4p quartets as bands, one per term.
    for (column, name) in [
        ("4P*", "$^{4}$P°"),
        ("4D*", "$^{4}$D°"),
        ("4F*", "$^{4}$F°"),
    ] {
        band(
            &mut series,
            column,
            f64::MIN,
            f64::MAX,
            &format!("3d$^{{9}}$4s4p {name}"),
        );
    }
    for l in &limits {
        series.limits.push(limit(
            l,
            &format!("Cu II 3d$^{{10}}$ $^{{1}}$S  {:.3} eV", l.energy),
        ));
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
    let categories = order.iter().map(|id| column(id)).collect();
    let mut c = grotrian("Cu I", "Energy (eV)", categories);
    groups(
        &mut c,
        &[
            ("Doublets", "2S", "2F*"),
            ("Quartets (3d$^{9}$4s4p)", "4P*", "4F*"),
        ],
    );
    // The levels leave no gap wide enough to break: `derived_break` finds
    // none, and the axis stays whole.
    energy_axis(&mut c, &series, -0.2, 8.2);
    let series = labelled(series);
    let theme = theme()?;
    continuum(&mut c, &series, "cu", 8.2)?;
    unit_note(&mut c, renderer::NoteAlign::BottomRight, &theme);
    themed(c, vec![spec("cu", series)], &theme)
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
        let label = (!row.term.is_empty() && row.term != "*")
            .then(|| plain(&format!("{}{}", term_symbol(&row.term), subscript(&row.j))));
        let mut l = level(row, &column(row), label);
        l.id = uid(row);
        series.levels.push(l);
    }
    // The unassigned even J = 6 levels above 23 000 cm⁻¹ as one band.
    band(&mut series, "e6", 23_000.0, 27_000.0, "6 even levels");
    for l in &limits {
        series.limits.push(limit(
            l,
            &format!("U II $^{{4}}$I°$_{{9/2}}$  {:.1} cm$^{{-1}}$", l.energy),
        ));
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
    let mut c = grotrian("U I", "Energy (cm$^{-1}$)", categories);
    c.config_mut().bottom_x.title_option.visible = true;
    c.config_mut().bottom_x.title_option.text = plain("J");
    groups(
        &mut c,
        &[
            ("Odd parity", &first_odd, &last_odd),
            ("Even parity", &first_even, &last_even),
        ],
    );
    // The limit sits far above the levels: the derived break leaves it a
    // thin piece above the gap.
    energy_axis(&mut c, &series, -500.0, 51_500.0);
    let series = labelled(series);
    let theme = theme()?;
    continuum(&mut c, &series, "u", 52_000.0)?;
    unit_note(&mut c, renderer::NoteAlign::BottomRight, &theme);
    themed(c, vec![spec("u", series)], &theme)
}
