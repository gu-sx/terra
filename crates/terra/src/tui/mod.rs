//! The terminal UI.
//!
//! The UI is generic over [`LibraryBackend`], so it is the same whether the
//! library is local or remote. It follows one loop:
//!
//! - [`handle_event`] turns a terminal event into an [`Action`] through the
//!   [`keymap`], holding a key typed to browse a list that is still loading
//!   until it has loaded;
//! - [`update`] applies the action to the [`App`] state and returns the
//!   [`Request`](app::Request)s for data it needs;
//! - [`fetch`] runs a request against the backend, and [`receive`] stores
//!   its [`Response`](app::Response), replays the keys held for it and
//!   returns the requests they lead to, such as reloading a playlist after
//!   an edit;
//! - [`render`] draws the state.
//!
//! The [`App`] commands the player it holds directly, through [`Playback`],
//! and [`player_event`] applies what the player reports, returning the plays
//! to record. The player's queue and state are read from the player, never
//! kept apart from it (see [`playback`]).
//!
//! Only the rows of the main list that fit on the terminal are drawn.
//!
//! [`run`] drives that loop on the caller's Tokio runtime, which also runs
//! the backend, over crossterm's event stream and the player's events, in
//! the terminal's alternate screen, and restores the terminal on exit and on
//! panic.

mod app;
mod fetch;
mod keymap;
mod playback;
mod playlist;
mod render;

#[cfg(test)]
mod tests;

use std::io;
use std::sync::mpsc;
use std::thread;

use crossterm::event::EventStream;
use futures_util::StreamExt;
use futures_util::stream::FuturesUnordered;
use ratatui::DefaultTerminal;
use terra_core::LibraryBackend;
use terra_core::testing::MemoryLibrary;
use terra_player::PlayerEvent;
use tokio::runtime::Runtime;
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

pub use app::{Action, App, handle_event, receive, update};
pub use fetch::fetch;
pub use playback::{NoPlayer, Playback, player_event};
pub use render::render;

/// Runs the TUI against the synthetic sample library until the user quits.
pub fn run_demo(
    runtime: &Runtime,
    app: App,
    events: mpsc::Receiver<PlayerEvent>,
) -> io::Result<()> {
    let library = MemoryLibrary::sample();
    run(runtime, &library, app, events)
}

/// Runs the TUI against `backend` on `runtime` until the user quits, with
/// the events of the player `app` holds.
pub fn run<B: LibraryBackend>(
    runtime: &Runtime,
    backend: &B,
    app: App,
    events: mpsc::Receiver<PlayerEvent>,
) -> io::Result<()> {
    let events = forward(events)?;
    let mut terminal = TerminalGuard::enter()?;
    runtime.block_on(event_loop(&mut terminal.terminal, backend, app, events))
}

/// Hands the player's events to the event loop from a thread of their own,
/// which ends when the player or the loop has.
fn forward(events: mpsc::Receiver<PlayerEvent>) -> io::Result<UnboundedReceiver<PlayerEvent>> {
    let (sender, receiver) = unbounded_channel();
    thread::Builder::new()
        .name("terra-player-events".to_owned())
        .spawn(move || {
            for event in events {
                if sender.send(event).is_err() {
                    break;
                }
            }
        })?;
    Ok(receiver)
}

/// The terminal in raw mode and the alternate screen, restored on drop.
/// Ratatui's initialisation also restores it before a panic is reported.
struct TerminalGuard {
    terminal: DefaultTerminal,
}

impl TerminalGuard {
    fn enter() -> io::Result<Self> {
        match ratatui::try_init() {
            Ok(terminal) => Ok(Self { terminal }),
            Err(error) => {
                ratatui::restore();
                Err(error)
            }
        }
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        ratatui::restore();
    }
}

async fn event_loop<B: LibraryBackend>(
    terminal: &mut DefaultTerminal,
    backend: &B,
    mut app: App,
    mut player_events: UnboundedReceiver<PlayerEvent>,
) -> io::Result<()> {
    let size = terminal.size()?;
    let mut requests = update(
        &mut app,
        Action::Resize {
            width: size.width,
            height: size.height,
        },
    );
    requests.extend(update(&mut app, Action::Start));
    let mut pending = FuturesUnordered::new();
    let mut events = EventStream::new();
    loop {
        for request in requests.drain(..) {
            pending.push(fetch(backend, request));
        }
        if app.quit {
            return exit(&app);
        }
        terminal.draw(|frame| render(&app, frame))?;
        // Finished responses go first, so that a key typed right after
        // another acts on the list the first one loaded. A key typed while
        // that list is still loading is held until it has.
        tokio::select! {
            biased;
            Some(response) = pending.next(), if !pending.is_empty() => {
                requests = receive(&mut app, response);
            }
            Some(event) = player_events.recv() => {
                requests = player_event(&mut app, event);
            }
            event = events.next() => match event {
                Some(Ok(event)) => requests = handle_event(&mut app, &event),
                Some(Err(error)) => return Err(error),
                None => return Ok(()),
            },
        }
    }
}

/// What the TUI returns once the user has quit: the error of an edit that
/// failed while it waited for the edits, so that it is reported after the
/// terminal is restored.
fn exit(app: &App) -> io::Result<()> {
    match &app.quit_error {
        Some(error) => Err(io::Error::other(format!(
            "playlist changes not saved: {error}"
        ))),
        None => Ok(()),
    }
}
