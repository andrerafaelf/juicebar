//! Per-device battery history and time-remaining estimates.

use crate::model::{Estimate, Sample};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

const KEEP_SECS: u64 = 7 * 24 * 3600;
const MAX_SAMPLES: usize = 3000;
/// Record a sample at least this often even when the level is unchanged.
const HEARTBEAT_SECS: u64 = 15 * 60;

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct History {
    series: HashMap<String, Vec<Sample>>,
}

impl History {
    /// Returns true when a sample was appended.
    pub fn record(&mut self, key: &str, level: u8, charging: bool, now: u64) -> bool {
        let s = self.series.entry(key.to_string()).or_default();
        let append = match s.last() {
            None => true,
            Some(last) => last.level != level || last.charging != charging || now.saturating_sub(last.t) >= HEARTBEAT_SECS,
        };
        if append {
            s.push(Sample { t: now, level, charging });
            let cutoff = now.saturating_sub(KEEP_SECS);
            let drop = s.iter().take_while(|x| x.t < cutoff).count().max(s.len().saturating_sub(MAX_SAMPLES));
            s.drain(..drop);
        }
        append
    }

    pub fn series(&self, key: &str) -> &[Sample] {
        self.series.get(key).map(Vec::as_slice).unwrap_or(&[])
    }

    pub fn forget(&mut self, key: &str) {
        self.series.remove(key);
    }

    pub fn retain_keys(&mut self, keep: impl Fn(&str) -> bool) {
        self.series.retain(|k, _| keep(k));
    }

    /// Estimate time to empty (or to full while charging) from the current run of samples.
    pub fn estimate(&self, key: &str) -> Option<Estimate> {
        estimate(self.series(key))
    }
}

pub fn estimate(samples: &[Sample]) -> Option<Estimate> {
    let last = *samples.last()?;
    // Walk back over the current charge/discharge session.
    let mut start = samples.len() - 1;
    while start > 0 {
        let prev = samples[start - 1];
        let cur = samples[start];
        let same_direction = if last.charging { prev.level <= cur.level + 1 } else { prev.level + 1 >= cur.level };
        if prev.charging != last.charging || !same_direction || last.t - prev.t > 2 * 24 * 3600 {
            break;
        }
        start -= 1;
    }
    let run = &samples[start..];
    let first = run[0];
    let span = last.t.saturating_sub(first.t);
    if run.len() < 3 || span < 20 * 60 || first.level.abs_diff(last.level) < 3 {
        return None;
    }
    // Least-squares slope, percent per hour.
    let n = run.len() as f64;
    let (mx, my) = run.iter().fold((0.0, 0.0), |(x, y), s| (x + (s.t - first.t) as f64 / 3600.0, y + s.level as f64));
    let (mx, my) = (mx / n, my / n);
    let (mut num, mut den) = (0.0, 0.0);
    for s in run {
        let x = (s.t - first.t) as f64 / 3600.0 - mx;
        num += x * (s.level as f64 - my);
        den += x * x;
    }
    if den <= f64::EPSILON {
        return None;
    }
    let rate = num / den;
    let (remaining, to_full) = if last.charging { (100.0 - last.level as f64, true) } else { (last.level as f64, false) };
    if (to_full && rate <= 0.05) || (!to_full && rate >= -0.05) {
        return None;
    }
    let minutes = (remaining / rate.abs() * 60.0).min(60.0 * 24.0 * 60.0);
    Some(Estimate { minutes: minutes.round() as u32, to_full, rate_per_hour: rate as f32 })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(t_min: u64, level: u8, charging: bool) -> Sample {
        Sample { t: 1_000_000 + t_min * 60, level, charging }
    }

    #[test]
    fn discharge_estimate() {
        // 10% per hour, now at 50% → ~5h left.
        let v = vec![s(0, 60, false), s(30, 55, false), s(60, 50, false)];
        let e = estimate(&v).unwrap();
        assert!(!e.to_full);
        assert!((295..=305).contains(&e.minutes), "{e:?}");
    }

    #[test]
    fn ignores_previous_session_and_short_runs() {
        let v = vec![s(0, 20, true), s(60, 80, true), s(61, 80, false), s(70, 79, false)];
        assert!(estimate(&v).is_none());
    }

    #[test]
    fn charging_estimate() {
        let v = vec![s(0, 40, true), s(15, 50, true), s(30, 60, true)];
        let e = estimate(&v).unwrap();
        assert!(e.to_full);
        assert!((55..=65).contains(&e.minutes), "{e:?}");
    }

    #[test]
    fn record_dedupes_and_prunes() {
        let mut h = History::default();
        assert!(h.record("a", 50, false, 100));
        assert!(!h.record("a", 50, false, 200));
        assert!(h.record("a", 49, false, 300));
        assert!(h.record("a", 49, false, 300 + HEARTBEAT_SECS));
        assert!(h.record("a", 48, false, 300 + KEEP_SECS + 10));
        assert_eq!(h.series("a").len(), 2);
    }
}
