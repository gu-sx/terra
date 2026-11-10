//! Stable identifiers assigned by the library.

use std::error::Error;
use std::fmt;
use std::num::ParseIntError;
use std::str::FromStr;

/// The error returned when a string is not a valid identifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseIdError {
    kind: &'static str,
    source: ParseIntError,
}

impl fmt::Display for ParseIdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid {} id", self.kind)
    }
}

impl Error for ParseIdError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.source)
    }
}

macro_rules! id_type {
    ($(#[$doc:meta])* $name:ident, $kind:literal) => {
        $(#[$doc])*
        ///
        /// It wraps the `i64` the library assigns. `Display` prints that
        /// number and `FromStr` parses it back.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(i64);

        impl $name {
            /// Wraps a raw identifier.
            pub const fn new(raw: i64) -> Self {
                Self(raw)
            }

            /// Returns the raw identifier.
            pub const fn get(self) -> i64 {
                self.0
            }
        }

        impl From<i64> for $name {
            fn from(raw: i64) -> Self {
                Self(raw)
            }
        }

        impl From<$name> for i64 {
            fn from(id: $name) -> Self {
                id.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                fmt::Display::fmt(&self.0, f)
            }
        }

        impl FromStr for $name {
            type Err = ParseIdError;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                s.parse()
                    .map(Self)
                    .map_err(|source| ParseIdError { kind: $kind, source })
            }
        }
    };
}

id_type!(
    /// Identifies a track.
    TrackId,
    "track"
);
id_type!(
    /// Identifies an album.
    AlbumId,
    "album"
);
id_type!(
    /// Identifies an artist.
    ArtistId,
    "artist"
);
id_type!(
    /// Identifies a playlist.
    PlaylistId,
    "playlist"
);
id_type!(
    /// Identifies one entry of a playlist. A track that appears in a playlist
    /// twice has two entries, and an entry keeps its identifier when it moves.
    PlaylistEntryId,
    "playlist entry"
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_and_from_str_round_trip() {
        for raw in [0, 1, 42, -7, i64::MAX, i64::MIN] {
            let id = TrackId::new(raw);
            assert_eq!(id.to_string().parse::<TrackId>(), Ok(id));
        }
        assert_eq!(PlaylistEntryId::new(12).to_string(), "12");
    }

    #[test]
    fn from_str_rejects_non_numbers() {
        for input in ["", " 1", "1 ", "one", "1.5", "99999999999999999999"] {
            let err = input.parse::<AlbumId>().unwrap_err();
            assert_eq!(err.to_string(), "invalid album id");
            assert!(err.source().is_some());
        }
    }
}
