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

/// Claude API를 호출해 새 ORGANIZER.md 초안과 요약 카드를 생성하고 DB에 저장한다.
/// R1: 이 함수는 ORGANIZER.md 를 직접 쓰지 않는다.
/// N2: API 키는 키체인에서만 읽는다.
pub fn propose_rule_change_tool(
    request: &str,
    conn: &rusqlite::Connection,
    root: &str,
) -> Result<Value, String> {
    // N2: env fallback(개발 주입) → OS 키체인(최종 사용자) 순서로 조회.
    let api_key = crate::keyutil::get_api_key()?;

    // ORGANIZER.md 읽기. 최초 생성 케이스: 파일이 없으면 빈 내용을 기준으로 삼는다
    // (diff의 before=""; 승인 시 apply가 파일을 새로 만든다).
    let organizer_path = std::path::Path::new(root).join("ORGANIZER.md");
    let before_content = match std::fs::read_to_string(&organizer_path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => {
            return Err(format!(
                "ORGANIZER.md 읽기 실패 ({}): {e}",
                organizer_path.display()
            ))
        }
    };

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

    // 2. 백업 (타임스탬프 접미). R4: 반영 전 기존 파일 보존.
    //    단, 최초 생성(파일 부재) 시에는 백업할 대상이 없으므로 건너뛴다.
    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let backup_path = if organizer_path.exists() {
        let backup_name = format!("ORGANIZER.md.bak.{now_secs}");
        let path = organizer_path
            .parent()
            .unwrap_or(std::path::Path::new("."))
            .join(&backup_name);
        std::fs::copy(organizer_path, &path)
            .map_err(|e| format!("ORGANIZER.md 백업 실패 → {}: {e}", path.display()))?;
        Some(path)
    } else {
        // 최초 생성: 상위 디렉터리 보장.
        if let Some(parent) = organizer_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("ORGANIZER.md 디렉터리 생성 실패: {e}"))?;
        }
        None
    };

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
        "backup_path":    backup_path.as_ref().map(|p| p.to_string_lossy().to_string()),
        "created":        backup_path.is_none(), // true면 최초 생성(백업 없음)
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

    #[test]
    fn dod6_r5_rejects_absolute_dest() {
        let content = "tag:foo -> /etc/passwd\n";
        assert!(
            validate_rule_destinations(content).is_err(),
            "R5: 절대 경로 dest 는 거부해야 함"
        );
    }

    #[test]
    fn dod6_r5_rejects_traversal_dest() {
        let content = "tag:foo -> ../../etc\n";
        assert!(
            validate_rule_destinations(content).is_err(),
            "R5: '..' 경로 탐색 dest 는 거부해야 함"
        );
    }

    // R5: /System 경로 거부 (G3 블랙리스트 대표).
    #[test]
    fn r5_rejects_system_path() {
        let content = "tag:foo -> /System/Library/evil\n";
        let err = validate_rule_destinations(content).unwrap_err();
        assert!(
            err.contains("R5") && err.contains("절대 경로"),
            "R5: /System 경로는 '절대 경로' 메시지와 함께 거부해야 함. got: {err}"
        );
    }

    // R5: staging 경로가 절대 경로이면 거부.
    #[test]
    fn r5_rejects_absolute_staging_path() {
        let content = "tag:bar -> /Users/mjwoon/.tidydog/staging\n";
        let err = validate_rule_destinations(content).unwrap_err();
        assert!(
            err.contains("R5"),
            "R5: staging 절대 경로는 거부해야 함. got: {err}"
        );
    }

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

        let written = fs::read_to_string(&organizer).unwrap();
        assert_eq!(written, after_content, "ORGANIZER.md 가 새 내용으로 교체되어야 함");

        let backup_path = result["backup_path"].as_str().unwrap();
        assert!(
            fs::metadata(backup_path).is_ok(),
            "백업 파일이 생성되어야 함: {backup_path}"
        );

        let status: String = conn
            .query_row(
                "SELECT status FROM rule_changes WHERE rule_change_id = 'rc-test-007'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(status, "applied");
    }

    // ── 최초 생성: ORGANIZER.md 부재 시 백업 없이 새로 만든다 ────────────────

    #[test]
    fn apply_creates_organizer_when_absent_without_backup() {
        let tmp = tempfile::tempdir().unwrap();
        let organizer = tmp.path().join("ORGANIZER.md");
        assert!(!organizer.exists(), "사전 조건: 파일이 없어야 함");

        let after_content = "tag:스크린샷 -> 사진/스크린샷\n";
        let conn = test_conn();
        conn.execute(
            "INSERT INTO rule_changes \
             (rule_change_id, created_at, status, natural_language, \
              before_content, after_content, diff_text, summary_cards) \
             VALUES ('rc-first', 1000, 'pending', 'test', '', ?1, '', '[]')",
            rusqlite::params![after_content],
        )
        .unwrap();

        let result = apply_rule_change_inner("rc-first", &conn, &organizer).unwrap();

        // 파일이 새로 생성되고 내용이 기록됨.
        let written = fs::read_to_string(&organizer).unwrap();
        assert_eq!(written, after_content, "최초 생성: 새 내용이 기록되어야 함");

        // 백업은 없어야 함(백업 대상 부재).
        assert!(result["backup_path"].is_null(), "최초 생성 시 backup_path는 null");
        assert_eq!(result["created"], serde_json::json!(true), "created=true 여야 함");

        // 백업 파일이 디렉터리에 생기지 않았는지 확인.
        let bak_count = fs::read_dir(tmp.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains(".bak."))
            .count();
        assert_eq!(bak_count, 0, "최초 생성 시 .bak 파일이 생기면 안 됨");

        let status: String = conn
            .query_row(
                "SELECT status FROM rule_changes WHERE rule_change_id = 'rc-first'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(status, "applied");
    }

    // ── R3: apply_rule_change 는 파일을 0개 이동한다 (sentinel 패턴) ──────────

    /// apply_rule_change_inner 실행 후 files 테이블이 불변임을 증명한다.
    /// sentinel 파일(새 규칙에 매칭됨)을 DB에 삽입하고, 적용 후 경로가 동일함을 확인.
    /// 추가로 journal 테이블에 새 항목이 없음(이동 0건)을 단언한다.
    #[test]
    fn r3_apply_rule_change_moves_zero_files_sentinel() {
        let tmp = tempfile::tempdir().unwrap();
        let organizer = tmp.path().join("ORGANIZER.md");
        let before_content = "tag:세금 -> 문서/세금\n";
        let after_content  = "tag:세금 -> 문서/세금\ntag:스크린샷 -> 사진/스크린샷\n";
        fs::write(&organizer, before_content).unwrap();

        let conn = test_conn();

        // sentinel: 새 규칙(tag:스크린샷)에 매칭될 파일을 DB에 삽입.
        conn.execute(
            "INSERT INTO files (content_hash, current_path, size, mtime, topic_tags) \
             VALUES ('sentinel-hash', '/home/user/capture.png', 512, 0, '[\"스크린샷\"]')",
            [],
        )
        .unwrap();

        let journal_before: i64 = conn
            .query_row("SELECT COUNT(*) FROM journal", [], |r| r.get(0))
            .unwrap();

        conn.execute(
            "INSERT INTO rule_changes \
             (rule_change_id, created_at, status, natural_language, \
              before_content, after_content, diff_text, summary_cards) \
             VALUES ('rc-r3-sentinel', 1000, 'pending', 'test', ?1, ?2, '', '[]')",
            rusqlite::params![before_content, after_content],
        )
        .unwrap();

        apply_rule_change_inner("rc-r3-sentinel", &conn, &organizer).unwrap();

        // 파일 경로 불변 — sentinel이 이동되지 않았음.
        let path: String = conn
            .query_row(
                "SELECT current_path FROM files WHERE content_hash = 'sentinel-hash'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            path, "/home/user/capture.png",
            "R3: apply_rule_change 는 files.current_path 를 변경하면 안 됨"
        );

        // journal 항목 추가 없음 (이동 0건).
        let journal_after: i64 = conn
            .query_row("SELECT COUNT(*) FROM journal", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            journal_before, journal_after,
            "R3: apply_rule_change 는 journal에 이동 항목을 추가하면 안 됨 \
             (before={journal_before}, after={journal_after})"
        );
    }

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
