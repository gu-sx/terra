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
#[ignore = "AAC in M4A is not gapless: Symphonia 0.5 ignores the MP4 edit list (#23)"]
fn gapless_aac() {
    assert_gapless("aac.m4a");
}

#[test]
fn each_half_decodes_to_its_length() {
    for format in ["mp3", "flac", "opus", "alac.m4a"] {
        let a = play_through(&[&format!("a.{format}")]);
        let b = play_through(&[&format!("b.{format}")]);
        assert_eq!(a.len(), analysis::SPLIT, "a.{format}");
        assert_eq!(b.len(), analysis::TOTAL - analysis::SPLIT, "b.{format}");
    }
}

#[test]
fn each_format_reports_its_exact_duration() {
    for format in ["mp3", "flac", "opus", "alac.m4a"] {
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
