use rusqlite::{params, Connection};
use std::path::Path;

const SCHEMA: &str = "
PRAGMA journal_mode = WAL;
PRAGMA foreign_keys = ON;
CREATE TABLE IF NOT EXISTS users (id INTEGER PRIMARY KEY, username TEXT UNIQUE NOT NULL, password TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS sessions (token_hash TEXT PRIMARY KEY, user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE, expires INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS enroll_keys (id INTEGER PRIMARY KEY, name TEXT NOT NULL, key_hash TEXT UNIQUE NOT NULL, created INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS hosts (
  id INTEGER PRIMARY KEY, machine_id TEXT UNIQUE NOT NULL, secret_hash TEXT NOT NULL, name TEXT NOT NULL,
  info TEXT NOT NULL DEFAULT '{}', last_seen INTEGER NOT NULL DEFAULT 0, created INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS apps (
  id INTEGER PRIMARY KEY, host_id INTEGER NOT NULL REFERENCES hosts(id) ON DELETE CASCADE, name TEXT NOT NULL,
  spec TEXT NOT NULL, last_sha TEXT, deploy_key TEXT, webhook_secret TEXT NOT NULL, created INTEGER NOT NULL,
  UNIQUE (host_id, name));
CREATE TABLE IF NOT EXISTS env (app_id INTEGER NOT NULL REFERENCES apps(id) ON DELETE CASCADE, key TEXT NOT NULL, value TEXT NOT NULL, PRIMARY KEY (app_id, key));
CREATE TABLE IF NOT EXISTS ssh_keys (id INTEGER PRIMARY KEY, host_id INTEGER NOT NULL REFERENCES hosts(id) ON DELETE CASCADE, name TEXT NOT NULL, pubkey TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS deployments (
  id INTEGER PRIMARY KEY, app_id INTEGER NOT NULL REFERENCES apps(id) ON DELETE CASCADE, sha TEXT, trigger TEXT NOT NULL,
  status TEXT NOT NULL, log TEXT NOT NULL DEFAULT '', started INTEGER NOT NULL, finished INTEGER);
CREATE INDEX IF NOT EXISTS deployments_app ON deployments (app_id, id);
CREATE TABLE IF NOT EXISTS audit (id INTEGER PRIMARY KEY, ts INTEGER NOT NULL, action TEXT NOT NULL, detail TEXT NOT NULL);
";

pub fn open(path: &Path) -> rusqlite::Result<Connection> {
    let c = Connection::open(path)?;
    c.execute_batch(SCHEMA)?;
    // A restart interrupts any deployment that was waiting on an agent.
    c.execute("UPDATE deployments SET status='failed', log=log || '\ninterrupted: server restarted', finished=?1 WHERE status='running'", params![crate::now()])?;
    c.execute("DELETE FROM sessions WHERE expires < ?1", params![crate::now()])?;
    Ok(c)
}

pub fn audit(db: &Connection, action: &str, detail: &str) {
    let _ = db.execute("INSERT INTO audit (ts, action, detail) VALUES (?1, ?2, ?3)", params![crate::now(), action, detail]);
}
