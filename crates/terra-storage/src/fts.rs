//! Full-text search: the FTS5 tables and the queries against them.
//!
//! `track_fts`, `album_fts` and `artist_fts` each have one row per record,
//! whose rowid is the record's identifier. Every write to a track, album or
//! artist calls the matching `sync_*` function in the same transaction, so a
//! search never sees a half-written record. SQL triggers could do the same,
//! but they would hide a write from the code that makes it.
//!
//! The rows hold the text folded by [`fold`], not as it is displayed, and
//! [`match_query`] folds the query the same way. Search reads the names it
//! returns from the record tables.

use icu_normalizer::DecomposingNormalizerBorrowed;
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
    let row: Option<(String, String, String)> = sqlx::query_as(
        "SELECT t.title, ar.name, IFNULL(al.title, '')
         FROM track t
         JOIN artist ar ON ar.id = t.artist_id
         LEFT JOIN album al ON al.id = t.album_id
         WHERE t.id = ?1",
    )
    .bind(id.get())
    .fetch_optional(&mut *conn)
    .await?;
    if let Some((title, artist, album)) = row {
        sqlx::query("INSERT INTO track_fts (rowid, title, artist, album) VALUES (?1, ?2, ?3, ?4)")
            .bind(id.get())
            .bind(fold(&title))
            .bind(fold(&artist))
            .bind(fold(&album))
            .execute(&mut *conn)
            .await?;
    }
    Ok(())
}

/// Rewrites an album's search row and those of its tracks. Removes the
/// album's row if the album no longer exists.
pub(crate) async fn sync_album(conn: &mut SqliteConnection, id: AlbumId) -> sqlx::Result<()> {
    sqlx::query("DELETE FROM album_fts WHERE rowid = ?1")
        .bind(id.get())
        .execute(&mut *conn)
        .await?;
    let row: Option<(String, String)> = sqlx::query_as(
        "SELECT al.title, IFNULL(ar.name, '')
         FROM album al
         LEFT JOIN artist ar ON ar.id = al.album_artist_id
         WHERE al.id = ?1",
    )
    .bind(id.get())
    .fetch_optional(&mut *conn)
    .await?;
    if let Some((title, album_artist)) = row {
        sqlx::query("INSERT INTO album_fts (rowid, title, album_artist) VALUES (?1, ?2, ?3)")
            .bind(id.get())
            .bind(fold(&title))
            .bind(fold(&album_artist))
            .execute(&mut *conn)
            .await?;
    }
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
    let name: Option<String> = sqlx::query_scalar("SELECT name FROM artist WHERE id = ?1")
        .bind(id.get())
        .fetch_optional(&mut *conn)
        .await?;
    if let Some(name) = name {
        sqlx::query("INSERT INTO artist_fts (rowid, name) VALUES (?1, ?2)")
            .bind(id.get())
            .bind(fold(&name))
            .execute(&mut *conn)
            .await?;
    }
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

/// Folds text for search, as the in-memory backend does: decomposes it
/// (NFD), drops the combining marks (general category `Mn`) and lower-cases
/// it. `Café`, `Cafe` followed by U+0301 and `CAFÉ` all fold to `cafe`, and
/// a precomposed Greek `ά` folds to `α`. Letters without a decomposition,
/// such as `ø`, `ł` and `ß`, stay as they are.
///
/// The FTS5 tokenizer, `unicode61 remove_diacritics 2`, removes some marks
/// itself but reads others, such as the overline U+0305, as separators.
/// Folding the indexed text and the query here, before the tokenizer sees
/// either, means it never gets a mark to treat either way.
pub(crate) fn fold(text: &str) -> String {
    let nfd = DecomposingNormalizerBorrowed::new_nfd();
    let category = CodePointMapData::<GeneralCategory>::new();
    let unmarked: String = nfd
        .normalize_iter(text.chars())
        .filter(|&c| category.get(c) != GeneralCategory::NonspacingMark)
        .collect();
    unmarked.to_lowercase()
}

/// Turns a user's query into an FTS5 query, or `None` if it has no terms.
///
/// The query is split into terms as the
/// [`LibraryBackend`](terra_core::LibraryBackend) contract says: at every
/// character that is not alphanumeric, except that a combining mark
/// (general category `Mn`) does not split a word, so `e` followed by U+0301
/// stays in its term. Each term is then folded with [`fold`], as the indexed
/// text was. A term that folding leaves empty, such as one made only of
/// marks, is dropped.
///
/// Each term becomes a quoted string with a prefix match, `"term"*`, and
/// FTS5 requires all of them. Quoting means no input is read as FTS5 syntax:
/// `NEAR`, `AND` or `col:` are plain words, and `"`, `*` and `-` never reach
/// FTS5 at all.
pub(crate) fn match_query(query: &str) -> Option<String> {
    let category = CodePointMapData::<GeneralCategory>::new();
    let terms: Vec<String> = query
        .split(|c: char| !c.is_alphanumeric() && category.get(c) != GeneralCategory::NonspacingMark)
        .map(fold)
        .filter(|term| !term.is_empty())
        .map(|term| format!("\"{}\"*", term.replace('"', "\"\"")))
        .collect();
    (!terms.is_empty()).then(|| terms.join(" "))
}

#[cfg(test)]
mod tests {
    use super::{fold, match_query};

    #[test]
    fn fold_drops_combining_marks_and_lower_cases() {
        for (text, folded) in [
            ("Café Ensemble", "cafe ensemble"),
            ("CAFÉ", "cafe"),
            // A combining acute accent (U+0301), overline (U+0305) and
            // cedilla (U+0327).
            ("Cafe\u{301}", "cafe"),
            ("ca\u{305}fe", "cafe"),
            ("c\u{327}afe", "cafe"),
            ("Ça", "ca"),
            // A precomposed Greek ά (U+03AC) and its decomposed form.
            ("ά", "α"),
            ("Ά\u{3b9}", "αι"),
            ("α\u{301}", "α"),
            // Letters without a decomposition stay.
            ("Øl Łódź Straße", "øl łodz straße"),
            ("", ""),
            ("\u{301}\u{305}", ""),
        ] {
            assert_eq!(fold(text), folded, "{text:?}");
        }
    }

    #[test]
    fn match_query_quotes_each_term_as_a_prefix() {
        assert_eq!(match_query("Mid dri").as_deref(), Some(r#""mid"* "dri"*"#));
        assert_eq!(
            match_query(r#"a"b NEAR(c* -d) e:f"#).as_deref(),
            Some(r#""a"* "b"* "near"* "c"* "d"* "e"* "f"*"#)
        );
        assert_eq!(
            match_query("Été 1999").as_deref(),
            Some(r#""ete"* "1999"*"#)
        );
    }

    #[test]
    fn match_query_folds_each_term() {
        // A combining mark stays in its word and folding drops it: U+0301,
        // U+0305 and U+0327 in the middle or at the end of a word.
        for (query, expected) in [
            ("e\u{301}te-cafe\u{301}", r#""ete"* "cafe"*"#),
            ("ca\u{301}fe lig", r#""cafe"* "lig"*"#),
            ("ca\u{305}fe", r#""cafe"*"#),
            ("CA\u{305}FE\u{305}", r#""cafe"*"#),
            ("c\u{327}afe", r#""cafe"*"#),
            ("Ça va", r#""ca"* "va"*"#),
            ("ά", r#""α"*"#),
            ("Άλφα βήτα", r#""αλφα"* "βητα"*"#),
        ] {
            assert_eq!(match_query(query).as_deref(), Some(expected), "{query:?}");
        }
    }

    #[test]
    fn match_query_without_terms_is_none() {
        // U+0345, the Greek ypogegrammeni, is a combining mark that counts
        // as alphabetic, so the term survives the split and folds to nothing.
        for query in [
            "",
            "   ",
            "-- !",
            r#""*""#,
            "\u{301}",
            " \u{301}\u{300} ",
            "\u{305}",
            "\u{345}",
            "- \u{345} ",
        ] {
            assert_eq!(match_query(query), None, "{query:?}");
        }
    }
}
