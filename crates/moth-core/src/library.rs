//! The library: every flame the user selects is kept here, so it can be found again.
//!
//! One SQLite database (WAL mode, so a crash mid-write cannot leave it half-written) in the
//! user data folder. A flame is identified by the SHA-256 of its `.flame` XML, so selecting the
//! same flame twice bumps one row instead of adding another. Edits are new rows that point at
//! their source through `parent_id`, so lineage is kept.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, Transaction, params};
use sha2::{Digest, Sha256};

const DB_FILE: &str = "library.sqlite3";
/// The schema version stored in `PRAGMA user_version`. Opening a newer database is refused.
const VERSION: i64 = 1;

#[derive(Debug, thiserror::Error)]
pub enum LibraryError {
    #[error("library database: {0}")]
    Sql(#[from] rusqlite::Error),
    #[error("library folder: {0}")]
    Io(#[from] std::io::Error),
    #[error(
        "this library was written by a newer Moth (schema {0}); this build reads up to {VERSION}"
    )]
    TooNew(i64),
}

pub type Result<T> = std::result::Result<T, LibraryError>;

/// A flame as stored.
#[derive(Clone, Debug, PartialEq)]
pub struct Stored {
    pub id: i64,
    pub name: String,
    pub xml: String,
    /// Unix seconds.
    pub created: i64,
    /// Unix seconds of the last time the flame was selected.
    pub last_selected: i64,
    pub parent: Option<i64>,
    pub favorite: bool,
    pub note: String,
}

pub struct Library {
    conn: Connection,
    dir: PathBuf,
}

impl Library {
    /// Opens (creating if needed) the library in `dir`.
    pub fn open(dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(dir)?;
        let conn = Connection::open(dir.join(DB_FILE))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", true)?;
        migrate(&conn)?;
        Ok(Self {
            conn,
            dir: dir.to_path_buf(),
        })
    }

    /// Stores `xml` as a flame selected at `now`. If the same flame is already stored, its
    /// `last_selected` is bumped and its id is returned. Otherwise a new row is made, with
    /// `parent` as its source (`None` for a flame that came from nowhere in particular).
    pub fn record(&mut self, xml: &str, name: &str, parent: Option<i64>, now: i64) -> Result<i64> {
        let hash = content_hash(xml);
        let tx = self.conn.transaction()?;
        let existing: Option<i64> = tx
            .query_row(
                "SELECT id FROM flames WHERE hash = ?1",
                params![hash],
                |r| r.get(0),
            )
            .optional()?;
        let id = match existing {
            Some(id) => {
                tx.execute(
                    "UPDATE flames SET last_selected = ?2 WHERE id = ?1",
                    params![id, now],
                )?;
                id
            }
            None => {
                tx.execute(
                    "INSERT INTO flames (hash, xml, name, created, last_selected, parent_id)
                     VALUES (?1, ?2, ?3, ?4, ?4, ?5)",
                    params![hash, xml, name, now, parent],
                )?;
                tx.last_insert_rowid()
            }
        };
        tx.commit()?;
        Ok(id)
    }

    pub fn get(&self, id: i64) -> Result<Option<Stored>> {
        Ok(self
            .conn
            .query_row(
                "SELECT id, name, xml, created, last_selected, parent_id, favorite, note
                 FROM flames WHERE id = ?1",
                params![id],
                row_to_stored,
            )
            .optional()?)
    }

    /// Removes a flame and its render records. Returns whether a row was removed.
    pub fn delete(&mut self, id: i64) -> Result<bool> {
        let tx = self.conn.transaction()?;
        let removed = tx.execute("DELETE FROM flames WHERE id = ?1", params![id])?;
        tx.commit()?;
        Ok(removed > 0)
    }

    pub fn len(&self) -> Result<i64> {
        Ok(self
            .conn
            .query_row("SELECT COUNT(*) FROM flames", [], |r| r.get(0))?)
    }

    pub fn is_empty(&self) -> Result<bool> {
        Ok(self.len()? == 0)
    }

    /// Flames, newest selection first. `search` matches name or note (case-insensitive);
    /// `favorites_only` keeps only starred flames.
    pub fn list(&self, search: &str, favorites_only: bool) -> Result<Vec<Stored>> {
        let pattern = format!("%{}%", search.replace('%', "\\%").replace('_', "\\_"));
        let mut stmt = self.conn.prepare(
            "SELECT id, name, xml, created, last_selected, parent_id, favorite, note
             FROM flames
             WHERE (name LIKE ?1 ESCAPE '\\' OR note LIKE ?1 ESCAPE '\\')
               AND (?2 = 0 OR favorite = 1)
             ORDER BY last_selected DESC, id DESC",
        )?;
        let rows = stmt.query_map(params![pattern, favorites_only], row_to_stored)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Stars or unstars a flame. Returns whether the flame exists.
    pub fn set_favorite(&mut self, id: i64, favorite: bool) -> Result<bool> {
        let changed = self.conn.execute(
            "UPDATE flames SET favorite = ?2 WHERE id = ?1",
            params![id, favorite],
        )?;
        Ok(changed > 0)
    }

    /// Replaces a flame's note. Returns whether the flame exists.
    pub fn set_note(&mut self, id: i64, note: &str) -> Result<bool> {
        let changed = self.conn.execute(
            "UPDATE flames SET note = ?2 WHERE id = ?1",
            params![id, note],
        )?;
        Ok(changed > 0)
    }

    /// Runs SQLite's integrity check. True when the database is intact.
    pub fn is_intact(&self) -> Result<bool> {
        let verdict: String = self
            .conn
            .pragma_query_value(None, "integrity_check", |r| r.get(0))?;
        Ok(verdict == "ok")
    }

    /// Where the thumbnail for `id` lives. Thumbnails are PNG files next to the database.
    pub fn thumbnail_path(&self, id: i64) -> PathBuf {
        self.dir.join("thumbnails").join(format!("{id}.png"))
    }
}

fn row_to_stored(r: &rusqlite::Row<'_>) -> rusqlite::Result<Stored> {
    Ok(Stored {
        id: r.get(0)?,
        name: r.get(1)?,
        xml: r.get(2)?,
        created: r.get(3)?,
        last_selected: r.get(4)?,
        parent: r.get(5)?,
        favorite: r.get::<_, i64>(6)? != 0,
        note: r.get(7)?,
    })
}

/// Lowercase hex SHA-256 of the flame's XML.
pub fn content_hash(xml: &str) -> String {
    Sha256::digest(xml.as_bytes())
        .iter()
        .fold(String::with_capacity(64), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
}

/// Unix seconds now.
pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Brings the schema up to `VERSION`. Each step runs in one transaction, so a crash leaves
/// the previous version intact.
fn migrate(conn: &Connection) -> Result<()> {
    let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if version > VERSION {
        return Err(LibraryError::TooNew(version));
    }
    if version < 1 {
        let tx = conn.unchecked_transaction()?;
        create_v1(&tx)?;
        tx.pragma_update(None, "user_version", 1)?;
        tx.commit()?;
    }
    Ok(())
}

fn create_v1(tx: &Transaction<'_>) -> Result<()> {
    tx.execute_batch(
        "CREATE TABLE flames (
             id            INTEGER PRIMARY KEY,
             hash          TEXT    NOT NULL UNIQUE,
             xml           TEXT    NOT NULL,
             name          TEXT    NOT NULL DEFAULT '',
             created       INTEGER NOT NULL,
             last_selected INTEGER NOT NULL,
             parent_id     INTEGER REFERENCES flames(id) ON DELETE SET NULL,
             favorite      INTEGER NOT NULL DEFAULT 0,
             note          TEXT    NOT NULL DEFAULT ''
         );
         CREATE INDEX flames_last_selected ON flames(last_selected);
         CREATE TABLE renders (
             id        INTEGER PRIMARY KEY,
             flame_id  INTEGER NOT NULL REFERENCES flames(id) ON DELETE CASCADE,
             path      TEXT    NOT NULL,
             settings  TEXT    NOT NULL,
             created   INTEGER NOT NULL
         );",
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn xml(n: u32) -> String {
        format!("<flame name=\"f{n}\"><xform weight=\"{n}\"/></flame>")
    }

    fn open_temp() -> (tempfile::TempDir, Library) {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::open(dir.path()).unwrap();
        (dir, lib)
    }

    #[test]
    fn insert_and_lookup() {
        let (_dir, mut lib) = open_temp();
        let id = lib.record(&xml(1), "one", None, 100).unwrap();
        let got = lib.get(id).unwrap().expect("stored");
        assert_eq!(got.xml, xml(1));
        assert_eq!(got.name, "one");
        assert_eq!((got.created, got.last_selected), (100, 100));
        assert_eq!(got.parent, None);
        assert!(!got.favorite);
        assert_eq!(lib.len().unwrap(), 1);
    }

    #[test]
    fn selecting_the_same_flame_twice_adds_no_row() {
        let (_dir, mut lib) = open_temp();
        let first = lib.record(&xml(1), "one", None, 100).unwrap();
        let again = lib.record(&xml(1), "renamed", None, 250).unwrap();
        assert_eq!(first, again);
        assert_eq!(lib.len().unwrap(), 1);
        let got = lib.get(first).unwrap().unwrap();
        assert_eq!(got.last_selected, 250);
        assert_eq!(got.created, 100, "creation time is kept");
        assert_eq!(got.name, "one", "the first name is kept");
    }

    #[test]
    fn five_selections_and_one_edit_make_six_rows_with_lineage() {
        let (_dir, mut lib) = open_temp();
        let ids: Vec<i64> = (1..=5)
            .map(|n| lib.record(&xml(n), "", None, n as i64).unwrap())
            .collect();
        let edited = lib.record(&xml(99), "edited", Some(ids[2]), 10).unwrap();
        assert_eq!(lib.len().unwrap(), 6);
        assert_eq!(lib.get(edited).unwrap().unwrap().parent, Some(ids[2]));
    }

    #[test]
    fn delete_removes_the_row() {
        let (_dir, mut lib) = open_temp();
        let id = lib.record(&xml(1), "", None, 1).unwrap();
        assert!(lib.delete(id).unwrap());
        assert!(lib.get(id).unwrap().is_none());
        assert!(!lib.delete(id).unwrap());
        assert!(lib.is_empty().unwrap());
    }

    #[test]
    fn deleting_a_parent_keeps_its_children() {
        let (_dir, mut lib) = open_temp();
        let parent = lib.record(&xml(1), "", None, 1).unwrap();
        let child = lib.record(&xml(2), "", Some(parent), 2).unwrap();
        lib.delete(parent).unwrap();
        assert_eq!(lib.get(child).unwrap().unwrap().parent, None);
    }

    #[test]
    fn data_survives_reopening() {
        let dir = tempfile::tempdir().unwrap();
        let id = {
            let mut lib = Library::open(dir.path()).unwrap();
            lib.record(&xml(7), "kept", None, 42).unwrap()
        };
        let lib = Library::open(dir.path()).unwrap();
        assert_eq!(lib.get(id).unwrap().unwrap().name, "kept");
    }

    /// A database from schema version 1 opens unchanged.
    #[test]
    fn opens_a_version_1_database() {
        let dir = tempfile::tempdir().unwrap();
        {
            let conn = Connection::open(dir.path().join(DB_FILE)).unwrap();
            let tx = conn.unchecked_transaction().unwrap();
            create_v1(&tx).unwrap();
            tx.pragma_update(None, "user_version", 1).unwrap();
            tx.execute(
                "INSERT INTO flames (hash, xml, created, last_selected) VALUES ('h', 'x', 5, 5)",
                [],
            )
            .unwrap();
            tx.commit().unwrap();
        }
        let lib = Library::open(dir.path()).unwrap();
        assert_eq!(lib.len().unwrap(), 1);
        assert_eq!(lib.get(1).unwrap().unwrap().xml, "x");
    }

    #[test]
    fn refuses_a_newer_database() {
        let dir = tempfile::tempdir().unwrap();
        {
            let conn = Connection::open(dir.path().join(DB_FILE)).unwrap();
            conn.pragma_update(None, "user_version", VERSION + 1)
                .unwrap();
        }
        assert!(matches!(
            Library::open(dir.path()),
            Err(LibraryError::TooNew(_))
        ));
    }

    /// A write that is never committed (as after a crash) leaves nothing behind.
    #[test]
    fn an_uncommitted_write_is_not_kept() {
        let dir = tempfile::tempdir().unwrap();
        {
            let mut lib = Library::open(dir.path()).unwrap();
            lib.record(&xml(1), "", None, 1).unwrap();
            let tx = lib.conn.transaction().unwrap();
            tx.execute(
                "INSERT INTO flames (hash, xml, created, last_selected) VALUES ('h2', 'y', 2, 2)",
                [],
            )
            .unwrap();
            // Dropped without commit, as if the process died here.
        }
        let lib = Library::open(dir.path()).unwrap();
        assert_eq!(lib.len().unwrap(), 1);
    }

    #[test]
    fn list_is_newest_first_and_searches_name_and_note() {
        let (_dir, mut lib) = open_temp();
        let a = lib.record(&xml(1), "sunrise", None, 10).unwrap();
        let b = lib.record(&xml(2), "river", None, 20).unwrap();
        lib.set_note(a, "100% warm").unwrap();
        let all: Vec<i64> = lib.list("", false).unwrap().iter().map(|s| s.id).collect();
        assert_eq!(all, vec![b, a], "newest selection first");
        let by_name: Vec<i64> = lib
            .list("river", false)
            .unwrap()
            .iter()
            .map(|s| s.id)
            .collect();
        assert_eq!(by_name, vec![b]);
        let by_note: Vec<i64> = lib
            .list("WARM", false)
            .unwrap()
            .iter()
            .map(|s| s.id)
            .collect();
        assert_eq!(by_note, vec![a], "search ignores case and matches notes");
        assert_eq!(lib.list("100%", false).unwrap().len(), 1, "% is literal");
        assert!(lib.list("100_", false).unwrap().is_empty(), "_ is literal");
    }

    #[test]
    fn favorites_filter_and_toggle() {
        let (_dir, mut lib) = open_temp();
        let a = lib.record(&xml(1), "", None, 1).unwrap();
        let b = lib.record(&xml(2), "", None, 2).unwrap();
        assert!(lib.set_favorite(a, true).unwrap());
        let favs: Vec<i64> = lib.list("", true).unwrap().iter().map(|s| s.id).collect();
        assert_eq!(favs, vec![a]);
        assert!(lib.get(a).unwrap().unwrap().favorite);
        lib.set_favorite(a, false).unwrap();
        assert!(lib.list("", true).unwrap().is_empty());
        assert!(!lib.set_favorite(999, true).unwrap(), "missing flame");
        assert!(lib.get(b).unwrap().is_some());
    }

    #[test]
    fn hash_is_stable_and_hex() {
        let h = content_hash("abc");
        assert_eq!(
            h,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn thumbnails_sit_beside_the_database_by_id() {
        let (dir, lib) = open_temp();
        assert_eq!(
            lib.thumbnail_path(7),
            dir.path().join("thumbnails").join("7.png")
        );
    }
}
