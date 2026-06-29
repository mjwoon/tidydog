/// wiki.rs — generate per-file wiki pages, index.md, and log.md.
///
/// All wiki files live under `<root>/.tidydog/wiki/`.

use std::io;
use std::path::Path;

// ── Helpers ───────────────────────────────────────────────────────────────────

fn wiki_dir(root: &Path) -> std::path::PathBuf {
    root.join(".tidydog").join("wiki")
}

fn ensure_wiki_dir(root: &Path) -> io::Result<std::path::PathBuf> {
    let dir = wiki_dir(root);
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn now_iso() -> String {
    // Use SystemTime to produce a simple ISO-8601 UTC timestamp without
    // pulling in a chrono dependency.
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // Format as YYYY-MM-DDTHH:MM:SSZ via manual conversion.
    unix_secs_to_iso(secs)
}

fn unix_secs_to_iso(secs: u64) -> String {
    // Simple integer-based UTC conversion (no leap-second handling needed).
    let s = secs % 60;
    let total_min = secs / 60;
    let m = total_min % 60;
    let total_hours = total_min / 60;
    let h = total_hours % 24;
    let total_days = total_hours / 24;

    // Days since 1970-01-01.
    let (year, month, day) = days_to_ymd(total_days);

    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        year, month, day, h, m, s
    )
}

fn days_to_ymd(mut days: u64) -> (u64, u64, u64) {
    // Gregorian calendar conversion.
    let mut year = 1970u64;
    loop {
        let dy = days_in_year(year);
        if days < dy {
            break;
        }
        days -= dy;
        year += 1;
    }
    let mut month = 1u64;
    loop {
        let dm = days_in_month(year, month);
        if days < dm {
            break;
        }
        days -= dm;
        month += 1;
    }
    (year, month, days + 1)
}

fn is_leap(year: u64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
}

fn days_in_year(year: u64) -> u64 {
    if is_leap(year) { 366 } else { 365 }
}

fn days_in_month(year: u64, month: u64) -> u64 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => if is_leap(year) { 29 } else { 28 },
        _ => 30,
    }
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Write a per-file wiki page at `<root>/.tidydog/wiki/<content_hash>.md`.
pub fn write_file_wiki_page(
    root: &Path,
    content_hash: &str,
    filename: &str,
    current_path: &str,
    summary: Option<&str>,
    topic_tags: &[String],
    language: Option<&str>,
    proposed_dest: Option<&str>,
    indexed_at: Option<u64>,
) -> io::Result<()> {
    let dir = ensure_wiki_dir(root)?;
    let page_path = dir.join(format!("{}.md", content_hash));

    let tags_str = topic_tags.join(", ");
    let lang_str = language.unwrap_or("(not set)");
    let dest_str = proposed_dest.unwrap_or("(not set)");
    let indexed_str = match indexed_at {
        Some(ts) => unix_secs_to_iso(ts),
        None => "(not set)".to_string(),
    };
    let summary_str = summary.unwrap_or("(not set)");

    let content = format!(
        "# {filename}\n\n\
**Path:** `{current_path}`  \n\
**Summary:** {summary_str}  \n\
**Tags:** {tags_str}  \n\
**Language:** {lang_str}  \n\
**Proposed destination:** {dest_str}  \n\
**Indexed:** {indexed_str}  \n",
    );

    std::fs::write(page_path, content)
}

/// Write `<root>/.tidydog/wiki/index.md` from the `files` table.
/// Only rows where `summary IS NOT NULL` are included.
pub fn write_index(root: &Path, conn: &rusqlite::Connection) -> io::Result<()> {
    let dir = ensure_wiki_dir(root)?;
    let index_path = dir.join("index.md");

    let mut stmt = conn
        .prepare(
            "SELECT content_hash, current_path, summary, topic_tags, language, proposed_dest \
             FROM files \
             WHERE summary IS NOT NULL \
             ORDER BY current_path ASC",
        )
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;

    struct Row {
        content_hash: String,
        current_path: String,
        summary: String,
        topic_tags: String,
        language: String,
        proposed_dest: Option<String>,
    }

    let rows: Vec<Row> = stmt
        .query_map([], |row| {
            Ok(Row {
                content_hash: row.get(0)?,
                current_path: row.get(1)?,
                summary: row.get(2)?,
                topic_tags: row.get::<_, Option<String>>(3)?.unwrap_or_default(),
                language: row.get::<_, Option<String>>(4)?.unwrap_or_default(),
                proposed_dest: row.get(5)?,
            })
        })
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?
        .filter_map(|r| r.ok())
        .collect();

    let mut table_rows = String::new();
    for row in &rows {
        let filename = std::path::Path::new(&row.current_path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(&row.current_path);

        // Parse topic_tags JSON array to a comma-separated string.
        let tags_display = parse_tags_json(&row.topic_tags);

        let dest_display = row
            .proposed_dest
            .as_deref()
            .unwrap_or("(not set)");

        // Escape pipe characters in cell values.
        let summary_cell = row.summary.replace('|', "\\|");
        let tags_cell = tags_display.replace('|', "\\|");
        let lang_cell = row.language.replace('|', "\\|");
        let dest_cell = dest_display.replace('|', "\\|");

        table_rows.push_str(&format!(
            "| [{filename}]({content_hash}.md) | {summary_cell} | {tags_cell} | {lang_cell} | {dest_cell} |\n",
            content_hash = row.content_hash,
        ));
    }

    let content = format!(
        "# TidyDog Index\n\nGenerated: {ts}\n\n\
| File | Summary | Tags | Language | Proposed Dest |\n\
|------|---------|------|----------|---------------|\n\
{table_rows}",
        ts = now_iso(),
    );

    std::fs::write(index_path, content)
}

/// Write `<root>/.tidydog/wiki/log.md` from the `journal` table.
/// Ordered by entry_id DESC, limit 200.
pub fn write_log(root: &Path, conn: &rusqlite::Connection) -> io::Result<()> {
    let dir = ensure_wiki_dir(root)?;
    let log_path = dir.join("log.md");

    let mut stmt = conn
        .prepare(
            "SELECT entry_id, plan_id, action, from_path, to_path, executed_at, completed_at \
             FROM journal \
             ORDER BY entry_id DESC \
             LIMIT 200",
        )
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;

    struct JournalRow {
        entry_id: i64,
        plan_id: String,
        action: String,
        from_path: String,
        to_path: String,
        executed_at: i64,
        completed_at: Option<i64>,
    }

    let rows: Vec<JournalRow> = stmt
        .query_map([], |row| {
            Ok(JournalRow {
                entry_id: row.get(0)?,
                plan_id: row.get(1)?,
                action: row.get(2)?,
                from_path: row.get(3)?,
                to_path: row.get(4)?,
                executed_at: row.get(5)?,
                completed_at: row.get(6)?,
            })
        })
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?
        .filter_map(|r| r.ok())
        .collect();

    let mut table_rows = String::new();
    for row in &rows {
        let executed = unix_secs_to_iso(row.executed_at as u64);
        let completed = match row.completed_at {
            Some(ts) => unix_secs_to_iso(ts as u64),
            None => "(pending)".to_string(),
        };
        table_rows.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {} |\n",
            row.entry_id,
            row.plan_id,
            row.action,
            row.from_path,
            row.to_path,
            executed,
            completed,
        ));
    }

    let content = format!(
        "# TidyDog Operation Log\n\nGenerated: {ts}\n\n\
| Entry | Plan | Action | From | To | Executed | Completed |\n\
|-------|------|--------|------|----|----------|-----------|\n\
{table_rows}",
        ts = now_iso(),
    );

    std::fs::write(log_path, content)
}

// ── Internal helpers ──────────────────────────────────────────────────────────

/// Parse a JSON array string like `["tag1","tag2"]` into a comma-separated string.
/// Falls back to returning the raw string if it isn't valid JSON.
fn parse_tags_json(json_str: &str) -> String {
    let trimmed = json_str.trim();
    if trimmed.starts_with('[') {
        // Minimal JSON array parser: extract quoted strings.
        let inner = trimmed.trim_start_matches('[').trim_end_matches(']');
        let tags: Vec<&str> = inner
            .split(',')
            .map(|s| s.trim().trim_matches('"'))
            .filter(|s| !s.is_empty())
            .collect();
        tags.join(", ")
    } else {
        trimmed.to_string()
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn tmp_root() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "tidydog_wiki_test_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn test_write_file_wiki_page() {
        let root = tmp_root();
        let tags = vec!["세금".to_string(), "tax".to_string()];
        write_file_wiki_page(
            &root,
            "abc123",
            "invoice.pdf",
            "/home/user/invoice.pdf",
            Some("An invoice document"),
            &tags,
            Some("ko"),
            Some("문서/세금"),
            Some(1_700_000_000),
        )
        .unwrap();

        let page = std::fs::read_to_string(root.join(".tidydog/wiki/abc123.md")).unwrap();
        assert!(page.contains("# invoice.pdf"));
        assert!(page.contains("**Path:** `/home/user/invoice.pdf`"));
        assert!(page.contains("세금"));
        assert!(page.contains("문서/세금"));
    }

    #[test]
    fn test_write_file_wiki_page_no_optionals() {
        let root = tmp_root();
        write_file_wiki_page(
            &root,
            "def456",
            "notes.txt",
            "/tmp/notes.txt",
            None,
            &[],
            None,
            None,
            None,
        )
        .unwrap();

        let page = std::fs::read_to_string(root.join(".tidydog/wiki/def456.md")).unwrap();
        assert!(page.contains("(not set)"));
    }

    #[test]
    fn test_parse_tags_json() {
        assert_eq!(parse_tags_json(r#"["tag1","tag2"]"#), "tag1, tag2");
        assert_eq!(parse_tags_json("[]"), "");
        assert_eq!(parse_tags_json("raw_value"), "raw_value");
    }

    #[test]
    fn test_unix_secs_to_iso() {
        // 2023-11-14T22:13:20Z = 1_700_000_000
        assert_eq!(unix_secs_to_iso(1_700_000_000), "2023-11-14T22:13:20Z");
        // Unix epoch
        assert_eq!(unix_secs_to_iso(0), "1970-01-01T00:00:00Z");
    }

    #[test]
    fn test_write_index_and_log_empty_db() {
        let root = tmp_root();
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE files (
                id INTEGER PRIMARY KEY,
                content_hash TEXT NOT NULL,
                current_path TEXT NOT NULL UNIQUE,
                size INTEGER NOT NULL DEFAULT 0,
                mtime INTEGER NOT NULL DEFAULT 0,
                summary TEXT,
                topic_tags TEXT,
                language TEXT,
                proposed_dest TEXT
            );
            CREATE TABLE journal (
                entry_id INTEGER PRIMARY KEY AUTOINCREMENT,
                plan_id TEXT NOT NULL,
                op_id TEXT NOT NULL,
                action TEXT NOT NULL,
                content_hash TEXT NOT NULL,
                from_path TEXT NOT NULL,
                to_path TEXT NOT NULL,
                executed_at INTEGER NOT NULL,
                completed_at INTEGER
            );",
        )
        .unwrap();

        write_index(&root, &conn).unwrap();
        write_log(&root, &conn).unwrap();

        let index = std::fs::read_to_string(root.join(".tidydog/wiki/index.md")).unwrap();
        assert!(index.contains("# TidyDog Index"));

        let log = std::fs::read_to_string(root.join(".tidydog/wiki/log.md")).unwrap();
        assert!(log.contains("# TidyDog Operation Log"));
    }
}
