//! Frame profiler for the window.
//!
//! Off by default. Set `OPENIRC_PROFILE=1` before starting OpenIRC to time
//! each frame and its main sections. View, Profiler shows the numbers live,
//! and on exit a summary is printed to stderr and written to
//! `profile.txt` in the config directory.
//!
//! Recording is a few `Instant` reads per frame, so it can stay on while
//! reproducing a slow case.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

/// Samples kept per section; older ones are dropped.
const KEEP: usize = 2_000;

#[derive(Default)]
pub struct Profiler {
    enabled: bool,
    sections: BTreeMap<&'static str, Samples>,
}

#[derive(Default)]
struct Samples {
    recent: std::collections::VecDeque<Duration>,
    count: u64,
    total: Duration,
    max: Duration,
}

/// Summary of one section.
#[derive(Clone, Debug, PartialEq)]
pub struct Summary {
    pub name: &'static str,
    pub count: u64,
    pub mean: Duration,
    pub p50: Duration,
    pub p95: Duration,
    pub max: Duration,
}

impl Profiler {
    pub fn from_env() -> Self {
        let enabled = std::env::var("OPENIRC_PROFILE")
            .map(|value| !value.is_empty() && value != "0")
            .unwrap_or(false);
        Self::new(enabled)
    }

    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            sections: BTreeMap::new(),
        }
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// Start timing. Returns `None` when profiling is off so callers pay
    /// nothing more than a branch.
    pub fn start(&self) -> Option<Instant> {
        self.enabled.then(Instant::now)
    }

    pub fn stop(&mut self, name: &'static str, started: Option<Instant>) {
        if let Some(started) = started {
            self.record(name, started.elapsed());
        }
    }

    pub fn record(&mut self, name: &'static str, took: Duration) {
        if !self.enabled {
            return;
        }
        let samples = self.sections.entry(name).or_default();
        samples.count += 1;
        samples.total += took;
        samples.max = samples.max.max(took);
        samples.recent.push_back(took);
        if samples.recent.len() > KEEP {
            samples.recent.pop_front();
        }
    }

    pub fn summaries(&self) -> Vec<Summary> {
        self.sections
            .iter()
            .map(|(name, samples)| {
                let mut sorted: Vec<Duration> = samples.recent.iter().copied().collect();
                sorted.sort_unstable();
                Summary {
                    name,
                    count: samples.count,
                    mean: if samples.count == 0 {
                        Duration::ZERO
                    } else {
                        samples.total / samples.count as u32
                    },
                    p50: percentile(&sorted, 50),
                    p95: percentile(&sorted, 95),
                    max: samples.max,
                }
            })
            .collect()
    }

    pub fn report(&self) -> String {
        let mut out =
            String::from("section                count     mean      p50      p95      max\n");
        for summary in self.summaries() {
            out.push_str(&format!(
                "{:<20} {:>7} {:>8} {:>8} {:>8} {:>8}\n",
                summary.name,
                summary.count,
                millis(summary.mean),
                millis(summary.p50),
                millis(summary.p95),
                millis(summary.max),
            ));
        }
        out
    }
}

/// `sorted` must be in ascending order.
pub fn percentile(sorted: &[Duration], percent: usize) -> Duration {
    if sorted.is_empty() {
        return Duration::ZERO;
    }
    let rank = (sorted.len() * percent).div_ceil(100).max(1) - 1;
    sorted[rank.min(sorted.len() - 1)]
}

pub fn millis(duration: Duration) -> String {
    format!("{:.3}ms", duration.as_secs_f64() * 1_000.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_profiler_records_nothing() {
        let mut profiler = Profiler::new(false);
        assert!(profiler.start().is_none());
        profiler.record("frame", Duration::from_millis(5));
        assert!(profiler.summaries().is_empty());
    }

    #[test]
    fn summaries_report_percentiles() {
        let mut profiler = Profiler::new(true);
        for ms in 1..=100 {
            profiler.record("frame", Duration::from_millis(ms));
        }
        let summary = &profiler.summaries()[0];
        assert_eq!(summary.count, 100);
        assert_eq!(summary.p50, Duration::from_millis(50));
        assert_eq!(summary.p95, Duration::from_millis(95));
        assert_eq!(summary.max, Duration::from_millis(100));
        assert!(profiler.report().contains("frame"));
    }
}
