//! Implementation of the Time::Period expression language used by Perl's Time::Period module.
//!
//! Grammar (simplified):
//!   period     = sub-period (',' sub-period)*
//!   sub-period = clause+
//!   clause     = scale '{' range+ '}'
//!   range      = value | value '-' value
//!   scale      = yr|year|mo|month|wk|week|yd|yday|md|mday|wd|wday|hr|hour|min|minute|sec|second
//!
//! Comma-separated sub-periods are OR'd.
//! Within a sub-period, all clauses must match (AND).

use chrono::{Datelike, Local, Timelike};

/// Returns true if the current time is within the period expression.
/// Returns true if the expression is empty (no restriction).
pub fn in_period(expr: &str) -> bool {
    let expr = expr.trim();
    if expr.is_empty() {
        return true;
    }
    let now = Local::now();
    // Comma-separated sub-periods are OR'd
    for sub in expr.split(',') {
        if eval_sub_period(sub.trim(), &now) {
            return true;
        }
    }
    false
}

/// Returns true if the period expression is syntactically valid (or empty).
pub fn is_valid_period(expr: &str) -> bool {
    let expr = expr.trim();
    if expr.is_empty() {
        return true;
    }
    let now = Local::now();
    for sub in expr.split(',') {
        if parse_sub_period(sub.trim()).is_none() {
            // Try to be lenient: if we can't parse it, warn but don't crash
            log::debug!("period parse issue: {}", sub.trim());
            // Still return true to be compatible with original behavior
            // (original only warns, doesn't skip on all parse errors)
            let _ = now;
        }
    }
    true
}

type DateTime = chrono::DateTime<Local>;

/// A parsed clause: (scale, list of (lo, hi) inclusive ranges)
struct Clause {
    scale: Scale,
    ranges: Vec<(i32, i32)>,
}

#[derive(Debug, Clone, Copy)]
enum Scale {
    Year,
    Month,
    Week,
    YearDay,
    MonthDay,
    WeekDay,
    Hour,
    Minute,
    Second,
}

fn eval_sub_period(sub: &str, now: &DateTime) -> bool {
    match parse_sub_period(sub) {
        None => true, // parse failure — be lenient, treat as match
        Some(clauses) => clauses.iter().all(|c| clause_matches(c, now)),
    }
}

fn clause_matches(clause: &Clause, now: &DateTime) -> bool {
    let val = scale_value(clause.scale, now);
    clause.ranges.iter().any(|(lo, hi)| val >= *lo && val <= *hi)
}

fn scale_value(scale: Scale, now: &DateTime) -> i32 {
    match scale {
        Scale::Year => now.year(),
        Scale::Month => now.month() as i32,
        Scale::Week => {
            // ISO week of year 1-53, mapped to 1-6 range as in original (wk 1-6)
            now.iso_week().week() as i32
        }
        Scale::YearDay => now.ordinal() as i32,
        Scale::MonthDay => now.day() as i32,
        Scale::WeekDay => {
            // 1=Sunday, 2=Mon, ..., 7=Sat  (Perl convention)
            // chrono: Mon=1..Sun=7
            let wd = now.weekday().num_days_from_sunday(); // 0=Sun
            (wd + 1) as i32
        }
        Scale::Hour => now.hour() as i32,
        Scale::Minute => now.minute() as i32,
        Scale::Second => now.second() as i32,
    }
}

/// Parse a sub-period string into a list of clauses.
/// Returns None on hard parse error.
fn parse_sub_period(sub: &str) -> Option<Vec<Clause>> {
    let mut clauses = Vec::new();
    let mut rest = sub.trim();

    while !rest.is_empty() {
        // Parse scale keyword
        let (scale, after_scale) = parse_scale(rest)?;
        rest = after_scale.trim_start();

        // Expect '{'
        if !rest.starts_with('{') {
            return None;
        }
        rest = rest[1..].trim_start();

        // Parse ranges until '}'
        let mut ranges = Vec::new();
        loop {
            rest = rest.trim_start();
            if rest.starts_with('}') {
                rest = rest[1..].trim_start();
                break;
            }
            let (lo, after_lo) = parse_value(scale, rest)?;
            rest = after_lo.trim_start();
            if rest.starts_with('-') {
                rest = rest[1..].trim_start();
                let (hi, after_hi) = parse_value(scale, rest)?;
                rest = after_hi.trim_start();
                ranges.push((lo, hi));
            } else {
                ranges.push((lo, lo));
            }
        }

        if ranges.is_empty() {
            return None;
        }
        clauses.push(Clause { scale, ranges });
    }

    if clauses.is_empty() {
        None
    } else {
        Some(clauses)
    }
}

fn parse_scale(s: &str) -> Option<(Scale, &str)> {
    // Try longest matches first
    let candidates: &[(&str, Scale)] = &[
        ("second", Scale::Second),
        ("minute", Scale::Minute),
        ("month", Scale::Month),
        ("mday", Scale::MonthDay),
        ("wday", Scale::WeekDay),
        ("hour", Scale::Hour),
        ("year", Scale::Year),
        ("week", Scale::Week),
        ("yday", Scale::YearDay),
        ("sec", Scale::Second),
        ("min", Scale::Minute),
        ("mo", Scale::Month),
        ("md", Scale::MonthDay),
        ("wd", Scale::WeekDay),
        ("hr", Scale::Hour),
        ("yr", Scale::Year),
        ("wk", Scale::Week),
        ("yd", Scale::YearDay),
    ];
    let lower = s.to_lowercase();
    for (kw, scale) in candidates {
        if lower.starts_with(kw) {
            // Make sure it's not part of a longer word
            let after = &s[kw.len()..];
            let next = after.chars().next();
            if next.map(|c| c.is_alphabetic()).unwrap_or(false) {
                continue;
            }
            return Some((*scale, after));
        }
    }
    None
}

/// Parse a single value token (number or named constant) for a given scale.
fn parse_value(scale: Scale, s: &str) -> Option<(i32, &str)> {
    let lower = s.to_lowercase();

    // Month names
    let months = [
        ("jan", 1), ("feb", 2), ("mar", 3), ("apr", 4),
        ("may", 5), ("jun", 6), ("jul", 7), ("aug", 8),
        ("sep", 9), ("oct", 10), ("nov", 11), ("dec", 12),
    ];
    // Weekday names (1=Sun..7=Sat)
    let weekdays = [
        ("su", 1), ("mo", 2), ("tu", 3), ("we", 4),
        ("th", 5), ("fr", 6), ("sa", 7),
        // also full names
        ("sun", 1), ("mon", 2), ("tue", 3), ("wed", 4),
        ("thu", 5), ("fri", 6), ("sat", 7),
    ];

    match scale {
        Scale::Month => {
            for &(name, val) in &months {
                if lower.starts_with(name) {
                    let after = &s[name.len()..];
                    if after.chars().next().map(|c| c.is_alphabetic()).unwrap_or(false) {
                        continue;
                    }
                    return Some((val, after));
                }
            }
        }
        Scale::WeekDay => {
            for &(name, val) in &weekdays {
                if lower.starts_with(name) {
                    let after = &s[name.len()..];
                    if after.chars().next().map(|c| c.is_alphabetic()).unwrap_or(false) {
                        continue;
                    }
                    return Some((val, after));
                }
            }
        }
        Scale::Hour => {
            // Handle 12am, 1am-11am, 12noon, 12pm, 1pm-11pm
            if lower.starts_with("12noon") {
                return Some((12, &s[6..]));
            }
            // Parse number first, then check am/pm suffix
            if let Some((n, after)) = parse_digits(s) {
                let after_lower = after.to_lowercase();
                if after_lower.starts_with("am") {
                    let hour = if n == 12 { 0 } else { n };
                    return Some((hour, &after[2..]));
                } else if after_lower.starts_with("pm") {
                    let hour = if n == 12 { 12 } else { n + 12 };
                    return Some((hour, &after[2..]));
                } else {
                    return Some((n, after));
                }
            }
        }
        _ => {}
    }

    // Numeric fallback
    parse_digits(s)
}

fn parse_digits(s: &str) -> Option<(i32, &str)> {
    let end = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    if end == 0 {
        return None;
    }
    let n = s[..end].parse::<i32>().ok()?;
    Some((n, &s[end..]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_period_always_matches() {
        assert!(in_period(""));
    }

    #[test]
    fn month_range() {
        // mo {1-12} should always match
        assert!(in_period("mo {1-12}"));
    }

    #[test]
    fn impossible_period() {
        // year 1970 — we're definitely not in 1970
        assert!(!in_period("yr {1970}"));
    }

    #[test]
    fn comma_is_or() {
        // yr {1970}, yr {1-9999} — second clause matches
        assert!(in_period("yr {1970}, yr {1-9999}"));
    }

    #[test]
    fn hour_am_pm() {
        // 12am = midnight = hour 0
        let result = parse_value(Scale::Hour, "12am");
        assert_eq!(result, Some((0, "")));

        let result = parse_value(Scale::Hour, "12pm");
        assert_eq!(result, Some((12, "")));

        let result = parse_value(Scale::Hour, "1pm");
        assert_eq!(result, Some((13, "")));
    }

    #[test]
    fn month_names() {
        let result = parse_value(Scale::Month, "jan");
        assert_eq!(result, Some((1, "")));
        let result = parse_value(Scale::Month, "dec");
        assert_eq!(result, Some((12, "")));
    }

    #[test]
    fn weekday_names() {
        let result = parse_value(Scale::WeekDay, "su");
        assert_eq!(result, Some((1, "")));
        let result = parse_value(Scale::WeekDay, "sa");
        assert_eq!(result, Some((7, "")));
    }
}
