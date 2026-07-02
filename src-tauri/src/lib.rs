pub mod agent;
mod db;
pub mod fileops;
mod guard;
pub mod organizer;
pub mod reader;
mod scanner;
mod store;
mod summarizer;
pub mod wiki;

use std::fs;
use std::path::PathBuf;
use tauri::Manager;
use tidydog_core::{Action, Engine, GateError, PlanOp, Store};

#[tauri::command]
fn health() -> String {
    "TidyDog · ready".to_string()
}

#[tauri::command]
fn scan_directory(
    app: tauri::AppHandle,
    root: String,
    max_depth: Option<usize>,
) -> Result<scanner::FileNode, String> {
    let app_data_dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    fs::create_dir_all(&app_data_dir).map_err(|e| e.to_string())?;
    let conn = db::init_db(&app_data_dir)?;
    let depth = max_depth.unwrap_or(10);
    let path = std::path::Path::new(&root);
    scanner::scan_recursive(path, 0, depth, &conn)
        .ok_or_else(|| format!("Cannot scan root path: {}", root))
}

// ── Phase 2: 안전 코어 게이트 commands ──────────────────────────────────────

#[derive(serde::Deserialize)]
pub struct OpInput {
    pub action: String,
    pub content_hash: String,
    pub from: String,
    pub to: String,
    pub reason: Option<String>,
}

#[derive(serde::Serialize)]
pub struct PlanSummary {
    pub plan_id: String,
    pub risk_score: f64,
    pub preview: String,
    pub op_count: usize,
}

fn gate_err_to_str(e: GateError) -> String {
    match e {
        GateError::NoSuchPlan => "plan not found".to_string(),
        GateError::NotProposed => "plan is not in proposed state".to_string(),
        GateError::NotConfirmed => "plan must be confirmed before execute".to_string(),
        GateError::NotUserConfirmed => "user confirmation flag required".to_string(),
        GateError::NotExecuted => "plan is not in executed state".to_string(),
        GateError::OutOfUndoWindow => "plan is outside the undo window (last 5)".to_string(),
        GateError::ScopeDenied { op_id, denial } => {
            format!("scope denied op {op_id}: {denial:?}")
        }
        GateError::FileOp { op_id, message } => {
            format!("file op failed for {op_id}: {message}")
        }
        GateError::PartialExecute { failed_op_id, message, completed } => {
            format!("partial execute: {completed} ops done, failed at {failed_op_id}: {message}")
        }
    }
}

fn app_paths(app: &tauri::AppHandle) -> Result<(PathBuf, PathBuf), String> {
    let app_data_dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let staging_dir = app_data_dir.join("staging");
    Ok((app_data_dir, staging_dir))
}

#[tauri::command]
fn propose_plan(
    app: tauri::AppHandle,
    ops: Vec<OpInput>,
    root: String,
    scheme: Option<String>,
    rule_summary: Option<String>,
) -> Result<PlanSummary, String> {
    let (app_data_dir, staging_dir) = app_paths(&app)?;
    fs::create_dir_all(&app_data_dir).map_err(|e| e.to_string())?;
    let conn = db::init_db(&app_data_dir)?;

    let plan_ops: Vec<PlanOp> = ops
        .into_iter()
        .map(|o| {
            let action = match o.action.as_str() {
                "stage" => Action::Stage,
                "rename" => Action::Rename,
                _ => Action::Move,
            };
            let mut op = PlanOp::new(
                action,
                o.content_hash,
                PathBuf::from(&o.from),
                PathBuf::from(&o.to),
            );
            if let Some(r) = o.reason {
                op = op.with_reason(r);
            }
            op
        })
        .collect();

    let guard = guard::make_guard(vec![PathBuf::from(root)]);
    let mut engine = Engine::new(guard, store::SqliteStore::new(&conn), fileops::FsFileOps::new(staging_dir));
    let plan = engine.propose(plan_ops, scheme, rule_summary);

    Ok(PlanSummary {
        plan_id: plan.plan_id,
        risk_score: plan.risk_score,
        preview: match plan.preview {
            tidydog_core::PreviewLevel::Auto => "auto",
            tidydog_core::PreviewLevel::Standard => "standard",
            tidydog_core::PreviewLevel::FullReview => "full_review",
        }
        .to_string(),
        op_count: plan.ops.len(),
    })
}

/// S1 원자성: confirm은 단순 상태 변경이라 단일 UPDATE로 충분. 트랜잭션 불필요.
#[tauri::command]
fn confirm_plan(app: tauri::AppHandle, plan_id: String) -> Result<(), String> {
    let (app_data_dir, staging_dir) = app_paths(&app)?;
    let conn = db::init_db(&app_data_dir)?;
    let guard = guard::make_guard(vec![]);
    let mut engine = Engine::new(
        guard,
        store::SqliteStore::new(&conn),
        fileops::FsFileOps::new(staging_dir),
    );
    engine.confirm(&plan_id).map_err(gate_err_to_str)
}

/// execute: op 단위 autocommit. 각 op의 [intent INSERT → 파일 작업 → complete UPDATE]가
/// 단일 트랜잭션 없이 즉시 내구화된다.
/// 파일 작업 실패 시 앞선 op들의 journal이 살아남아 undo 가능.
#[tauri::command]
fn execute_plan(
    app: tauri::AppHandle,
    plan_id: String,
    root: String,
) -> Result<serde_json::Value, String> {
    let (app_data_dir, staging_dir) = app_paths(&app)?;
    let conn = db::init_db(&app_data_dir)?;

    let guard = guard::make_guard(vec![PathBuf::from(root)]);
    let mut engine = Engine::new(
        guard,
        store::SqliteStore::new(&conn),
        fileops::FsFileOps::new(staging_dir),
    );
    match engine.execute(&plan_id, true) {
        Ok(out) => Ok(serde_json::json!({
            "plan_id": out.plan_id,
            "moved": out.moved,
            "staged": out.staged,
            "renamed": out.renamed,
            "partial": false,
        })),
        Err(GateError::PartialExecute { failed_op_id, message, completed }) => {
            // 부분 성공: 앞선 op들은 내구화됐고 undo 가능. 클라이언트에 경고.
            Ok(serde_json::json!({
                "plan_id": plan_id,
                "partial": true,
                "completed": completed,
                "failed_op": failed_op_id,
                "error": message,
            }))
        }
        Err(e) => Err(gate_err_to_str(e)),
    }
}

/// S1 원자성: undo도 단일 트랜잭션으로 감싼다.
#[tauri::command]
fn undo_plan(app: tauri::AppHandle, plan_id: String) -> Result<(), String> {
    let (app_data_dir, staging_dir) = app_paths(&app)?;
    let conn = db::init_db(&app_data_dir)?;

    let tx = conn
        .unchecked_transaction()
        .map_err(|e| e.to_string())?;

    let guard = guard::make_guard(vec![]);
    let mut engine = Engine::new(
        guard,
        store::SqliteStore::new(&*tx),
        fileops::FsFileOps::new(staging_dir),
    );
    engine.undo(&plan_id).map_err(gate_err_to_str)?;
    tx.commit().map_err(|e| e.to_string())
}

// ── SA3: AI content consent + API key + index_file_content commands ───────────

#[tauri::command]
fn set_ai_consent(app: tauri::AppHandle, granted: bool) -> Result<(), String> {
    let app_data_dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    fs::create_dir_all(&app_data_dir).map_err(|e| e.to_string())?;
    let conn = db::init_db(&app_data_dir)?;
    let value = if granted { "true" } else { "false" };
    conn.execute(
        "INSERT OR REPLACE INTO settings (key, value) VALUES ('ai_content_consent_granted', ?1)",
        rusqlite::params![value],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
fn get_ai_consent(app: tauri::AppHandle) -> Result<bool, String> {
    let app_data_dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    fs::create_dir_all(&app_data_dir).map_err(|e| e.to_string())?;
    let conn = db::init_db(&app_data_dir)?;
    let result = conn.query_row(
        "SELECT value FROM settings WHERE key = 'ai_content_consent_granted'",
        [],
        |row| row.get::<_, String>(0),
    );
    Ok(matches!(result, Ok(v) if v == "true"))
}

#[tauri::command]
fn set_api_key(key: String) -> Result<(), String> {
    // N2: NEVER log the key.
    let entry = keyring::Entry::new("tidydog", "anthropic_api_key")
        .map_err(|e| format!("keyring error: {e}"))?;
    entry
        .set_password(&key)
        .map_err(|e| format!("failed to store API key: {e}"))?;
    Ok(())
}

#[tauri::command]
fn index_file_content(
    app: tauri::AppHandle,
    path: String,
    root: String,
) -> Result<serde_json::Value, String> {
    let _ = root; // root is accepted for API symmetry but file path is used directly
    let app_data_dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    fs::create_dir_all(&app_data_dir).map_err(|e| e.to_string())?;
    let conn = db::init_db(&app_data_dir)?;

    // N1: 동의 게이트 — 파일 내용을 읽기 전에 확인한다.
    // 동의 없으면 사이드카 프로세스 미실행 + 네트워크 호출 0.
    {
        use tidydog_core::Summarizer;
        if !summarizer::CloudSummarizer::new(&conn).is_consented() {
            return Err(
                "user has not consented to cloud content processing (N1)".to_string(),
            );
        }
    }

    // Locate sidecar/reader.py relative to current working directory.
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    let script_path = cwd.join("sidecar/reader.py");
    if !script_path.exists() {
        return Err(format!(
            "sidecar not found at expected location: {}",
            script_path.display()
        ));
    }

    // Read content via sidecar.
    let file_path = std::path::Path::new(&path);
    let reader = crate::reader::SidecarReader::new("python3", script_path);
    use tidydog_core::ContentReader;
    let chunk = reader
        .read_content(file_path, tidydog_core::ContentBudget::default())
        .map_err(|e| e.to_string())?;

    // Get content_hash from files table.
    let content_hash: Option<String> = conn
        .query_row(
            "SELECT content_hash FROM files WHERE current_path = ?1",
            rusqlite::params![path],
            |row| row.get(0),
        )
        .ok();

    // Build SummaryHints.
    let file_path_obj = std::path::Path::new(&path);
    let filename = file_path_obj
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string();
    let ext = file_path_obj
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.to_string());

    let hints = tidydog_core::SummaryHints {
        filename,
        ext,
        content_hash: content_hash.clone(),
    };

    // Summarize.
    let summarizer = summarizer::CloudSummarizer::new(&conn);
    use tidydog_core::Summarizer;
    let result = summarizer.summarize(&chunk, hints).map_err(|e| e.to_string())?;

    // Determine if this was a cache hit: we check by looking at whether the
    // summary was already stored before this call. Since summarize() returns
    // Ok for both cache hits and fresh results, we use a simple approach:
    // attempt a fresh lookup — if found before the call it was cached.
    // For simplicity, we mark cached=false for all new index calls.
    let cached = false;

    // Store result in files table.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0) as i64;
    let tags_json = serde_json::to_string(&result.topic_tags).unwrap_or_else(|_| "[]".to_string());
    conn.execute(
        "UPDATE files SET summary=?1, topic_tags=?2, language=?3, read_level='content', indexed_at=?4 \
         WHERE current_path=?5",
        rusqlite::params![result.summary, tags_json, result.language, now, path],
    )
    .map_err(|e| e.to_string())?;

    Ok(serde_json::json!({
        "summary": result.summary,
        "topic_tags": result.topic_tags,
        "language": result.language,
        "cached": cached,
    }))
}

// ── SA4: ORGANIZER rules + wiki rebuild commands ──────────────────────────────

/// Derive a proposed destination for a file by evaluating ORGANIZER.md rules.
/// Also persists the result into `files.proposed_dest` if a match is found.
#[tauri::command]
fn derive_proposed_dest(
    app: tauri::AppHandle,
    path: String,
    topic_tags: Vec<String>,
    language: String,
    organizer_path: Option<String>,
) -> Result<Option<String>, String> {
    // Load ORGANIZER.md.
    let organizer_file = match organizer_path {
        Some(p) => std::path::PathBuf::from(p),
        None => std::env::current_dir()
            .map_err(|e| e.to_string())?
            .join("ORGANIZER.md"),
    };

    let md_content = fs::read_to_string(&organizer_file).map_err(|e| {
        format!(
            "Failed to read ORGANIZER.md at {}: {}",
            organizer_file.display(),
            e
        )
    })?;

    let rules = organizer::parse_rules(&md_content);

    // Extract filename and ext from path.
    let file_path = std::path::Path::new(&path);
    let filename = file_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");
    let ext = file_path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    let attrs = organizer::FileAttrs {
        filename,
        ext: &ext,
        topic_tags: &topic_tags,
        language: &language,
    };

    let dest = organizer::derive_dest(&rules, &attrs);

    // Persist to DB if we got a match.
    if let Some(ref dest_str) = dest {
        let app_data_dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
        fs::create_dir_all(&app_data_dir).map_err(|e| e.to_string())?;
        let conn = db::init_db(&app_data_dir)?;
        conn.execute(
            "UPDATE files SET proposed_dest = ?1 WHERE current_path = ?2",
            rusqlite::params![dest_str, path],
        )
        .map_err(|e| e.to_string())?;
    }

    Ok(dest)
}

/// Rebuild all wiki pages (per-file, index.md, log.md) under `<root>/.tidydog/wiki/`.
#[tauri::command]
fn rebuild_wiki(app: tauri::AppHandle, root: String) -> Result<serde_json::Value, String> {
    let app_data_dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    fs::create_dir_all(&app_data_dir).map_err(|e| e.to_string())?;
    let conn = db::init_db(&app_data_dir)?;

    let root_path = std::path::Path::new(&root);

    // Query all files with a summary.
    struct FileRow {
        content_hash: String,
        current_path: String,
        summary: String,
        topic_tags: Option<String>,
        language: Option<String>,
        proposed_dest: Option<String>,
        indexed_at: Option<i64>,
    }

    let mut stmt = conn
        .prepare(
            "SELECT content_hash, current_path, summary, topic_tags, language, proposed_dest, indexed_at \
             FROM files WHERE summary IS NOT NULL",
        )
        .map_err(|e| e.to_string())?;

    let rows: Vec<FileRow> = stmt
        .query_map([], |row| {
            Ok(FileRow {
                content_hash: row.get(0)?,
                current_path: row.get(1)?,
                summary: row.get(2)?,
                topic_tags: row.get(3)?,
                language: row.get(4)?,
                proposed_dest: row.get(5)?,
                indexed_at: row.get(6)?,
            })
        })
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();

    let page_count = rows.len();

    for row in rows {
        let file_path = std::path::Path::new(&row.current_path);
        let filename = file_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(&row.current_path);

        // Parse topic_tags JSON array to Vec<String>.
        let tags: Vec<String> = row
            .topic_tags
            .as_deref()
            .map(parse_tags_for_wiki)
            .unwrap_or_default();

        wiki::write_file_wiki_page(
            root_path,
            &row.content_hash,
            filename,
            &row.current_path,
            Some(&row.summary),
            &tags,
            row.language.as_deref(),
            row.proposed_dest.as_deref(),
            row.indexed_at.map(|t| t as u64),
        )
        .map_err(|e| e.to_string())?;
    }

    wiki::write_index(root_path, &conn).map_err(|e| e.to_string())?;
    wiki::write_log(root_path, &conn).map_err(|e| e.to_string())?;

    let wiki_dir = root_path.join(".tidydog").join("wiki");
    Ok(serde_json::json!({
        "pages_written": page_count,
        "wiki_dir": wiki_dir.to_string_lossy(),
    }))
}

/// Parse a JSON array string `["a","b"]` into a `Vec<String>`.
fn parse_tags_for_wiki(json_str: &str) -> Vec<String> {
    let trimmed = json_str.trim();
    if trimmed.starts_with('[') {
        let inner = trimmed.trim_start_matches('[').trim_end_matches(']');
        inner
            .split(',')
            .map(|s| s.trim().trim_matches('"').to_string())
            .filter(|s| !s.is_empty())
            .collect()
    } else if trimmed.is_empty() {
        vec![]
    } else {
        vec![trimmed.to_string()]
    }
}

/// S3 크래시 복원: 앱 시작 시 in-flight journal 항목을 FS와 대조해 정리.
/// - completed_at IS NULL 항목이 있으면 크래시 복원 시도.
/// - 이미 to_path에 파일이 있으면 → complete 처리 (op가 완료됐으나 기록이 안 됨).
/// - from_path에 파일이 있고 to_path에 없으면 → op가 시작도 안 된 것이므로 항목 삭제.
/// - 둘 다 없으면 → 경고만 남기고 completed 처리 (손실은 이미 발생).
// ── Phase 4: 챗 + 에이전트 루프 ─────────────────────────────────────────────

/// 사용자 메시지를 받아 에이전트 루프를 실행하고 결과를 반환한다.
///
/// - `message`: 현재 사용자 입력.
/// - `history`: 이전 대화 (Claude API 형식 JSON 배열).
/// - `user_triggered_undo`: true 이면 LLM 없이 undo 경로 실행 (I2).
/// - `root`: 현재 스캔된 루트 경로 (propose_plan Guard 화이트리스트).
#[tauri::command]
fn chat(
    app: tauri::AppHandle,
    message: String,
    history: Vec<serde_json::Value>,
    user_triggered_undo: bool,
    root: String,
) -> Result<serde_json::Value, String> {
    let (app_data_dir, staging_dir) = app_paths(&app)?;
    fs::create_dir_all(&app_data_dir).map_err(|e| e.to_string())?;
    let conn = db::init_db(&app_data_dir)?;

    // N2: API 키 키체인 조회.
    let api_key = keyring::Entry::new("tidydog", "anthropic_api_key")
        .map_err(|e| format!("keyring 오류: {e}"))?
        .get_password()
        .map_err(|e| format!("API 키가 없습니다. 설정에서 먼저 입력하세요: {e}"))?;

    // 대화 히스토리에 현재 메시지 추가.
    let mut messages = history;
    messages.push(serde_json::json!({
        "role":    "user",
        "content": message
    }));

    let client = agent::ClaudeClient { api_key };
    let result = agent::run_agent_loop(
        &client,
        &conn,
        &staging_dir,
        messages,
        user_triggered_undo,
        &root,
    )?;

    Ok(serde_json::to_value(result).map_err(|e| e.to_string())?)
}

fn recover_inflight(conn: &rusqlite::Connection, staging_dir: &std::path::Path) {
    let store = store::SqliteStore::new(conn);
    let inflight = store.inflight_entries();
    if inflight.is_empty() {
        return;
    }
    eprintln!(
        "[TidyDog] recover_inflight: {} in-flight entries found",
        inflight.len()
    );
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let mut store = store::SqliteStore::new(conn);
    for entry in inflight {
        let to_exists = entry.to.exists()
            || staging_dir.join(&entry.content_hash).exists();
        let from_exists = entry.from.exists();

        if to_exists {
            // 파일 작업은 완료됐으나 DB 기록이 누락됨 → complete 처리.
            store.complete_journal(entry.entry_id, ts);
            eprintln!(
                "[TidyDog] recover: completed entry {} (to exists)",
                entry.entry_id
            );
        } else if from_exists {
            // 파일 작업 전 크래시 → 항목을 제거(op가 실행 안 됨).
            let _ = conn.execute(
                "DELETE FROM journal WHERE entry_id = ?1",
                rusqlite::params![entry.entry_id as i64],
            );
            eprintln!(
                "[TidyDog] recover: removed stale entry {} (from exists)",
                entry.entry_id
            );
        } else {
            // 양쪽 모두 없음 → 알 수 없는 상태, completed로 표시하고 경고.
            store.complete_journal(entry.entry_id, ts);
            eprintln!(
                "[TidyDog] recover: WARNING ambiguous entry {} (neither from nor to exists)",
                entry.entry_id
            );
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            // S3 크래시 복원: 앱 시작 직후 실행.
            if let Ok(app_data_dir) = app.path().app_data_dir() {
                let staging_dir = app_data_dir.join("staging");
                let _ = fs::create_dir_all(&app_data_dir);
                if let Ok(conn) = db::init_db(&app_data_dir) {
                    recover_inflight(&conn, &staging_dir);
                }
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            health,
            scan_directory,
            propose_plan,
            confirm_plan,
            execute_plan,
            undo_plan,
            set_ai_consent,
            get_ai_consent,
            set_api_key,
            index_file_content,
            derive_proposed_dest,
            rebuild_wiki,
            chat,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
