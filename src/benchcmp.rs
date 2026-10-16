//! Comparing benchmark results with saved ones, for `examples/bench.rs`.
//!
//! A comparison that was asked for either compares every benchmark or
//! fails: an unreadable or damaged file, a benchmark missing on either
//! side, or a tolerance that is not a number never lets a run pass
//! without being compared.

use std::collections::HashMap;

/// Saved results: benchmark name and nanoseconds per run.
pub type Saved = HashMap<String, f64>;

/// Read saved results, one `name<TAB>nanoseconds` line each.
pub fn parse_saved(text: &str) -> Result<Saved, String> {
    let mut saved = Saved::new();
    for (number, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let number = number + 1;
        let (name, nanos) = line
            .rsplit_once('\t')
            .ok_or_else(|| format!("line {number} has no tab between name and time"))?;
        let nanos: f64 = nanos
            .trim()
            .parse()
            .map_err(|_| format!("line {number}: `{}` is not a time", nanos.trim()))?;
        if !nanos.is_finite() || nanos <= 0.0 {
            return Err(format!("line {number}: the time must be above zero"));
        }
        if saved.insert(name.to_string(), nanos).is_some() {
            return Err(format!("line {number}: `{name}` is there twice"));
        }
    }
    if saved.is_empty() {
        return Err("there are no saved results in it".into());
    }
    Ok(saved)
}

/// A tolerance in percent: a finite number, zero or more.
pub fn parse_tolerance(text: &str) -> Result<f64, String> {
    match text.trim().parse::<f64>() {
        Ok(value) if value.is_finite() && value >= 0.0 => Ok(value),
        _ => Err(format!(
            "the tolerance `{text}` must be a number of percent, 0 or more"
        )),
    }
}

/// The change of each result from its saved time, in percent, in the
/// order of `results`. Fails when a benchmark has no saved time or a saved
/// one did not run, since then part of the run was never compared.
pub fn changes(saved: &Saved, results: &[(&str, f64)]) -> Result<Vec<f64>, String> {
    changes_with(saved, results, false).map(|changes| {
        changes
            .into_iter()
            .map(|change| change.unwrap_or(0.0))
            .collect()
    })
}

/// `changes`, where with `allow_new` a benchmark that has no saved time is
/// new rather than a fault: its change is `None`. This is for comparing
/// with results of an older version of the code, which cannot know a
/// benchmark added since. A saved benchmark that did not run still fails:
/// it could otherwise be dropped unseen.
pub fn changes_with(
    saved: &Saved,
    results: &[(&str, f64)],
    allow_new: bool,
) -> Result<Vec<Option<f64>>, String> {
    let missing: Vec<&str> = results
        .iter()
        .map(|(name, _)| *name)
        .filter(|name| !saved.contains_key(*name))
        .collect();
    if !missing.is_empty() && !allow_new {
        return Err(format!(
            "no saved result for {}; save new results to compare with",
            missing.join(", ")
        ));
    }
    let gone: Vec<&str> = saved
        .keys()
        .map(String::as_str)
        .filter(|name| !results.iter().any(|(ran, _)| ran == name))
        .collect();
    if !gone.is_empty() {
        let mut gone = gone;
        gone.sort_unstable();
        return Err(format!(
            "saved results for benchmarks that no longer run: {}; save new results to compare with",
            gone.join(", ")
        ));
    }
    Ok(results
        .iter()
        .map(|(name, nanos)| {
            saved
                .get(*name)
                .map(|before| (nanos - before) / before * 100.0)
        })
        .collect())
}

/// The names of the results slower than their saved time by more than
/// `tolerance` percent.
pub fn slower<'a>(
    saved: &Saved,
    results: &[(&'a str, f64)],
    tolerance: f64,
) -> Result<Vec<&'a str>, String> {
    slower_with(saved, results, tolerance, false)
}

/// `slower`, with new benchmarks allowed as in `changes_with`.
pub fn slower_with<'a>(
    saved: &Saved,
    results: &[(&'a str, f64)],
    tolerance: f64,
    allow_new: bool,
) -> Result<Vec<&'a str>, String> {
    let changes = changes_with(saved, results, allow_new)?;
    Ok(results
        .iter()
        .zip(changes)
        .filter(|(_, change)| change.is_some_and(|change| change > tolerance))
        .map(|((name, _), _)| *name)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_results_must_be_complete_and_sound() {
        assert!(parse_saved("").is_err(), "an empty file");
        assert!(parse_saved("\n\n").is_err());
        assert!(parse_saved("only a name\n").is_err());
        assert!(parse_saved("a\tfast\n").is_err());
        for bad in ["NaN", "inf", "-5", "0"] {
            assert!(parse_saved(&format!("a\t{bad}\n")).is_err(), "{bad}");
        }
        assert!(parse_saved("a\t5\na\t6\n").is_err(), "twice");
        let saved = parse_saved("pump: one line\t120.5\nui frame\t9000\n").unwrap();
        assert_eq!(saved.len(), 2);
        assert_eq!(saved["pump: one line"], 120.5);
    }

    #[test]
    fn a_tolerance_is_a_number_of_percent() {
        assert_eq!(parse_tolerance("50"), Ok(50.0));
        assert_eq!(parse_tolerance("0"), Ok(0.0));
        for bad in ["NaN", "inf", "-1", "", "fifty"] {
            assert!(parse_tolerance(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_new_benchmark_may_be_allowed_but_a_dropped_one_never() {
        let saved = parse_saved("a\t100\nb\t100\n").unwrap();
        // Allowed: the new one has no change, the others are compared.
        assert_eq!(
            changes_with(&saved, &[("a", 150.0), ("c", 1.0), ("b", 160.0)], true),
            Ok(vec![Some(50.0), None, Some(60.0)])
        );
        assert_eq!(
            slower_with(
                &saved,
                &[("a", 150.0), ("c", 1.0), ("b", 160.0)],
                50.0,
                true
            ),
            Ok(vec!["b"])
        );
        // Not allowed: as before.
        assert!(
            slower_with(
                &saved,
                &[("a", 150.0), ("c", 1.0), ("b", 160.0)],
                50.0,
                false
            )
            .is_err()
        );
        // A saved benchmark that did not run fails either way.
        assert!(slower_with(&saved, &[("a", 100.0), ("c", 1.0)], 50.0, true).is_err());
        assert!(slower_with(&saved, &[("a", 100.0)], 50.0, true).is_err());
    }

    #[test]
    fn every_benchmark_is_compared_or_the_run_fails() {
        let saved = parse_saved("a\t100\nb\t100\n").unwrap();
        assert_eq!(
            slower(&saved, &[("a", 140.0), ("b", 151.0)], 50.0),
            Ok(vec!["b"])
        );
        assert_eq!(
            slower(&saved, &[("a", 100.0), ("b", 99.0)], 0.0),
            Ok(vec![])
        );
        // A new benchmark, or one that no longer runs, cannot pass unseen.
        assert!(slower(&saved, &[("a", 1.0), ("b", 1.0), ("c", 1.0)], 50.0).is_err());
        assert!(slower(&saved, &[("a", 1.0)], 50.0).is_err());
        let other = parse_saved("unknown\t100\n").unwrap();
        assert!(slower(&other, &[("a", 1.0)], 50.0).is_err());
    }
}
