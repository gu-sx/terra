//! The terminal UI.
//!
//! The UI is generic over [`LibraryBackend`], so it is the same whether the
//! library is local or remote. It follows one loop:
//!
//! - [`keymap`] turns a terminal event into an [`Action`];
//! - [`update`] applies the action to the [`App`] state and returns the
//!   [`Request`](app::Request)s for data it needs;
//! - [`fetch`] runs a request against the backend, and [`receive`] stores
//!   its [`Response`](app::Response);
//! - [`render`] draws the state.
//!
//! Only the rows of the main list that fit on the terminal are drawn.
//!
//! [`run`] drives that loop on a Tokio runtime over crossterm's event
//! stream, in the terminal's alternate screen, and restores the terminal on
//! exit and on panic.

mod app;
mod fetch;
mod keymap;
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

pub use app::{Action, App, receive, update};
pub use fetch::fetch;
pub use render::render;

/// Runs the TUI against the synthetic sample library until the user quits.
pub fn run_demo() -> io::Result<()> {
    let library = MemoryLibrary::sample();
    run(&library, App::new("demo library"))
}

/// Runs the TUI against `backend` until the user quits.
pub fn run<B: LibraryBackend>(backend: &B, app: App) -> io::Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread().build()?;
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
        // another acts on the list the first one loaded.
        tokio::select! {
            biased;
            Some(response) = pending.next(), if !pending.is_empty() => receive(&mut app, response),
            event = events.next() => match event {
                Some(Ok(event)) => {
                    if let Some(action) = keymap::action_for_event(&app, &event) {
                        requests = update(&mut app, action);
                    }
                }
                Some(Err(error)) => return Err(error),
                None => return Ok(()),
            },
        }
    }
}
