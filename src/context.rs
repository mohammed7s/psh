use rusqlite::{Connection, Result, params};
use dirs::home_dir;
use std::fs;

#[derive(Clone)]
pub struct Entry {
    pub cwd: String,
    pub command: String,
    pub output: String,
    pub exit_code: i32,
}

pub struct Db {
    conn: Connection,
}

impl Db {
    pub fn open() -> Self {
        let dir = home_dir().unwrap_or_default().join(".psh");
        fs::create_dir_all(&dir).ok();
        let path = dir.join("history.db");
        let conn = Connection::open(&path).expect("failed to open history db");

        conn.execute_batch("
            CREATE TABLE IF NOT EXISTS history (
                id        INTEGER PRIMARY KEY,
                timestamp DATETIME DEFAULT CURRENT_TIMESTAMP,
                session   TEXT,
                cwd       TEXT,
                command   TEXT,
                output    TEXT,
                exit_code INTEGER,
                was_nl    INTEGER DEFAULT 0,
                nl_input  TEXT
            );
        ").ok();

        Self { conn }
    }

    pub fn insert(&self, session: &str, entry: &Entry) {
        self.conn.execute(
            "INSERT INTO history (session, cwd, command, output, exit_code)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![session, entry.cwd, entry.command, entry.output, entry.exit_code],
        ).ok();
    }

    // Returns last N entries for context injection into AI prompt
    pub fn recent(&self, session: &str, limit: usize) -> Vec<Entry> {
        let mut stmt = self.conn.prepare(
            "SELECT cwd, command, output, exit_code FROM history
             WHERE session = ?1
             ORDER BY timestamp DESC LIMIT ?2"
        ).unwrap();

        stmt.query_map(params![session, limit as i64], |row| {
            Ok(Entry {
                cwd:       row.get(0)?,
                command:   row.get(1)?,
                output:    row.get(2)?,
                exit_code: row.get(3)?,
            })
        })
        .unwrap()
        .filter_map(Result::ok)
        .collect()
    }
}
