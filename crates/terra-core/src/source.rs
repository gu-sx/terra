use std::path::PathBuf;

use url::Url;

/// Where the player reads a track's audio from.
///
/// A local backend returns a path to the file; a remote backend returns a
/// stream URL. Either feeds the same local player.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum AudioSource {
    /// A file on this machine.
    LocalPath(PathBuf),
    /// An audio stream, such as a homelab server's stream endpoint.
    Url(Url),
}
