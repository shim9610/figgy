//! Mechanical gate: no render-type branch may answer for a variant it has never
//! seen.
//!
//! `DataRenderType` gains variants as figgy gains plot types, and the dangerous
//! failure is not a crash — it is a branch that keeps compiling and quietly
//! gives the wrong answer. The case that motivated this gate:
//! `has_scatter` was `!matches!(rt, DataRenderType::Line { .. })`, "anything
//! that is not a bare line has points". A histogram-bar variant would inherit
//! `true`, and because `ensure_precise_variants_for_items` and
//! `build_series_layers` read the *same* predicate they would agree, compile a
//! scatter pipeline, and draw a scatter layer for the bars — with no scatter
//! config to read, `create_style_for_series_scaled` falls back to a black 4 px
//! filled circle. Black dots at every data point, no error anywhere.
//!
//! So the rule: a `match` on a render type carries no wildcard arm, and a render
//! type is not tested with `matches!`. Both forms let a new variant fall into a
//! default; an exhaustive `match` makes the compiler stop at every site that has
//! to decide.
//!
//! **Limits, stated so nobody trusts this further than it goes.** It is a
//! line-based scan: it sees `match` and `matches!` over an expression naming a
//! render type, and it does not see `if let` chains, `HashMap` lookups keyed by
//! variant, or a wildcard hidden behind a type alias. Those remain a matter of
//! review. What it does guarantee is that the two forms that already bit us
//! cannot come back unnoticed.
//!
//! Run manually:
//!     cargo test -p renderer --test render_type_exhaustive

use std::fs;
use std::path::{Path, PathBuf};

/// Whether an expression *is* a render type, as opposed to merely mentioning
/// one.
///
/// The distinction is the whole calibration of this gate. A scrutinee containing
/// a call or a tuple is a match over that call's result, not over the enum:
/// `match (precise_style_map, extract_scatter(rt))` has a `_ => None` arm that is
/// correct precisely *because* `extract_scatter` is the exhaustive one, and
/// flagging it would train everyone to ignore this test. So a call or tuple
/// disqualifies, and what remains must be `rt` — this workspace's universal
/// short name — or a path ending in `.render_type`.
fn is_render_type_expr(expr: &str) -> bool {
    if expr.contains('(') {
        return false;
    }
    let bare = expr
        .trim()
        .trim_start_matches('&')
        .trim_start_matches("mut ")
        .trim_start_matches('*')
        .trim();
    bare == "rt" || bare.rsplit('.').next() == Some("render_type")
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("the renderer crate lives under <workspace>/crates")
        .to_path_buf()
}

/// Production Rust in the workspace. Examples and the parked `unsupported/`
/// integration are excluded: they are not shipped, and a new variant breaking
/// them is a compile error a developer sees immediately.
fn production_sources() -> Vec<PathBuf> {
    let mut out = Vec::new();
    collect(&workspace_root().join("crates"), &mut out);
    out.sort();
    out
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if matches!(name, "target" | "examples" | "unsupported" | "tests") {
                continue;
            }
            collect(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}

/// Strip the trailing `#[cfg(test)] mod tests { .. }` block, if any.
fn without_test_module(src: &str) -> &str {
    match src.find("\nmod tests {") {
        Some(at) => &src[..at],
        None => src,
    }
}

/// The scrutinee of a `match` appearing anywhere on the line, if there is one.
fn match_scrutinee(line: &str) -> Option<&str> {
    let mut from = 0usize;
    while let Some(at) = line[from..].find("match ") {
        let start = from + at;
        // Word boundary before, so `rematch ` / `.match ` do not count.
        let preceded_by_word = line[..start]
            .chars()
            .next_back()
            .is_some_and(|c| c.is_alphanumeric() || c == '_');
        from = start + "match ".len();
        if preceded_by_word {
            continue;
        }
        let rest = line[from..].trim();
        let scrutinee = rest.trim_end_matches('{').trim();
        if !scrutinee.is_empty() {
            return Some(scrutinee);
        }
    }
    None
}

#[derive(Debug)]
struct Offender {
    file: String,
    line: usize,
    text: String,
    reason: &'static str,
}

/// A wildcard arm at the match's own depth. Arms of a *nested* match are its own
/// business, so depth is tracked rather than the whole body being scanned.
fn wildcard_arm_line(lines: &[&str], match_line: usize) -> Option<usize> {
    let mut depth = 0i32;
    let mut started = false;
    for (offset, line) in lines[match_line..].iter().enumerate() {
        let trimmed = line.trim_start();
        let wildcard_arm = trimmed.starts_with("_ =>") || trimmed.starts_with("_=>");
        if started && depth == 1 && wildcard_arm {
            return Some(match_line + offset);
        }
        for ch in line.chars() {
            match ch {
                '{' => {
                    depth += 1;
                    started = true;
                }
                '}' => depth -= 1,
                _ => {}
            }
        }
        if started && depth <= 0 {
            break;
        }
    }
    None
}

#[test]
fn no_render_type_branch_can_default_a_new_variant() {
    let root = workspace_root();
    let mut offenders: Vec<Offender> = Vec::new();
    let mut matches_scanned = 0usize;

    for path in production_sources() {
        let raw = fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {:?}: {e}", path))
            .replace("\r\n", "\n");
        let src = without_test_module(&raw);
        let rel = path
            .strip_prefix(&root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        let lines: Vec<&str> = src.lines().collect();

        for (index, line) in lines.iter().enumerate() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("//") || trimmed.starts_with("///") {
                continue;
            }

            // `match <expr> {` where the scrutinee names a render type. The
            // `match` need not start the line: `=> match &series[i].render_type {`
            // and `let c = match rt {` are the same hazard, and looking only at
            // line starts missed a live wildcard in crates/web.
            if let Some(rest) = match_scrutinee(line)
                && is_render_type_expr(rest)
            {
                matches_scanned += 1;
                if let Some(arm) = wildcard_arm_line(&lines, index) {
                    offenders.push(Offender {
                        file: rel.clone(),
                        line: arm + 1,
                        text: lines[arm].trim().to_string(),
                        reason: "wildcard arm in a match over a render type",
                    });
                }
            }

            // `matches!(<expr naming a render type>, ..)`, negated or not.
            if let Some(at) = line.find("matches!(") {
                let inside = &line[at + "matches!(".len()..];
                let scrutinee = inside.split(',').next().unwrap_or("");
                if is_render_type_expr(scrutinee) {
                    offenders.push(Offender {
                        file: rel.clone(),
                        line: index + 1,
                        text: trimmed.to_string(),
                        reason: "matches! over a render type — a new variant silently answers false",
                    });
                }
            }
        }
    }

    assert!(
        matches_scanned > 0,
        "scanned no render-type matches at all — the scrutinee spelling must have \
         changed, and this gate is checking nothing"
    );

    assert!(
        offenders.is_empty(),
        "render-type branch(es) that would answer for a variant they have never \
         seen. Replace the wildcard or the `matches!` with an exhaustive `match` \
         listing every variant, so adding one fails to compile here and somebody \
         decides what it means. See this test's header for the bug that motivated \
         the rule.\n{}",
        offenders
            .iter()
            .map(|o| format!("  {}:{}: {} — {}", o.file, o.line, o.text, o.reason))
            .collect::<Vec<_>>()
            .join("\n")
    );
}
