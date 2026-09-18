//! Filter grammar: `growth [><] <size> in <duration>`, `kind:<k>`,
//! `project:<name>`, `idle > <dur>`, `merge-complete`.
//!
//! `idle` and `merge-complete` parse today but have no data source until
//! the github enricher (#35) lands; applying them yields
//! [`Filter::NoDataYet`] rather than filtering anything out.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Op {
    Gt,
    Lt,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Filter {
    /// No filter text at all (`0` was pressed, or the line is empty).
    None,
    Growth {
        op: Op,
        bytes: u64,
        within_secs: u64,
    },
    Kind(String),
    Project(String),
    /// Parses, but has no data source yet (#35).
    NoDataYet(String),
}

/// The filter shown on open, per DESIGN.md.
pub fn default_filter() -> Filter {
    Filter::Growth {
        op: Op::Gt,
        bytes: 100 * 1024 * 1024,
        within_secs: 7 * 86_400,
    }
}

pub fn default_filter_text() -> &'static str {
    "growth > 100MB in 7d"
}

fn parse_size(s: &str) -> Option<u64> {
    let s = s.trim();
    let idx = s.find(|c: char| c.is_alphabetic())?;
    let (num, unit) = s.split_at(idx);
    let num: f64 = num.trim().parse().ok()?;
    let mult: f64 = match unit.trim().to_ascii_uppercase().as_str() {
        "B" => 1.0,
        "KB" => 1024.0,
        "MB" => 1024.0 * 1024.0,
        "GB" => 1024.0 * 1024.0 * 1024.0,
        _ => return None,
    };
    Some((num * mult) as u64)
}

fn parse_duration(s: &str) -> Option<u64> {
    let s = s.trim();
    let idx = s.find(|c: char| c.is_alphabetic())?;
    let (num, unit) = s.split_at(idx);
    let num: f64 = num.trim().parse().ok()?;
    let mult: f64 = match unit.trim().to_ascii_lowercase().as_str() {
        "s" => 1.0,
        "m" => 60.0,
        "h" => 3600.0,
        "d" => 86_400.0,
        "w" => 7.0 * 86_400.0,
        _ => return None,
    };
    Some((num * mult) as u64)
}

/// Parses one filter line. Returns `Err(message)` on a grammar error; the
/// caller keeps the previous filter applied and shows the message inline.
pub fn parse(input: &str) -> Result<Filter, String> {
    let raw = input.trim();
    if raw.is_empty() || raw == "0" {
        return Ok(Filter::None);
    }
    if raw == "merge-complete" {
        return Ok(Filter::NoDataYet(raw.to_string()));
    }
    if let Some(rest) = raw.strip_prefix("kind:") {
        if rest.trim().is_empty() {
            return Err("kind: needs a value".to_string());
        }
        return Ok(Filter::Kind(rest.trim().to_string()));
    }
    if let Some(rest) = raw.strip_prefix("project:") {
        if rest.trim().is_empty() {
            return Err("project: needs a value".to_string());
        }
        return Ok(Filter::Project(rest.trim().to_string()));
    }
    if let Some(rest) = raw.strip_prefix("idle") {
        let rest = rest.trim();
        let rest = rest
            .strip_prefix('>')
            .ok_or_else(|| "idle needs > <duration>".to_string())?;
        parse_duration(rest.trim()).ok_or_else(|| format!("bad duration: {rest:?}"))?;
        return Ok(Filter::NoDataYet(raw.to_string()));
    }
    if let Some(rest) = raw.strip_prefix("growth") {
        let rest = rest.trim();
        let (op, rest) = if let Some(r) = rest.strip_prefix('>') {
            (Op::Gt, r)
        } else if let Some(r) = rest.strip_prefix('<') {
            (Op::Lt, r)
        } else {
            return Err("growth needs > or <".to_string());
        };
        let rest = rest.trim();
        let parts: Vec<&str> = rest.splitn(2, " in ").collect();
        let size_part = parts[0].trim();
        let bytes = parse_size(size_part).ok_or_else(|| format!("bad size: {size_part:?}"))?;
        let within_secs = if parts.len() == 2 {
            let dur_part = parts[1].trim();
            parse_duration(dur_part).ok_or_else(|| format!("bad duration: {dur_part:?}"))?
        } else {
            u64::MAX
        };
        return Ok(Filter::Growth {
            op,
            bytes,
            within_secs,
        });
    }
    Err(format!("unrecognized filter: {raw:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_default_filter() {
        assert_eq!(parse(default_filter_text()).unwrap(), default_filter());
    }

    #[test]
    fn zero_clears() {
        assert_eq!(parse("0").unwrap(), Filter::None);
        assert_eq!(parse("").unwrap(), Filter::None);
    }

    #[test]
    fn parses_kind_and_project() {
        assert_eq!(parse("kind:cache").unwrap(), Filter::Kind("cache".into()));
        assert_eq!(
            parse("project:mole").unwrap(),
            Filter::Project("mole".into())
        );
    }

    #[test]
    fn kind_needs_value() {
        assert!(parse("kind:").is_err());
    }

    #[test]
    fn idle_and_merge_complete_parse_as_no_data_yet() {
        assert!(matches!(parse("idle > 3d").unwrap(), Filter::NoDataYet(_)));
        assert!(matches!(
            parse("merge-complete").unwrap(),
            Filter::NoDataYet(_)
        ));
    }

    #[test]
    fn idle_rejects_bad_duration() {
        assert!(parse("idle > banana").is_err());
    }

    #[test]
    fn growth_lt_without_duration() {
        let f = parse("growth < 50MB").unwrap();
        assert_eq!(
            f,
            Filter::Growth {
                op: Op::Lt,
                bytes: 50 * 1024 * 1024,
                within_secs: u64::MAX
            }
        );
    }

    #[test]
    fn garbage_is_an_error() {
        assert!(parse("bananas").is_err());
    }

    #[test]
    fn bad_size_unit_is_an_error() {
        assert!(parse("growth > 100XB in 7d").is_err());
    }
}
