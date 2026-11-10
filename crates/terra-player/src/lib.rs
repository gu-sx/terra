//! Local playback: the queue, the player handle and the engine behind it.
//!
//! [`Player`] is a handle to a player that runs on its own thread with a
//! [`Queue`] of [`AudioSource`](terra_core::AudioSource)s. It takes commands
//! and reports what happens as [`PlayerEvent`]s on a channel. Each player
//! keeps its own queue and state, so each device has its own (brief §4.3).
//!
//! The engine is Rodio, as `docs/decisions/0002-playback-engine.md`
//! decides, behind an internal trait that tests replace with a fake. The
//! item after the current one is handed to the engine while the current one
//! plays, so the join between them is gapless. `docs/design/player.md`
//! describes the behaviour.
//!
//! [`Player::capture`] runs the real engine without an audio device, for
//! tests: the caller pulls the samples.

mod engine;
mod error;
mod player;
mod queue;

pub use error::PlayerError;
pub use player::{Capture, Output, Player, PlayerEvent};
pub use queue::{Queue, QueueItemId, Repeat};
