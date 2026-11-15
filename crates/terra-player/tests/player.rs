//! The player with the real engine and no device: the test pulls the
//! samples a device would. Nothing needs an audio device or the network.

use std::num::NonZero;
use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use std::time::Duration;

use terra_core::AudioSource;
use terra_player::{Capture, Player, PlayerEvent, QueueItemId};

/// A player with no device, in the fixtures' own format: mono, 48 kHz.
fn capture() -> (Player, Receiver<PlayerEvent>, Capture) {
    Player::capture(NonZero::new(1).unwrap(), NonZero::new(48_000).unwrap()).unwrap()
}

fn fixture(name: &str) -> AudioSource {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    AudioSource::LocalPath(path)
}

/// Waits until the player has forwarded every event the engine sent so far.
/// The engine sends them through the player's thread, which applies a
/// command only after the messages before it, and the command returns once
/// it is applied. Setting the volume to full changes nothing here.
fn sync(player: &Player) {
    player.set_volume(1.0).unwrap();
}

/// Pulls 10 ms of samples at a time, collecting the events, until `done`
/// says so. Panics after 10 s of audio.
fn pull_until(
    player: &Player,
    capture: &mut Capture,
    events: &Receiver<PlayerEvent>,
    seen: &mut Vec<PlayerEvent>,
    done: impl Fn(&[PlayerEvent]) -> bool,
) {
    for _ in 0..1000 {
        capture.by_ref().take(480).for_each(drop);
        sync(player);
        seen.extend(events.try_iter());
        if done(seen) {
            return;
        }
    }
    panic!("not done after 10 s of audio: {seen:?}");
}

fn positions(events: &[PlayerEvent], of: QueueItemId) -> Vec<Duration> {
    events
        .iter()
        .filter_map(|event| match event {
            PlayerEvent::Position { item, elapsed, .. } if *item == of => Some(*elapsed),
            _ => None,
        })
        .collect()
}

#[test]
fn two_files_play_in_order_with_increasing_positions() {
    let (player, events, mut capture) = capture();
    let a = player.play(fixture("a.flac")).unwrap();
    let b = player.append(fixture("b.flac")).unwrap();
    let mut seen = Vec::new();
    pull_until(&player, &mut capture, &events, &mut seen, |seen| {
        seen.contains(&PlayerEvent::Stopped)
    });

    let order: Vec<&PlayerEvent> = seen
        .iter()
        .filter(|event| !matches!(event, PlayerEvent::Position { .. }))
        .collect();
    assert_eq!(
        order,
        [
            &PlayerEvent::TrackStarted { item: a },
            &PlayerEvent::TrackEnded { item: a },
            &PlayerEvent::TrackStarted { item: b },
            &PlayerEvent::TrackEnded { item: b },
            &PlayerEvent::Stopped,
        ]
    );
    for item in [a, b] {
        let elapsed = positions(&seen, item);
        // 1.5 s each, reported every 100 ms.
        assert!(elapsed.len() >= 10, "{elapsed:?}");
        assert!(elapsed.windows(2).all(|w| w[0] < w[1]), "{elapsed:?}");
    }
    // Every position of a comes before b starts.
    let b_starts = seen
        .iter()
        .position(|event| *event == PlayerEvent::TrackStarted { item: b })
        .unwrap();
    assert!(seen[b_starts..].iter().all(|event| !matches!(
        event,
        PlayerEvent::Position { item, .. } if *item == a
    )));
    assert!(seen.iter().all(|event| match event {
        PlayerEvent::Position { elapsed, total, .. } => Some(*elapsed) <= *total,
        _ => true,
    }));
}

#[test]
fn after_a_seek_the_next_position_is_at_least_the_target() {
    let (player, events, mut capture) = capture();
    let a = player.play(fixture("a.flac")).unwrap();
    let mut seen = Vec::new();
    pull_until(&player, &mut capture, &events, &mut seen, |seen| {
        positions(seen, a).len() >= 2
    });
    assert!(
        positions(&seen, a)
            .iter()
            .all(|p| *p < Duration::from_millis(500))
    );

    player.seek(Duration::from_millis(500)).unwrap();
    // The seek has reached the player; what the engine reported before it
    // is already here.
    events.try_iter().for_each(drop);
    let mut after = Vec::new();
    pull_until(&player, &mut capture, &events, &mut after, |after| {
        !positions(after, a).is_empty()
    });
    let next = positions(&after, a)[0];
    assert!(next >= Duration::from_millis(500), "{next:?}");
}

#[test]
fn pause_holds_the_position_and_resume_continues() {
    let (player, events, mut capture) = capture();
    let a = player.play(fixture("a.flac")).unwrap();
    let mut seen = Vec::new();
    pull_until(&player, &mut capture, &events, &mut seen, |seen| {
        !positions(seen, a).is_empty()
    });
    player.pause().unwrap();
    // Rodio applies the pause within 5 ms of audio.
    capture.by_ref().take(480).for_each(drop);
    sync(&player);
    events.try_iter().for_each(drop);
    let silent = capture.by_ref().take(48_000).all(|sample| sample == 0.0);
    assert!(silent);
    sync(&player);
    assert_eq!(events.try_iter().count(), 0);
    player.resume().unwrap();
    pull_until(&player, &mut capture, &events, &mut seen, |seen| {
        seen.contains(&PlayerEvent::TrackEnded { item: a })
    });
}

#[test]
fn a_missing_file_is_reported_and_skipped() {
    let (player, events, mut capture) = capture();
    let missing = player.play(fixture("missing.flac")).unwrap();
    let mut seen: Vec<PlayerEvent> = events.try_iter().collect();
    assert!(matches!(
        seen[..],
        [PlayerEvent::Error { item: Some(item), .. }, PlayerEvent::Stopped] if item == missing
    ));
    let a = player.append(fixture("a.flac")).unwrap();
    player.resume().unwrap();
    seen.clear();
    pull_until(&player, &mut capture, &events, &mut seen, |seen| {
        seen.contains(&PlayerEvent::TrackStarted { item: a })
    });
}

#[test]
fn stream_urls_are_reported_as_unsupported_without_their_secrets() {
    let (player, events, _capture) = capture();
    // A stream URL can carry credentials, as OpenSubsonic's token does.
    let url = "http://listener:synthetic-password@127.0.0.1:9/rest/stream?id=1&t=synthetic-token"
        .parse()
        .unwrap();
    player.play(AudioSource::Url(url)).unwrap();
    let Some(PlayerEvent::Error {
        error: error @ terra_player::PlayerError::Unsupported(_),
        ..
    }) = events.try_iter().next()
    else {
        panic!("no unsupported error");
    };
    for shown in [error.to_string(), format!("{error:?}")] {
        assert!(!shown.contains("synthetic"), "{shown}");
    }
}

#[test]
fn every_handle_controls_the_same_player_until_the_last_is_dropped() {
    let (player, events, _capture) = capture();
    let other = player.clone();
    std::thread::spawn(move || other.stop().unwrap())
        .join()
        .unwrap();
    drop(player);
    // The player's thread ends and closes its event channel.
    assert!(events.recv_timeout(Duration::from_secs(5)).is_err());
}

#[test]
fn status_follows_the_queue_through_the_player_thread() {
    let (player, events, mut capture) = capture();
    let a = player.play(fixture("a.flac")).unwrap();
    let b = player.append(fixture("b.flac")).unwrap();
    let status = player.status().unwrap();
    assert_eq!(status.items, [a, b]);
    assert_eq!(status.current, Some(a));
    assert_eq!(status.state, terra_player::PlaybackState::Playing);

    let mut seen = Vec::new();
    pull_until(&player, &mut capture, &events, &mut seen, |seen| {
        seen.contains(&PlayerEvent::TrackStarted { item: b })
    });
    assert_eq!(player.status().unwrap().current, Some(b));
}
