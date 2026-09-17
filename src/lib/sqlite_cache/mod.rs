use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, params};

use crate::cache::{CacheEntry, CommentCache};
use crate::source::BoxError;

#[cfg(test)]
mod tests;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("could not open the comment cache database")]
    Sqlite(#[from] rusqlite::Error),
}

// In-memory and lost on restart by design: the mailing list is the only
// authoritative store, this only avoids an IMAP round trip on every page view.
pub struct SqliteCache {
    conn: Mutex<Connection>,
}

impl SqliteCache {
    pub fn open_in_memory() -> Result<SqliteCache, Error> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(
            "CREATE TABLE messages (slug TEXT NOT NULL, raw BLOB NOT NULL);
             CREATE INDEX messages_slug ON messages (slug);
             CREATE TABLE refreshed_at (slug TEXT PRIMARY KEY, at INTEGER NOT NULL);",
        )?;
        Ok(SqliteCache {
            conn: Mutex::new(conn),
        })
    }
}

impl CommentCache for SqliteCache {
    fn get(&self, slug: &str) -> Result<CacheEntry, BoxError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT raw FROM messages WHERE slug = ?1")?;
        let raw_messages = stmt
            .query_map(params![slug], |row| row.get::<_, Vec<u8>>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        let refreshed_at = conn
            .query_row(
                "SELECT at FROM refreshed_at WHERE slug = ?1",
                params![slug],
                |row| row.get::<_, i64>(0),
            )
            .ok()
            .map(|secs| UNIX_EPOCH + Duration::from_secs(secs.max(0) as u64));
        Ok(CacheEntry {
            raw_messages,
            refreshed_at,
        })
    }

    fn store(&self, slug: &str, raw_messages: &[Vec<u8>]) -> Result<(), BoxError> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM messages WHERE slug = ?1", params![slug])?;
        for raw in raw_messages {
            tx.execute(
                "INSERT INTO messages (slug, raw) VALUES (?1, ?2)",
                params![slug, raw],
            )?;
        }
        touch(&tx, slug)?;
        tx.commit()?;
        Ok(())
    }

    fn mark_refresh_attempted(&self, slug: &str) -> Result<(), BoxError> {
        touch(&self.conn.lock().unwrap(), slug)?;
        Ok(())
    }

    fn invalidate(&self, slug: &str) -> Result<(), BoxError> {
        self.conn
            .lock()
            .unwrap()
            .execute("DELETE FROM refreshed_at WHERE slug = ?1", params![slug])?;
        Ok(())
    }
}

fn touch(conn: &Connection, slug: &str) -> Result<(), rusqlite::Error> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    conn.execute(
        "INSERT INTO refreshed_at (slug, at) VALUES (?1, ?2)
         ON CONFLICT(slug) DO UPDATE SET at = excluded.at",
        params![slug, now as i64],
    )?;
    Ok(())
}
