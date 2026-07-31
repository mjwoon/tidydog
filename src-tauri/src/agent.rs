//! Phase 4 — 에이전트 루프 + 도구 카탈로그.
//!
//! 안전 불변식:
//!  I1. execute 는 tool_schemas()에 존재하지 않는다.
//!  I2. undo 는 user_triggered=true 일 때만 디스패치된다.
//!  I4. 루프는 MAX_STEPS 에서 반드시 종료된다.

use serde_json::{json, Value};
use std::path::Path;

pub const MAX_STEPS: usize = 10;
/// Claude model used for the agent loop (tool-use capable).
/// Upgrade to `claude-sonnet-4-6` here when quality matters more than cost.
const AGENT_MODEL: &str = "claude-haiku-4-5-20251001";

const SYSTEM_PROMPT: &str = "당신은 TidyDog의 파일 정리 AI 어시스턴트입니다. \
사용자의 폴더를 스캔·분석해 정리 플랜을 제안합니다.\n\
\n\
■ 두 가지 별개 흐름 — 절대 혼동하지 마라:\n\
  A) 파일 이동/정리 흐름: propose_plan → (사용자 승인) → 실행.\n\
     '이 PDF들을 문서/PDF로 옮겨줘' 같은 지금-한-번 정리 요청이 여기 해당.\n\
  B) 규칙 학습 흐름: propose_rule_change → (사용자 승인) → ORGANIZER.md 반영.\n\
     '앞으로/항상/매번/기본으로/규칙으로' 같은 습관 변경 요청이 여기 해당.\n\
  이 둘은 서로 독립이다. 규칙(ORGANIZER.md)은 '플랜 승인 후'에 만들어지는 게 아니다 —\n\
  규칙은 오직 propose_rule_change 승인으로만 반영된다. 파일 이동이 규칙을 만들지 않고,\n\
  규칙 반영이 파일을 이동시키지 않는다.\n\
\n\
■ 제안은 반드시 '도구 호출'로 하라 (매우 중요):\n\
  플랜이나 규칙 변경을 텍스트로 나열하거나 말로 설명하고 끝내지 마라.\n\
  정리가 필요하면 반드시 propose_plan을, 규칙 변경이면 반드시 propose_rule_change를\n\
  실제 tool_use로 호출하라. '~하겠습니다' 같은 서술로 대체하는 것은 실패다.\n\
\n\
■ 삭제/제거 처리 — delete 동작은 존재하지 않는다:\n\
  삭제/제거/버리기/중복본 정리 의도는 stage(격리) action 으로 표현한다.\n\
  TidyDog은 파일을 영구 삭제하지 않고 staging(격리)으로 보내며 복원 가능하다.\n\
  delete/trash 같은 op 는 없다 — '제거'가 필요하면 propose_plan 의 op action 을 stage 로 한다.\n\
\n\
■ 현황 조회 vs 플랜 수립 — 도구 역할 분리:\n\
  현황/목록만 볼 때는 list_files(가벼움, 재스캔 없음). 실제 정리 플랜을 만들 때는\n\
  scan_directory 로 content_hash 를 확보한 뒤 propose_plan 을 호출한다. 중복본 판정은\n\
  content_hash 일치로 하므로 플랜 수립 전 scan_directory 를 반드시 거친다.\n\
\n\
규칙:\n\
1. propose_plan으로 플랜을 '제안'만 합니다 — 실행은 사용자 승인 후 처리됩니다.\n\
2. undo 도구는 사용자가 /undo를 직접 입력할 때만 허용됩니다. 자율 호출 금지.\n\
3. 플랜 수립 필수 순서: scan_directory → read_content(파일마다 반드시 호출) → propose_plan.\n\
   사용자가 정리를 요청하면 이 순서를 끝까지 수행해 propose_plan까지 반드시 호출한다.\n\
   (단순 현황 조회는 list_files 만으로 답하고 이 순서를 강제하지 않는다.)\n\
4. read_content로 각 파일의 실제 텍스트를 읽고 주제/도메인을 파악한다.\n\
5. propose_plan 각 op의 'to' 경로는 반드시 읽은 내용 기반으로 결정한다:\n\
   - 코드·프로그래밍 내용 → {루트}/개발/{파일명}\n\
   - 요리·레시피 내용    → {루트}/요리/{파일명}\n\
   - 재무·예산·회계 내용 → {루트}/문서/재무/{파일명}\n\
   - 그 외 주제          → {루트}/문서/{주제}/{파일명}\n\
   파일 확장자나 파일명만 보고 to를 결정하면 안 된다. 내용을 먼저 읽어라.\n\
6. propose_plan의 ops에는 scan_directory 결과의 content_hash가 필요합니다.\n\
7. 한국어로 응답합니다.\n\
8. 사용자가 '앞으로도', '항상', '매번', '기본으로', '규칙으로' 같은 분류 습관 변경 패턴을\n\
   표현하면 propose_rule_change 도구를 호출해 ORGANIZER.md 변경안을 제안한다.\n\
   이 도구는 제안만 하고 파일을 직접 쓰지 않는다(R1). ORGANIZER.md가 아직 없어도\n\
   propose_rule_change는 최초 생성안을 제안할 수 있다.";

// ── 도구 카탈로그 ───────────────────────────────────────────────────────────────

/// Phase 4 에이전트 도구 카탈로그.
/// I1: execute 및 execute_plan 은 이 목록에 절대 포함되지 않는다.
pub fn tool_schemas() -> Vec<Value> {
    vec![
        json!({
            "name": "scan_directory",
            "description": "폴더를 재귀 스캔해 파일 목록과 content_hash 를 반환한다. \
                           정리 플랜을 수립할 때 사용한다 — propose_plan 의 ops 에 필요한 content_hash 를 \
                           여기서 얻고, 중복본은 content_hash 일치로 판정한다.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "스캔할 폴더 절대 경로"}
                },
                "required": ["path"]
            }
        }),
        json!({
            "name": "list_files",
            "description": "이미 스캔되어 DB에 있는 파일 목록을 읽어 반환한다(재스캔하지 않음). \
                           현재 정리 대상 폴더의 현황(파일 경로·확장자·태그·요약·제안 목적지)을 \
                           물을 때만 사용한다. 파일시스템을 다시 걷지 않고 DB 행만 읽는 읽기 전용 도구다. \
                           content_hash 를 반환하지 않으므로 플랜 수립에는 쓰지 않는다 — \
                           플랜을 만들 때는 scan_directory 를 사용하라. \
                           path 를 생략하면 현재 정리 대상 폴더 전체를 대상으로 한다.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "조회할 폴더 절대 경로(생략 시 현재 대상)"}
                }
            }
        }),
        json!({
            "name": "read_content",
            "description": "파일의 텍스트 내용을 읽어 반환한다. \
                           반환된 'text' 필드를 보고 파일의 주제/도메인을 파악해 \
                           propose_plan의 to 경로 결정에 사용한다. \
                           사용자 동의 필요(N1/G1) · 범위 내 파일만 가능(G2/G3) · 최대 4096자(N3).",
            "input_schema": {
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "읽을 파일의 절대 경로 (범위 내)"}
                },
                "required": ["path"]
            }
        }),
        json!({
            "name": "summarize",
            "description": "파일 내용을 읽어 AI 요약과 태그를 반환한다. \
                           사용자 동의 필요(N1/G1) · 사용자 지정 범위 내 파일만 가능(G2/G3).",
            "input_schema": {
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "요약할 파일의 절대 경로 (범위 내)"}
                },
                "required": ["path"]
            }
        }),
        json!({
            "name": "propose_plan",
            "description": "파일 이동/격리/리네임 플랜을 생성한다. 실행하지 않음 — 사용자 승인 후 실행됨. \
                           플랜 수립 전 scan_directory 로 content_hash 를 확보한다(중복본은 해시 일치로 판정). \
                           삭제/제거 동작은 없다 — 제거 의도는 action=stage 로 표현한다.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "ops": {
                        "type": "array",
                        "description": "수행할 작업 목록",
                        "items": {
                            "type": "object",
                            "properties": {
                                "action":       {"type": "string", "enum": ["move", "stage", "rename"],
                                                 "description": "move = 다른 폴더로 이동. \
                                                                 stage = 격리(소프트 삭제) — 삭제/제거/버리기/중복본 처리는 이 값으로 한다(영구삭제 아님, 복원 가능). \
                                                                 rename = 같은 폴더 내 이름 변경. \
                                                                 delete/trash 는 존재하지 않는다."},
                                "content_hash": {"type": "string", "description": "scan_directory 결과의 파일 해시(중복 판정 기준)"},
                                "from":         {"type": "string", "description": "현재 파일 경로"},
                                "to":           {"type": "string", "description": "목적 경로 — read_content로 읽은 파일 내용의 주제를 기반으로 결정한다. 예: Python 코드 → {루트}/개발/{파일명}, 레시피 → {루트}/요리/{파일명}, 재무문서 → {루트}/문서/재무/{파일명}"},
                                "reason":       {"type": "string", "description": "이 작업을 제안하는 이유"}
                            },
                            "required": ["action", "content_hash", "from", "to"]
                        }
                    }
                },
                "required": ["ops"]
            }
        }),
        json!({
            "name": "propose_rule_change",
            "description": "사용자가 '앞으로도', '항상', '매번' 같은 습관 규칙을 말할 때 \
                           ORGANIZER.md 변경안(diff)을 생성한다. \
                           파일을 직접 쓰지 않음(R1). 사용자 승인 후 반영된다.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "request": {
                        "type": "string",
                        "description": "사용자의 규칙 변경 요청 (자연어). \
                                       예: '앞으로 스크린샷은 사진/스크린샷 폴더로 모아줘'"
                    }
                },
                "required": ["request"]
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
        ops:         Vec<Value>,
    },
    StepLimitReached {
        partial_message: String,
    },
    UndoCompleted {
        plan_id: String,
    },
    RuleProposed {
        rule_change_id: String,
        summary_cards:  Vec<crate::rule_change::SummaryCard>,
        diff:           String,
        before:         String,
        after:          String,
    },
}

// ── LLM 클라이언트 트레이트 ────────────────────────────────────────────────────

/// 테스트에서 fake 로 교체 가능한 LLM 인터페이스.
pub trait LlmClient {
    fn complete(&self, messages: &[Value], tools: &[Value]) -> Result<Value, String>;
}

pub struct ClaudeClient {
    pub api_key: String,
    /// 현재 활성 정리 대상 폴더. Some 이면 시스템 컨텍스트에 주입되어
    /// 에이전트가 경로를 되묻지 않고 이 폴더를 대상으로 진행한다.
    pub target_dir: Option<String>,
}

/// 활성 대상 경로를 시스템 프롬프트에 덧붙일 컨텍스트 문자열로 만든다.
/// 대상이 없으면(첫 실행·폴더 미선택) None → 되묻기 분기 안내만 유지.
fn build_system_prompt(target_dir: Option<&str>) -> String {
    match target_dir {
        Some(path) if !path.is_empty() => format!(
            "{SYSTEM_PROMPT}\n\n\
             ■ 현재 정리 대상: {path}\n\
             사용자가 경로를 명시하지 않으면 이 폴더를 대상으로 한다. 경로를 되묻지 마라.\n\
             폴더 현황(어떤 파일이 어떻게 정리돼 있는지)을 물으면 list_files 도구로 \
             이미 스캔된 DB 파일 목록을 읽는다. scan_directory(재스캔)는 사용자가 \
             명시적으로 다시 스캔을 요청하거나 대상이 바뀐 경우에만 쓴다."
        ),
        _ => format!(
            "{SYSTEM_PROMPT}\n\n\
             ■ 현재 활성 대상 폴더가 없다. 사용자에게 정리할 폴더를 먼저 선택/알려달라고 요청한다. \
             임의의 예시 경로(특히 Windows 형식 C:\\... 등)를 지어내지 마라."
        ),
    }
}

impl LlmClient for ClaudeClient {
    fn complete(&self, messages: &[Value], tools: &[Value]) -> Result<Value, String> {
        let client = reqwest::blocking::Client::new();
        let body = json!({
            "model":      AGENT_MODEL,
            "max_tokens": 4096,
            "system":     build_system_prompt(self.target_dir.as_deref()),
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

    for step in 0..MAX_STEPS {
        eprintln!("[agent:loop] step={step} → LLM call (model={AGENT_MODEL})");
        let response = client.complete(&msgs, &tools)?;

        let stop_reason = response["stop_reason"].as_str().unwrap_or("end_turn");
        eprintln!("[agent:loop] step={step} ← stop_reason={stop_reason}");
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
            "tool_use" | "max_tokens" => {
                let tool_blocks: Vec<&Value> = content_arr
                    .iter()
                    .filter(|b| b["type"] == "tool_use")
                    .collect();

                // max_tokens 절단: tool_use 없이 프로즈만 왔으면 완료로 오인하지 말고
                // 간결 재시도를 유도해 루프를 계속한다(MAX_STEPS 로 무한루프 방지).
                if tool_blocks.is_empty() {
                    if stop_reason == "max_tokens" {
                        msgs.push(json!({
                            "role": "user",
                            "content": "이전 응답이 max_tokens 로 잘렸습니다. 설명은 최소화하고 곧바로 propose_plan(정리) 또는 propose_rule_change(규칙) 도구를 호출해 마무리하세요."
                        }));
                        continue;
                    }
                    // tool_use 인데 블록이 없는 비정상 → 방어적으로 텍스트 반환.
                    return Ok(AgentResult::Text { message: extract_text(&content_arr) });
                }

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
                                        ops:         val["ops"].as_array().cloned().unwrap_or_default(),
                                    });
                                }
                            }
                            // propose_rule_change 성공 → 루프 즉시 종료.
                            if name == "propose_rule_change" {
                                if let Some(rid) = val["rule_change_id"].as_str() {
                                    let summary_cards = val["summary_cards"]
                                        .as_array()
                                        .map(|arr| {
                                            arr.iter()
                                                .filter_map(|c| {
                                                    serde_json::from_value::<crate::rule_change::SummaryCard>(c.clone()).ok()
                                                })
                                                .collect::<Vec<_>>()
                                        })
                                        .unwrap_or_default();
                                    return Ok(AgentResult::RuleProposed {
                                        rule_change_id: rid.to_string(),
                                        summary_cards,
                                        diff:   val["diff"].as_str().unwrap_or("").to_string(),
                                        before: val["before"].as_str().unwrap_or("").to_string(),
                                        after:  val["after"].as_str().unwrap_or("").to_string(),
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
    eprintln!("[agent:tool] → {name}");
    let result = _dispatch_tool(name, input, conn, staging_dir, root);
    match &result {
        Ok(v) => {
            // read_content: 실제로 내용이 돌아왔는지 미리보기로 확인.
            if name == "read_content" {
                let preview = v["text"].as_str().unwrap_or("");
                let end = preview.char_indices().nth(120).map(|(i, _)| i).unwrap_or(preview.len());
                eprintln!("[agent:tool] ← read_content: OK text_len={} preview={:?}",
                    preview.len(), &preview[..end]);
            // propose_plan: op별 from→to+reason 을 출력.
            } else if name == "propose_plan" {
                if let Some(ops) = v["ops"].as_array() {
                    for (i, op) in ops.iter().enumerate() {
                        eprintln!("[agent:plan] op[{i}] {} | {} → {}",
                            op["action"].as_str().unwrap_or("?"),
                            op["from"].as_str().unwrap_or("?"),
                            op["to"].as_str().unwrap_or("?"));
                        if let Some(r) = op["reason"].as_str() {
                            eprintln!("[agent:plan]   reason: {r}");
                        }
                    }
                }
                eprintln!("[agent:tool] ← propose_plan: OK plan_id={} ops={}",
                    v["plan_id"].as_str().unwrap_or("?"),
                    v["op_count"].as_u64().unwrap_or(0));
            } else {
                let keys: Vec<&str> = v.as_object()
                    .map(|o| o.keys().map(|k| k.as_str()).collect())
                    .unwrap_or_default();
                eprintln!("[agent:tool] ← {name}: OK keys={keys:?}");
            }
        }
        Err(e) => eprintln!("[agent:tool] ← {name}: ERR {e}"),
    }
    result
}

fn _dispatch_tool(
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
        "list_files" => {
            // path 생략 시 현재 정리 대상(root)을 사용. DB 읽기 전용(재스캔 없음).
            let path = input["path"].as_str().filter(|s| !s.is_empty()).unwrap_or(root);
            tool_list_files(path, conn)
        }
        "read_content" => {
            let path = input["path"].as_str()
                .ok_or_else(|| "read_content: 'path' 없음".to_string())?;
            tool_read_content(path, conn, root)
        }
        "summarize" => {
            let path = input["path"].as_str()
                .ok_or_else(|| "summarize: 'path' 없음".to_string())?;
            tool_summarize(path, conn, root)
        }
        "propose_plan" => {
            let ops = input["ops"].as_array()
                .ok_or_else(|| "propose_plan: 'ops' 없음".to_string())?;
            tool_propose(ops, conn, staging_dir, root)
        }
        "propose_rule_change" => {
            let request = input["request"]
                .as_str()
                .ok_or_else(|| "propose_rule_change: 'request' 없음".to_string())?;
            crate::rule_change::propose_rule_change_tool(request, conn, root)
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
    // scan_and_prune: 재스캔 + stale 인덱스 제거 → 에이전트가 유령 중복본을 보지 않게.
    let node = scanner::scan_and_prune(root_path, 10, conn)
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

/// list_files: 이미 스캔된 DB의 files 행을 대상 경로로 필터해 읽는다(읽기 전용).
/// 파일시스템을 다시 걷지 않고(재스캔 없음), DB에 쓰지 않는다.
/// 좌측 트리(스캔 결과)와 동일한 상태를 에이전트가 보게 하여 혼란을 방지한다.
fn tool_list_files(path: &str, conn: &rusqlite::Connection) -> Result<Value, String> {
    let mut stmt = conn
        .prepare(
            "SELECT current_path, ext, size, topic_tags, summary, proposed_dest
             FROM files WHERE current_path LIKE ?1 || '%'
             ORDER BY current_path LIMIT 200",
        )
        .map_err(|e| e.to_string())?;

    let files: Vec<Value> = stmt
        .query_map(rusqlite::params![path], |row| {
            Ok(json!({
                "path":          row.get::<_, String>(0)?,
                "ext":           row.get::<_, Option<String>>(1)?,
                "size_bytes":    row.get::<_, Option<i64>>(2)?,
                "topic_tags":    row.get::<_, Option<String>>(3)?,
                "summary":       row.get::<_, Option<String>>(4)?,
                "proposed_dest": row.get::<_, Option<String>>(5)?,
            }))
        })
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();

    Ok(json!({
        "path":        path,
        "total_files": files.len(),
        "files":       files,
        "source":      "db", // 재스캔이 아니라 이미 스캔된 DB 행임을 명시.
    }))
}

/// 사이드카 스크립트 경로를 현재 작업 디렉터리 기준으로 해석한다.
/// sidecar/reader.py 위치 해석.
///
/// tauri dev는 바이너리를 src-tauri/에서 실행하므로 cwd 기준 단일 경로로는
/// 워크스페이스 루트의 sidecar/를 못 찾는다. 여러 후보를 순서대로 탐색한다:
///   1. {cwd}/sidecar/reader.py            (워크스페이스 루트에서 직접 실행 시)
///   2. {cwd}/../sidecar/reader.py         (cwd=src-tauri, tauri dev의 일반 케이스)
///   3. {CARGO_MANIFEST_DIR}/../sidecar/reader.py  (컴파일 시각 앵커 — dev에서 가장 견고)
///
/// (배포 번들의 리소스 경로 해석은 P6 과제.)
pub fn resolve_sidecar() -> Result<std::path::PathBuf, String> {
    let mut candidates: Vec<std::path::PathBuf> = Vec::new();

    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join("sidecar/reader.py"));
        candidates.push(cwd.join("../sidecar/reader.py"));
    }
    candidates.push(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../sidecar/reader.py"),
    );

    for c in &candidates {
        if c.exists() {
            return Ok(c.clone());
        }
    }

    Err(format!(
        "사이드카를 찾을 수 없습니다. 탐색한 경로: {} — sidecar/reader.py 위치를 확인하세요.",
        candidates
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    ))
}

/// read_content 도구: G1(동의) → G2/G3(범위·블랙리스트) → 사이드카 읽기.
fn tool_read_content(
    path: &str,
    conn: &rusqlite::Connection,
    root: &str,
) -> Result<Value, String> {
    // G1: 동의 게이트 — 사이드카 실행 전 확인(N1).
    {
        use tidydog_core::Summarizer;
        if !crate::summarizer::CloudSummarizer::new(conn).is_consented() {
            return Err(
                "파일 내용을 읽으려면 AI 콘텐츠 처리 동의가 필요합니다 (N1/G1). \
                 설정에서 동의를 활성화하세요."
                    .to_string(),
            );
        }
    }

    // G2+G3: 사용자 지정 범위 + C2 블랙리스트 검사.
    {
        use crate::guard;
        use tidydog_core::Guard;
        let g = guard::make_guard(vec![std::path::PathBuf::from(root)]);
        g.check(std::path::Path::new(path)).map_err(|d| {
            format!(
                "read_content 범위 거부 (G2/G3): {d:?} \
                 — 사용자 지정 범위({root}) 내 파일만 읽을 수 있습니다."
            )
        })?;
    }

    // 사이드카로 내용 읽기 (N3: 최대 4096자 / 5페이지).
    let sidecar = resolve_sidecar()?;
    let reader = crate::reader::SidecarReader::new("python3", sidecar);
    use tidydog_core::ContentReader;
    let chunk = reader
        .read_content(
            std::path::Path::new(path),
            tidydog_core::ContentBudget::default(),
        )
        .map_err(|e| format!("read_content 사이드카 오류: {e}"))?;

    Ok(json!({
        "path":       path,
        "text":       chunk.text,
        "truncated":  chunk.truncated,
        "pages_read": chunk.pages_read,
    }))
}

/// summarize 도구: G1(동의) → G2/G3(범위) → 사이드카 읽기 → CloudSummarizer.
fn tool_summarize(
    path: &str,
    conn: &rusqlite::Connection,
    root: &str,
) -> Result<Value, String> {
    // G1: 동의 게이트.
    {
        use tidydog_core::Summarizer;
        if !crate::summarizer::CloudSummarizer::new(conn).is_consented() {
            return Err(
                "AI 요약에는 콘텐츠 처리 동의가 필요합니다 (N1/G1).".to_string(),
            );
        }
    }

    // G2+G3: 범위·블랙리스트 게이트.
    {
        use crate::guard;
        use tidydog_core::Guard;
        let g = guard::make_guard(vec![std::path::PathBuf::from(root)]);
        g.check(std::path::Path::new(path)).map_err(|d| {
            format!("summarize 범위 거부 (G2/G3): {d:?}")
        })?;
    }

    // DB에서 content_hash 조회 (캐시용).
    let content_hash: Option<String> = conn
        .query_row(
            "SELECT content_hash FROM files WHERE current_path = ?1",
            rusqlite::params![path],
            |row| row.get(0),
        )
        .ok();

    // 사이드카 읽기 (N3 예산).
    let sidecar = resolve_sidecar()?;
    let reader = crate::reader::SidecarReader::new("python3", sidecar);
    use tidydog_core::ContentReader;
    let chunk = reader
        .read_content(
            std::path::Path::new(path),
            tidydog_core::ContentBudget::default(),
        )
        .map_err(|e| format!("summarize 사이드카 오류: {e}"))?;

    // CloudSummarizer로 요약 (N2: API 키 키체인).
    let file_path = std::path::Path::new(path);
    let filename = file_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string();
    let ext = file_path
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.to_string());

    let hints = tidydog_core::SummaryHints { filename, ext, content_hash };
    let summarizer = crate::summarizer::CloudSummarizer::new(conn);
    use tidydog_core::Summarizer;
    let result = summarizer
        .summarize(&chunk, hints)
        .map_err(|e| e.to_string())?;

    Ok(json!({
        "path":       path,
        "summary":    result.summary,
        "topic_tags": result.topic_tags,
        "language":   result.language,
    }))
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

    // ops 상세: 로깅과 AgentResult 조합에 사용.
    // 프론트 PlanReview 의 PlanOp 타입과 필드명·형태를 맞춘다
    // (op_id/seq/action/content_hash/from/to/conflict/reason).
    let ops_detail: Vec<Value> = plan.ops.iter().map(|o| {
        let action_str = match o.action {
            Action::Move   => "move",
            Action::Stage  => "stage",
            Action::Rename => "rename",
        };
        json!({
            "op_id":        o.op_id,
            "seq":          o.seq,
            "action":       action_str,
            "content_hash": o.content_hash,
            "from":         o.from.to_string_lossy(),
            "to":           o.to.to_string_lossy(),
            "conflict":     o.conflict.as_str(),
            "reason":       o.reason,
        })
    }).collect();

    Ok(json!({
        "plan_id":     plan.plan_id,
        "op_count":    plan.ops.len(),
        "move_count":  move_count,
        "stage_count": stage_count,
        "risk_score":  plan.risk_score,
        "preview":     preview_str,
        "ops":         ops_detail,
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

    // I2 독립성: undo 거부가 LLM 에 전달되면 LLM 이 ack 하고 end_turn 반환.
    // MAX_STEPS 와 완전히 독립적으로 do_undo() 미실행을 DB 상태로 직접 증명.
    struct UndoOnceThenStopClient {
        call_count: std::sync::Mutex<usize>,
    }
    impl UndoOnceThenStopClient {
        fn new() -> Self { Self { call_count: std::sync::Mutex::new(0) } }
    }
    impl LlmClient for UndoOnceThenStopClient {
        fn complete(&self, _msgs: &[Value], _tools: &[Value]) -> Result<Value, String> {
            let mut count = self.call_count.lock().unwrap();
            *count += 1;
            if *count == 1 {
                Ok(json!({
                    "stop_reason": "tool_use",
                    "content": [{
                        "type": "tool_use",
                        "id": "t-undo-1",
                        "name": "undo",
                        "input": {"plan_id": "plan-sentinel"}
                    }]
                }))
            } else {
                Ok(json!({
                    "stop_reason": "end_turn",
                    "content": [{"type": "text", "text": "알겠습니다, 되돌리지 않겠습니다."}]
                }))
            }
        }
    }

    #[test]
    fn i2_undo_never_executes_regardless_of_step_count() {
        let conn = test_conn();
        // 'executed' 상태 플랜을 미리 삽입 — do_undo() 가 실행되면 'undone' 으로 바뀐다.
        conn.execute(
            "INSERT INTO plans (plan_id, created_at, status, risk_score, preview, ops_json)
             VALUES (?1, 1000000, 'executed', 0.0, 'auto', '[]')",
            rusqlite::params!["plan-sentinel"],
        ).unwrap();

        let client = UndoOnceThenStopClient::new();
        let tmp = std::env::temp_dir();
        let result = run_agent_loop(
            &client,
            &conn,
            &tmp,
            vec![json!({"role": "user", "content": "되돌려줘"})],
            false,
            "/tmp",
        );

        // 거부 메시지가 LLM 에 전달되고, LLM 이 end_turn 으로 응답했으므로 Text 여야 함.
        // StepLimitReached 도, UndoCompleted 도 아니어야 함.
        assert!(
            matches!(result, Ok(AgentResult::Text { .. })),
            "Rejection must be delivered; LLM must ack with end_turn → Text. Got: {result:?}"
        );

        // plan-sentinel 이 여전히 'executed' 상태여야 함 — do_undo() 가 호출되지 않았음을 DB 로 증명.
        let status: String = conn.query_row(
            "SELECT status FROM plans WHERE plan_id = 'plan-sentinel'",
            [],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(
            status, "executed",
            "do_undo must not have mutated DB: status must remain 'executed', got '{status}'"
        );
    }

    // ── 전달 체인 회귀 잠금: propose_plan ops → PlanProposed variant → 직렬화 ──
    //
    // 회귀 대상: variant 에 ops 필드가 없어 run_agent_loop 이 개수 필드만 통과시키고
    // ops 배열을 버렸던 버그(끊긴 8곳 체인). I1 sentinel 과 동일한 "부재/누락 증명" 패턴 —
    // 체인 어느 한 곳이라도 ops 전달을 끊으면 이 테스트가 반드시 실패한다.
    struct PlanProposingClient;
    impl LlmClient for PlanProposingClient {
        fn complete(&self, _msgs: &[Value], _tools: &[Value]) -> Result<Value, String> {
            Ok(json!({
                "stop_reason": "tool_use",
                "content": [{
                    "type": "tool_use",
                    "id": "t-plan-1",
                    "name": "propose_plan",
                    "input": { "ops": [
                        {"action": "move", "content_hash": "h1",
                         "from": "/tmp/tdg_root/tax.pdf", "to": "/tmp/tdg_root/문서/재무/tax.pdf",
                         "reason": "세금 문서"},
                        {"action": "move", "content_hash": "h2",
                         "from": "/tmp/tdg_root/main.py", "to": "/tmp/tdg_root/개발/main.py",
                         "reason": "파이썬 코드"}
                    ]}
                }]
            }))
        }
    }

    #[test]
    fn plan_proposed_variant_carries_ops_end_to_end() {
        let conn = test_conn();
        let tmp = std::env::temp_dir();
        let result = run_agent_loop(
            &PlanProposingClient,
            &conn,
            &tmp,
            vec![json!({"role": "user", "content": "정리해줘"})],
            false,
            "/tmp/tdg_root",
        )
        .expect("run_agent_loop 성공");

        // (1) variant 레벨: ops 가 비어있지 않게 실려야 한다.
        match &result {
            AgentResult::PlanProposed { ops, op_count, .. } => {
                assert_eq!(*op_count, 2, "op_count 요약 필드 유지");
                assert_eq!(
                    ops.len(),
                    2,
                    "전달 체인 회귀: variant 가 ops 를 잃으면 안 됨 (개수만 통과 = 버그)"
                );
                // 각 op 의 핵심 필드가 살아있어야 한다.
                assert_eq!(ops[0]["action"].as_str().unwrap(), "move");
                assert_eq!(ops[0]["from"].as_str().unwrap(), "/tmp/tdg_root/tax.pdf");
                assert!(ops[0]["to"].as_str().unwrap().contains("문서/재무"));
                // conflict 필드가 payload 에 실제로 실려야 한다(FE 렌더가 읽는 필드).
                assert_eq!(
                    ops[0]["conflict"].as_str().unwrap(),
                    "none",
                    "conflict 필드는 payload 에 존재해야 함(이 시나리오는 목적지가 비어 none; \
                     P-1 이후 목적지 존재 시 rename 으로 채워진다)"
                );
            }
            other => panic!("PlanProposed 여야 함, got: {other:?}"),
        }

        // (2) 직렬화 경계(= ChatResponse.ops): serde JSON 에도 ops 가 손실 없이 실려야 한다.
        let serialized = serde_json::to_value(&result).expect("직렬화 성공");
        assert_eq!(serialized["kind"], "plan_proposed");
        let ops_json = serialized["ops"]
            .as_array()
            .expect("직렬화된 JSON 에 ops 배열이 존재해야 함(ChatResponse 경계)");
        assert_eq!(
            ops_json.len(),
            2,
            "ChatResponse 직렬화 경계에서 ops 손실 금지"
        );
        assert_eq!(ops_json[1]["conflict"].as_str().unwrap(), "none");
    }

    // ── 삭제→stage 매핑: move+stage 혼합 플랜이 정상 관통하고 카운트가 맞는가 ──
    // 제거 의도는 stage action 으로 표현되므로(새 action 없음), 혼합 플랜이
    // variant 까지 손실 없이 흐르고 move_count/stage_count 가 분리 집계돼야 한다.
    struct MoveAndStageClient;
    impl LlmClient for MoveAndStageClient {
        fn complete(&self, _msgs: &[Value], _tools: &[Value]) -> Result<Value, String> {
            Ok(json!({
                "stop_reason": "tool_use",
                "content": [{
                    "type": "tool_use", "id": "t-mix-1", "name": "propose_plan",
                    "input": { "ops": [
                        {"action": "move",  "content_hash": "h1",
                         "from": "/tmp/tdg_root/main.py", "to": "/tmp/tdg_root/개발/main.py",
                         "reason": "파이썬 코드"},
                        {"action": "stage", "content_hash": "h2",
                         "from": "/tmp/tdg_root/dup.pdf", "to": "/tmp/tdg_root/dup.pdf",
                         "reason": "중복본 — 격리(소프트 삭제)"}
                    ]}
                }]
            }))
        }
    }

    #[test]
    fn delete_intent_maps_to_stage_op_and_counts() {
        let conn = test_conn();
        let tmp = std::env::temp_dir();
        let result = run_agent_loop(
            &MoveAndStageClient, &conn, &tmp,
            vec![json!({"role": "user", "content": "중복본 제거하고 코드 정리해줘"})],
            false, "/tmp/tdg_root",
        ).expect("run_agent_loop 성공");

        match &result {
            AgentResult::PlanProposed { ops, op_count, move_count, stage_count, .. } => {
                assert_eq!(*op_count, 2);
                assert_eq!(*move_count, 1, "move 1건");
                assert_eq!(*stage_count, 1, "제거→stage 1건으로 분리 집계");
                // stage op 가 실제로 stage action 으로 실려야 한다(delete 아님).
                let has_stage = ops.iter().any(|o| o["action"].as_str() == Some("stage"));
                assert!(has_stage, "제거 의도가 stage op 로 관통해야 함");
                // delete/trash action 은 존재하지 않는다.
                assert!(
                    ops.iter().all(|o| matches!(o["action"].as_str(), Some("move") | Some("stage") | Some("rename"))),
                    "action 은 move/stage/rename 만 — delete/trash 없음"
                );
            }
            other => panic!("PlanProposed 여야 함, got: {other:?}"),
        }
    }

    // ── 시스템 프롬프트: 삭제→stage 매핑·역할 분리 안내가 포함되는가 ────────────
    #[test]
    fn system_prompt_documents_delete_maps_to_stage() {
        let sys = build_system_prompt(Some("/tmp/x"));
        assert!(sys.contains("stage"), "stage 매핑 언급");
        assert!(
            sys.contains("삭제") && sys.contains("격리"),
            "삭제/제거 의도를 격리(stage)로 표현하라는 안내 포함"
        );
        assert!(
            sys.contains("delete") || sys.contains("영구 삭제하지 않"),
            "delete 동작 부재/영구삭제 안 함 명시"
        );
        assert!(
            sys.contains("list_files") && sys.contains("scan_directory"),
            "현황(list_files) vs 플랜(scan_directory) 역할 분리 안내 포함"
        );
    }

    // ── 대상 경로 관통: 시스템 프롬프트에 활성 대상이 주입되는가 ────────────────

    #[test]
    fn system_prompt_injects_target_dir_when_present() {
        let sys = build_system_prompt(Some("/Users/me/Downloads/files"));
        assert!(
            sys.contains("현재 정리 대상: /Users/me/Downloads/files"),
            "활성 대상 경로가 시스템 컨텍스트에 주입돼야 함"
        );
        assert!(
            sys.contains("경로를 되묻지 마라"),
            "대상이 있으면 되묻지 말라는 지시가 있어야 함"
        );
        assert!(sys.contains("list_files"), "현황 조회 시 DB 우선(list_files) 안내 포함");
    }

    #[test]
    fn system_prompt_asks_for_folder_when_no_target() {
        // 대상 없음(첫 실행/미선택) → 되묻기 정당 + 윈도우 하드코딩 예시 금지 지시.
        let sys = build_system_prompt(None);
        assert!(
            sys.contains("활성 대상 폴더가 없다"),
            "대상 없을 때만 폴더 요청 분기"
        );
        assert!(
            sys.contains("Windows") || sys.contains("C:\\"),
            "윈도우 예시를 '지어내지 말라'는 지시가 명시돼야 함"
        );
        // 대상 주입 문구는 없어야 한다.
        assert!(
            !sys.contains("현재 정리 대상:"),
            "대상이 없으면 '현재 정리 대상' 주입 문구가 없어야 함"
        );
        // 빈 문자열도 None 과 동일 취급.
        assert_eq!(build_system_prompt(Some("")), build_system_prompt(None));
    }

    // ── DoD 3: list_files 는 DB 행만 읽는다(재스캔 아님) ──────────────────────

    #[test]
    fn list_files_reads_db_rows_scoped_to_target_no_rescan() {
        let conn = test_conn();
        let root = "/Users/me/Downloads/files";
        // 대상 하위 2건 + 범위 밖 1건 삽입.
        conn.execute(
            "INSERT INTO files (content_hash, current_path, size, mtime, ext, topic_tags, proposed_dest)
             VALUES ('h1', ?1, 100, 0, 'py', '[\"개발\"]', '개발/main.py')",
            rusqlite::params![format!("{root}/main.py")],
        ).unwrap();
        conn.execute(
            "INSERT INTO files (content_hash, current_path, size, mtime, ext, topic_tags, proposed_dest)
             VALUES ('h2', ?1, 200, 0, 'pdf', '[\"문서\"]', '문서/tax.pdf')",
            rusqlite::params![format!("{root}/tax.pdf")],
        ).unwrap();
        conn.execute(
            "INSERT INTO files (content_hash, current_path, size, mtime, ext)
             VALUES ('h3', '/Users/me/Other/x.txt', 50, 0, 'txt')",
            [],
        ).unwrap();

        let result = tool_list_files(root, &conn).unwrap();

        // 재스캔이 아니라 DB 소스임을 명시.
        assert_eq!(result["source"].as_str().unwrap(), "db");
        // 범위 내 2건만, 범위 밖 제외.
        assert_eq!(result["total_files"].as_u64().unwrap(), 2, "대상 하위 파일만 조회");
        let files = result["files"].as_array().unwrap();
        let paths: Vec<&str> = files.iter().filter_map(|f| f["path"].as_str()).collect();
        assert!(paths.iter().all(|p| p.starts_with(root)), "범위 밖 파일이 새면 안 됨: {paths:?}");
        // 메타데이터(태그·제안 목적지)가 함께 실려 좌측 트리와 동일 상태를 보게 한다.
        assert!(files.iter().any(|f| f["topic_tags"].as_str() == Some("[\"개발\"]")));
        assert!(files.iter().any(|f| f["proposed_dest"].as_str() == Some("문서/tax.pdf")));
    }

    // ── G1: 미동의 시 read_content/summarize 차단 ─────────────────────────────

    fn grant_consent(conn: &rusqlite::Connection) {
        conn.execute(
            "INSERT INTO settings (key,value) VALUES ('ai_content_consent_granted','true')",
            [],
        )
        .unwrap();
    }

    /// G1(read_content): 동의 없으면 dispatch에서 거부 — 사이드카 미실행 증명.
    #[test]
    fn g1_read_content_blocked_without_consent() {
        let conn = test_conn(); // settings 행 없음 = 미동의
        let tmp = std::env::temp_dir();
        let result = dispatch_tool(
            "read_content",
            json!({"path": "/tmp/test.txt"}),
            &conn,
            &tmp,
            "/tmp",
        );
        assert!(result.is_err(), "read_content must reject without consent (G1)");
        let msg = result.unwrap_err();
        assert!(
            msg.contains("동의") || msg.contains("N1") || msg.contains("G1"),
            "rejection must mention consent gate. got: {msg}"
        );
    }

    /// G1(summarize): 동의 없으면 dispatch에서 거부.
    #[test]
    fn g1_summarize_blocked_without_consent() {
        let conn = test_conn();
        let tmp = std::env::temp_dir();
        let result = dispatch_tool(
            "summarize",
            json!({"path": "/tmp/test.txt"}),
            &conn,
            &tmp,
            "/tmp",
        );
        assert!(result.is_err(), "summarize must reject without consent (G1)");
        let msg = result.unwrap_err();
        assert!(
            msg.contains("동의") || msg.contains("N1") || msg.contains("G1"),
            "rejection must mention consent gate. got: {msg}"
        );
    }

    // ── G2: 지정 범위 밖 경로 → 명시적 거부 ──────────────────────────────────

    #[test]
    fn g2_read_content_outside_root_blocked() {
        let conn = test_conn();
        grant_consent(&conn);
        let tmp = std::env::temp_dir();
        // root = /tmp/allowed_subdir, 요청 경로 = /tmp/other.txt (범위 밖)
        let result = dispatch_tool(
            "read_content",
            json!({"path": "/tmp/outside_scope.txt"}),
            &conn,
            &tmp,
            "/tmp/allowed_subdir",
        );
        assert!(result.is_err(), "path outside root must be rejected (G2)");
        let msg = result.unwrap_err();
        assert!(
            msg.contains("G2") || msg.contains("G3") || msg.contains("범위")
                || msg.contains("OutsideScope"),
            "rejection must mention scope. got: {msg}"
        );
    }

    // ── G3: C2 블랙리스트 경로 → 스코프 가드 거부 ────────────────────────────

    #[test]
    fn g3_read_content_blacklisted_path_blocked() {
        let conn = test_conn();
        grant_consent(&conn);
        let tmp = std::env::temp_dir();
        // /usr は C2 블랙리스트에 포함. root=/usr 설정해도 블랙리스트가 막는다.
        let result = dispatch_tool(
            "read_content",
            json!({"path": "/usr/local/test.txt"}),
            &conn,
            &tmp,
            "/usr",
        );
        assert!(result.is_err(), "blacklisted path must be rejected (G3)");
        let msg = result.unwrap_err();
        assert!(
            msg.contains("G2") || msg.contains("G3") || msg.contains("Blacklisted")
                || msg.contains("범위"),
            "rejection must mention scope guard. got: {msg}"
        );
    }

    // ── resolve_sidecar: cwd 무관하게 reader.py 를 찾아야 함 ──────────────────
    // 회귀: tauri dev 는 cwd=src-tauri 로 실행 → 단일 cwd/sidecar 경로로는 부재.
    // CARGO_MANIFEST_DIR 앵커 후보가 있으므로 어떤 cwd 에서도 해석되어야 한다.
    #[test]
    fn resolve_sidecar_finds_reader_from_src_tauri_cwd() {
        // cwd 를 src-tauri(=CARGO_MANIFEST_DIR)로 설정 — tauri dev 와 동일 조건.
        let manifest = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        std::env::set_current_dir(&manifest).expect("set CWD to src-tauri");

        let resolved = resolve_sidecar().expect("사이드카가 해석되어야 함(cwd=src-tauri)");
        assert!(
            resolved.exists(),
            "해석된 경로가 실재해야 함: {}",
            resolved.display()
        );
        assert!(
            resolved.ends_with("sidecar/reader.py"),
            "reader.py 로 끝나야 함: {}",
            resolved.display()
        );
    }

    // ── DoD 1 e2e: 실제 Claude API + 실제 사이드카 + 실제 키체인 ─────────────
    // 실행: cargo test --manifest-path src-tauri/Cargo.toml -- --ignored e2e_dod1 --nocapture
    #[test]
    #[ignore = "requires keychain API key and network — run manually"]
    fn e2e_dod1_tool_sequence() {
        use crate::guard;
        use tidydog_core::Guard;

        // sidecar/reader.py 위치 확보: CARGO_MANIFEST_DIR(src-tauri/) 의 부모 = 워크스페이스 루트.
        let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("workspace root");
        std::env::set_current_dir(workspace_root).expect("set CWD to workspace root");

        // ── 가드를 통과하는 임시 디렉터리 확보 ───────────────────────────────
        // macOS: std::env::temp_dir() → /var/folders/… → canonicalize → /private/var/… → Blacklisted.
        // 시스템 temp 가 가드를 통과하면 그대로 쓰고, 막히면 홈 아래 tempdir 를 시도한다.
        // HOME 도 없으면 이 환경에서 e2e 를 실행할 수 없다 → skip(return).
        let tmp_dir: tempfile::TempDir = {
            // 1차: 시스템 temp
            let sys = tempfile::Builder::new()
                .prefix("tidydog_e2e_")
                .tempdir()
                .expect("create tempdir");
            let sys_path = sys.path().to_path_buf();
            let probe = guard::make_guard(vec![sys_path.clone()]);
            if probe.check(&sys_path).is_ok() {
                sys
            } else {
                // 2차: 홈 디렉터리 (dirs 크레이트로 조회, 하드코딩 없음)
                let home = match dirs::home_dir() {
                    Some(h) => h,
                    None => {
                        eprintln!("[e2e] SKIP: HOME unavailable — cannot find guard-safe tmpdir");
                        return;
                    }
                };
                tempfile::Builder::new()
                    .prefix("tidydog_e2e_")
                    .tempdir_in(&home)
                    .expect("create tempdir in home")
            }
        };
        let tmp = tmp_dir.path().to_path_buf();

        // 선택된 경로가 실제로 가드를 통과하는지 사전 검증.
        {
            let probe = guard::make_guard(vec![tmp.clone()]);
            assert!(
                probe.check(&tmp).is_ok(),
                "test dir {:?} must pass scope guard — check blacklist",
                tmp
            );
        }

        // 스테이징 디렉터리 (tmp 하위이므로 TempDir 와 함께 자동 정리됨).
        let staging = tmp.join("_staging");
        std::fs::create_dir_all(&staging).unwrap();

        // ── 테스트 파일 3개 생성 ────────────────────────────────────────────
        std::fs::write(
            tmp.join("python_notes.txt"),
            "Python 학습 노트\n\n1. 리스트 컴프리헨션: [x*2 for x in range(10)]\n\
             2. 람다: lambda x: x+1\n3. 데코레이터: @property\n\
             4. async/await 비동기 패턴\n5. 타입 힌트: def add(a: int, b: int) -> int",
        ).unwrap();
        std::fs::write(
            tmp.join("recipe_pasta.txt"),
            "카르보나라 레시피\n\n재료: 스파게티 200g, 베이컨 150g, 계란 2개, 파르메산 치즈 50g\n\n\
             조리법:\n1. 스파게티 알덴테로 삶기\n2. 베이컨+마늘 볶기\n\
             3. 계란+치즈 소스 만들기\n4. 뜨거운 파스타에 버무리기",
        ).unwrap();
        std::fs::write(
            tmp.join("budget_q3.txt"),
            "3분기 프로젝트 예산안\n\n총 예산: 5,000만원\n\
             - 인건비: 3,000만원 (60%)\n- 서버: 800만원 (16%)\n\
             - 마케팅: 700만원 (14%)\n- 기타: 500만원 (10%)\n\n집행 기한: 2026-09-30",
        ).unwrap();

        // ── DB 초기화 + AI 동의 ──────────────────────────────────────────────
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::db::init_schema(&conn).unwrap();
        conn.execute(
            "INSERT INTO settings (key, value) VALUES ('ai_content_consent_granted', 'true')",
            [],
        ).unwrap();

        // ── API 키: ANTHROPIC_API_KEY env → TIDYDOG_API_KEY env → 키체인 순 ──
        let api_key = crate::keyutil::get_api_key()
            .expect("API key not found — set ANTHROPIC_API_KEY env or store in keychain");

        let root = tmp.to_string_lossy().to_string();
        let client = ClaudeClient { api_key, target_dir: Some(root.clone()) };

        eprintln!("\n══ DoD1 e2e start ══ root={root}");

        let messages = vec![json!({
            "role": "user",
            "content": format!(
                "폴더 {root} 를 정리해줘. 각 파일 내용을 읽고 분류해서 정리 플랜을 만들어줘."
            )
        })];

        let result = run_agent_loop(&client, &conn, &staging, messages, false, &root);

        eprintln!("\n══ DoD1 e2e result ══");
        match &result {
            Ok(AgentResult::PlanProposed { plan_id, op_count, move_count, stage_count, risk_score, preview, ops: _ }) => {
                eprintln!("✅ PlanProposed plan_id={plan_id} ops={op_count} (move={move_count}/stage={stage_count}) risk={risk_score:.2} preview={preview}");
            }
            Ok(AgentResult::Text { message }) => eprintln!("Text: {message}"),
            Ok(AgentResult::StepLimitReached { partial_message }) => eprintln!("StepLimit: {partial_message}"),
            Ok(other) => eprintln!("Other: {other:?}"),
            Err(e) => eprintln!("❌ Error: {e}"),
        }
        eprintln!("════════════════════\n");

        // tmp_dir 여기서 drop → 파일 자동 정리.

        assert!(
            matches!(result, Ok(AgentResult::PlanProposed { .. })),
            "DoD1: expected PlanProposed"
        );
    }

    // ── DoD 3: R1 — propose_rule_change 는 ORGANIZER.md 를 직접 쓰지 않는다 ──

    #[test]
    fn dod3_r1_propose_does_not_write_organizer_md() {
        let tmp = tempfile::tempdir().unwrap();
        let organizer = tmp.path().join("ORGANIZER.md");
        let original = "tag:세금 -> 문서/세금\n";
        std::fs::write(&organizer, original).unwrap();

        let conn = test_conn();
        conn.execute(
            "INSERT INTO settings (key,value) VALUES ('ai_content_consent_granted','true')",
            [],
        )
        .unwrap();

        let root = tmp.path().to_string_lossy().to_string();
        // API 키가 없으므로 실패하지만 ORGANIZER.md 는 변경되지 않아야 함.
        let _ = crate::rule_change::propose_rule_change_tool(
            "스크린샷 규칙 추가", &conn, &root,
        );

        let after = std::fs::read_to_string(&organizer).unwrap();
        assert_eq!(
            original, after,
            "R1: propose_rule_change 는 ORGANIZER.md 를 직접 쓰면 안 됨"
        );
    }

    // ── DoD 4: R2 — apply_rule_change 는 에이전트 카탈로그에 없다 ────────────

    #[test]
    fn dod4_r2_apply_rule_change_not_in_tool_catalog() {
        let schemas = tool_schemas();
        let names: Vec<&str> = schemas
            .iter()
            .filter_map(|s| s["name"].as_str())
            .collect();
        assert!(
            !names.contains(&"apply_rule_change"),
            "R2: apply_rule_change 는 에이전트 카탈로그에 없어야 함. names={names:?}"
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
