//! Phase 4 — 에이전트 루프 + 도구 카탈로그.
//!
//! 안전 불변식:
//!  I1. execute 는 tool_schemas()에 존재하지 않는다.
//!  I2. undo 는 user_triggered=true 일 때만 디스패치된다.
//!  I4. 루프는 MAX_STEPS 에서 반드시 종료된다.

use serde_json::{json, Value};
use std::path::Path;

pub const MAX_STEPS: usize = 10;
const AGENT_MODEL: &str = "claude-haiku-4-5-20251001";

const SYSTEM_PROMPT: &str = "당신은 TidyDog의 파일 정리 AI 어시스턴트입니다. \
사용자의 폴더를 스캔·분석해 정리 플랜을 제안합니다.\n\
\n\
규칙:\n\
1. propose_plan으로 플랜을 '제안'만 합니다 — 실행은 사용자 승인 후 처리됩니다.\n\
2. undo 도구는 사용자가 /undo를 직접 입력할 때만 허용됩니다. 자율 호출 금지.\n\
3. 순서: scan_directory → (선택) summarize → propose_plan.\n\
4. propose_plan의 ops에는 scan_directory 결과의 content_hash가 필요합니다.\n\
5. 한국어로 응답합니다.";

// ── 도구 카탈로그 ───────────────────────────────────────────────────────────────

/// Phase 4 에이전트 도구 카탈로그.
/// I1: execute 및 execute_plan 은 이 목록에 절대 포함되지 않는다.
pub fn tool_schemas() -> Vec<Value> {
    vec![
        json!({
            "name": "scan_directory",
            "description": "폴더를 재귀 스캔해 파일 목록과 content_hash 를 반환한다.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "스캔할 폴더 절대 경로"}
                },
                "required": ["path"]
            }
        }),
        json!({
            "name": "summarize",
            "description": "파일의 메타데이터와 저장된 요약 정보를 반환한다. \
                           (이번 슬라이스: DB 메타데이터 반환. 콘텐츠 읽기는 다음 단계.)",
            "input_schema": {
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "조회할 파일 절대 경로"}
                },
                "required": ["path"]
            }
        }),
        json!({
            "name": "propose_plan",
            "description": "파일 이동/격리/리네임 플랜을 생성한다. 실행하지 않음 — 사용자 승인 후 실행됨.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "ops": {
                        "type": "array",
                        "description": "수행할 작업 목록",
                        "items": {
                            "type": "object",
                            "properties": {
                                "action":       {"type": "string", "enum": ["move", "stage", "rename"]},
                                "content_hash": {"type": "string", "description": "scan_directory 결과의 파일 해시"},
                                "from":         {"type": "string", "description": "현재 파일 경로"},
                                "to":           {"type": "string", "description": "목적 경로"},
                                "reason":       {"type": "string", "description": "이 작업을 제안하는 이유"}
                            },
                            "required": ["action", "content_hash", "from", "to"]
                        }
                    }
                },
                "required": ["ops"]
            }
        }),
        // I2: undo 는 카탈로그에 있으나 루프 내 자율 호출 시 차단된다.
        json!({
            "name": "undo",
            "description": "[사용자 트리거 전용] /undo 명령으로만 호출 가능. \
                           에이전트 자율 추론 중 호출하면 거부됩니다 (I2).",
            "input_schema": {
                "type": "object",
                "properties": {
                    "plan_id": {"type": "string", "description": "되돌릴 플랜 ID"}
                },
                "required": ["plan_id"]
            }
        }),
        // I1: execute / execute_plan 은 이 목록에 없다. 절대 추가 금지.
    ]
}

// ── 결과 타입 ──────────────────────────────────────────────────────────────────

#[derive(Debug, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AgentResult {
    Text {
        message: String,
    },
    PlanProposed {
        plan_id:     String,
        op_count:    usize,
        move_count:  usize,
        stage_count: usize,
        risk_score:  f64,
        preview:     String,
    },
    StepLimitReached {
        partial_message: String,
    },
    UndoCompleted {
        plan_id: String,
    },
}

// ── LLM 클라이언트 트레이트 ────────────────────────────────────────────────────

/// 테스트에서 fake 로 교체 가능한 LLM 인터페이스.
pub trait LlmClient {
    fn complete(&self, messages: &[Value], tools: &[Value]) -> Result<Value, String>;
}

pub struct ClaudeClient {
    pub api_key: String,
}

impl LlmClient for ClaudeClient {
    fn complete(&self, messages: &[Value], tools: &[Value]) -> Result<Value, String> {
        let client = reqwest::blocking::Client::new();
        let body = json!({
            "model":      AGENT_MODEL,
            "max_tokens": 1024,
            "system":     SYSTEM_PROMPT,
            "tools":      tools,
            "messages":   messages,
        });
        let resp = client
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .map_err(|e| format!("API 요청 실패: {e}"))?;

        if resp.status().as_u16() == 429 {
            return Err("API 요청 한도 초과 (429). 잠시 후 다시 시도하세요.".to_string());
        }
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().unwrap_or_default();
            return Err(format!("API 오류 {status}: {text}"));
        }
        resp.json::<Value>().map_err(|e| format!("응답 파싱 실패: {e}"))
    }
}

// ── 에이전트 루프 ──────────────────────────────────────────────────────────────

/// 에이전트 루프를 실행하고 `AgentResult` 를 반환한다.
///
/// - `user_triggered_undo`: true 이면 LLM 없이 undo 경로로 직행 (I2).
/// - `messages`: 현재 대화 히스토리 (Claude API 형식, user 메시지 포함).
/// - `root`: 스캔 루트 경로 (Guard 화이트리스트).
pub fn run_agent_loop(
    client: &dyn LlmClient,
    conn: &rusqlite::Connection,
    staging_dir: &Path,
    messages: Vec<Value>,
    user_triggered_undo: bool,
    root: &str,
) -> Result<AgentResult, String> {
    // I2: /undo 경로 — LLM 자율 추론 없이 직접 undo.
    if user_triggered_undo {
        return do_undo(conn, staging_dir);
    }

    let tools = tool_schemas();
    let mut msgs = messages;

    for _step in 0..MAX_STEPS {
        let response = client.complete(&msgs, &tools)?;

        let stop_reason = response["stop_reason"].as_str().unwrap_or("end_turn");
        let content_arr: Vec<Value> = response["content"]
            .as_array()
            .cloned()
            .unwrap_or_default();

        // 어시스턴트 턴 추가.
        msgs.push(json!({ "role": "assistant", "content": content_arr }));

        match stop_reason {
            "end_turn" => {
                return Ok(AgentResult::Text {
                    message: extract_text(&content_arr),
                });
            }
            "tool_use" => {
                let tool_blocks: Vec<&Value> = content_arr
                    .iter()
                    .filter(|b| b["type"] == "tool_use")
                    .collect();

                let mut results: Vec<Value> = Vec::new();

                for block in tool_blocks {
                    let tid  = block["id"].as_str().unwrap_or("").to_string();
                    let name = block["name"].as_str().unwrap_or("").to_string();
                    let inp  = block["input"].clone();

                    // I2: undo 자율 호출 차단.
                    if name == "undo" {
                        results.push(json!({
                            "type":        "tool_result",
                            "tool_use_id": tid,
                            "is_error":    true,
                            "content":     "undo는 사용자가 /undo를 직접 입력할 때만 실행됩니다 (I2). 에이전트 자율 호출 차단."
                        }));
                        continue;
                    }

                    match dispatch_tool(&name, inp, conn, staging_dir, root) {
                        Ok(val) => {
                            // propose_plan 성공 → 루프 즉시 종료.
                            if name == "propose_plan" {
                                if let Some(pid) = val["plan_id"].as_str() {
                                    return Ok(AgentResult::PlanProposed {
                                        plan_id:     pid.to_string(),
                                        op_count:    val["op_count"].as_u64().unwrap_or(0) as usize,
                                        move_count:  val["move_count"].as_u64().unwrap_or(0) as usize,
                                        stage_count: val["stage_count"].as_u64().unwrap_or(0) as usize,
                                        risk_score:  val["risk_score"].as_f64().unwrap_or(0.0),
                                        preview:     val["preview"].as_str().unwrap_or("auto").to_string(),
                                    });
                                }
                            }
                            results.push(json!({
                                "type":        "tool_result",
                                "tool_use_id": tid,
                                "content":     val.to_string()
                            }));
                        }
                        Err(e) => {
                            results.push(json!({
                                "type":        "tool_result",
                                "tool_use_id": tid,
                                "is_error":    true,
                                "content":     e
                            }));
                        }
                    }
                }

                msgs.push(json!({ "role": "user", "content": results }));
            }
            _ => {
                // 알 수 없는 stop_reason → 텍스트 추출 후 종료.
                return Ok(AgentResult::Text {
                    message: extract_text(&content_arr),
                });
            }
        }
    }

    // I4: MAX_STEPS 도달 — 루프 강제 종료.
    Ok(AgentResult::StepLimitReached {
        partial_message: last_assistant_text(&msgs),
    })
}

// ── 도구 디스패치 ──────────────────────────────────────────────────────────────

/// 도구를 이름으로 디스패치한다.
/// I2: undo 는 이 함수에 도달하지 않아야 한다 (루프에서 먼저 차단).
///     방어적으로 도달해도 Err 반환.
pub fn dispatch_tool(
    name: &str,
    input: Value,
    conn: &rusqlite::Connection,
    staging_dir: &Path,
    root: &str,
) -> Result<Value, String> {
    match name {
        "scan_directory" => {
            let path = input["path"].as_str()
                .ok_or_else(|| "scan_directory: 'path' 없음".to_string())?;
            tool_scan(path, conn)
        }
        "summarize" => {
            let path = input["path"].as_str()
                .ok_or_else(|| "summarize: 'path' 없음".to_string())?;
            tool_summarize(path, conn)
        }
        "propose_plan" => {
            let ops = input["ops"].as_array()
                .ok_or_else(|| "propose_plan: 'ops' 없음".to_string())?;
            tool_propose(ops, conn, staging_dir, root)
        }
        "undo" => {
            // I2: 자율 경로에서는 항상 차단.
            Err("undo 도구는 에이전트 자율 경로에서 호출할 수 없습니다 (I2). /undo 를 직접 입력하세요.".to_string())
        }
        _ => Err(format!("알 수 없는 도구: {name}")),
    }
}

fn tool_scan(path: &str, conn: &rusqlite::Connection) -> Result<Value, String> {
    use crate::scanner;
    let root_path = Path::new(path);
    let node = scanner::scan_recursive(root_path, 0, 10, conn)
        .ok_or_else(|| format!("스캔 실패 (경로 없음 또는 권한 오류): {path}"))?;

    // DB에서 content_hash 포함 파일 목록 조회 (최대 50건).
    let mut stmt = conn
        .prepare(
            "SELECT current_path, content_hash, ext, size
             FROM files WHERE current_path LIKE ?1 || '%'
             ORDER BY current_path LIMIT 50",
        )
        .map_err(|e| e.to_string())?;

    let files: Vec<Value> = stmt
        .query_map(rusqlite::params![path], |row| {
            Ok(json!({
                "path":         row.get::<_, String>(0)?,
                "content_hash": row.get::<_, String>(1)?,
                "ext":          row.get::<_, Option<String>>(2)?,
                "size_bytes":   row.get::<_, Option<i64>>(3)?
            }))
        })
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();

    Ok(json!({
        "path":        path,
        "total_dirs":  count_dirs(&node),
        "total_files": files.len(),
        "files":       files,
    }))
}

fn count_dirs(node: &crate::scanner::FileNode) -> usize {
    if !node.is_dir { return 0; }
    1 + node.children.iter().map(count_dirs).sum::<usize>()
}

fn tool_summarize(path: &str, conn: &rusqlite::Connection) -> Result<Value, String> {
    // 이번 슬라이스: 사이드카 파이프 없이 DB 메타데이터 + 저장된 요약만 반환.
    conn.query_row(
        "SELECT content_hash, summary, topic_tags, language
         FROM files WHERE current_path = ?1",
        rusqlite::params![path],
        |row| {
            Ok(json!({
                "path":         path,
                "content_hash": row.get::<_, String>(0)?,
                "summary":      row.get::<_, Option<String>>(1)?,
                "topic_tags":   row.get::<_, Option<String>>(2)?,
                "language":     row.get::<_, Option<String>>(3)?,
                "note":         "메타데이터 기반 (콘텐츠 읽기는 동의 후 활성화)"
            }))
        },
    )
    .map_err(|_| format!("파일을 DB에서 찾을 수 없습니다. scan_directory를 먼저 실행하세요: {path}"))
}

fn tool_propose(
    ops_val: &[Value],
    conn: &rusqlite::Connection,
    staging_dir: &Path,
    root: &str,
) -> Result<Value, String> {
    use crate::{fileops, guard, store};
    use tidydog_core::{Action, Engine, PlanOp};
    use std::path::PathBuf;

    let ops: Vec<PlanOp> = ops_val
        .iter()
        .map(|op| {
            let action = match op["action"].as_str().unwrap_or("move") {
                "stage"  => Action::Stage,
                "rename" => Action::Rename,
                _        => Action::Move,
            };
            let mut p = PlanOp::new(
                action,
                op["content_hash"].as_str().unwrap_or("").to_string(),
                PathBuf::from(op["from"].as_str().unwrap_or("")),
                PathBuf::from(op["to"].as_str().unwrap_or("")),
            );
            if let Some(r) = op["reason"].as_str() {
                p = p.with_reason(r.to_string());
            }
            p
        })
        .collect();

    let g = guard::make_guard(vec![PathBuf::from(root)]);
    let mut engine = Engine::new(
        g,
        store::SqliteStore::new(conn),
        fileops::FsFileOps::new(staging_dir.to_path_buf()),
    );
    let plan = engine.propose(ops, None, Some("에이전트 제안".to_string()));

    let move_count  = plan.ops.iter().filter(|o| matches!(o.action, Action::Move)).count();
    let stage_count = plan.ops.iter().filter(|o| matches!(o.action, Action::Stage)).count();
    let preview_str = match plan.preview {
        tidydog_core::PreviewLevel::Auto       => "auto",
        tidydog_core::PreviewLevel::Standard   => "standard",
        tidydog_core::PreviewLevel::FullReview => "full_review",
    };

    Ok(json!({
        "plan_id":     plan.plan_id,
        "op_count":    plan.ops.len(),
        "move_count":  move_count,
        "stage_count": stage_count,
        "risk_score":  plan.risk_score,
        "preview":     preview_str,
    }))
}

fn do_undo(conn: &rusqlite::Connection, staging_dir: &Path) -> Result<AgentResult, String> {
    use crate::{fileops, guard, store};
    use tidydog_core::Engine;

    // 가장 최근 executed 플랜 조회.
    let plan_id: String = conn
        .query_row(
            "SELECT plan_id FROM plans WHERE status = 'executed'
             ORDER BY created_at DESC LIMIT 1",
            [],
            |row| row.get(0),
        )
        .map_err(|_| "되돌릴 수 있는 실행된 플랜이 없습니다.".to_string())?;

    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    let g = guard::make_guard(vec![]);
    let mut engine = Engine::new(
        g,
        store::SqliteStore::new(&*tx),
        fileops::FsFileOps::new(staging_dir.to_path_buf()),
    );
    engine.undo(&plan_id).map_err(|e| format!("{e:?}"))?;
    tx.commit().map_err(|e| e.to_string())?;

    Ok(AgentResult::UndoCompleted { plan_id })
}

// ── 유틸 ───────────────────────────────────────────────────────────────────────

fn extract_text(content: &[Value]) -> String {
    content
        .iter()
        .filter(|b| b["type"] == "text")
        .map(|b| b["text"].as_str().unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n")
}

fn last_assistant_text(msgs: &[Value]) -> String {
    msgs.iter()
        .rev()
        .filter(|m| m["role"] == "assistant")
        .find_map(|m| {
            m["content"].as_array().and_then(|arr| {
                arr.iter()
                    .find(|b| b["type"] == "text")
                    .and_then(|b| b["text"].as_str())
                    .map(|s| s.to_string())
            })
        })
        .unwrap_or_else(|| format!("최대 {MAX_STEPS}단계에 도달했습니다."))
}

// ── 안전 불변식 테스트 ─────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn test_conn() -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::db::init_schema(&conn).unwrap();
        conn
    }

    // (a) I1: tool_schemas()에 execute / execute_plan 이 없음.
    #[test]
    fn i1_execute_not_in_tool_catalog() {
        let schemas = tool_schemas();
        let names: Vec<&str> = schemas
            .iter()
            .filter_map(|s| s["name"].as_str())
            .collect();
        assert!(
            !names.contains(&"execute"),
            "execute must not be in tool catalog. names={names:?}"
        );
        assert!(
            !names.contains(&"execute_plan"),
            "execute_plan must not be in tool catalog. names={names:?}"
        );
    }

    // (b) I2: dispatch_tool("undo", …) 는 항상 Err 를 반환한다.
    #[test]
    fn i2_undo_blocked_in_dispatch_tool() {
        let conn = test_conn();
        let tmp = std::env::temp_dir();
        let result = dispatch_tool(
            "undo",
            json!({"plan_id": "some-plan"}),
            &conn,
            &tmp,
            "/tmp",
        );
        assert!(result.is_err(), "dispatch_tool(undo) must return Err (I2)");
        let msg = result.unwrap_err();
        assert!(
            msg.contains("I2") || msg.contains("자율"),
            "Error must mention I2 or blocking reason. Got: {msg}"
        );
    }

    // (b) I2: LLM 이 undo 를 요청해도 루프가 차단하고 계속 진행.
    struct UndoForcingClient;
    impl LlmClient for UndoForcingClient {
        fn complete(&self, _msgs: &[Value], _tools: &[Value]) -> Result<Value, String> {
            Ok(json!({
                "stop_reason": "tool_use",
                "content": [{
                    "type": "tool_use",
                    "id": "t1",
                    "name": "undo",
                    "input": {"plan_id": "any-plan"}
                }]
            }))
        }
    }

    #[test]
    fn i2_agent_loop_blocks_autonomous_undo() {
        let conn = test_conn();
        let tmp = std::env::temp_dir();
        let result = run_agent_loop(
            &UndoForcingClient,
            &conn,
            &tmp,
            vec![json!({"role": "user", "content": "되돌려줘"})],
            false, // user_triggered = false
            "/tmp",
        );
        // undo 가 계속 차단되어 MAX_STEPS 에서 StepLimitReached 반환해야 함.
        assert!(
            matches!(result, Ok(AgentResult::StepLimitReached { .. })),
            "Autonomous undo must be blocked; loop must hit StepLimitReached. Got: {result:?}"
        );
    }

    // (c) I4: MAX_STEPS 도달 시 루프가 StepLimitReached 를 반환한다.
    struct AlwaysToolClient;
    impl LlmClient for AlwaysToolClient {
        fn complete(&self, _msgs: &[Value], _tools: &[Value]) -> Result<Value, String> {
            Ok(json!({
                "stop_reason": "tool_use",
                "content": [{
                    "type": "tool_use",
                    "id": "t1",
                    "name": "scan_directory",
                    "input": {"path": "/nonexistent_path_xyz"}
                }]
            }))
        }
    }

    #[test]
    fn i4_max_steps_terminates_loop() {
        let conn = test_conn();
        let tmp = std::env::temp_dir();
        let result = run_agent_loop(
            &AlwaysToolClient,
            &conn,
            &tmp,
            vec![json!({"role": "user", "content": "계속 스캔해"})],
            false,
            "/tmp",
        );
        assert!(
            matches!(result, Ok(AgentResult::StepLimitReached { .. })),
            "Loop must terminate at MAX_STEPS={MAX_STEPS}. Got: {result:?}"
        );
    }
}
