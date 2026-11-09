//! Host-side reading of NIST Atomic Spectra Database CSV exports (#203,
//! #205): figgy ships no importer, so the examples parse the exports with
//! the standard library. Handles the `="…"` quoting, `[ ]` and `( )` marks
//! around derived values, `?` and `+x` suffixes, and the `Limit` rows. A
//! levels export may be in eV or in cm⁻¹; the unit is read from the header.

use std::error::Error;

pub type Result<T = ()> = std::result::Result<T, Box<dyn Error>>;

/// One CSV record per line, NIST's `="…"` quoting removed.
pub fn records(text: &str) -> Vec<Vec<String>> {
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
pub fn number(field: &str) -> Option<f64> {
    field
        .trim_matches(|c| matches!(c, '[' | ']' | '(' | ')' | ' '))
        .trim_end_matches('?')
        .trim_end_matches("+x")
        .parse()
        .ok()
}

fn column(header: &[String], names: &[&str]) -> Result<usize> {
    header
        .iter()
        .position(|h| names.contains(&h.as_str()))
        .ok_or_else(|| format!("no `{}` column in the export", names[0]).into())
}

/// The energy unit of a levels export.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unit {
    Ev,
    Wavenumber,
}

/// One row of a levels export.
#[derive(Clone, Debug)]
pub struct LevelRow {
    pub configuration: String,
    pub term: String,
    pub j: String,
    pub g: Option<f64>,
    pub energy: f64,
    pub uncertainty: Option<f64>,
    pub reference: String,
}

impl LevelRow {
    /// The id the lines export names the level by.
    pub fn id(&self) -> String {
        format!("{} {} {}", self.configuration, self.term, self.j)
    }
    /// The outer electron of the configuration (`4p`, `7s2`).
    pub fn outer(&self) -> &str {
        self.configuration
            .rsplit('.')
            .next()
            .unwrap_or(&self.configuration)
    }
    /// Whether the term is odd (`*`, NIST's °).
    pub fn odd(&self) -> bool {
        self.term.ends_with('*')
    }
}

/// The levels, limits and unit of a levels export.
pub struct Levels {
    pub unit: Unit,
    pub levels: Vec<LevelRow>,
    pub limits: Vec<LevelRow>,
}

pub fn levels(text: &str) -> Result<Levels> {
    let rows = records(text);
    let (header, rows) = rows.split_first().ok_or("an empty levels export")?;
    let unit = if header.iter().any(|h| h == "Level (cm-1)") {
        Unit::Wavenumber
    } else {
        Unit::Ev
    };
    let conf = column(header, &["Configuration"])?;
    let term = column(header, &["Term"])?;
    let j = column(header, &["J"])?;
    let g = column(header, &["g"])?;
    let energy = column(header, &["Level (eV)", "Level (cm-1)"])?;
    let unc = column(header, &["Uncertainty (eV)", "Uncertainty (cm-1)"])?;
    let reference = column(header, &["Reference"])?;
    let field = |row: &[String], i: usize| row.get(i).cloned().unwrap_or_default();
    let mut out = Levels {
        unit,
        levels: Vec::new(),
        limits: Vec::new(),
    };
    for row in rows {
        let Some(e) = number(&field(row, energy)) else {
            continue;
        };
        let level = LevelRow {
            configuration: field(row, conf),
            term: field(row, term),
            j: field(row, j),
            g: number(&field(row, g)),
            energy: e,
            uncertainty: number(&field(row, unc)),
            reference: field(row, reference),
        };
        if level.term == "Limit" {
            out.limits.push(level);
        } else {
            out.levels.push(level);
        }
    }
    Ok(out)
}

/// One row of a lines export.
#[derive(Clone, Debug)]
pub struct LineRow {
    pub observed: Option<f64>,
    pub ritz: Option<f64>,
    pub intensity: String,
    pub aki: Option<f64>,
    pub accuracy: String,
    pub lower: String,
    pub upper: String,
}

impl LineRow {
    /// The wavelength the host writes into `Transition::wavelength`: the
    /// observed one, else the Ritz one. The renderer chooses between no
    /// values (#258).
    pub fn wavelength(&self) -> Option<f64> {
        self.observed.or(self.ritz)
    }
}

pub fn lines(text: &str) -> Result<Vec<LineRow>> {
    let rows = records(text);
    let (header, rows) = rows.split_first().ok_or("an empty lines export")?;
    let col = |names: &[&str]| column(header, names);
    let obs = col(&["obs_wl_air(nm)", "obs_wl_vac(nm)"])?;
    let ritz = col(&["ritz_wl_air(nm)", "ritz_wl_vac(nm)"])?;
    let intens = col(&["intens"])?;
    let aki = col(&["Aki(s^-1)"])?;
    let acc = col(&["Acc"])?;
    let [ci, ti, ji, ck, tk, jk] = [
        col(&["conf_i"])?,
        col(&["term_i"])?,
        col(&["J_i"])?,
        col(&["conf_k"])?,
        col(&["term_k"])?,
        col(&["J_k"])?,
    ];
    let field = |row: &[String], i: usize| row.get(i).cloned().unwrap_or_default();
    Ok(rows
        .iter()
        .map(|row| LineRow {
            observed: number(&field(row, obs)),
            ritz: number(&field(row, ritz)),
            intensity: field(row, intens),
            aki: number(&field(row, aki)),
            accuracy: field(row, acc),
            lower: format!("{} {} {}", field(row, ci), field(row, ti), field(row, ji)),
            upper: format!("{} {} {}", field(row, ck), field(row, tk), field(row, jk)),
        })
        .collect())
}
