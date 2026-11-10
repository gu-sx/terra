//! Full-text search: the FTS5 tables and the queries against them.
//!
//! `track_fts`, `album_fts` and `artist_fts` each have one row per record,
//! whose rowid is the record's identifier. Every write to a track, album or
//! artist calls the matching `sync_*` function in the same transaction, so a
//! search never sees a half-written record. SQL triggers could do the same,
//! but they would hide a write from the code that makes it.

use icu_properties::CodePointMapData;
use icu_properties::props::GeneralCategory;
use sqlx::SqliteConnection;
use terra_core::{AlbumId, ArtistId, TrackId};

/// Rewrites a track's search row from the track, its artist and its album.
/// Removes the row if the track no longer exists.
pub(crate) async fn sync_track(conn: &mut SqliteConnection, id: TrackId) -> sqlx::Result<()> {
    sqlx::query("DELETE FROM track_fts WHERE rowid = ?1")
        .bind(id.get())
        .execute(&mut *conn)
        .await?;
    sqlx::query(
        "INSERT INTO track_fts (rowid, title, artist, album)
         SELECT t.id, t.title, ar.name, IFNULL(al.title, '')
         FROM track t
         JOIN artist ar ON ar.id = t.artist_id
         LEFT JOIN album al ON al.id = t.album_id
         WHERE t.id = ?1",
    )
    .bind(id.get())
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// Rewrites an album's search row and those of its tracks. Removes the
/// album's row if the album no longer exists.
pub(crate) async fn sync_album(conn: &mut SqliteConnection, id: AlbumId) -> sqlx::Result<()> {
    sqlx::query("DELETE FROM album_fts WHERE rowid = ?1")
        .bind(id.get())
        .execute(&mut *conn)
        .await?;
    sqlx::query(
        "INSERT INTO album_fts (rowid, title, album_artist)
         SELECT al.id, al.title, IFNULL(ar.name, '')
         FROM album al
         LEFT JOIN artist ar ON ar.id = al.album_artist_id
         WHERE al.id = ?1",
    )
    .bind(id.get())
    .execute(&mut *conn)
    .await?;
    let tracks: Vec<i64> = sqlx::query_scalar("SELECT id FROM track WHERE album_id = ?1")
        .bind(id.get())
        .fetch_all(&mut *conn)
        .await?;
    for track in tracks {
        sync_track(conn, TrackId::new(track)).await?;
    }
    Ok(())
}

/// Rewrites an artist's search row and those of the albums and tracks that
/// show its name. Removes the artist's row if the artist no longer exists.
pub(crate) async fn sync_artist(conn: &mut SqliteConnection, id: ArtistId) -> sqlx::Result<()> {
    sqlx::query("DELETE FROM artist_fts WHERE rowid = ?1")
        .bind(id.get())
        .execute(&mut *conn)
        .await?;
    sqlx::query("INSERT INTO artist_fts (rowid, name) SELECT id, name FROM artist WHERE id = ?1")
        .bind(id.get())
        .execute(&mut *conn)
        .await?;
    let albums: Vec<i64> = sqlx::query_scalar("SELECT id FROM album WHERE album_artist_id = ?1")
        .bind(id.get())
        .fetch_all(&mut *conn)
        .await?;
    for album in albums {
        sync_album(conn, AlbumId::new(album)).await?;
    }
    let tracks: Vec<i64> = sqlx::query_scalar("SELECT id FROM track WHERE artist_id = ?1")
        .bind(id.get())
        .fetch_all(&mut *conn)
        .await?;
    for track in tracks {
        sync_track(conn, TrackId::new(track)).await?;
    }
    Ok(())
}

/// Turns a user's query into an FTS5 query, or `None` if it has no terms.
///
/// The query is split into terms as the
/// [`LibraryBackend`](terra_core::LibraryBackend) contract says: at every
/// character that is not alphanumeric, except that a combining mark
/// (general category `Mn`) does not split a word, so `e` followed by U+0301
/// stays in its term. The terms keep their accents: the FTS5 tokenizer,
/// `unicode61 remove_diacritics 2`, folds the query exactly as it folded the
/// indexed text, which keeps the two consistent in scripts whose accents it
/// does not remove. A term made only of marks is dropped.
///
/// Each term becomes a quoted string with a prefix match, `"term"*`, and
/// FTS5 requires all of them. Quoting means no input is read as FTS5 syntax:
/// `NEAR`, `AND` or `col:` are plain words, and `"`, `*` and `-` never reach
/// FTS5 at all.
pub(crate) fn match_query(query: &str) -> Option<String> {
    let category = CodePointMapData::<GeneralCategory>::new();
    let terms: Vec<String> = query
        .split(|c: char| !c.is_alphanumeric() && category.get(c) != GeneralCategory::NonspacingMark)
        .filter(|term| term.chars().any(char::is_alphanumeric))
        .map(|term| format!("\"{}\"*", term.replace('"', "\"\"")))
        .collect();
    (!terms.is_empty()).then(|| terms.join(" "))
}

#[cfg(test)]
mod tests {
    use super::match_query;

    #[test]
    fn match_query_quotes_each_term_as_a_prefix() {
        assert_eq!(match_query("Mid dri").as_deref(), Some(r#""Mid"* "dri"*"#));
        assert_eq!(
            match_query(r#"a"b NEAR(c* -d) e:f"#).as_deref(),
            Some(r#""a"* "b"* "NEAR"* "c"* "d"* "e"* "f"*"#)
        );
        assert_eq!(
            match_query("Été 1999").as_deref(),
            Some(r#""Été"* "1999"*"#)
        );
        // A combining mark stays in its word; SQLite's tokenizer folds it.
        assert_eq!(
            match_query("e\u{301}te-cafe\u{301}").as_deref(),
            Some("\"e\u{301}te\"* \"cafe\u{301}\"*")
        );
    }

    #[test]
    fn match_query_without_terms_is_none() {
        for query in ["", "   ", "-- !", r#""*""#, "\u{301}", " \u{301}\u{300} "] {
            assert_eq!(match_query(query), None, "{query:?}");
        }
    }
}
