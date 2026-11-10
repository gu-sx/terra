//! Walking the library roots for audio files.
//!
//! The walk blocks on the file system; the scanner runs it on Tokio's
//! blocking threads.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use walkdir::{DirEntry, WalkDir};

use crate::time::to_nanos;

/// The extensions of the files the scanner reads, compared ignoring ASCII
/// case. The file's content, not its extension, decides how it is read.
pub(crate) const AUDIO_EXTENSIONS: &[&str] = &["mp3", "flac", "ogg", "opus", "m4a", "wav"];

/// A library root to walk.
#[derive(Debug, Clone)]
pub(crate) struct Root {
    pub(crate) id: i64,
    pub(crate) path: PathBuf,
}

/// An audio file the walk found.
#[derive(Debug, Clone)]
pub(crate) struct Seen {
    pub(crate) root: i64,
    /// The path within the root, with `/` between components.
    pub(crate) rel_path: String,
    pub(crate) path: PathBuf,
    pub(crate) size: u64,
    pub(crate) mtime_ns: i64,
}

/// What a walk found: the audio files, and the paths it could not read with
/// the reason.
#[derive(Debug, Default)]
pub(crate) struct Walk {
    pub(crate) seen: Vec<Seen>,
    pub(crate) errors: Vec<(PathBuf, String)>,
}

/// Walks every root without following symbolic links, skipping hidden
/// files and directories (those whose name starts with `.`), and keeps the
/// regular files with an audio extension.
pub(crate) fn walk(roots: &[Root]) -> Walk {
    let mut walk = Walk::default();
    for root in roots {
        let entries = WalkDir::new(&root.path)
            .follow_links(false)
            .sort_by_file_name()
            .into_iter()
            // The root itself is walked even if its name starts with `.`.
            .filter_entry(|entry| entry.depth() == 0 || !is_hidden(entry));
        for entry in entries {
            match entry {
                Ok(entry) if entry.file_type().is_file() && is_audio(entry.path()) => {
                    match seen(root, &entry) {
                        Ok(seen) => walk.seen.push(seen),
                        Err(err) => walk.errors.push((entry.into_path(), err)),
                    }
                }
                Ok(_) => {}
                Err(err) => {
                    let path = err.path().unwrap_or(&root.path).to_owned();
                    walk.errors.push((path, err.to_string()));
                }
            }
        }
    }
    walk
}

fn is_hidden(entry: &DirEntry) -> bool {
    entry.file_name().as_encoded_bytes().starts_with(b".")
}

fn is_audio(path: &Path) -> bool {
    path.extension().and_then(OsStr::to_str).is_some_and(|ext| {
        AUDIO_EXTENSIONS
            .iter()
            .any(|audio| ext.eq_ignore_ascii_case(audio))
    })
}

fn seen(root: &Root, entry: &DirEntry) -> Result<Seen, String> {
    let rel = entry
        .path()
        .strip_prefix(&root.path)
        .map_err(|err| err.to_string())?;
    let mut components = Vec::new();
    for component in rel.components() {
        let component = component.as_os_str();
        let text = component
            .to_str()
            .ok_or("the path is not valid UTF-8, which the library cannot store")?;
        components.push(text);
    }
    let metadata = entry.metadata().map_err(|err| err.to_string())?;
    let mtime = metadata.modified().map_err(|err| err.to_string())?;
    Ok(Seen {
        root: root.id,
        rel_path: components.join("/"),
        path: entry.path().to_owned(),
        size: metadata.len(),
        mtime_ns: to_nanos(mtime).ok_or("the modification time is out of range")?,
    })
}
