//! Prints OpenIRC micro-benchmark results.
//!
//! `cargo run --release --features bench --example bench`
//!
//! The UI frame benchmarks read settings from the config directory, so run
//! them with a throwaway one: `XDG_CONFIG_HOME=$(mktemp -d)`.
//!
//! Options:
//!
//! - `--save <file>` writes the results (name and nanoseconds per run) so a
//!   later run can be compared with them.
//! - `--compare <file>` shows each result next to the saved one and exits
//!   with status 1 when one is slower by more than the tolerance. The file
//!   must hold a saved time for every benchmark, and nothing else; when it
//!   cannot be read or does not match, the run exits with status 2 instead
//!   of passing without a comparison.
//! - `--tolerance <percent>` sets that limit (default 50). Timings move with
//!   the machine's load, so the limit catches real slowdowns, not noise.

use std::process::ExitCode;

use openirc::benchcmp;

fn fail(message: &str) -> ExitCode {
    eprintln!("bench: {message}");
    ExitCode::from(2)
}

fn main() -> ExitCode {
    let mut save = None;
    let mut compare = None;
    let mut tolerance = 50.0_f64;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut operand = || {
            args.next()
                .filter(|value| !value.starts_with("--"))
                .ok_or_else(|| format!("{arg} needs a value"))
        };
        match arg.as_str() {
            "--save" => match operand() {
                Ok(path) => save = Some(path),
                Err(err) => return fail(&err),
            },
            "--compare" => match operand() {
                Ok(path) => compare = Some(path),
                Err(err) => return fail(&err),
            },
            "--tolerance" => match operand().and_then(|value| benchcmp::parse_tolerance(&value)) {
                Ok(value) => tolerance = value,
                Err(err) => return fail(&err),
            },
            other => {
                return fail(&format!(
                    "unknown option {other}; see the top of examples/bench.rs"
                ));
            }
        }
    }
    let saved = match &compare {
        Some(path) => match std::fs::read_to_string(path)
            .map_err(|err| err.to_string())
            .and_then(|text| benchcmp::parse_saved(&text))
        {
            Ok(saved) => Some(saved),
            Err(err) => return fail(&format!("cannot compare with {path}: {err}")),
        },
        None => None,
    };

    let results = openirc::bench::run();
    let timings: Vec<(&str, f64)> = results
        .iter()
        .map(|result| (result.name, result.per_op().as_nanos() as f64))
        .collect();
    let changes = match &saved {
        Some(saved) => match benchcmp::changes(saved, &timings) {
            Ok(changes) => changes.into_iter().map(Some).collect(),
            Err(err) => return fail(&err),
        },
        None => vec![None; results.len()],
    };

    println!(
        "{:<42} {:>10} {:>14} {:>9}  notes",
        "benchmark", "runs", "per run", "change"
    );
    let mut written = String::new();
    for ((result, (_, nanos)), change) in results.iter().zip(&timings).zip(&changes) {
        println!(
            "{:<42} {:>10} {:>14} {:>9}  {}",
            result.name,
            result.iterations,
            format!("{:.3?}", result.per_op()),
            change.map_or(String::new(), |change| format!("{change:+.0}%")),
            result.note
        );
        written.push_str(&format!("{}\t{nanos}\n", result.name));
    }
    if let Some(path) = save {
        if let Err(err) = std::fs::write(&path, written) {
            return fail(&format!("could not save {path}: {err}"));
        }
        println!("saved to {path}");
    }
    if let Some(saved) = &saved {
        let slower = match benchcmp::slower(saved, &timings, tolerance) {
            Ok(slower) => slower,
            Err(err) => return fail(&err),
        };
        if !slower.is_empty() {
            eprintln!(
                "slower than the saved results by more than {tolerance}%: {}",
                slower.join(", ")
            );
            return ExitCode::from(1);
        }
    }
    ExitCode::SUCCESS
}
