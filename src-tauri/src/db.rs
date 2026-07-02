use rusqlite::Connection;
use std::path::Path;

/// 기존 Connection에 스키마를 적용한다. 테스트용 in-memory DB에서도 사용.
pub fn init_schema(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         CREATE TABLE IF NOT EXISTS files (
             id            INTEGER PRIMARY KEY,
             content_hash  TEXT    NOT NULL,
             current_path  TEXT    NOT NULL UNIQUE,
             size          INTEGER NOT NULL,
             mtime         INTEGER NOT NULL,
             created_at    INTEGER,
             mime          TEXT,
             ext           TEXT,
             read_level    TEXT    CHECK(read_level IN ('content','metadata')),
             summary       TEXT, topic_tags TEXT, language TEXT, cluster_id INTEGER,
             dup_group     TEXT, proposed_dest TEXT, risk_score REAL,
             last_action   TEXT, last_seen INTEGER, indexed_at INTEGER
         );
         CREATE INDEX IF NOT EXISTS idx_files_hash ON files(content_hash);
         CREATE TABLE IF NOT EXISTS plans (
             plan_id      TEXT    PRIMARY KEY,
             created_at   INTEGER NOT NULL,
             status       TEXT    NOT NULL CHECK(status IN ('proposed','confirmed','executed','undone','expired')),
             risk_score   REAL    NOT NULL DEFAULT 0.0,
             preview      TEXT    NOT NULL CHECK(preview IN ('auto','standard','full_review')),
             scheme       TEXT,
             rule_summary TEXT,
             ops_json     TEXT    NOT NULL
         );
         CREATE TABLE IF NOT EXISTS journal (
             entry_id     INTEGER PRIMARY KEY AUTOINCREMENT,
             plan_id      TEXT    NOT NULL,
             op_id        TEXT    NOT NULL,
             action       TEXT    NOT NULL CHECK(action IN ('move','stage','rename')),
             content_hash TEXT    NOT NULL,
             from_path    TEXT    NOT NULL,
             to_path      TEXT    NOT NULL,
             executed_at  INTEGER NOT NULL,
             completed_at INTEGER,
             undoable     INTEGER NOT NULL DEFAULT 1,
             undone_at    INTEGER
         );
         CREATE INDEX IF NOT EXISTS idx_journal_plan ON journal(plan_id);
         CREATE INDEX IF NOT EXISTS idx_journal_inflight ON journal(completed_at) WHERE completed_at IS NULL;
         CREATE TABLE IF NOT EXISTS staging (
             content_hash  TEXT    PRIMARY KEY,
             original_path TEXT    NOT NULL,
             staged_path   TEXT    NOT NULL,
             staged_at     INTEGER NOT NULL,
             purge_after   INTEGER
         );
         CREATE TABLE IF NOT EXISTS plan_ops (
             op_id        TEXT    NOT NULL PRIMARY KEY,
             plan_id      TEXT    NOT NULL REFERENCES plans(plan_id),
             seq          INTEGER NOT NULL,
             action       TEXT    NOT NULL CHECK(action IN ('move','stage','rename')),
             content_hash TEXT    NOT NULL,
             from_path    TEXT    NOT NULL,
             to_path      TEXT    NOT NULL,
             conflict     TEXT    NOT NULL DEFAULT 'none' CHECK(conflict IN ('none','rename')),
             reason       TEXT
         );
         CREATE INDEX IF NOT EXISTS idx_plan_ops_plan ON plan_ops(plan_id, seq);
         CREATE TABLE IF NOT EXISTS settings (
             key   TEXT PRIMARY KEY,
             value TEXT NOT NULL
         );
         CREATE TABLE IF NOT EXISTS summaries (
             content_hash TEXT PRIMARY KEY,
             summary      TEXT NOT NULL,
             topic_tags   TEXT NOT NULL,
             language     TEXT NOT NULL,
             created_at   INTEGER NOT NULL
         );",
    )
    .map_err(|e| e.to_string())?;
    // 기존 DB 마이그레이션: completed_at 컬럼이 없으면 추가.
    let _ = conn.execute_batch("ALTER TABLE journal ADD COLUMN completed_at INTEGER;");
    Ok(())
}

pub fn init_db(app_data_dir: &Path) -> Result<Connection, String> {
    let db_path = app_data_dir.join("index.db");
    let conn = Connection::open(&db_path).map_err(|e| e.to_string())?;
    init_schema(&conn)?;
    Ok(conn)
}
