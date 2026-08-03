//! Anthropic API 호출 공유 계층 — 재시도(지수 백오프) + 구조적 에러 + 사용자향 메시지.
//!
//! agent(챗 루프)와 summarizer가 같은 엔드포인트에 blocking POST 하던 것을 여기로 묶는다.
//! 핵심: 529/429/5xx/네트워크 등 일시적 오류는 제한 재시도로 자동 복구하고, 실패 시 raw JSON
//! 대신 사용자향 메시지를 낸다. request_id 는 로그에만 보존하고 사용자에겐 노출하지 않는다.

use serde_json::Value;
use std::time::Duration;

const ANTHROPIC_URL: &str = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_VERSION: &str = "2023-06-01";
const MAX_ATTEMPTS: u32 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiErrorKind {
    /// 529 overloaded — 재시도 대상.
    Overloaded,
    /// 429 rate limit — 재시도 대상.
    RateLimited,
    /// 5xx 서버 오류 — 재시도 대상.
    ServerError,
    /// 전송/타임아웃 등 네트워크 — 재시도 대상.
    Network,
    /// 401/403 인증 — 재시도 무의미.
    Auth,
    /// 그 외 4xx — 재시도 무의미.
    Client,
    /// 응답 파싱 실패.
    Parse,
}

impl ApiErrorKind {
    pub fn is_retryable(self) -> bool {
        matches!(
            self,
            ApiErrorKind::Overloaded
                | ApiErrorKind::RateLimited
                | ApiErrorKind::ServerError
                | ApiErrorKind::Network
        )
    }
}

#[derive(Debug)]
pub struct ApiError {
    pub kind: ApiErrorKind,
    /// Anthropic `request-id` 헤더. 디버깅용 — 로그에만 남기고 사용자엔 노출하지 않는다.
    pub request_id: Option<String>,
    /// 원본 상세(응답 본문/에러 문자열). 로그용, 사용자 비노출.
    pub detail: String,
}

impl ApiError {
    /// 사용자에게 보여줄 문구. raw JSON·request_id·detail 을 절대 포함하지 않는다.
    pub fn user_message(&self) -> String {
        match self.kind {
            ApiErrorKind::Overloaded | ApiErrorKind::RateLimited => {
                "AI 서버가 혼잡합니다. 잠시 후 다시 시도해주세요.".to_string()
            }
            ApiErrorKind::ServerError => {
                "AI 서버에 일시적 오류가 발생했습니다. 잠시 후 다시 시도해주세요.".to_string()
            }
            ApiErrorKind::Network => "네트워크 연결을 확인해주세요.".to_string(),
            ApiErrorKind::Auth => {
                "API 키가 유효하지 않습니다. 설정(⚙)에서 확인해주세요.".to_string()
            }
            ApiErrorKind::Client => "요청을 처리할 수 없습니다.".to_string(),
            ApiErrorKind::Parse => "AI 응답을 해석하지 못했습니다.".to_string(),
        }
    }
}

/// 지수 백오프 지연: 1s, 2s, 4s … (테스트에서 sleep 주입으로 무시 가능).
fn backoff_delay(attempt_no: u32) -> Duration {
    Duration::from_millis(1000u64.saturating_mul(1u64 << attempt_no.min(6)))
}

/// 재시도 오케스트레이터. HTTP 를 몰라 순수하게 테스트 가능하다.
/// `sleep` 은 백오프 대기(테스트에선 no-op), `attempt` 는 1회 시도.
pub fn retry_with_backoff<T, F, S>(
    max_attempts: u32,
    mut sleep: S,
    mut attempt: F,
) -> Result<T, ApiError>
where
    F: FnMut(u32) -> Result<T, ApiError>,
    S: FnMut(Duration),
{
    let mut attempt_no = 0;
    loop {
        match attempt(attempt_no) {
            Ok(v) => return Ok(v),
            Err(e) => {
                let is_last = attempt_no + 1 >= max_attempts;
                if e.kind.is_retryable() && !is_last {
                    eprintln!(
                        "[llm] attempt {attempt_no} failed: {:?} (request_id={:?}) — retrying",
                        e.kind, e.request_id
                    );
                    sleep(backoff_delay(attempt_no));
                    attempt_no += 1;
                    continue;
                }
                return Err(e);
            }
        }
    }
}

/// 단일 HTTP 시도. 상태코드 → ApiErrorKind 분류, request_id 캡처.
pub fn anthropic_post(api_key: &str, body: &Value) -> Result<Value, ApiError> {
    let client = reqwest::blocking::Client::new();
    let resp = client
        .post(ANTHROPIC_URL)
        .header("x-api-key", api_key)
        .header("anthropic-version", ANTHROPIC_VERSION)
        .header("content-type", "application/json")
        .json(body)
        .send()
        .map_err(|e| ApiError {
            kind: ApiErrorKind::Network,
            request_id: None,
            detail: e.to_string(),
        })?;

    let request_id = resp
        .headers()
        .get("request-id")
        .or_else(|| resp.headers().get("x-request-id"))
        .and_then(|v| v.to_str().ok())
        .map(String::from);

    let status = resp.status().as_u16();
    if status == 200 {
        return resp.json::<Value>().map_err(|e| ApiError {
            kind: ApiErrorKind::Parse,
            request_id,
            detail: e.to_string(),
        });
    }

    let kind = match status {
        429 => ApiErrorKind::RateLimited,
        529 => ApiErrorKind::Overloaded,
        401 | 403 => ApiErrorKind::Auth,
        500..=599 => ApiErrorKind::ServerError,
        _ => ApiErrorKind::Client,
    };
    Err(ApiError {
        kind,
        request_id,
        detail: resp.text().unwrap_or_default(),
    })
}

/// 재시도까지 포함한 호출. 최종 실패는 로그에 kind·request_id·detail 을 남긴다(사용자 비노출).
pub fn anthropic_post_with_retry(api_key: &str, body: &Value) -> Result<Value, ApiError> {
    let result = retry_with_backoff(
        MAX_ATTEMPTS,
        |d| std::thread::sleep(d),
        |_i| anthropic_post(api_key, body),
    );
    if let Err(ref e) = result {
        eprintln!(
            "[llm] request failed after retries: {:?} (request_id={:?}) detail={}",
            e.kind, e.request_id, e.detail
        );
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn err(kind: ApiErrorKind) -> ApiError {
        ApiError { kind, request_id: None, detail: String::new() }
    }

    #[test]
    fn retries_transient_then_succeeds() {
        let calls = Cell::new(0u32);
        let r = retry_with_backoff(3, |_d| {}, |_i| {
            let n = calls.get();
            calls.set(n + 1);
            if n < 2 { Err(err(ApiErrorKind::Overloaded)) } else { Ok(42) }
        });
        assert_eq!(r.unwrap(), 42);
        assert_eq!(calls.get(), 3, "2회 실패 후 3번째 성공");
    }

    #[test]
    fn exhausts_and_returns_last_error() {
        let calls = Cell::new(0u32);
        let r: Result<i32, _> = retry_with_backoff(3, |_d| {}, |_i| {
            calls.set(calls.get() + 1);
            Err(err(ApiErrorKind::Overloaded))
        });
        assert_eq!(r.unwrap_err().kind, ApiErrorKind::Overloaded);
        assert_eq!(calls.get(), 3, "최대 시도 횟수만큼만 호출");
    }

    #[test]
    fn does_not_retry_non_retryable() {
        let calls = Cell::new(0u32);
        let r: Result<i32, _> = retry_with_backoff(3, |_d| {}, |_i| {
            calls.set(calls.get() + 1);
            Err(err(ApiErrorKind::Auth))
        });
        assert_eq!(r.unwrap_err().kind, ApiErrorKind::Auth);
        assert_eq!(calls.get(), 1, "비재시도 에러는 즉시 반환");
    }

    #[test]
    fn is_retryable_classification() {
        for k in [ApiErrorKind::Overloaded, ApiErrorKind::RateLimited, ApiErrorKind::ServerError, ApiErrorKind::Network] {
            assert!(k.is_retryable(), "{k:?} 는 재시도 대상");
        }
        for k in [ApiErrorKind::Auth, ApiErrorKind::Client, ApiErrorKind::Parse] {
            assert!(!k.is_retryable(), "{k:?} 는 비재시도");
        }
    }

    #[test]
    fn user_message_hides_raw_detail_and_request_id() {
        let e = ApiError {
            kind: ApiErrorKind::Overloaded,
            request_id: Some("req_abc123".into()),
            detail: "{\"type\":\"error\",\"error\":{\"type\":\"overloaded_error\"}}".into(),
        };
        let msg = e.user_message();
        assert!(msg.contains("혼잡"));
        assert!(!msg.contains("error"), "raw JSON 노출 금지");
        assert!(!msg.contains("req_abc123"), "request_id 사용자 비노출");
    }
}
