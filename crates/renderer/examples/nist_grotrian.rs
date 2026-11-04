//! An energy-level (Grotrian) diagram of He I from NIST Atomic Spectra
//! Database exports (#203): the host parses the levels and lines CSV files,
//! fills a DataLess `EnergyLevels` series and draws it on a categorical
//! term axis, singlets beside triplets, with the ground state below a
//! broken energy axis and the continuum above the ionization limit.
//!
//! The parsing is the host's (figgy ships no NIST importer); this example
//! shows one way with the standard library only. It handles the exports'
//! `="…"` quoting, the `[ ]` marks around derived energies, J lists such as
//! `0,1,2`, intensity flags such as `1*` and the `Limit` row, and it computes
//! the g-weighted centroid of every resolved fine-structure multiplet (host
//! work under CLAUDE.md rule 1).
//!
//! Run from the workspace root, optionally with your own exports:
//! cargo run --locked -p figgy-renderer --example nist_grotrian -- target/nist_grotrian [levels.csv lines.csv]
//!
//! The bundled files are trimmed He I exports (n ≤ 6, S to F terms) from
//! <https://physics.nist.gov/PhysRefData/ASD/levels_form.html> and
//! <https://physics.nist.gov/PhysRefData/ASD/lines_form.html>.

#[allow(dead_code)]
#[path = "support/gallery.rs"]
mod gallery;
use gallery::*;
use renderer::config::{AxisBreak, AxisBreakStyle, AxisBreaks, BreakMark, Category};
use renderer::energy_levels::{
    EnergyLevel, EnergyLevelSeries, IonizationLimit, LevelRole, Transition,
};
use renderer::text::RichText;
use renderer::{Color, DataRenderType, Renderer, SeriesConfig};
use std::path::{Path, PathBuf};

/// One CSV record: the fields of one line, NIST's `="…"` quoting removed.
fn records(text: &str) -> Vec<Vec<String>> {
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let mut fields = Vec::new();
            let mut field = String::new();
            let mut quoted = false;
            let mut chars = line.chars().peekable();
            while let Some(c) = chars.next() {
                match c {
                    '"' if quoted && chars.peek() == Some(&'"') => {
                        field.push('"');
                        chars.next();
                    }
                    '"' => quoted = !quoted,
                    ',' if !quoted => fields.push(std::mem::take(&mut field)),
                    _ => field.push(c),
                }
            }
            fields.push(field);
            fields
                .into_iter()
                .map(|f| {
                    let f = f.trim();
                    f.strip_prefix("=\"")
                        .and_then(|f| f.strip_suffix('"'))
                        .unwrap_or(f)
                        .to_string()
                })
                .collect()
        })
        .collect()
}

/// A number from a NIST field: `[ ]`, `( )`, `?` and `+x` marks removed.
fn number(field: &str) -> Option<f64> {
    field
        .trim_matches(|c| matches!(c, '[' | ']' | '(' | ')' | ' '))
        .trim_end_matches('?')
        .trim_end_matches("+x")
        .parse()
        .ok()
}

/// The column of a field by its header name.
fn column(header: &[String], name: &str) -> Result<usize> {
    header
        .iter()
        .position(|h| h == name)
        .ok_or_else(|| format!("no `{name}` column in the export").into())
}

/// The term column a level is drawn in (`1S`, `3P*`), or `None` for a term
/// this diagram does not lay out.
fn term_column(term: &str) -> Option<&str> {
    let mut chars = term.chars();
    let multiplicity = chars.next()?;
    let l = chars.next()?;
    let rest: String = chars.collect();
    (matches!(multiplicity, '1' | '3') && "SPDF".contains(l) && (rest.is_empty() || rest == "*"))
        .then_some(term)
}

/// The energy-level series of a levels export and a lines export.
fn parse(levels_csv: &str, lines_csv: &str) -> Result<EnergyLevelSeries> {
    let rows = records(levels_csv);
    let (header, rows) = rows.split_first().ok_or("an empty levels export")?;
    let [conf, term, j, g, prefix, energy, suffix, unc, reference] = [
        "Configuration",
        "Term",
        "J",
        "g",
        "Prefix",
        "Level (eV)",
        "Suffix",
        "Uncertainty (eV)",
        "Reference",
    ]
    .map(|name| column(header, name));
    let (conf, term, j, g, prefix, energy, suffix, unc, reference) = (
        conf?, term?, j?, g?, prefix?, energy?, suffix?, unc?, reference?,
    );
    let mut out = EnergyLevelSeries::default();
    let field = |row: &[String], i: usize| row.get(i).cloned().unwrap_or_default();
    for row in rows {
        let Some(e) = number(&field(row, energy)) else {
            continue;
        };
        if field(row, term) == "Limit" {
            out.limits.push(IonizationLimit {
                id: field(row, conf),
                energy: e,
                label: Some(RichText::plain(
                    &format!("{}  {e:.3} eV", field(row, conf)),
                    Color::from_rgb8(40, 50, 120),
                    13.0,
                    "",
                )),
                label_offset: [0.0; 2],
                uncertainty: number(&field(row, unc)),
                reference: field(row, reference),
            });
            continue;
        }
        let t = field(row, term);
        let Some(column) = term_column(&t) else {
            continue;
        };
        let configuration = field(row, conf);
        out.levels.push(EnergyLevel {
            id: format!("{configuration} {t} {}", field(row, j)),
            column: column.to_string(),
            configuration,
            term: t.clone(),
            j: field(row, j),
            g: number(&field(row, g)),
            prefix: field(row, prefix),
            energy: e,
            suffix: field(row, suffix),
            uncertainty: number(&field(row, unc)),
            reference: field(row, reference),
            role: LevelRole::Level,
            label: None,
            label_offset: [0.0; 2],
        });
    }
    // Resolved multiplets: every J component keeps its row, and the host
    // adds the g-weighted centroid the main view draws.
    let mut centroids = Vec::new();
    for (i, level) in out.levels.iter().enumerate() {
        let key = (&level.configuration, &level.term);
        if centroids
            .iter()
            .any(|c: &EnergyLevel| (&c.configuration, &c.term) == key)
        {
            continue;
        }
        let members: Vec<&EnergyLevel> = out.levels[i..]
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
        centroids.push(EnergyLevel {
            id: format!("{} {}", level.configuration, level.term),
            j: String::new(),
            g: Some(weight),
            energy,
            uncertainty: None,
            role: LevelRole::Centroid,
            ..level.clone()
        });
    }
    for level in &mut out.levels {
        let id = format!("{} {}", level.configuration, level.term);
        if centroids.iter().any(|c| c.id == id) {
            level.role = LevelRole::Component { multiplet: id };
        }
    }
    out.levels.extend(centroids);
    // Labels: the outer electron of the levels up to n = 4.
    for level in &mut out.levels {
        if level.role == LevelRole::Level || level.role == LevelRole::Centroid {
            let outer = level
                .configuration
                .rsplit('.')
                .next()
                .unwrap_or(&level.configuration);
            let n: u32 = outer
                .chars()
                .take_while(char::is_ascii_digit)
                .collect::<String>()
                .parse()
                .unwrap_or(1);
            if n <= 4 {
                level.label = Some(RichText::plain(
                    outer,
                    Color::from_rgb8(70, 76, 90),
                    12.0,
                    "",
                ));
            }
        }
    }

    // Lines: the strongest visible line between each pair of drawn levels
    // of one multiplicity (no intercombination lines), each end taken to its
    // multiplet's centroid when it has one, and the twelve with the largest
    // Aki of those: the host chooses what is drawn.
    let rows = records(lines_csv);
    let (header, rows) = rows.split_first().ok_or("an empty lines export")?;
    let col = |name| column(header, name);
    let (obs, ritz, intens, aki, acc, conf_i, term_i, j_i, conf_k, term_k, j_k) = (
        col("obs_wl_air(nm)")?,
        col("ritz_wl_air(nm)")?,
        col("intens")?,
        col("Aki(s^-1)")?,
        col("Acc")?,
        col("conf_i")?,
        col("term_i")?,
        col("J_i")?,
        col("conf_k")?,
        col("term_k")?,
        col("J_k")?,
    );
    let level_id = |c: String, t: String, j: String| -> Option<String> {
        let multiplet = format!("{c} {t}");
        if out.levels.iter().any(|l| l.id == multiplet) {
            return Some(multiplet);
        }
        let id = format!("{c} {t} {j}");
        out.levels.iter().any(|l| l.id == id).then_some(id)
    };
    let mut transitions: Vec<Transition> = Vec::new();
    for row in rows {
        let wavelength = number(&field(row, obs)).or_else(|| number(&field(row, ritz)));
        let Some(wl) = wavelength.filter(|wl| (380.0..=720.0).contains(wl)) else {
            continue;
        };
        if field(row, term_i).chars().next() != field(row, term_k).chars().next() {
            continue;
        }
        let (Some(lower), Some(upper)) = (
            level_id(field(row, conf_i), field(row, term_i), field(row, j_i)),
            level_id(field(row, conf_k), field(row, term_k), field(row, j_k)),
        ) else {
            continue;
        };
        let a = number(&field(row, aki));
        if let Some(old) = transitions
            .iter_mut()
            .find(|t| t.lower == lower && t.upper == upper)
        {
            if a > old.aki {
                old.aki = a;
            }
            continue;
        }
        transitions.push(Transition {
            id: format!("{wl:.2}"),
            lower,
            upper,
            label: Some(RichText::plain(
                &format!("{wl:.1}"),
                Color::from_rgb8(150, 90, 0),
                11.0,
                "",
            )),
            label_offset: [0.0; 2],
            value: a,
            x_offset: 0.0,
            observed_wavelength: number(&field(row, obs)),
            observed_uncertainty: None,
            ritz_wavelength: number(&field(row, ritz)),
            ritz_uncertainty: None,
            intensity: field(row, intens),
            aki: a,
            accuracy: field(row, acc),
            line_type: String::new(),
            tp_reference: String::new(),
            line_reference: String::new(),
        });
    }
    transitions.sort_by(|a, b| {
        b.aki
            .partial_cmp(&a.aki)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    transitions.truncate(12);
    // Spread the arrows that leave one level so their heads do not meet.
    for i in 0..transitions.len() {
        let siblings = transitions[..i]
            .iter()
            .filter(|t| t.lower == transitions[i].lower)
            .count();
        transitions[i].x_offset = (siblings as f32 * 0.12).min(0.3) - 0.1;
    }
    out.transitions = transitions;
    Ok(out)
}

pub fn render(r: &mut Renderer, out: &Path, levels: &Path, lines: &Path) -> Result {
    let mut series = parse(
        &std::fs::read_to_string(levels)?,
        &std::fs::read_to_string(lines)?,
    )?;
    series.style.level.color = Color::from_rgb8(34, 52, 88);
    series.style.level.width = 2.5;
    series.style.transition.color = Color::from_rgb8(214, 120, 20);
    series.style.transition.width = 1.6;

    let mut c = chart("He I energy levels", "", "Energy (eV)", (1100, 780));
    // Singlets, a spacer, then triplets, each term label drawn as a term
    // symbol, under two group headers.
    let tex = |term: &str| {
        let (multiplicity, rest) = term.split_at(1);
        let (l, odd) = rest.split_at(1);
        let parity = if odd == "*" { "^o" } else { "" };
        RichText::from_tex(
            &format!("$^{multiplicity}\\mathrm{{{l}}}{parity}$"),
            Color::from_rgb8(30, 30, 30),
            16.0,
            "",
        )
    };
    let mut categories = Vec::new();
    for term in ["1S", "1P*", "1D", "1F*", "gap", "3S", "3P*", "3D", "3F*"] {
        let mut category = Category::new(term, if term == "gap" { "" } else { term });
        if term != "gap" {
            category.rich_label = Some(tex(term)?);
        }
        categories.push(category);
    }
    let header = |text: &str| RichText::plain(text, Color::from_rgb8(24, 39, 63), 16.0, "");
    c.config_mut().bottom_x.category_groups = Some(renderer::config::CategoryGroups {
        groups: vec![
            renderer::config::CategoryGroup {
                label: header("Singlets (S = 0)"),
                first: "1S".into(),
                last: "1F*".into(),
                bracket: true,
            },
            renderer::config::CategoryGroup {
                label: header("Triplets (S = 1)"),
                first: "3S".into(),
                last: "3F*".into(),
                bracket: true,
            },
        ],
        offset: [0.0; 2],
    });
    c.config_mut().bottom_x.out_margin = 84.0;
    c.config_mut().bottom_x.categories = Some(categories);
    // A Grotrian diagram reads by its levels: no grid, no mirrored ticks.
    let config = c.config_mut();
    config.grid.show_major_x = false;
    config.grid.show_major_y = false;
    config.grid.show_minor_y = false;
    config.top_x.tick = renderer::config::TickVisibility::None;
    config.right_y.tick = renderer::config::TickVisibility::None;
    // The ground state keeps a thin strip below the break.
    c.set_y_range(-0.15, 25.2);
    c.config_mut().left_y.minor_count = 1;
    c.config_mut().left_y.breaks = Some(AxisBreaks {
        ranges: vec![AxisBreak::new(0.25, 19.6)],
        style: AxisBreakStyle::Continuous,
        gap: 0.04,
        mark: BreakMark::default(),
        frame_each_panel: false,
    });
    // The continuum: annotations written from the limits.
    let top = 25.2;
    series.write_continuum(
        &mut c.config_mut().annotations,
        "he",
        top,
        Color::from_rgba8(80, 100, 220, 36),
    )?;
    let spec = SeriesConfig {
        series_id: "he".into(),
        source_id: None,
        label: None,
        x_column: String::new(),
        y_column: String::new(),
        z_column: None,
        render_type: DataRenderType::EnergyLevels(series),
    };
    save(r, &c, &[spec], &out.join("nist-grotrian-he-i.png"))
}

fn main() -> Result {
    let out = output_directory("target/nist_grotrian")?;
    let mut args = std::env::args().skip(2);
    let data = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/data/nist");
    let levels = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| data.join("he_i_levels.csv"));
    let lines = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| data.join("he_i_lines.csv"));
    with_renderer(|r| render(r, &out, &levels, &lines))
}
