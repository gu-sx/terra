//! The library roots and the scanner, which keeps the library in step with
//! the audio files under them.
//!
//! `docs/design/library-schema.md` ("Scanning") describes the algorithm.

mod read;
#[cfg(test)]
mod tests;
mod walk;

use std::collections::{HashMap, HashSet};
use std::fs::{File, TryLockError};
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use sqlx::SqliteConnection;
use terra_core::{AlbumKind, TrackId};

use crate::error::Error;
use crate::library::{LocalLibrary, SCAN_LOCK_FILE};
use crate::time::from_nanos;
use crate::write::{
    NewAlbum, NewFile, NewTrack, RootId, clear_missing, delete_root, ensure_album, ensure_artist,
    insert_root, insert_track, mark_missing, update_album, update_track,
};
use read::FileInfo;
use walk::{Root, Seen, Walk};

/// How far a scan has got, which [`LocalLibrary::scan`] reports after each
/// file it found.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ScanProgress {
    /// The files handled so far, this one included.
    pub done: usize,
    /// The audio files the walk found.
    pub total: usize,
    /// The file just handled.
    pub path: PathBuf,
}

/// What a scan did. Each file the walk found counts once, in `added`,
/// `updated`, `moved`, `unchanged` or `errors`. The known files of a root
/// the scan could not look into count in none of them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScanReport {
    /// New tracks.
    pub added: usize,
    /// Known files that changed, or that were missing and are back.
    pub updated: usize,
    /// Known files found at a new path.
    pub moved: usize,
    /// Known files the scan did not find, including those already missing
    /// before it, except those it left as they were because it could not
    /// look for them.
    pub missing: usize,
    /// Known files whose size and modification time are unchanged.
    pub unchanged: usize,
    /// The paths the scan could not read or store, with the reason, and each
    /// root it could not read or found empty while it has tracks. The scan
    /// goes on after each.
    pub errors: Vec<(PathBuf, String)>,
}

/// A file the library knows, as the scan compares it.
#[derive(Debug, sqlx::FromRow)]
struct Known {
    id: i64,
    track_id: i64,
    root_id: i64,
    rel_path: String,
    size: i64,
    mtime_ns: i64,
    content_hash: Vec<u8>,
    missing_since: Option<i64>,
}

impl LocalLibrary {
    /// Adds a folder for [`scan`](Self::scan) to read, and returns the path
    /// it stored: `path` made absolute, with symbolic links resolved.
    ///
    /// # Errors
    ///
    /// [`Error::Io`] if `path` is not a directory that can be read,
    /// [`Error::NonUtf8Path`] if its path is not valid UTF-8, and
    /// [`Error::RootOverlaps`] if it is a library root already, or is inside
    /// one or contains one.
    pub async fn add_root(&self, path: impl AsRef<Path>) -> Result<PathBuf, Error> {
        let path = std::fs::canonicalize(path.as_ref())?;
        if !std::fs::metadata(&path)?.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::NotADirectory,
                format!("{} is not a directory", path.display()),
            )
            .into());
        }
        if path.to_str().is_none() {
            return Err(Error::NonUtf8Path(path));
        }
        let mut tx = self.pool.begin().await?;
        for existing in roots(&mut tx).await? {
            if path.starts_with(&existing.path) || existing.path.starts_with(&path) {
                return Err(Error::RootOverlaps {
                    path,
                    existing: existing.path,
                });
            }
        }
        insert_root(&mut tx, &path, SystemTime::now()).await?;
        tx.commit().await?;
        Ok(path)
    }

    /// Removes a library root, with every track whose file is under it, and
    /// those tracks' playlist entries and plays. Artists and albums left
    /// without tracks go too. Returns whether `path` was a root.
    ///
    /// `path` is compared after resolving it as [`add_root`](Self::add_root)
    /// does, or as it is when that fails, such as for a folder that no longer
    /// exists.
    ///
    /// # Errors
    ///
    /// [`Error::Database`] if the database fails.
    pub async fn remove_root(&self, path: impl AsRef<Path>) -> Result<bool, Error> {
        let path = path.as_ref();
        let resolved = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_owned());
        let mut tx = self.pool.begin().await?;
        let root = roots(&mut tx)
            .await?
            .into_iter()
            .find(|root| root.path == resolved || root.path == path);
        let Some(root) = root else {
            return Ok(false);
        };
        let removed = delete_root(&mut tx, RootId(root.id)).await?;
        tx.commit().await?;
        Ok(removed)
    }

    /// The library roots, ordered by path.
    ///
    /// # Errors
    ///
    /// [`Error::Database`] if the database fails.
    pub async fn list_roots(&self) -> Result<Vec<PathBuf>, Error> {
        let mut conn = self.pool.acquire().await?;
        Ok(roots(&mut conn)
            .await?
            .into_iter()
            .map(|root| root.path)
            .collect())
    }

    /// Scans every library root and brings the library in step with the
    /// audio files under them, calling `progress` after each file.
    ///
    /// New files become tracks, changed files update their tracks, moved
    /// files keep their tracks, and files that are gone are marked missing.
    /// A file that cannot be read or stored is reported in
    /// [`ScanReport::errors`], and the scan goes on. Each file is written in
    /// its own transaction, so an interrupted scan keeps what it did.
    ///
    /// A root that cannot be read, or holds no audio file while some of its
    /// tracks are not missing, such as an unmounted drive, is reported once
    /// in [`ScanReport::errors`], with the cause if it cannot be read, and
    /// its files are left as they were: not marked missing, and not taken
    /// for moved by a copy elsewhere. So are the files under a folder that
    /// cannot be read. A root emptied on purpose therefore keeps its tracks;
    /// [`remove_root`](Self::remove_root) removes them.
    ///
    /// Only one scan of a library runs at a time: a scan holds an exclusive
    /// lock on [`SCAN_LOCK_FILE`](crate::SCAN_LOCK_FILE) in the library's
    /// directory, which it creates if missing, until it returns. The system
    /// releases the lock when a process ends, so a crash leaves none behind.
    ///
    /// # Errors
    ///
    /// [`Error::ScanInProgress`] at once, before reading anything, if another
    /// scan of the library holds the lock, [`Error::Io`] if the lock file
    /// cannot be opened or locked, and [`Error::Database`] if the database
    /// fails, which ends the scan.
    pub async fn scan(&self, progress: impl FnMut(ScanProgress)) -> Result<ScanReport, Error> {
        // Held until the scan returns, on every path; dropping it unlocks.
        let _lock = lock(&self.dir)?;
        let mut conn = self.pool.acquire().await?;
        let roots = roots(&mut conn).await?;
        let known = known(&mut conn).await?;
        drop(conn);

        let walk = {
            let roots = roots.clone();
            blocking(move || walk::walk(&roots)).await?
        };
        self.apply(&roots, &known, walk, progress).await
    }

    /// Brings the library in step with what `walk` found under `roots`,
    /// where `known` are the files the library knew before the walk.
    async fn apply(
        &self,
        roots: &[Root],
        known: &[Known],
        walk: Walk,
        mut progress: impl FnMut(ScanProgress),
    ) -> Result<ScanReport, Error> {
        let left_alone = left_alone(roots, known, &walk);
        let mut report = ScanReport {
            errors: walk.errors,
            ..ScanReport::default()
        };
        for (root, files, reason) in left_alone.roots {
            let mut message = format!(
                "the root could not be read or holds no audio files; \
                 its {files} files were left as they were"
            );
            if let Some(reason) = reason {
                message.push_str(": ");
                message.push_str(&reason);
            }
            report.errors.push((root.to_owned(), message));
        }
        let total = walk.seen.len();
        let mut done = 0;
        let mut step = |path: &Path| {
            done += 1;
            progress(ScanProgress {
                done,
                total,
                path: path.to_owned(),
            });
        };

        // Known paths first: unchanged, or read again and updated in place.
        let by_path: HashMap<(i64, &str), &Known> = known
            .iter()
            .map(|file| ((file.root_id, file.rel_path.as_str()), file))
            .collect();
        let mut seen_files = vec![false; known.len()];
        let known_index: HashMap<i64, usize> = known
            .iter()
            .enumerate()
            .map(|(index, file)| (file.id, index))
            .collect();
        let mut unknown = Vec::new();
        for seen in &walk.seen {
            let Some(file) = by_path.get(&(seen.root, seen.rel_path.as_str())) else {
                unknown.push(seen);
                continue;
            };
            seen_files[known_index[&file.id]] = true;
            let same = u64::try_from(file.size) == Ok(seen.size) && file.mtime_ns == seen.mtime_ns;
            if same && file.missing_since.is_none() {
                report.unchanged += 1;
            } else if same {
                let mut conn = self.pool.acquire().await?;
                clear_missing(&mut conn, file.id).await?;
                report.updated += 1;
            } else {
                match read(&seen.path).await? {
                    Ok(info) => {
                        self.write(seen, &info, Some(TrackId::new(file.track_id)))
                            .await?;
                        report.updated += 1;
                    }
                    Err(err) => report.errors.push((seen.path.clone(), err)),
                }
            }
            step(&seen.path);
        }

        // The known files the walk did not see may have moved. A file the
        // walk saw is never one, so a copy is never taken for a move. Nor is
        // a file the walk could not look for.
        let mut candidates: HashMap<&[u8], Vec<&Known>> = HashMap::new();
        let unseen = known
            .iter()
            .zip(&seen_files)
            .zip(&left_alone.files)
            .filter(|((_, seen), left)| !**seen && !**left);
        for ((file, _), _) in unseen {
            candidates
                .entry(file.content_hash.as_slice())
                .or_default()
                .push(file);
        }

        // Unknown paths in (root, rel_path) order, roots ordered by path,
        // so that the first of several copies takes the move.
        let root_order: HashMap<i64, usize> = roots
            .iter()
            .enumerate()
            .map(|(index, root)| (root.id, index))
            .collect();
        unknown.sort_by(|a, b| {
            (root_order[&a.root], &a.rel_path).cmp(&(root_order[&b.root], &b.rel_path))
        });
        for seen in unknown {
            match read(&seen.path).await? {
                Ok(info) => {
                    let moved = candidates
                        .get_mut(info.content_hash.as_slice())
                        .and_then(|files| (!files.is_empty()).then(|| files.remove(0)));
                    match moved {
                        Some(file) => {
                            self.write(seen, &info, Some(TrackId::new(file.track_id)))
                                .await?;
                            report.moved += 1;
                        }
                        None => {
                            self.write(seen, &info, None).await?;
                            report.added += 1;
                        }
                    }
                }
                Err(err) => report.errors.push((seen.path.clone(), err)),
            }
            step(&seen.path);
        }

        // The candidates left are missing.
        let now = SystemTime::now();
        let mut tx = self.pool.begin().await?;
        for file in candidates.into_values().flatten() {
            mark_missing(&mut tx, file.id, now).await?;
            report.missing += 1;
        }
        tx.commit().await?;
        Ok(report)
    }

    /// Writes a file that was read, and its track, in one transaction: a
    /// new track, or an update of `track` that keeps its identifier.
    async fn write(
        &self,
        seen: &Seen,
        info: &FileInfo,
        track: Option<TrackId>,
    ) -> Result<TrackId, Error> {
        let tags = &info.tags;
        let mut tx = self.pool.begin().await?;
        let artist = ensure_artist(&mut tx, &tags.artist, &tags.artist_sort).await?;
        let album = match &tags.album {
            Some(title) => {
                let album_artist = match &tags.album_artist {
                    Some((name, sort)) => Some(ensure_artist(&mut tx, name, sort).await?),
                    // A compilation without an album artist keeps none,
                    // rather than splitting into one album per artist.
                    None if tags.compilation => None,
                    None => Some(artist),
                };
                let kind = if tags.compilation {
                    AlbumKind::Compilation
                } else {
                    AlbumKind::Album
                };
                let album = NewAlbum {
                    title,
                    album_artist,
                    year: tags.year,
                    kind,
                };
                let id = ensure_album(&mut tx, &album).await?;
                // An album found by its key still takes this file's year and
                // kind, so that a retag reaches it.
                update_album(&mut tx, id, &album).await?;
                Some(id)
            }
            None => None,
        };
        let now = SystemTime::now();
        let new = NewTrack {
            title: &tags.title,
            artist,
            album,
            disc_no: tags.disc_no,
            track_no: tags.track_no,
            duration: info.duration,
            added_at: now,
            file: NewFile {
                root: RootId(seen.root),
                rel_path: &seen.rel_path,
                size: seen.size,
                mtime: from_nanos(seen.mtime_ns),
                content_hash: &info.content_hash,
                format: &info.format,
                has_embedded_art: info.has_embedded_art,
            },
        };
        let id = match track {
            Some(id) => {
                update_track(&mut tx, id, &new, now).await?;
                id
            }
            None => insert_track(&mut tx, &new).await?,
        };
        tx.commit().await?;
        Ok(id)
    }
}

/// The known files a scan leaves as they were, because the walk could not
/// look for them.
#[derive(Debug)]
struct LeftAlone<'a> {
    /// The roots the walk could not read, or found no audio file under
    /// while some of their known files were not missing, with the number of
    /// known files each has and, for a root the walk could not read, why.
    roots: Vec<(&'a Path, usize, Option<String>)>,
    /// For each known file, in order, whether it is left as it was: it is
    /// in one of those roots, or at or under a path the walk could not read.
    files: Vec<bool>,
}

fn left_alone<'a>(roots: &'a [Root], known: &[Known], walk: &Walk) -> LeftAlone<'a> {
    let found: HashSet<i64> = walk.seen.iter().map(|seen| seen.root).collect();
    let unread: HashMap<i64, &String> = walk
        .unread_roots
        .iter()
        .map(|(root, reason)| (*root, reason))
        .collect();
    let unavailable: Vec<&Root> = roots
        .iter()
        .filter(|root| {
            unread.contains_key(&root.id)
                || (!found.contains(&root.id)
                    && known
                        .iter()
                        .any(|file| file.root_id == root.id && file.missing_since.is_none()))
        })
        .collect();
    let paths: HashMap<i64, &Path> = roots
        .iter()
        .map(|root| (root.id, root.path.as_path()))
        .collect();
    let files = known
        .iter()
        .map(|file| {
            unavailable.iter().any(|root| root.id == file.root_id)
                || paths.get(&file.root_id).is_some_and(|root| {
                    let path = root.join(&file.rel_path);
                    walk.unread.iter().any(|unread| path.starts_with(unread))
                })
        })
        .collect();
    let roots = unavailable
        .into_iter()
        .map(|root| {
            let files = known.iter().filter(|file| file.root_id == root.id).count();
            (
                root.path.as_path(),
                files,
                unread.get(&root.id).map(|reason| (*reason).clone()),
            )
        })
        .collect();
    LeftAlone { roots, files }
}

/// Takes the scan lock of the library in `dir`: an exclusive lock on its
/// [`SCAN_LOCK_FILE`], created if missing. Closing the returned file releases
/// it.
fn lock(dir: &Path) -> Result<File, Error> {
    let file = File::options()
        .write(true)
        .create(true)
        .truncate(false)
        .open(dir.join(SCAN_LOCK_FILE))?;
    match file.try_lock() {
        Ok(()) => Ok(file),
        Err(TryLockError::WouldBlock) => Err(Error::ScanInProgress),
        Err(TryLockError::Error(err)) => Err(err.into()),
    }
}

async fn roots(conn: &mut SqliteConnection) -> Result<Vec<Root>, Error> {
    let rows: Vec<(i64, String)> =
        sqlx::query_as("SELECT id, path FROM library_root ORDER BY path")
            .fetch_all(conn)
            .await?;
    Ok(rows
        .into_iter()
        .map(|(id, path)| Root {
            id,
            path: PathBuf::from(path),
        })
        .collect())
}

async fn known(conn: &mut SqliteConnection) -> Result<Vec<Known>, Error> {
    Ok(sqlx::query_as(
        "SELECT id, track_id, root_id, rel_path, size, mtime_ns, content_hash, missing_since
         FROM media_file
         ORDER BY id",
    )
    .fetch_all(conn)
    .await?)
}

/// Reads a file on a blocking thread. The inner error is the file's: a
/// message for the report. A panic while reading, such as from a malformed
/// file, is one too.
async fn read(path: &Path) -> Result<Result<FileInfo, String>, Error> {
    let path = path.to_owned();
    match tokio::task::spawn_blocking(move || read::read_file(&path)).await {
        Ok(result) => Ok(result),
        Err(err) if err.is_panic() => Ok(Err("reading the file failed unexpectedly".to_owned())),
        Err(err) => Err(io::Error::other(err).into()),
    }
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Result<T, Error> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|err| io::Error::other(err).into())
}
