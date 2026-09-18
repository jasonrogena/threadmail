use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use std::{fs, io};

use rusqlite::{Connection, params};

use crate::source::BoxError;
use crate::store::{IncomingComment, IncomingCommentStore, OutgoingComment, OutgoingCommentStore};

#[cfg(test)]
mod tests;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("could not create the database's parent directory")]
    Io(#[from] io::Error),
    #[error("could not open the database")]
    Sqlite(#[from] rusqlite::Error),
}

// One file backs both the comment cache and the outgoing-comment outbox:
// the cache is never authoritative and could be wiped on restart without
// harm, but the outbox must survive one, so the file as a whole is durable.
pub struct SqliteStore {
    conn: Mutex<Connection>,
}

impl SqliteStore {
    pub fn open(path: &str) -> Result<SqliteStore, Error> {
        if let Some(parent) = Path::new(path)
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
        {
            fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS messages (slug TEXT NOT NULL, raw BLOB NOT NULL);
             CREATE INDEX IF NOT EXISTS messages_slug ON messages (slug);
             CREATE TABLE IF NOT EXISTS refreshed_at (slug TEXT PRIMARY KEY, at INTEGER NOT NULL);
             CREATE TABLE IF NOT EXISTS pending (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 slug TEXT NOT NULL,
                 from_address TEXT NOT NULL,
                 to_address TEXT NOT NULL,
                 raw BLOB NOT NULL,
                 created_at INTEGER NOT NULL
             );",
        )?;
        Ok(SqliteStore {
            conn: Mutex::new(conn),
        })
    }
}

impl IncomingCommentStore for SqliteStore {
    fn get(&self, slug: &str) -> Result<IncomingComment, BoxError> {
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
        Ok(IncomingComment {
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

impl OutgoingCommentStore for SqliteStore {
    fn enqueue(
        &self,
        slug: &str,
        from_address: &str,
        to_address: &str,
        raw: &[u8],
    ) -> Result<OutgoingComment, BoxError> {
        let conn = self.conn.lock().unwrap();
        let created_at_secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        conn.execute(
            "INSERT INTO pending (slug, from_address, to_address, raw, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![slug, from_address, to_address, raw, created_at_secs as i64],
        )?;
        Ok(OutgoingComment {
            id: conn.last_insert_rowid(),
            slug: slug.to_string(),
            from_address: from_address.to_string(),
            to_address: to_address.to_string(),
            raw: raw.to_vec(),
            // Truncated to whole seconds, matching what a later read of the
            // same row will produce (the column only has second precision).
            created_at: UNIX_EPOCH + Duration::from_secs(created_at_secs),
        })
    }

    fn pending(&self) -> Result<Vec<OutgoingComment>, BoxError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT id, slug, from_address, to_address, raw, created_at FROM pending")?;
        let pending = stmt
            .query_map([], |row| {
                let created_at_secs: i64 = row.get(5)?;
                Ok(OutgoingComment {
                    id: row.get(0)?,
                    slug: row.get(1)?,
                    from_address: row.get(2)?,
                    to_address: row.get(3)?,
                    raw: row.get(4)?,
                    created_at: UNIX_EPOCH + Duration::from_secs(created_at_secs.max(0) as u64),
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(pending)
    }

    fn remove(&self, id: i64) -> Result<(), BoxError> {
        self.conn
            .lock()
            .unwrap()
            .execute("DELETE FROM pending WHERE id = ?1", params![id])?;
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
