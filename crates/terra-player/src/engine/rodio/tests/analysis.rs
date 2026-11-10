//! Measures the join between two halves of a continuous sine: the spike's
//! `analysis.rs` (`docs/decisions/0002-playback-engine.md`, Appendix A.5),
//! for the 3 s fixtures of `tests/fixtures/gen.sh`.
//!
//! The fixture is x[m] = 0.5 sin(2π·1000·m / 48000), m = 0..144000, split at
//! m = 72517. An engine's output y[n] is x delayed by `lead` samples before
//! the join and by `lead + gap` after it. A sine fit over a whole number of
//! periods gives each delay modulo the 48-sample period. The coarse value
//! that resolves it comes from an envelope (where |y| first or last exceeds
//! 0.2): for `lead`, the onset of the marker fixture, which is the same tone
//! after 3000 samples of silence, decoded by the same engine.

use std::f64::consts::PI;

pub(super) const PERIOD: usize = 48; // 1 kHz at 48 kHz
pub(super) const SPLIT: usize = 72_517;
pub(super) const TOTAL: usize = 144_000;
const AMP: f64 = 0.5;
const WINDOW: usize = PERIOD * 100;
/// Where the steady residual is measured, after the lead: well before the
/// join. The spike's 20 s fixture used 100 000.
const STEADY_FROM: i64 = 24_000;

fn omega() -> f64 {
    2.0 * PI / PERIOD as f64
}

fn ideal(m: i64) -> f64 {
    AMP * (omega() * m as f64).sin()
}

/// Delay modulo the period, from a least-squares sine fit over `y[from..]`.
fn delay_mod(y: &[f32], from: usize) -> f64 {
    let w = omega();
    let (mut s, mut c) = (0.0, 0.0);
    for (i, v) in y[from..from + WINDOW].iter().enumerate() {
        let n = (from + i) as f64;
        s += *v as f64 * (w * n).sin();
        c += *v as f64 * (w * n).cos();
    }
    // y[n] = A sin(w n + phi) with phi = -w d.
    let phi = c.atan2(s);
    (-phi / w).rem_euclid(PERIOD as f64)
}

/// The integer delay closest to `coarse` that matches the fit at `from`.
fn resolve(y: &[f32], from: usize, coarse: i64) -> i64 {
    let fine = delay_mod(y, from);
    let p = PERIOD as f64;
    let k = ((coarse as f64 - fine) / p).round();
    (fine + k * p).round() as i64
}

fn first_loud(y: &[f32]) -> Option<usize> {
    y.iter().position(|v| v.abs() > 0.2)
}

fn last_loud(y: &[f32]) -> Option<usize> {
    y.iter().rposition(|v| v.abs() > 0.2)
}

#[derive(Debug)]
#[allow(dead_code, reason = "the tests print every field with Debug")]
pub(super) struct Join {
    pub(super) len_error: i64,
    pub(super) lead: i64,
    pub(super) gap: i64,
    pub(super) trail: i64,
    /// Largest |y - x| within 2048 samples either side of the join, outside
    /// the gap itself: a click even when the gap is zero.
    pub(super) max_residual: f64,
    /// Where that maximum is, relative to the join.
    pub(super) max_residual_at: i64,
    /// Largest |y - x| over 4096 samples far from the join.
    pub(super) steady_residual: f64,
}

const MARKER_ONSET: i64 = 3000;

/// Samples an engine adds (positive) or drops (negative) at the start of a
/// file, from its output for the marker fixture.
pub(super) fn marker_lead(y: &[f32]) -> i64 {
    let x: Vec<f32> = (0..MARKER_ONSET + 1000)
        .map(|m| {
            if m < MARKER_ONSET {
                0.0
            } else {
                ideal(m) as f32
            }
        })
        .collect();
    let coarse = first_loud(y).unwrap() as i64 - first_loud(&x).unwrap() as i64;
    resolve(y, (MARKER_ONSET + coarse + 480) as usize, coarse)
}

/// Analyses y, the output of playing the two halves back to back, given the
/// engine's lead from `marker_lead`.
pub(super) fn join(y: &[f32], marker_lead: i64) -> Join {
    let len = y.len() as i64;
    let x: Vec<f32> = (0..TOTAL as i64).map(|m| ideal(m) as f32).collect();
    let lead = resolve(y, (marker_lead.max(0) as usize) + 10 * WINDOW, marker_lead);
    let tail_y = len - 1 - last_loud(y).unwrap() as i64;
    let tail_x = TOTAL as i64 - 1 - last_loud(&x).unwrap() as i64;
    let trail_coarse = tail_y - tail_x;
    let gap_coarse = (len - TOTAL as i64) - lead - trail_coarse;
    let after = (SPLIT as i64 + lead + gap_coarse.max(0) + 10 * WINDOW as i64) as usize;
    let gap = resolve(y, after, lead + gap_coarse) - lead;
    let trail = len - TOTAL as i64 - lead - gap;

    let join_at = SPLIT as i64 + lead; // first output index of the gap
    let mut max_residual = (0.0, 0);
    let mut track = |n: i64, delay: i64| {
        let r = (y[n as usize] as f64 - ideal(n - delay)).abs();
        if r > max_residual.0 {
            max_residual = (r, n - join_at);
        }
    };
    for n in (join_at - 2048)..join_at {
        track(n, lead);
    }
    for n in (join_at + gap.max(0))..(join_at + gap.max(0) + 2048) {
        track(n, lead + gap);
    }
    // The same measure far from the join: the codec's own error.
    let from = lead + STEADY_FROM;
    let steady_residual = (from..from + 4096)
        .map(|n| (y[n as usize] as f64 - ideal(n - lead)).abs())
        .fold(0.0, f64::max);
    Join {
        len_error: len - TOTAL as i64,
        lead,
        gap,
        trail,
        max_residual: max_residual.0,
        max_residual_at: max_residual.1,
        steady_residual,
    }
}
