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
//! Only the rows of the main list that fit on the terminal are drawn.
//!
//! [`run`] drives that loop on the caller's Tokio runtime, which also runs
//! the backend, over crossterm's event stream, in the terminal's alternate
//! screen, and restores the terminal on exit and on panic.

mod app;
mod fetch;
mod keymap;
mod playlist;
mod render;

#[cfg(test)]
mod tests;

use std::io;

use crossterm::event::EventStream;
use futures_util::StreamExt;
use futures_util::stream::FuturesUnordered;
use ratatui::DefaultTerminal;
use terra_core::LibraryBackend;
use terra_core::testing::MemoryLibrary;
use tokio::runtime::Runtime;

pub use app::{Action, App, handle_event, receive, update};
pub use fetch::fetch;
pub use render::render;

/// Runs the TUI against the synthetic sample library until the user quits.
pub fn run_demo(runtime: &Runtime) -> io::Result<()> {
    let library = MemoryLibrary::sample();
    run(runtime, &library, App::new("demo library"))
}

/// Runs the TUI against `backend` on `runtime` until the user quits.
pub fn run<B: LibraryBackend>(runtime: &Runtime, backend: &B, app: App) -> io::Result<()> {
    let mut terminal = TerminalGuard::enter()?;
    runtime.block_on(event_loop(&mut terminal.terminal, backend, app))
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
            return Ok(());
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
            event = events.next() => match event {
                Some(Ok(event)) => requests = handle_event(&mut app, &event),
                Some(Err(error)) => return Err(error),
                None => return Ok(()),
            },
        }
    }
}
