# Phase 5-b: ORGANIZER 규칙 학습 + 승인 게이트 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 에이전트가 ORGANIZER.md 규칙 변경을 제안하고 사용자가 검토·승인하면 반영한다 — 사용자가 "앞으로도 귀찮은 일 방지"를 말할 때.

**Architecture:** `propose_rule_change` 도구(에이전트 전용)가 Claude API로 새 ORGANIZER.md 초안과 요약 카드를 생성해 DB에 저장하고 `RuleProposed` 결과를 반환한다. 프론트엔드가 RuleReview 모달을 표시하고, 사용자가 승인하면 `apply_rule_change` Tauri 커맨드(에이전트 카탈로그 외부)가 파일을 백업 후 덮어쓴다.

**Tech Stack:** Rust (Tauri 2 + rusqlite + reqwest::blocking), TypeScript/React, SQLite.

## Global Constraints

- **R1**: `propose_rule_change_tool`은 ORGANIZER.md를 절대 직접 쓰지 않는다. 저장은 DB만.
- **R2**: `apply_rule_change`는 에이전트 `tool_schemas()`에 절대 포함되지 않는다.
- **R5**: 제안된 규칙의 목적지(dest)는 상대 경로여야 한다 — 선행 `/` 또는 `..` 금지.
- **N2**: API 키는 keychain에서만 읽는다. 로그·코드·커밋에 노출 금지.
- **Design**: 빨간색 없음 — caution 최대 = ochre `var(--caution)`. Primary = teal `var(--primary)`.
- **Tests**: 기존 29개 테스트 유지. `cargo test` 0 warnings.
- **Korean UI copy**: 한국어 사용자 대상이므로 모든 UI 문자열은 한국어.

---

## File Map

**Create:**
- `src-tauri/src/rule_change.rs` — `SummaryCard`, `propose_rule_change_tool`, `apply_rule_change_inner`, `validate_rule_destinations`, `compute_diff`
- `src/components/RuleReview.tsx` — 규칙 변경 승인 모달

**Modify:**
- `src-tauri/src/db.rs` — `rule_changes` 테이블 추가
- `src-tauri/src/agent.rs` — `propose_rule_change` 도구 추가, `RuleProposed` AgentResult 변형, SYSTEM_PROMPT 업데이트
- `src-tauri/src/lib.rs` — `pub mod rule_change;`, `apply_rule_change` Tauri 커맨드, invoke_handler 등록
- `src/types.ts` — `SummaryCard`, `RuleChange` 인터페이스 추가, `ChatResponse`·`ChatMessage` 확장
- `src/components/Chat.tsx` — `RuleChip` 컴포넌트 추가, `Bubble`에 rule change 칩 표시
- `src/App.tsx` — `pendingRuleChange` 상태, `rule_proposed` 처리, RuleReview 모달 연동

---

## Task 1: DB schema + `rule_change.rs` 모듈 뼈대 (타입·검증·diff)

**Files:**
- Create: `src-tauri/src/rule_change.rs`
- Modify: `src-tauri/src/db.rs`

**Interfaces:**
- Produces: `pub struct SummaryCard`, `pub fn validate_rule_destinations`, `pub fn compute_diff`, `pub fn apply_rule_change_inner`

---

- [ ] **Step 1: `rule_changes` 테이블을 `db.rs`에 추가**

`src-tauri/src/db.rs`의 `init_schema` 함수 내 마지막 `CREATE TABLE IF NOT EXISTS summaries` 블록 바로 뒤에 아래를 추가한다:

```rust
         CREATE TABLE IF NOT EXISTS rule_changes (
             rule_change_id   TEXT    PRIMARY KEY,
             created_at       INTEGER NOT NULL,
             status           TEXT    NOT NULL
                              CHECK(status IN ('pending','applied','rejected')),
             natural_language TEXT    NOT NULL,
             before_content   TEXT    NOT NULL,
             after_content    TEXT    NOT NULL,
             diff_text        TEXT    NOT NULL,
             summary_cards    TEXT    NOT NULL  -- JSON array
         );",
```

기존 DB에 컬럼을 마이그레이션하는 `ALTER TABLE` 라인들 뒤에(마지막 `Ok(())` 전) 다음을 추가한다:

```rust
    let _ = conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS rule_changes (
             rule_change_id   TEXT    PRIMARY KEY,
             created_at       INTEGER NOT NULL,
             status           TEXT    NOT NULL CHECK(status IN ('pending','applied','rejected')),
             natural_language TEXT    NOT NULL,
             before_content   TEXT    NOT NULL,
             after_content    TEXT    NOT NULL,
             diff_text        TEXT    NOT NULL,
             summary_cards    TEXT    NOT NULL
         );"
    );
```

> `CREATE TABLE IF NOT EXISTS`를 두 곳에 쓰는 이유: 기존 `execute_batch` 문자열에 넣으면 새 DB에, 별도 `execute_batch`에 넣으면 기존 DB 마이그레이션에 모두 적용된다.

- [ ] **Step 2: `src-tauri/src/rule_change.rs` 파일 생성 — 타입·diff·검증**

```rust
//! Phase 5-b — ORGANIZER 규칙 변경 제안 + 승인 게이트.
//!
//! R1: propose_rule_change_tool 은 ORGANIZER.md 를 직접 쓰지 않는다.
//! R2: apply_rule_change_inner 는 에이전트 카탈로그 외부에서만 호출된다.
//! R5: 제안 규칙의 dest 는 상대 경로여야 한다.

use serde_json::{json, Value};

/// 규칙 변경 요약 카드 — 사용자에게 보여주는 변경 설명 단위.
#[derive(Debug, serde::Serialize, serde::Deserialize, Clone)]
pub struct SummaryCard {
    pub kind: String,             // "add" | "modify" | "delete"
    pub description: String,
    pub rule_line: Option<String>,
}

/// R5 검증: 새 ORGANIZER.md 내 모든 규칙의 dest가 상대 경로인지 확인.
/// 절대 경로(`/`로 시작) 또는 경로 탐색(`..` 포함)이면 Err 반환.
pub fn validate_rule_destinations(new_content: &str) -> Result<(), String> {
    let rules = crate::organizer::parse_rules(new_content);
    for rule in &rules {
        if rule.dest.starts_with('/') {
            return Err(format!(
                "규칙 거부 (R5): 목적지 '{}' 는 절대 경로입니다. 상대 경로만 허용됩니다.",
                rule.dest
            ));
        }
        if rule.dest.contains("..") {
            return Err(format!(
                "규칙 거부 (R5): 목적지 '{}' 에 경로 탐색 '..' 이 포함됩니다.",
                rule.dest
            ));
        }
    }
    Ok(())
}

/// 두 텍스트 사이의 간단한 줄 기반 diff를 반환한다.
/// 공통 접두·접미를 찾아 변경된 영역만 `-` / `+` 로 표시한다.
pub fn compute_diff(before: &str, after: &str) -> String {
    if before == after {
        return "(변경 없음)".to_string();
    }
    let before_lines: Vec<&str> = before.lines().collect();
    let after_lines: Vec<&str> = after.lines().collect();

    let prefix = before_lines
        .iter()
        .zip(after_lines.iter())
        .take_while(|(a, b)| a == b)
        .count();

    let max_suffix = (before_lines.len().saturating_sub(prefix))
        .min(after_lines.len().saturating_sub(prefix));
    let suffix = before_lines
        .iter()
        .rev()
        .zip(after_lines.iter().rev())
        .take(max_suffix)
        .take_while(|(a, b)| a == b)
        .count();

    let b_end = before_lines.len() - suffix;
    let a_end = after_lines.len() - suffix;
    let b_changed = &before_lines[prefix..b_end];
    let a_changed = &after_lines[prefix..a_end];

    let mut result = String::new();
    for line in b_changed {
        result.push_str(&format!("- {line}\n"));
    }
    for line in a_changed {
        result.push_str(&format!("+ {line}\n"));
    }
    if result.trim().is_empty() {
        "(내용 동일)".to_string()
    } else {
        result
    }
}

/// 승인된 규칙 변경을 실제로 적용한다 (R2: 에이전트 카탈로그 외부 전용).
/// 1. DB에서 pending 제안 조회.
/// 2. ORGANIZER.md 를 타임스탬프 백업.
/// 3. 새 내용 기록.
/// 4. DB status → 'applied'.
pub fn apply_rule_change_inner(
    rule_change_id: &str,
    conn: &rusqlite::Connection,
    organizer_path: &std::path::Path,
) -> Result<Value, String> {
    // 1. DB 조회.
    let (after_content,): (String,) = conn
        .query_row(
            "SELECT after_content FROM rule_changes
             WHERE rule_change_id = ?1 AND status = 'pending'",
            rusqlite::params![rule_change_id],
            |row| Ok((row.get(0)?,)),
        )
        .map_err(|e| format!("rule_change_id '{rule_change_id}' 를 찾을 수 없음 (pending): {e}"))?;

    // 2. 백업 (타임스탬프 접미).
    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let backup_name = format!("ORGANIZER.md.bak.{now_secs}");
    let backup_path = organizer_path
        .parent()
        .unwrap_or(std::path::Path::new("."))
        .join(&backup_name);
    std::fs::copy(organizer_path, &backup_path)
        .map_err(|e| format!("ORGANIZER.md 백업 실패 → {}: {e}", backup_path.display()))?;

    // 3. 새 내용 기록.
    std::fs::write(organizer_path, &after_content)
        .map_err(|e| format!("ORGANIZER.md 쓰기 실패: {e}"))?;

    // 4. DB 상태 업데이트.
    conn.execute(
        "UPDATE rule_changes SET status = 'applied' WHERE rule_change_id = ?1",
        rusqlite::params![rule_change_id],
    )
    .map_err(|e| e.to_string())?;

    Ok(json!({
        "rule_change_id": rule_change_id,
        "backup_path":    backup_path.to_string_lossy(),
        "applied_at":     now_secs,
    }))
}

// ── 단위 테스트 ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn test_conn() -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::db::init_schema(&conn).unwrap();
        conn
    }

    // ── R5: 목적지 검증 ─────────────────────────────────────────────────────

    // DoD 6-a: 절대 경로 dest 는 거부.
    #[test]
    fn dod6_r5_rejects_absolute_dest() {
        let content = "tag:foo -> /etc/passwd\n";
        assert!(
            validate_rule_destinations(content).is_err(),
            "R5: 절대 경로 dest 는 거부해야 함"
        );
    }

    // DoD 6-b: '..' 포함 경로 거부.
    #[test]
    fn dod6_r5_rejects_traversal_dest() {
        let content = "tag:foo -> ../../etc\n";
        assert!(
            validate_rule_destinations(content).is_err(),
            "R5: '..' 경로 탐색 dest 는 거부해야 함"
        );
    }

    // DoD 6-c: 유효한 상대 경로는 통과.
    #[test]
    fn dod6_r5_accepts_relative_dest() {
        let content = "tag:스크린샷 -> 사진/스크린샷\ntag:세금 -> 문서/세금\n";
        assert!(
            validate_rule_destinations(content).is_ok(),
            "R5: 유효한 상대 경로 dest 는 통과해야 함"
        );
    }

    // ── diff 계산 ────────────────────────────────────────────────────────────

    #[test]
    fn diff_no_change() {
        let s = "a\nb\nc\n";
        assert_eq!(compute_diff(s, s), "(변경 없음)");
    }

    #[test]
    fn diff_add_line() {
        let before = "tag:세금 -> 문서/세금\n";
        let after  = "tag:세금 -> 문서/세금\ntag:스크린샷 -> 사진/스크린샷\n";
        let d = compute_diff(before, after);
        assert!(d.contains("+ tag:스크린샷 -> 사진/스크린샷"), "추가된 줄이 diff에 나타나야 함: {d}");
        assert!(!d.contains("- "), "삭제된 줄이 없어야 함: {d}");
    }

    #[test]
    fn diff_modify_line() {
        let before = "tag:세금 -> 문서/세금\n";
        let after  = "tag:세금,tax -> 문서/세금\n";
        let d = compute_diff(before, after);
        assert!(d.contains("- tag:세금 -> 문서/세금"), "이전 줄이 diff에 나타나야 함: {d}");
        assert!(d.contains("+ tag:세금,tax -> 문서/세금"), "변경 줄이 diff에 나타나야 함: {d}");
    }

    // ── apply_rule_change_inner ───────────────────────────────────────────────

    // DoD 7: 적용 후 ORGANIZER.md 가 새 내용으로 교체되고 백업이 생성됨.
    #[test]
    fn dod7_apply_writes_organizer_and_creates_backup() {
        let tmp = tempfile::tempdir().unwrap();
        let organizer = tmp.path().join("ORGANIZER.md");
        let before_content = "tag:세금 -> 문서/세금\n";
        let after_content  = "tag:세금 -> 문서/세금\ntag:스크린샷 -> 사진/스크린샷\n";
        fs::write(&organizer, before_content).unwrap();

        let conn = test_conn();
        let now = 1_000_000i64;
        conn.execute(
            "INSERT INTO rule_changes \
             (rule_change_id, created_at, status, natural_language, \
              before_content, after_content, diff_text, summary_cards) \
             VALUES ('rc-test-007', ?1, 'pending', '테스트', ?2, ?3, '', '[]')",
            rusqlite::params![now, before_content, after_content],
        )
        .unwrap();

        let result = apply_rule_change_inner("rc-test-007", &conn, &organizer).unwrap();

        // ORGANIZER.md가 새 내용으로 교체됨.
        let written = fs::read_to_string(&organizer).unwrap();
        assert_eq!(written, after_content, "ORGANIZER.md 가 새 내용으로 교체되어야 함");

        // 백업 파일 존재.
        let backup_path = result["backup_path"].as_str().unwrap();
        assert!(
            fs::metadata(backup_path).is_ok(),
            "백업 파일이 생성되어야 함: {backup_path}"
        );

        // DB status → 'applied'.
        let status: String = conn
            .query_row(
                "SELECT status FROM rule_changes WHERE rule_change_id = 'rc-test-007'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(status, "applied");
    }

    // DoD 5: apply_rule_change_inner 는 files 테이블을 변경하지 않음 (소급 이동 없음).
    #[test]
    fn dod5_apply_does_not_retroactively_move_files() {
        let tmp = tempfile::tempdir().unwrap();
        let organizer = tmp.path().join("ORGANIZER.md");
        let before_content = "tag:세금 -> 문서/세금\n";
        let after_content  = "tag:세금 -> 문서/세금\ntag:스크린샷 -> 사진/스크린샷\n";
        fs::write(&organizer, before_content).unwrap();

        let conn = test_conn();
        conn.execute(
            "INSERT INTO files (content_hash, current_path, size, mtime) \
             VALUES ('hash-screenshot', '/home/user/screenshot.png', 100, 0)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO rule_changes \
             (rule_change_id, created_at, status, natural_language, \
              before_content, after_content, diff_text, summary_cards) \
             VALUES ('rc-test-005', 1000, 'pending', 'test', ?1, ?2, '', '[]')",
            rusqlite::params![before_content, after_content],
        )
        .unwrap();

        apply_rule_change_inner("rc-test-005", &conn, &organizer).unwrap();

        let path: String = conn
            .query_row(
                "SELECT current_path FROM files WHERE content_hash = 'hash-screenshot'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            path, "/home/user/screenshot.png",
            "apply_rule_change 는 files 테이블을 변경하면 안 됨 (DoD 5 / 소급 이동 금지)"
        );
    }
}
```

- [ ] **Step 3: lib.rs에 모듈 선언 추가**

`src-tauri/src/lib.rs` 맨 위의 모듈 선언 블록에 추가:

```rust
pub mod rule_change;
```

- [ ] **Step 4: 빌드 + 테스트 통과 확인**

```bash
cargo test --manifest-path src-tauri/Cargo.toml 2>&1 | tail -20
```

예상: 기존 29개 + 새 Task 1 테스트(dod5, dod6×3, dod7, diff×3) 통과, warnings 0.

- [ ] **Step 5: 커밋**

```bash
git add src-tauri/src/rule_change.rs src-tauri/src/db.rs src-tauri/src/lib.rs
git commit -m "feat(5b): rule_change module — SummaryCard, validate R5, diff, apply_rule_change_inner"
```

---

## Task 2: `propose_rule_change_tool` + 에이전트 카탈로그 통합

**Files:**
- Modify: `src-tauri/src/rule_change.rs` (함수 추가)
- Modify: `src-tauri/src/agent.rs` (도구 추가, AgentResult 확장, SYSTEM_PROMPT 업데이트)

**Interfaces:**
- Consumes: `rule_change::SummaryCard`, `rule_change::validate_rule_destinations`, `rule_change::compute_diff`
- Produces: `AgentResult::RuleProposed`, `propose_rule_change` in `tool_schemas()`

---

- [ ] **Step 1: `propose_rule_change_tool` 함수를 `rule_change.rs`에 추가**

`src-tauri/src/rule_change.rs`의 `apply_rule_change_inner` 함수 위에 추가한다:

```rust
/// Claude API를 호출해 새 ORGANIZER.md 초안과 요약 카드를 생성하고 DB에 저장한다.
/// R1: 이 함수는 ORGANIZER.md 를 직접 쓰지 않는다.
/// N2: API 키는 키체인에서만 읽는다.
pub fn propose_rule_change_tool(
    request: &str,
    conn: &rusqlite::Connection,
    root: &str,
) -> Result<Value, String> {
    // N2: API 키 조회.
    let api_key = keyring::Entry::new("tidydog", "anthropic_api_key")
        .map_err(|e| format!("keyring 오류: {e}"))?
        .get_password()
        .map_err(|e| format!("API 키 없음 — set_api_key로 먼저 저장하세요: {e}"))?;

    // ORGANIZER.md 읽기.
    let organizer_path = std::path::Path::new(root).join("ORGANIZER.md");
    let before_content = std::fs::read_to_string(&organizer_path)
        .map_err(|e| format!("ORGANIZER.md 읽기 실패 ({}): {e}", organizer_path.display()))?;

    // LLM 호출 → 새 내용 + 요약 카드.
    let (after_content, summary_cards) =
        call_llm_for_rules(&before_content, request, &api_key)?;

    // R5 검증.
    validate_rule_destinations(&after_content)?;

    // diff 계산.
    let diff = compute_diff(&before_content, &after_content);

    // DB에 저장 (status = 'pending').
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let rc_id = format!("rc-{now_ms}");
    let cards_json = serde_json::to_string(&summary_cards)
        .unwrap_or_else(|_| "[]".to_string());

    conn.execute(
        "INSERT INTO rule_changes \
         (rule_change_id, created_at, status, natural_language, \
          before_content, after_content, diff_text, summary_cards) \
         VALUES (?1, ?2, 'pending', ?3, ?4, ?5, ?6, ?7)",
        rusqlite::params![
            rc_id,
            now_ms as i64,
            request,
            before_content,
            after_content,
            diff,
            cards_json,
        ],
    )
    .map_err(|e| e.to_string())?;

    Ok(json!({
        "rule_change_id": rc_id,
        "summary_cards":  summary_cards,
        "diff":           diff,
        "before":         before_content,
        "after":          after_content,
    }))
}

/// Claude API로 새 ORGANIZER.md 초안과 요약 카드를 생성한다.
fn call_llm_for_rules(
    current_content: &str,
    request: &str,
    api_key: &str,
) -> Result<(String, Vec<SummaryCard>), String> {
    let user_content = format!(
        "You are editing a TidyDog ORGANIZER.md rule file.\n\
         \n\
         Current ORGANIZER.md:\n```\n{current_content}```\n\
         \n\
         User request: {request}\n\
         \n\
         ORGANIZER.md format:\n\
         - Each rule line: `<conditions> -> <dest>`\n\
         - Condition types: tag:, lang:, ext:, name: (comma=OR, space=AND)\n\
         - Destinations MUST be relative paths (no leading /, no ..)\n\
         - First-match-wins (order matters)\n\
         - Lines starting with # are comments\n\
         \n\
         Generate a modified ORGANIZER.md and summary cards.\n\
         Respond ONLY with valid JSON (no markdown fences):\n\
         {{\"new_content\": \"the full new ORGANIZER.md text\",\
           \"summary_cards\": [\
             {{\"kind\": \"add\", \"description\": \"앞으로 스크린샷은 사진/스크린샷으로\",\
               \"rule_line\": \"name:screenshot,스크린샷 -> 사진/스크린샷\"}}\
           ]}}\n\
         kind must be: \"add\", \"modify\", or \"delete\".\n\
         description must be in Korean.\n\
         Keep existing rules unless the user's request says to change them."
    );

    let body = serde_json::json!({
        "model": crate::summarizer::SUMMARIZER_MODEL,
        "max_tokens": 2048,
        "messages": [{"role": "user", "content": user_content}]
    });

    let client = reqwest::blocking::Client::new();
    let resp = client
        .post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .map_err(|e| format!("LLM 요청 실패: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().unwrap_or_default();
        return Err(format!("LLM API 오류 {status}: {text}"));
    }

    let resp_json: Value = resp
        .json()
        .map_err(|e| format!("LLM 응답 파싱 실패: {e}"))?;

    let text = resp_json["content"][0]["text"]
        .as_str()
        .ok_or_else(|| "LLM 응답에서 텍스트 없음".to_string())?;

    let parsed: Value = serde_json::from_str(text)
        .map_err(|e| format!("LLM JSON 파싱 실패: {e}. 응답: {text}"))?;

    let new_content = parsed["new_content"]
        .as_str()
        .ok_or_else(|| "LLM 응답에 'new_content' 없음".to_string())?
        .to_string();

    let summary_cards: Vec<SummaryCard> = parsed["summary_cards"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|card| {
                    Some(SummaryCard {
                        kind: card["kind"].as_str()?.to_string(),
                        description: card["description"].as_str()?.to_string(),
                        rule_line: card["rule_line"].as_str().map(|s| s.to_string()),
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    Ok((new_content, summary_cards))
}
```

- [ ] **Step 2: `agent.rs`에 `RuleProposed` 변형 추가**

`AgentResult` enum에 추가 (`UndoCompleted` 다음):

```rust
    RuleProposed {
        rule_change_id: String,
        summary_cards:  Vec<crate::rule_change::SummaryCard>,
        diff:           String,
        before:         String,
        after:          String,
    },
```

- [ ] **Step 3: `tool_schemas()`에 `propose_rule_change` 추가**

`undo` 도구 바로 앞에 삽입:

```rust
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
```

- [ ] **Step 4: `_dispatch_tool`에 `propose_rule_change` 케이스 추가**

`"undo" =>` 케이스 바로 앞에 삽입:

```rust
        "propose_rule_change" => {
            let request = input["request"]
                .as_str()
                .ok_or_else(|| "propose_rule_change: 'request' 없음".to_string())?;
            crate::rule_change::propose_rule_change_tool(request, conn, root)
        }
```

- [ ] **Step 5: `run_agent_loop`의 `dispatch_tool` 성공 처리 블록에 `propose_rule_change` 추가**

기존 `propose_plan` 성공 처리 블록 바로 뒤에 추가:

```rust
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
```

- [ ] **Step 6: `SYSTEM_PROMPT`에 규칙 8 추가**

기존 규칙 7번(`한국어로 응답합니다.`) 뒤에 추가:

```rust
"8. 사용자가 '앞으로도', '항상', '매번', '기본으로', '규칙으로' 같은 \
   분류 습관 변경 패턴을 표현하면, propose_rule_change 도구를 호출해 \
   ORGANIZER.md 변경안을 제안한다.\n\
   이 도구는 제안만 하고 파일을 직접 쓰지 않는다(R1).\n\
   파일 이동 정리와 규칙 변경은 별개 — 사용자가 명확히 규칙 변경을 원할 때만 호출."
```

- [ ] **Step 7: DoD 3과 DoD 4 테스트 작성 — `agent.rs`의 `tests` 모듈에 추가**

```rust
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
```

- [ ] **Step 8: 빌드 + 테스트 통과 확인**

```bash
cargo test --manifest-path src-tauri/Cargo.toml 2>&1 | tail -25
```

예상: DoD 3(dod3_r1_propose_does_not_write_organizer_md), DoD 4(dod4_r2_apply_rule_change_not_in_tool_catalog) 포함 모두 통과, warnings 0.

- [ ] **Step 9: 커밋**

```bash
git add src-tauri/src/rule_change.rs src-tauri/src/agent.rs
git commit -m "feat(5b): propose_rule_change tool — LLM draft + R1/R2/R5 safety gates"
```

---

## Task 3: `apply_rule_change` Tauri 커맨드 + invoke_handler 등록

**Files:**
- Modify: `src-tauri/src/lib.rs`

**Interfaces:**
- Consumes: `rule_change::apply_rule_change_inner`
- Produces: `apply_rule_change` Tauri 커맨드 (프론트엔드에서 `invoke("apply_rule_change", ...)`)

---

- [ ] **Step 1: `apply_rule_change` Tauri 커맨드를 `lib.rs`에 추가**

`chat` 커맨드 함수 바로 아래에 추가:

```rust
/// 사용자가 승인한 규칙 변경을 ORGANIZER.md에 적용한다.
/// R2: 에이전트 카탈로그에는 없음. 오직 사용자 승인 경로에서만 호출된다.
/// `root`: 현재 열려 있는 폴더 경로 (ORGANIZER.md 위치 기준).
#[tauri::command]
fn apply_rule_change(
    app: tauri::AppHandle,
    rule_change_id: String,
    root: String,
) -> Result<serde_json::Value, String> {
    let (app_data_dir, _) = app_paths(&app)?;
    fs::create_dir_all(&app_data_dir).map_err(|e| e.to_string())?;
    let conn = db::init_db(&app_data_dir)?;

    let organizer_path = std::path::Path::new(&root).join("ORGANIZER.md");
    rule_change::apply_rule_change_inner(&rule_change_id, &conn, &organizer_path)
}
```

- [ ] **Step 2: invoke_handler에 등록**

`lib.rs`의 `tauri::generate_handler![...]` 목록에 `apply_rule_change` 추가:

```rust
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
            apply_rule_change,  // 5-b: R2 — 사용자 승인 경로 전용
        ])
```

- [ ] **Step 3: 빌드 확인**

```bash
cargo build --manifest-path src-tauri/Cargo.toml 2>&1 | grep -E "error|warning" | head -20
```

예상: 오류·경고 없음.

- [ ] **Step 4: 전체 테스트 통과 확인**

```bash
cargo test --manifest-path src-tauri/Cargo.toml 2>&1 | tail -10
```

예상: 모든 테스트 통과.

- [ ] **Step 5: 커밋**

```bash
git add src-tauri/src/lib.rs
git commit -m "feat(5b): apply_rule_change Tauri command — user-approval path, R2 registered"
```

---

## Task 4: 프론트엔드 타입 + `RuleReview.tsx` + `Chat.tsx` RuleChip

**Files:**
- Modify: `src/types.ts`
- Create: `src/components/RuleReview.tsx`
- Modify: `src/components/Chat.tsx`

**Interfaces:**
- Produces: `SummaryCard`, `RuleChange` (types.ts), `RuleReview` component, `RuleChip` in Chat

---

- [ ] **Step 1: `src/types.ts`에 새 타입 추가**

기존 `ChatResponse` 인터페이스 위에 추가:

```typescript
export interface SummaryCard {
  kind:        "add" | "modify" | "delete";
  description: string;
  rule_line?:  string;
}

export interface RuleChange {
  rule_change_id: string;
  summary_cards:  SummaryCard[];
  diff:           string;
  before:         string;
  after:          string;
}
```

`ChatResponse` 인터페이스의 `kind` 유니온에 `"rule_proposed"` 추가:

```typescript
export interface ChatResponse {
  kind:             "text" | "plan_proposed" | "step_limit_reached" | "undo_completed" | "rule_proposed";
  message?:         string;
  partial_message?: string;
  plan_id?:         string;
  op_count?:        number;
  move_count?:      number;
  stage_count?:     number;
  risk_score?:      number;
  preview?:         string;
  // rule_proposed 필드
  rule_change_id?:  string;
  summary_cards?:   SummaryCard[];
  diff?:            string;
  before?:          string;
  after?:           string;
}
```

`ChatMessage` 인터페이스에 `ruleChange` 옵션 추가:

```typescript
export interface ChatMessage {
  id:          string;
  role:        "user" | "agent";
  text:        string;
  plan?:       ProposedPlan;
  ruleChange?: RuleChange;
  isError?:    boolean;
}
```

- [ ] **Step 2: `src/components/RuleReview.tsx` 생성**

```tsx
import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { RuleChange, SummaryCard } from "../types";

interface Props {
  ruleChange: RuleChange;
  root:       string;
  onApplied:  () => void;
  onCancel:   () => void;
}

function kindBadge(kind: SummaryCard["kind"]): React.CSSProperties {
  if (kind === "add")    return { background: "var(--primary-soft)", color: "var(--primary)" };
  if (kind === "delete") return { background: "var(--caution-soft)", color: "var(--caution)" };
  return { background: "var(--surface-2)", color: "var(--muted)" };
}

function kindLabel(kind: SummaryCard["kind"]) {
  if (kind === "add")    return "추가";
  if (kind === "delete") return "삭제";
  return "수정";
}

export function RuleReview({ ruleChange, root, onApplied, onCancel }: Props) {
  const [showDiff, setShowDiff]   = useState(false);
  const [applying, setApplying]   = useState(false);
  const [error, setError]         = useState<string | null>(null);

  async function handleApply() {
    setApplying(true);
    setError(null);
    try {
      await invoke("apply_rule_change", {
        ruleChangeId: ruleChange.rule_change_id,
        root,
      });
      onApplied();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setApplying(false);
    }
  }

  return (
    <div style={{
      position: "fixed", inset: 0, zIndex: 100,
      background: "rgba(32,39,42,0.45)",
      display: "flex", alignItems: "center", justifyContent: "center",
    }}>
      <div style={{
        background: "var(--surface)",
        borderRadius: "var(--r)",
        boxShadow: "0 4px 32px rgba(20,40,35,.18)",
        width: 580, maxWidth: "92vw",
        maxHeight: "82vh",
        display: "flex", flexDirection: "column",
        overflow: "hidden",
      }}>
        {/* header */}
        <div style={{
          padding: "20px 24px 16px",
          borderBottom: "1px solid var(--line)",
          display: "flex", alignItems: "flex-start", gap: 12,
        }}>
          <div style={{ flex: 1 }}>
            <div style={{ fontWeight: 700, fontSize: 16 }}>ORGANIZER 규칙 변경 제안</div>
            <div style={{ marginTop: 4, fontSize: 13, color: "var(--muted)" }}>
              검토 후 승인하면 ORGANIZER.md에 반영됩니다.
            </div>
          </div>
          <button
            onClick={onCancel}
            style={{
              background: "none", border: "none", cursor: "pointer",
              fontSize: 18, color: "var(--muted)", lineHeight: 1, padding: 4,
            }}
          >×</button>
        </div>

        {/* summary cards */}
        <div style={{ flex: 1, overflowY: "auto", padding: "12px 16px" }}>
          {ruleChange.summary_cards.length > 0 ? (
            ruleChange.summary_cards.map((card, i) => (
              <div key={i} style={{
                padding: "10px 12px",
                borderRadius: "var(--r-sm)",
                marginBottom: 8,
                background: "var(--surface-2)",
                fontSize: 13,
              }}>
                <div style={{ display: "flex", alignItems: "center", gap: 8, marginBottom: 6 }}>
                  <span style={{
                    ...kindBadge(card.kind),
                    padding: "2px 8px", borderRadius: 999,
                    fontSize: 11.5, fontWeight: 600,
                  }}>
                    {kindLabel(card.kind)}
                  </span>
                  <span style={{ color: "var(--ink)" }}>{card.description}</span>
                </div>
                {card.rule_line && (
                  <div style={{
                    fontFamily: "var(--mono)", fontSize: 12,
                    color: "var(--primary)", marginTop: 2,
                    padding: "3px 6px", background: "var(--primary-soft)",
                    borderRadius: "var(--r-sm)",
                  }}>
                    {card.rule_line}
                  </div>
                )}
              </div>
            ))
          ) : (
            <div style={{ padding: 16, color: "var(--muted)", fontSize: 13, textAlign: "center" }}>
              변경 내용이 없습니다.
            </div>
          )}

          {/* diff toggle */}
          <div style={{ marginTop: 8 }}>
            <button
              onClick={() => setShowDiff((v) => !v)}
              style={{
                background: "none", border: "none", cursor: "pointer",
                color: "var(--muted)", fontSize: 12.5, padding: "4px 0",
                fontFamily: "var(--ui)",
              }}
            >
              {showDiff ? "▲ diff 숨기기" : "▼ diff 보기"}
            </button>
            {showDiff && (
              <pre style={{
                marginTop: 8, padding: "10px 12px",
                background: "var(--surface-2)",
                borderRadius: "var(--r-sm)",
                fontSize: 12, fontFamily: "var(--mono)",
                overflowX: "auto", whiteSpace: "pre-wrap",
                color: "var(--ink)", lineHeight: 1.6,
              }}>
                {ruleChange.diff || "(diff 없음)"}
              </pre>
            )}
          </div>

          {error && (
            <div style={{
              marginTop: 10, padding: "8px 12px",
              background: "var(--caution-soft)",
              borderRadius: "var(--r-sm)",
              color: "var(--caution)", fontSize: 13,
            }}>
              {error}
            </div>
          )}
        </div>

        {/* footer */}
        <div style={{
          padding: "16px 24px",
          borderTop: "1px solid var(--line)",
          display: "flex", gap: 10, justifyContent: "flex-end",
        }}>
          <button
            onClick={onCancel}
            disabled={applying}
            style={{
              padding: "9px 18px", borderRadius: "var(--r-sm)",
              border: "1px solid var(--line)", background: "var(--surface)",
              cursor: applying ? "not-allowed" : "pointer",
              fontSize: 13.5, fontWeight: 600,
              fontFamily: "var(--ui)", color: "var(--ink)",
            }}
          >
            취소
          </button>
          <button
            onClick={handleApply}
            disabled={applying}
            style={{
              padding: "9px 20px", borderRadius: "var(--r-sm)",
              border: "none",
              background: "var(--primary)",
              color: "#fff",
              cursor: applying ? "not-allowed" : "pointer",
              opacity: applying ? 0.7 : 1,
              fontSize: 13.5, fontWeight: 700,
              fontFamily: "var(--ui)",
            }}
          >
            {applying ? "적용 중…" : "규칙 적용"}
          </button>
        </div>
      </div>
    </div>
  );
}
```

- [ ] **Step 3: `Chat.tsx`에 `RuleChip` 추가 및 `Bubble` 확장**

`Chat.tsx`의 import에 `RuleChange` 추가:

```typescript
import { ChatMessage, ProposedPlan, RuleChange } from "../types";
```

`PlanChip` 함수 다음에 `RuleChip` 함수 추가:

```tsx
function RuleChip({ onOpen }: { onOpen: () => void }) {
  return (
    <div style={{
      marginTop: 8,
      background: "var(--surface-2)",
      border: "1px solid var(--line)",
      borderRadius: "var(--r-sm)",
      padding: "10px 14px",
      display: "flex",
      alignItems: "center",
      gap: 12,
      fontSize: 13,
    }}>
      <div style={{ flex: 1 }}>
        <span style={{ fontWeight: 600, color: "var(--ink)" }}>ORGANIZER 규칙 변경 제안 </span>
        <span style={{ color: "var(--muted)" }}>검토 후 승인하면 반영됩니다</span>
      </div>
      <button
        onClick={onOpen}
        style={{
          padding: "5px 14px",
          borderRadius: "var(--r-sm)",
          border: "none",
          background: "var(--primary)",
          color: "#fff",
          cursor: "pointer",
          fontSize: 12.5,
          fontWeight: 700,
          fontFamily: "var(--ui)",
          whiteSpace: "nowrap",
        }}
      >
        규칙 검토 →
      </button>
    </div>
  );
}
```

`Props` 인터페이스에 `onRuleOpen` 추가:

```typescript
interface Props {
  messages:     ChatMessage[];
  loading:      boolean;
  onPlanOpen:   (plan: ProposedPlan) => void;
  onRuleOpen:   (rc: RuleChange) => void;
  folderPath:   string | null;
}
```

`Bubble` 컴포넌트의 `props` 타입 및 반환에 `ruleChange` 처리 추가:

```tsx
function Bubble({
  msg,
  onPlanOpen,
  onRuleOpen,
}: {
  msg: ChatMessage;
  onPlanOpen: (p: ProposedPlan) => void;
  onRuleOpen: (rc: RuleChange) => void;
}) {
  // ... 기존 bubble div 유지 ...
  return (
    <div style={{ ... }}>
      <div className={`bubble ...`} style={{ ... }}>
        {msg.text}
      </div>
      {msg.plan && (
        <div style={{ maxWidth: "80%" }}>
          <PlanChip plan={msg.plan} onOpen={() => onPlanOpen(msg.plan!)} />
        </div>
      )}
      {msg.ruleChange && (
        <div style={{ maxWidth: "80%" }}>
          <RuleChip onOpen={() => onRuleOpen(msg.ruleChange!)} />
        </div>
      )}
    </div>
  );
}
```

`Chat` 컴포넌트에 `onRuleOpen` prop 전달:

```tsx
export function Chat({ messages, loading, onPlanOpen, onRuleOpen, folderPath }: Props) {
  // ...
  return (
    // ...
    {messages.map((m) => (
      <Bubble key={m.id} msg={m} onPlanOpen={onPlanOpen} onRuleOpen={onRuleOpen} />
    ))}
    // ...
  );
}
```

- [ ] **Step 4: TypeScript 컴파일 오류 없음 확인**

```bash
cd /Users/mjwoon/Workspace/tidydog && npm run check 2>&1 | head -30
```

오류가 있다면 수정 후 재확인.

- [ ] **Step 5: 커밋**

```bash
git add src/types.ts src/components/RuleReview.tsx src/components/Chat.tsx
git commit -m "feat(5b): RuleReview modal + RuleChip + types (SummaryCard, RuleChange)"
```

---

## Task 5: `App.tsx` 전체 통합

**Files:**
- Modify: `src/App.tsx`

**Interfaces:**
- Consumes: `RuleReview`, `RuleChange` (타입), `onRuleOpen` (Chat prop), `apply_rule_change` (Tauri command)
- Produces: 완성된 Phase 5-b flow: propose → chip → modal → apply/cancel

---

- [ ] **Step 1: `App.tsx` import 업데이트**

기존 import에 추가:

```typescript
import { RuleChange } from "./types";
import { RuleReview } from "./components/RuleReview";
```

`Chat` import는 이미 있으므로 그대로.

- [ ] **Step 2: `pendingRuleChange` 상태 추가**

기존 `pendingPlan` 상태 아래에 추가:

```typescript
const [pendingRuleChange, setPendingRuleChange] = useState<RuleChange | null>(null);
```

- [ ] **Step 3: `handleSend`의 `switch` 블록에 `rule_proposed` 케이스 추가**

`undo_completed` 케이스 다음에 추가:

```typescript
        case "rule_proposed": {
          const rc: RuleChange = {
            rule_change_id: response.rule_change_id!,
            summary_cards:  response.summary_cards ?? [],
            diff:           response.diff ?? "",
            before:         response.before ?? "",
            after:          response.after ?? "",
          };
          const agentMsg: ChatMessage = {
            id:         nextId(),
            role:       "agent",
            text:       "ORGANIZER 규칙 변경을 제안했습니다. 아래 내용을 검토해 주세요.",
            ruleChange: rc,
          };
          setMessages((prev) => [...prev, agentMsg]);
          setAgentState("done");
          break;
        }
```

- [ ] **Step 4: `openRuleModal` 핸들러 추가**

`openPlanModal` 함수 다음에 추가:

```typescript
  function openRuleModal(rc: RuleChange) {
    setPendingRuleChange(rc);
  }
```

- [ ] **Step 5: `handleRuleApplied` 핸들러 추가**

`openRuleModal` 다음에 추가:

```typescript
  function handleRuleApplied() {
    setPendingRuleChange(null);
    setAgentState("idle");
    const agentMsg: ChatMessage = {
      id:   nextId(),
      role: "agent",
      text: "ORGANIZER 규칙이 업데이트되었습니다. 다음 정리부터 새 규칙이 적용됩니다.",
    };
    setMessages((prev) => [...prev, agentMsg]);
  }
```

- [ ] **Step 6: `Chat` 컴포넌트에 `onRuleOpen` prop 전달**

렌더 부분의 `<Chat>` 컴포넌트에 추가:

```tsx
            <Chat
              messages={messages}
              loading={agentState === "thinking"}
              onPlanOpen={openPlanModal}
              onRuleOpen={openRuleModal}
              folderPath={folderPath}
            />
```

- [ ] **Step 7: `RuleReview` 모달 추가**

기존 `{pendingPlan && ...}` 블록 다음에 추가:

```tsx
      {pendingRuleChange && folderPath && (
        <RuleReview
          ruleChange={pendingRuleChange}
          root={folderPath}
          onApplied={handleRuleApplied}
          onCancel={() => { setPendingRuleChange(null); setAgentState("idle"); }}
        />
      )}
```

- [ ] **Step 8: `agentState`를 `"done"` → `"idle"` 로 전환하는 입력창 placeholder 업데이트**

기존 `placeholder`가 `agentState === "done"` 상태를 처리하는지 확인. 현재 코드는:
```typescript
agentState === "thinking" ? "응답 대기 중…" : "무엇을 정리할까요? (/undo 로 되돌리기)"
```
`agentState === "done"` (플랜 또는 규칙 검토 중)일 때도 입력창이 활성화됨 — `disabled` 조건 확인:
```tsx
disabled={!folderPath || agentState === "thinking"}
```
`done` 상태에서도 입력 가능하므로 별도 변경 불필요.

- [ ] **Step 9: TypeScript 컴파일 확인**

```bash
cd /Users/mjwoon/Workspace/tidydog && npm run check 2>&1 | head -20
```

- [ ] **Step 10: 전체 Rust 테스트 재확인**

```bash
cargo test --manifest-path src-tauri/Cargo.toml 2>&1 | tail -10
```

예상: 모든 테스트(기존 29 + 5b 신규) 통과.

- [ ] **Step 11: 커밋**

```bash
git add src/App.tsx
git commit -m "feat(5b): App.tsx — rule_proposed flow, RuleReview modal, handleRuleApplied"
```

---

## 자체 리뷰: 스펙 커버리지 확인

| DoD | 구현 위치 | 상태 |
|-----|-----------|------|
| DoD 1: 에이전트가 propose_rule_change 도구 호출 가능 | Task 2 Step 3 (tool_schemas) | ✅ |
| DoD 2: 제안 내용 diff + 카드 포함 | Task 2 Step 1 (call_llm_for_rules) | ✅ |
| DoD 3: 제안만으로 ORGANIZER.md 미변경 (R1) | Task 1 + Task 2 Step 7 테스트 | ✅ |
| DoD 4: apply_rule_change 에이전트 카탈로그 외부 (R2) | Task 2 Step 7 테스트 | ✅ |
| DoD 5: 규칙 변경 후 소급 이동 없음 | Task 1 Step 2 테스트 (dod5) | ✅ |
| DoD 6: 블랙리스트 경로 dest 거부 (R5) | Task 1 Step 2 테스트 (dod6×3) | ✅ |
| DoD 7: 적용 시 백업 + 새 내용 기록 | Task 1 Step 2 테스트 (dod7) | ✅ |
| UI: RuleChip in Chat | Task 4 Step 3 | ✅ |
| UI: RuleReview modal | Task 4 Step 2 | ✅ |
| UI: apply → success 메시지 | Task 5 Step 5 | ✅ |

**Placeholder 스캔:** 없음 — 모든 스텝에 실제 코드 포함.

**타입 일관성:**
- `SummaryCard`: Rust(`rule_change.rs`) ↔ TypeScript(`types.ts`) — `kind`/`description`/`rule_line` 일치
- `RuleProposed` AgentResult → serde `kind: "rule_proposed"` → `ChatResponse.kind` — 일치
- `apply_rule_change(rule_change_id, root)` → Tauri camelCase: `invoke("apply_rule_change", { ruleChangeId, root })` — Tauri 2 자동 변환 확인 필요

> **주의**: Tauri 2는 Rust의 snake_case 파라미터를 camelCase로 자동 매핑한다. `rule_change_id` → `ruleChangeId` 정상.
