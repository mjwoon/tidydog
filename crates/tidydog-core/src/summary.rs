//! C-C 공유 계약: Summarizer 트레이트 + 관련 타입.
//! 구현(CloudSummarizer 등)은 src-tauri 어댑터 레이어에서 제공.
//! N1: is_consented()가 false이면 호출 전에 차단해야 한다.

use crate::content::ContentChunk;

/// 요약 요청에 같이 주는 힌트.
pub struct SummaryHints {
    pub filename: String,
    pub ext: Option<String>,
    /// 캐시 룩업용 BLAKE3 해시.
    pub content_hash: Option<String>,
}

/// 요약 결과. files 테이블의 summary/topic_tags/language 컬럼에 매핑된다.
#[derive(Debug)]
pub struct SummaryResult {
    pub summary: String,
    pub topic_tags: Vec<String>,
    /// BCP-47 언어 코드 ("ko", "en" 등).
    pub language: String,
}

/// Summarizer 오류.
#[derive(Debug)]
pub enum SummaryError {
    /// N1: 사용자가 아직 동의하지 않음.
    NotConsented,
    /// API 호출 실패.
    ApiError(String),
    /// 응답 파싱 실패.
    ParseError(String),
    /// Rate limit.
    RateLimited,
}

impl std::fmt::Display for SummaryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SummaryError::NotConsented => {
                write!(f, "user has not consented to cloud content processing")
            }
            SummaryError::ApiError(s) => write!(f, "API error: {s}"),
            SummaryError::ParseError(s) => write!(f, "parse error: {s}"),
            SummaryError::RateLimited => write!(f, "rate limited"),
        }
    }
}

/// 콘텐츠 청크를 받아 요약·태그·언어를 반환하는 트레이트.
/// 구현체는 N1·N2·N3를 준수해야 한다.
pub trait Summarizer {
    /// N1 게이트: 이 메서드 진입 전 is_consented() 체크가 반드시 선행돼야 한다.
    fn summarize(
        &self,
        chunk: &ContentChunk,
        hints: SummaryHints,
    ) -> Result<SummaryResult, SummaryError>;

    /// N1: 사용자 동의 여부 조회. false이면 summarize가 NotConsented를 반환해야 한다.
    fn is_consented(&self) -> bool;
}
