//! C-B 공유 계약: ContentReader 트레이트 + 관련 타입.
//! 구현(SidecarReader 등)은 src-tauri 어댑터 레이어에서 제공.
//! N5: 파서/리더는 원본 파일을 수정·이동하지 않는다.

use std::path::Path;

/// 콘텐츠 읽기 예산 — N3 전송 범위 최소화.
pub struct ContentBudget {
    /// 최대 문자 수. 이 이상은 잘라낸다.
    pub max_chars: usize,
    /// PDF의 경우 최대 페이지 수.
    pub max_pages: Option<u8>,
}

impl Default for ContentBudget {
    fn default() -> Self {
        ContentBudget {
            max_chars: 4096,
            max_pages: Some(5),
        }
    }
}

/// 읽기 결과 청크.
#[derive(Debug)]
pub struct ContentChunk {
    pub text: String,
    /// PDF에서 실제 읽은 페이지 수.
    pub pages_read: Option<u8>,
    /// 예산으로 잘렸으면 true.
    pub truncated: bool,
}

/// ContentReader 오류.
#[derive(Debug)]
pub enum ReaderError {
    /// 지원하지 않는 파일 형식.
    Unsupported(String),
    /// 파서 실패(깨진 PDF, 인코딩 오류 등).
    ParseFailed(String),
    /// 사이드카 프로세스를 시작할 수 없음.
    SidecarUnavailable(String),
}

impl std::fmt::Display for ReaderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReaderError::Unsupported(s) => write!(f, "unsupported format: {s}"),
            ReaderError::ParseFailed(s) => write!(f, "parse failed: {s}"),
            ReaderError::SidecarUnavailable(s) => write!(f, "sidecar unavailable: {s}"),
        }
    }
}

/// 파일에서 텍스트를 읽는 트레이트. 구현체는 N5를 준수해야 한다(읽기 전용).
pub trait ContentReader {
    fn read_content(&self, path: &Path, budget: ContentBudget) -> Result<ContentChunk, ReaderError>;
    /// 이 리더가 지원하는 확장자인지(점 없이, 소문자: "txt", "pdf" 등).
    fn supports_ext(&self, ext: &str) -> bool;
}
