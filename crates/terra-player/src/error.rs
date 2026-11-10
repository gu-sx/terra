use std::error::Error;
use std::fmt;

/// What went wrong in the player. Commands fail only with
/// [`Closed`](PlayerError::Closed); the rest arrive as
/// [`PlayerEvent::Error`](crate::PlayerEvent::Error).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum PlayerError {
    /// The player's thread has stopped, so it takes no more commands.
    Closed,
    /// The output could not be opened. The message says why.
    Output(String),
    /// A source could not be opened or decoded. The message names it.
    Decode(String),
    /// The engine cannot play this kind of source yet.
    Unsupported(String),
    /// Seeking within the current item failed.
    Seek(String),
}

impl fmt::Display for PlayerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Closed => f.write_str("the player has stopped"),
            Self::Output(message) => write!(f, "cannot open the audio output: {message}"),
            Self::Decode(message) => write!(f, "cannot decode {message}"),
            Self::Unsupported(message) => write!(f, "unsupported: {message}"),
            Self::Seek(message) => write!(f, "cannot seek: {message}"),
        }
    }
}

impl Error for PlayerError {}
