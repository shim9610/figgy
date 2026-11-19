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
//! work under CLAUDE.md rule 1). The host also writes the notation (#357):
//! NIST's `1P*` as the term symbol ¹P°, the configuration `1s2` as 1s²,
//! and it derives the energy axis' break from its own levels. The chart
//! starts in the Grotrian frame (`default::config_for_series`) without a
//! theme.
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
#[allow(dead_code)]
#[path = "support/grotrian_sets.rs"]
mod grotrian_sets;
#[allow(dead_code)]
#[path = "support/nist_csv.rs"]
mod nist_csv;
use gallery::*;
use grotrian_sets::{configuration, derived_break, subscript, term_symbol};
use renderer::config::Category;
use renderer::energy_levels::{
    EnergyLevel, EnergyLevelSeries, IonizationLimit, LevelRole, Transition,
};
use renderer::text::RichText;
use renderer::{Chart, Color, Renderer, SeriesConfig, default};
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

/// A limit row's name in physics notation: NIST's `He II (1s 2S<1/2>)`
/// as `He II 1s ²S₁/₂`, the ion's core configuration and term. A name of
/// another form is kept as written.
fn limit_name(name: &str) -> String {
    let Some((ion, core)) = name.split_once(" (") else {
        return name.to_string();
    };
    let core = core.trim_end_matches(')');
    let Some((config, term)) = core.rsplit_once(' ') else {
        return name.to_string();
    };
    let (term, j) = term
        .strip_suffix('>')
        .and_then(|t| t.split_once('<'))
        .unwrap_or((term, ""));
    format!(
        "{ion} {} {}{}",
        configuration(config),
        term_symbol(term),
        subscript(j)
    )
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
                    &format!("{}  {e:.3} eV", limit_name(&field(row, conf))),
                    Color::from_rgb8(40, 50, 120),
                    19.0,
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
    // Labels: the outer subshell of the levels up to n = 4, its occupancy
    // raised (1s²).
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
                    &configuration(outer),
                    Color::from_rgb8(70, 76, 90),
                    20.0,
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
            // The renderer writes the label from the wavelength (#258).
            label: None,
            label_offset: [0.0; 2],
            value: a,
            x_offset: 0.0,
            // The host chooses the column: the observed wavelength, else
            // the Ritz one.
            wavelength: Some(wl),
            wavelength_uncertainty: None,
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
    // The strokes keep the series' default widths (3 px levels, 2.25 px
    // arrows with 13 × 10 px heads, #357); the host picks the colours.
    series.style.level.color = Color::from_rgb8(34, 52, 88);
    series.style.transition.color = Color::from_rgb8(214, 120, 20);
    // Each line labelled with its wavelength to one decimal; the unit is
    // shown once, in a note in the plot's corner (#258).
    series.style.wavelength_labels = Some(renderer::energy_levels::WavelengthLabels {
        format: renderer::format::PlainNumberFormat {
            decimals: Some(1),
            trim_zeros: false,
        },
        unit: None,
        text: RichText::plain("", Color::from_rgb8(150, 90, 0), 18.0, ""),
    });

    let spec = SeriesConfig::energy_levels("he", series);
    let renderer::DataRenderType::EnergyLevels(series) = &spec.render_type else {
        return Err("an energy-level series".into());
    };
    // The default config in the frame of an energy-level chart (#357): the
    // Grotrian frame, the energy axis alone with the term columns named
    // without a value frame, no grid and no mirrored ticks.
    let mut c = Chart::new(default::config_for_series(std::slice::from_ref(&spec)))
        .with_area(0, 0, 1100, 780)
        .with_title("He I energy levels")
        .with_y_title("Energy (eV)");
    // Singlets, a spacer, then triplets, each column named by its term
    // symbol (¹P°), the diagram's column headings, under two group headers.
    let mut categories = Vec::new();
    for term in ["1S", "1P*", "1D", "1F*", "gap", "3S", "3P*", "3D", "3F*"] {
        if term == "gap" {
            categories.push(Category::new(term, ""));
            continue;
        }
        let symbol = term_symbol(term);
        let mut category = Category::new(term, symbol.as_str());
        category.rich_label = Some(RichText::plain(
            &symbol,
            Color::from_rgb8(30, 30, 30),
            24.0,
            "",
        ));
        categories.push(category);
    }
    // The headers name the total spin S in TeX.
    let header = |text: &str| RichText::from_tex(text, Color::from_rgb8(24, 39, 63), 21.0, "");
    c.config_mut().bottom_x.category_groups = Some(renderer::config::CategoryGroups {
        groups: vec![
            renderer::config::CategoryGroup {
                label: header(r"Singlets ($S = 0$)")?,
                first: "1S".into(),
                last: "1F*".into(),
                bracket: true,
            },
            renderer::config::CategoryGroup {
                label: header(r"Triplets ($S = 1$)")?,
                first: "3S".into(),
                last: "3F*".into(),
                bracket: true,
            },
        ],
        offset: [0.0; 2],
    });
    c.config_mut().bottom_x.categories = Some(categories);
    // Text read beside the levels: energy ticks at 20 px, the title at 22.
    let config = c.config_mut();
    config.left_y.label_style.font_size = 20.0;
    config.left_y.title_option.text.font_size = 22.0;
    // The host finds where its levels leave the axis empty, the range
    // between the ground state and the first excited level, and writes it
    // as the break; the ground state keeps a thin strip below it. The
    // renderer computes no break (#357 item 13).
    c.set_y_range(-0.15, 25.2);
    c.config_mut().left_y.minor_count = 1;
    c.config_mut().left_y.breaks = derived_break(series, -0.15, 25.2);
    // The continuum: annotations written from the limits.
    let top = 25.2;
    series.write_continuum(
        &mut c.config_mut().annotations,
        "he",
        top,
        Color::from_rgba8(80, 100, 220, 36),
    )?;
    let note = c.config_mut().annotations.add_corner_note(
        RichText::plain("Wavelengths in nm", Color::from_rgb8(150, 90, 0), 18.0, ""),
        renderer::NoteAlign::BottomRight,
    );
    let plot = c.config().data_area().map(|area| area.0);
    if let renderer::AnnotationKind::Text { anchor, .. } =
        &mut c.config_mut().annotations.items[note].kind
    {
        // Off the plot's frame: the corner's ratio moved in by 10 px.
        if let (renderer::Anchor::Ratio { x, y }, Ok(plot)) = (anchor, plot) {
            *x -= 10.0 / plot.width.max(1) as f32;
            *y -= 10.0 / plot.height.max(1) as f32;
        }
    }
    save(r, &mut c, &[spec], &out.join("nist-grotrian-he-i.png"))
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
