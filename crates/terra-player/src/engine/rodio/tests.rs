//! The Rodio engine on the real decoders, with no device: the output is
//! pulled as the device callback would pull it, which is the capture method
//! of `docs/decisions/0002-playback-engine.md`.

mod analysis;

use std::num::NonZero;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use rodio::queue::SourcesQueueOutput;
use terra_core::AudioSource;

use rodio::Source;

use super::{RodioEngine, open};
use crate::QueueItemId;
use crate::engine::deck::Format;
use crate::engine::{Engine, EngineEvent, EventSink, frames_to_duration};

fn fixture(name: &str) -> AudioSource {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    AudioSource::LocalPath(path)
}

fn capture() -> (RodioEngine, SourcesQueueOutput, Receiver<EngineEvent>) {
    let (sender, events) = mpsc::channel();
    let sink = EventSink::new(move |event| {
        let _ = sender.send(event);
    });
    // The fixtures' own format: mono, 48 kHz.
    let format = Format {
        channels: NonZero::new(1).unwrap(),
        rate: NonZero::new(48_000).unwrap(),
    };
    let (engine, output) = RodioEngine::capture(sink, format);
    (engine, output, events)
}

/// Plays `names` back to back, the first loaded and the rest handed over as
/// the next item one at a time, and returns exactly the samples from the
/// first item's first to the last item's last. The engine reports in the
/// pull that plays the sample, so the samples are told apart by the events
/// each pull produced.
fn play_through(names: &[&str]) -> Vec<f32> {
    let (mut engine, mut output, events) = capture();
    let items: Vec<QueueItemId> = names.iter().map(|_| QueueItemId::fresh()).collect();
    let sources: Vec<AudioSource> = names.iter().map(|name| fixture(name)).collect();
    engine.load(items[0], &sources[0]).unwrap();
    let mut following = 1;
    if following < items.len() {
        engine
            .set_next(Some((items[following], &sources[following])))
            .unwrap();
        following += 1;
    }
    let last = *items.last().unwrap();
    let (mut started, mut y) = (false, Vec::new());
    for _ in 0..10 * 48_000 * names.len() {
        let sample = output.next().unwrap();
        for event in events.try_iter() {
            match event {
                EngineEvent::Started(item) if item == items[0] => started = true,
                EngineEvent::Started(_) if following < items.len() => {
                    engine
                        .set_next(Some((items[following], &sources[following])))
                        .unwrap();
                    following += 1;
                }
                EngineEvent::Ended(item) if item == last => return y,
                EngineEvent::Error { error, .. } => panic!("{error}"),
                _ => {}
            }
        }
        if started {
            y.push(sample);
        }
    }
    panic!("{names:?} did not end");
}

/// The record's threshold for #7: a gap within ±1 sample, a total length
/// within ±1 sample and a join residual of at most 0.1.
fn assert_gapless(format: &str) {
    let marker = play_through(&[&format!("marker.{format}")]);
    let y = play_through(&[&format!("a.{format}"), &format!("b.{format}")]);
    let join = analysis::join(&y, analysis::marker_lead(&marker));
    eprintln!("{format}: {join:?}");
    assert!(join.gap.abs() <= 1, "{format}: gap {}", join.gap);
    assert!(
        join.len_error.abs() <= 1,
        "{format}: length {}",
        join.len_error
    );
    assert!(
        join.max_residual <= 0.1,
        "{format}: join residual {}",
        join.max_residual
    );
}

#[test]
fn gapless_mp3() {
    assert_gapless("mp3");
}

#[test]
fn gapless_flac() {
    assert_gapless("flac");
}

#[test]
fn gapless_opus() {
    assert_gapless("opus");
}

#[test]
fn gapless_alac() {
    assert_gapless("alac.m4a");
}

#[test]
fn gapless_aac() {
    assert_gapless("aac.m4a");
}

/// Where `window` sits in `whole`: the offset within 2048 frames of
/// `near` with the least squared difference.
fn locate(whole: &[f32], window: &[f32], near: usize) -> usize {
    let last = whole.len() - window.len();
    let error = |at: usize| -> f64 {
        whole[at..]
            .iter()
            .zip(window)
            .map(|(a, b)| f64::from(a - b).powi(2))
            .sum()
    };
    (near.saturating_sub(2048)..=(near + 2048).min(last))
        .min_by(|&a, &b| error(a).total_cmp(&error(b)))
        .unwrap()
}

/// A seek in AAC lands on the target: the samples after it are those at the
/// target in the file's own decode from the start, whose timeline
/// `gapless_aac` checks. The targets fall before, on and after AAC's
/// 1024-frame boundaries, within the 4096-frame pre-roll, mid-file and in
/// the last frame.
#[test]
fn a_seek_in_aac_lands_on_the_target() {
    let noise = fixture("noise.aac.m4a");
    let whole: Vec<f32> = open(&noise).unwrap().collect();
    assert_eq!(whole.len(), 48_000);
    for target in [0, 1, 1023, 1024, 4095, 4096, 23_456, 47_000] {
        let mut source = open(&noise).unwrap();
        source
            .try_seek(frames_to_duration(target as u64, 48_000))
            .unwrap();
        let window: Vec<f32> = source.by_ref().take(512).collect();
        let landed = locate(&whole, &window, target);
        assert!(
            landed.abs_diff(target) <= 1,
            "seek to {target} landed at {landed}"
        );
        // Decoded with the frames before it, so the same as from the start.
        let residual = whole[landed..]
            .iter()
            .zip(&window)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0, f32::max);
        assert!(residual < 1e-4, "seek to {target}: residual {residual}");
        // The edit list still ends it.
        assert_eq!(512 + source.count(), 48_000 - target, "seek to {target}");
    }
}

#[test]
fn a_seek_past_the_end_of_aac_ends_it() {
    let mut source = open(&fixture("noise.aac.m4a")).unwrap();
    source.try_seek(Duration::from_secs(10)).unwrap();
    assert_eq!(source.next(), None);
}

#[test]
fn each_half_decodes_to_its_length() {
    for format in ["mp3", "flac", "opus", "aac.m4a", "alac.m4a"] {
        let a = play_through(&[&format!("a.{format}")]);
        let b = play_through(&[&format!("b.{format}")]);
        assert_eq!(a.len(), analysis::SPLIT, "a.{format}");
        assert_eq!(b.len(), analysis::TOTAL - analysis::SPLIT, "b.{format}");
    }
}

#[test]
fn each_format_reports_its_exact_duration() {
    for format in ["mp3", "flac", "opus", "aac.m4a", "alac.m4a"] {
        let a = open(&fixture(&format!("a.{format}"))).unwrap();
        assert_eq!(
            a.total_duration(),
            Some(frames_to_duration(analysis::SPLIT as u64, 48_000)),
            "a.{format}"
        );
    }
}

#[test]
fn the_next_item_can_be_replaced_before_the_join() {
    let (mut engine, mut output, events) = capture();
    let (a, b, c) = (
        QueueItemId::fresh(),
        QueueItemId::fresh(),
        QueueItemId::fresh(),
    );
    engine.load(a, &fixture("a.flac")).unwrap();
    engine.set_next(Some((b, &fixture("b.flac")))).unwrap();
    output.by_ref().take(48_000).for_each(drop);
    engine.set_next(Some((c, &fixture("marker.flac")))).unwrap();
    output.by_ref().take(48_000).for_each(drop);
    let started: Vec<QueueItemId> = events
        .try_iter()
        .filter_map(|event| match event {
            EngineEvent::Started(item) => Some(item),
            _ => None,
        })
        .collect();
    assert_eq!(started, [a, c]);
}

#[test]
fn a_seek_reports_the_new_position_first() {
    let (mut engine, mut output, events) = capture();
    let a = QueueItemId::fresh();
    engine.load(a, &fixture("a.flac")).unwrap();
    output.by_ref().take(4_800).for_each(drop);
    events.try_iter().for_each(drop);
    engine.seek(Duration::from_millis(500));
    output.by_ref().take(4_800).for_each(drop);
    let first = events.try_iter().next();
    assert_eq!(
        first,
        Some(EngineEvent::Position {
            item: a,
            elapsed: Duration::from_millis(500),
            total: Some(Duration::from_nanos(1_510_770_833)),
        })
    );
}

#[test]
fn a_seek_past_the_end_reports_the_end() {
    let (mut engine, mut output, events) = capture();
    let a = QueueItemId::fresh();
    engine.load(a, &fixture("a.flac")).unwrap();
    output.by_ref().take(4_800).for_each(drop);
    events.try_iter().for_each(drop);
    engine.seek(Duration::from_secs(10));
    output.by_ref().take(480).for_each(drop);
    let total = frames_to_duration(analysis::SPLIT as u64, 48_000);
    assert_eq!(
        events.try_iter().next(),
        Some(EngineEvent::Position {
            item: a,
            elapsed: total,
            total: Some(total),
        })
    );
}
