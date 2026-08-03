//! SA3: CloudSummarizer — Summarizer implementation backed by the Claude API.
//! N1: Consent gate — checks `settings` table before any API call.
//! N2: API key stored in OS keychain via `keyring`; NEVER logged or stored in struct.
//! N3: Transmission budget — passes ContentChunk.text as-is (already budgeted ≤4096 chars).

use rusqlite::params;
use tidydog_core::{ContentChunk, SummaryError, SummaryHints, SummaryResult, Summarizer};

/// Claude model used for file summarization.
/// Upgrade to `claude-sonnet-4-6` here if summarizer needs higher quality.
pub const SUMMARIZER_MODEL: &str = "claude-haiku-4-5-20251001";

/// CloudSummarizer calls the Claude API to produce summaries.
/// It holds only a DB connection reference; the API key is looked up from the
/// OS keychain on every call (N2).
pub struct CloudSummarizer<'a> {
    conn: &'a rusqlite::Connection,
}

impl<'a> CloudSummarizer<'a> {
    pub fn new(conn: &'a rusqlite::Connection) -> Self {
        CloudSummarizer { conn }
    }

    #[cfg(test)]
    fn in_memory_for_test(conn: &'a rusqlite::Connection) -> Self {
        CloudSummarizer { conn }
    }

    /// Look up cached summary by content_hash. Returns None if not found.
    fn lookup_cache(&self, content_hash: &str) -> Option<SummaryResult> {
        let result = self.conn.query_row(
            "SELECT summary, topic_tags, language FROM summaries WHERE content_hash = ?1",
            params![content_hash],
            |row| {
                let summary: String = row.get(0)?;
                let tags_json: String = row.get(1)?;
                let language: String = row.get(2)?;
                Ok((summary, tags_json, language))
            },
        );

        match result {
            Ok((summary, tags_json, language)) => {
                let topic_tags: Vec<String> =
                    serde_json::from_str(&tags_json).unwrap_or_default();
                Some(SummaryResult {
                    summary,
                    topic_tags,
                    language,
                })
            }
            Err(_) => None,
        }
    }

    /// Store a summary result in the cache.
    fn store_cache(&self, content_hash: &str, result: &SummaryResult) {
        let tags_json = serde_json::to_string(&result.topic_tags).unwrap_or_else(|_| "[]".to_string());
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0) as i64;

        let _ = self.conn.execute(
            "INSERT OR REPLACE INTO summaries (content_hash, summary, topic_tags, language, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![content_hash, result.summary, tags_json, result.language, now],
        );
    }

    /// Call the Claude API and return a SummaryResult.
    fn call_api(
        &self,
        chunk: &ContentChunk,
        hints: &SummaryHints,
        api_key: &str,
    ) -> Result<SummaryResult, SummaryError> {
        let filename = match &hints.ext {
            Some(ext) => format!("{}.{}", hints.filename, ext),
            None => hints.filename.clone(),
        };

        let user_content = format!(
            "Analyze this file and respond with JSON only (no markdown). File: {}\n\nContent:\n{}\n\nRespond ONLY with this exact JSON (no other text):\n{{\"summary\": \"1-2 sentence summary\", \"topic_tags\": [\"tag1\", \"tag2\"], \"language\": \"ko\"}}",
            filename,
            chunk.text
        );

        let body = serde_json::json!({
            "model": SUMMARIZER_MODEL,
            "max_tokens": 256,
            "messages": [
                {
                    "role": "user",
                    "content": user_content
                }
            ]
        });

        // 공유 llm_http 로 호출(재시도 + 구조적 에러). ApiError → SummaryError 매핑.
        let resp_json: serde_json::Value =
            crate::llm_http::anthropic_post_with_retry(api_key, &body).map_err(|e| {
                use crate::llm_http::ApiErrorKind;
                match e.kind {
                    ApiErrorKind::RateLimited | ApiErrorKind::Overloaded => SummaryError::RateLimited,
                    ApiErrorKind::Parse => SummaryError::ParseError(e.detail),
                    _ => SummaryError::ApiError(e.user_message()),
                }
            })?;

        let text = resp_json
            .get("content")
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("text"))
            .and_then(|t| t.as_str())
            .ok_or_else(|| {
                SummaryError::ParseError("missing content[0].text in API response".to_string())
            })?;

        let parsed: serde_json::Value = serde_json::from_str(text)
            .map_err(|e| SummaryError::ParseError(format!("invalid JSON from model: {e}")))?;

        let summary = parsed
            .get("summary")
            .and_then(|v| v.as_str())
            .ok_or_else(|| SummaryError::ParseError("missing 'summary' field".to_string()))?
            .to_string();

        let topic_tags: Vec<String> = parsed
            .get("topic_tags")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();

        let language = parsed
            .get("language")
            .and_then(|v| v.as_str())
            .unwrap_or("en")
            .to_string();

        Ok(SummaryResult {
            summary,
            topic_tags,
            language,
        })
    }
}

/// summarizer 단위 테스트 — N1/캐시/is_consented 증명.
#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    use tidydog_core::{ContentChunk, Summarizer, SummaryError, SummaryHints};

    fn setup_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
             CREATE TABLE summaries (
                 content_hash TEXT PRIMARY KEY,
                 summary      TEXT NOT NULL,
                 topic_tags   TEXT NOT NULL,
                 language     TEXT NOT NULL,
                 created_at   INTEGER NOT NULL
             );",
        )
        .unwrap();
        conn
    }

    fn chunk() -> ContentChunk {
        ContentChunk { text: "hello world".into(), pages_read: None, truncated: false }
    }

    fn hints(hash: Option<&str>) -> SummaryHints {
        SummaryHints {
            filename: "test".into(),
            ext: Some("txt".into()),
            content_hash: hash.map(|s| s.to_string()),
        }
    }

    // ── N1: 동의 게이트 위치 ────────────────────────────────────────────────

    /// settings 행이 없으면 summarize()가 파일 읽기·네트워크 전에 NotConsented 반환.
    /// index_file_content는 이 호출 전에 is_consented()를 체크하므로(lib.rs 수정 후)
    /// 사이드카 프로세스도 실행되지 않는다.
    #[test]
    fn n1_no_consent_row_returns_not_consented() {
        let conn = setup_db();
        let s = CloudSummarizer::in_memory_for_test(&conn);
        let err = s.summarize(&chunk(), hints(None)).unwrap_err();
        assert!(
            matches!(err, SummaryError::NotConsented),
            "expected NotConsented, got {err:?}"
        );
    }

    /// value = 'false' 인 경우에도 NotConsented.
    #[test]
    fn n1_consent_false_returns_not_consented() {
        let conn = setup_db();
        conn.execute(
            "INSERT INTO settings (key,value) VALUES ('ai_content_consent_granted','false')",
            [],
        )
        .unwrap();
        let s = CloudSummarizer::in_memory_for_test(&conn);
        let err = s.summarize(&chunk(), hints(None)).unwrap_err();
        assert!(matches!(err, SummaryError::NotConsented));
    }

    /// value = 'true'이면 is_consented() == true.
    #[test]
    fn is_consented_true_when_setting_true() {
        let conn = setup_db();
        conn.execute(
            "INSERT INTO settings (key,value) VALUES ('ai_content_consent_granted','true')",
            [],
        )
        .unwrap();
        assert!(CloudSummarizer::in_memory_for_test(&conn).is_consented());
    }

    // ── 캐시: 같은 파일 재요약 시 API 호출 생략 ─────────────────────────────

    /// content_hash가 summaries 테이블에 이미 있으면 캐시 결과를 반환하고
    /// keychain/API를 호출하지 않는다.
    /// (동의 true + 캐시 선점 → API 미도달 → 성공 반환)
    #[test]
    fn cache_hit_returns_stored_result_no_api_call() {
        let conn = setup_db();
        conn.execute(
            "INSERT INTO settings (key,value) VALUES ('ai_content_consent_granted','true')",
            [],
        )
        .unwrap();
        // 캐시 선점
        conn.execute(
            "INSERT INTO summaries (content_hash,summary,topic_tags,language,created_at)
             VALUES ('deadbeef','Cached summary','[\"cached_tag\"]','ko',1000)",
            [],
        )
        .unwrap();

        let s = CloudSummarizer::in_memory_for_test(&conn);
        // 'deadbeef' 해시 → 캐시 히트 → keychain/API 없이 성공
        let result = s.summarize(&chunk(), hints(Some("deadbeef"))).unwrap();
        assert_eq!(result.summary, "Cached summary");
        assert_eq!(result.topic_tags, vec!["cached_tag".to_string()]);
        assert_eq!(result.language, "ko");
    }

    /// 다른 해시를 주면 캐시 미스 → API 오류(테스트 환경엔 키 없음)가 나는데,
    /// 그 오류는 NotConsented가 아니어야 한다(동의는 됐음).
    #[test]
    fn cache_miss_with_consent_fails_at_keychain_not_at_consent_gate() {
        let conn = setup_db();
        conn.execute(
            "INSERT INTO settings (key,value) VALUES ('ai_content_consent_granted','true')",
            [],
        )
        .unwrap();
        let s = CloudSummarizer::in_memory_for_test(&conn);
        let err = s.summarize(&chunk(), hints(Some("no-such-hash"))).unwrap_err();
        assert!(
            !matches!(err, SummaryError::NotConsented),
            "consent is granted — failure must be at keychain/API stage, not consent gate"
        );
    }
}

impl<'a> Summarizer for CloudSummarizer<'a> {
    fn is_consented(&self) -> bool {
        let result = self.conn.query_row(
            "SELECT value FROM settings WHERE key = 'ai_content_consent_granted'",
            [],
            |row| row.get::<_, String>(0),
        );
        matches!(result, Ok(v) if v == "true")
    }

    fn summarize(
        &self,
        chunk: &ContentChunk,
        hints: SummaryHints,
    ) -> Result<SummaryResult, SummaryError> {
        // N1: Consent gate — must check before any API call.
        if !self.is_consented() {
            return Err(SummaryError::NotConsented);
        }

        // SQLite cache lookup.
        if let Some(hash) = &hints.content_hash {
            if let Some(cached) = self.lookup_cache(hash) {
                return Ok(cached);
            }
        }

        // N2: env fallback(개발 주입) → OS 키체인(최종 사용자) 순서로 조회.
        let api_key = crate::keyutil::get_api_key()
            .map_err(|e| SummaryError::ApiError(e))?;

        // Call the API.
        let result = self.call_api(chunk, &hints, &api_key)?;

        // Store in cache.
        if let Some(hash) = &hints.content_hash {
            self.store_cache(hash, &result);
        }

        Ok(result)
    }
}
