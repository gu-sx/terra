//! The deck through Rodio's mixer, as on a device, with no device.

use std::f64::consts::PI;
use std::num::NonZero;
use std::sync::mpsc;

use rodio::buffer::SamplesBuffer;
use rodio::mixer::mixer;

use super::{Deck, Format, Track};
use crate::QueueItemId;
use crate::engine::{EngineEvent, EventSink};

fn tone(n: f64, rate: f64) -> f64 {
    0.5 * (2.0 * PI * 1000.0 * n / rate).sin()
}

/// Samples `from..to` of a 1 kHz sine of amplitude 0.5, mono, at `rate`.
fn sine(rate: u32, from: u32, to: u32) -> SamplesBuffer {
    let samples: Vec<f32> = (from..to)
        .map(|n| tone(f64::from(n), f64::from(rate)) as f32)
        .collect();
    SamplesBuffer::new(
        NonZero::new(1).unwrap(),
        NonZero::new(rate).unwrap(),
        samples,
    )
}

/// Plays `parts` back to back through the deck and a mono 48 kHz mixer, as
/// on a device, and returns the mixer's output from the first sample of the
/// first part to the end of the last.
fn through_mixer(parts: Vec<SamplesBuffer>) -> Vec<f32> {
    let (sender, events) = mpsc::channel();
    let format = Format {
        channels: NonZero::new(1).unwrap(),
        rate: NonZero::new(48_000).unwrap(),
    };
    let (deck, control) = Deck::new(
        EventSink::new(move |event| {
            let _ = sender.send(event);
        }),
        format,
    );
    let (mixer, mut output) = mixer(format.channels, format.rate);
    let player = rodio::Player::connect_new(&mixer);
    player.append(deck);
    let items: Vec<QueueItemId> = parts.iter().map(|_| QueueItemId::fresh()).collect();
    let mut parts = items.iter().zip(parts).map(|(item, part)| Track {
        item: *item,
        source: Box::new(part),
    });
    control.load(parts.next().unwrap());
    let last = *items.last().unwrap();
    let (mut started, mut y) = (false, Vec::new());
    for _ in 0..48_000 * 10 {
        let sample = output.next().unwrap();
        for event in events.try_iter() {
            match event {
                EngineEvent::Started(_) => {
                    started = true;
                    control.set_next(parts.next());
                }
                EngineEvent::Ended(item) if item == last => return y,
                _ => {}
            }
        }
        if started {
            y.push(sample);
        }
    }
    panic!("the parts did not end");
}

/// The largest difference between `y` and the 48 kHz sine delayed by
/// `delay` samples, over `range`.
fn deviation(y: &[f32], range: std::ops::Range<usize>, delay: f64) -> f64 {
    range
        .map(|n| (f64::from(y[n]) - tone(n as f64 - delay, 48_000.0)).abs())
        .fold(0.0, f64::max)
}

#[test]
fn a_resampled_join_keeps_its_length_and_phase() {
    // 2 s at 44.1 kHz, split at an odd sample, played on a 48 kHz output.
    let y = through_mixer(vec![sine(44_100, 0, 44_117), sine(44_100, 44_117, 88_200)]);
    assert!((y.len() as i64 - 96_000).abs() <= 2, "length {}", y.len());
    // Linear interpolation of this tone alone stays under 0.002, so 0.01
    // rules out drift and lost samples. Each item has its own converter, so
    // the second half may sit a fraction of a sample off the first: within
    // the record's ±1 sample at the join.
    let join = (44_117.0_f64 * 48_000.0 / 44_100.0).round() as usize;
    let before = deviation(&y, 48..join - 2, 0.0);
    assert!(before < 0.01, "before the join: {before}");
    let after = (-100..=100)
        .map(|d| deviation(&y, join + 2..y.len() - 48, f64::from(d) / 100.0))
        .fold(f64::MAX, f64::min);
    assert!(
        after < 0.01,
        "after the join, at the best delay within ±1 sample: {after}"
    );
}

#[test]
fn output_in_the_mixers_format_passes_through_unchanged() {
    let y = through_mixer(vec![sine(48_000, 0, 24_017), sine(48_000, 24_017, 48_000)]);
    assert_eq!(y.len(), 48_000);
    assert!(deviation(&y, 0..y.len(), 0.0) < 1e-6);
}
