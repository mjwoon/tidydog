//! SA2: SidecarReader — ContentReader implementation backed by sidecar/reader.py.
//! N5: read-only; passes paths to Python for content extraction, never modifies files.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use tidydog_core::{ContentBudget, ContentChunk, ContentReader, ReaderError};

/// Drives `sidecar/reader.py` over a one-shot stdin/stdout JSON protocol (C-A).
pub struct SidecarReader {
    pub python_bin: String,
    pub script_path: PathBuf,
}

impl SidecarReader {
    pub fn new(python_bin: impl Into<String>, script_path: PathBuf) -> Self {
        SidecarReader {
            python_bin: python_bin.into(),
            script_path,
        }
    }
}

impl ContentReader for SidecarReader {
    fn supports_ext(&self, ext: &str) -> bool {
        matches!(ext, "txt" | "md" | "rst" | "pdf")
    }

    fn read_content(&self, path: &Path, budget: ContentBudget) -> Result<ContentChunk, ReaderError> {
        // Build request JSON.
        let request = serde_json::json!({
            "path": path.to_string_lossy(),
            "max_chars": budget.max_chars,
        });
        let request_line = format!("{}\n", request);

        // Spawn: python_bin script_path, with stdin/stdout piped, stderr discarded.
        let mut child = Command::new(&self.python_bin)
            .arg(&self.script_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| {
                ReaderError::SidecarUnavailable(format!(
                    "failed to spawn {} {}: {}",
                    self.python_bin,
                    self.script_path.display(),
                    e
                ))
            })?;

        // Write request and drop stdin so Python sees EOF.
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(request_line.as_bytes()).map_err(|e| {
                ReaderError::SidecarUnavailable(format!("failed to write to sidecar stdin: {e}"))
            })?;
            // stdin dropped here, signalling EOF to Python
        }

        // Read the response: wait for process and collect stdout.
        let output = child.wait_with_output().map_err(|e| {
            ReaderError::SidecarUnavailable(format!("sidecar process error: {e}"))
        })?;

        // Parse the first line of stdout as JSON.
        let stdout_str = String::from_utf8_lossy(&output.stdout);
        let first_line = stdout_str.lines().next().unwrap_or("").trim();

        if first_line.is_empty() {
            return Err(ReaderError::SidecarUnavailable(
                "sidecar produced no output".to_string(),
            ));
        }

        let response: serde_json::Value =
            serde_json::from_str(first_line).map_err(|e| {
                ReaderError::ParseFailed(format!("invalid JSON from sidecar: {e}"))
            })?;

        // Check for error field.
        if let Some(err_val) = response.get("error") {
            if !err_val.is_null() {
                let msg = err_val
                    .as_str()
                    .unwrap_or("unknown sidecar error")
                    .to_string();
                return Err(ReaderError::ParseFailed(msg));
            }
        }

        let text = response
            .get("text")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let truncated = response
            .get("truncated")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        // Map pages → Option<u8>, clamping to u8::MAX if > 255.
        let pages_read = response.get("pages").and_then(|v| {
            if v.is_null() {
                None
            } else {
                v.as_u64().map(|n| n.min(u8::MAX as u64) as u8)
            }
        });

        Ok(ContentChunk {
            text,
            pages_read,
            truncated,
        })
    }
}

/// reader 단위 테스트 — N3 예산 강제 + 사이드카 에러 표면 증명.
/// 사이드카(sidecar/reader.py)와 python3가 없으면 자동 스킵.
#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tidydog_core::ContentBudget;

    fn sidecar_path() -> Option<PathBuf> {
        let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../sidecar/reader.py");
        if p.exists() { Some(p) } else { None }
    }

    // ── N3: 분량 상한 ─────────────────────────────────────────────────────────

    /// 파일이 max_chars보다 크면 잘려서 오고 truncated=true.
    #[test]
    fn n3_budget_enforced_large_file() {
        let Some(script) = sidecar_path() else { return; };
        let tmp = std::env::temp_dir().join("tidydog_n3_large.txt");
        fs::write(&tmp, "A".repeat(8000)).unwrap();

        let reader = SidecarReader::new("python3", script);
        let chunk = reader
            .read_content(&tmp, ContentBudget { max_chars: 100, max_pages: None })
            .unwrap();

        assert!(
            chunk.text.len() <= 100,
            "text len {} exceeds budget 100",
            chunk.text.len()
        );
        assert!(chunk.truncated, "truncated must be true when file > budget");
        fs::remove_file(&tmp).ok();
    }

    /// 파일이 max_chars 이내면 전체가 오고 truncated=false.
    #[test]
    fn n3_small_file_not_truncated() {
        let Some(script) = sidecar_path() else { return; };
        let tmp = std::env::temp_dir().join("tidydog_n3_small.txt");
        fs::write(&tmp, b"hello").unwrap();

        let reader = SidecarReader::new("python3", script);
        let chunk = reader
            .read_content(&tmp, ContentBudget { max_chars: 4096, max_pages: None })
            .unwrap();

        assert!(!chunk.truncated, "small file should not be truncated");
        assert_eq!(chunk.text.trim(), "hello");
        fs::remove_file(&tmp).ok();
    }

    // ── N4/사이드카 에러 표면 ────────────────────────────────────────────────

    /// 깨진 PDF(유효하지 않은 바이너리)는 Err를 반환해야 한다.
    /// 호출자(index_file_content)는 이 Err를 ? 로 전파해
    /// UPDATE files SET read_level='content' 행에 도달하지 않는다.
    #[test]
    fn broken_pdf_returns_err_not_content_chunk() {
        let Some(script) = sidecar_path() else { return; };
        let tmp = std::env::temp_dir().join("tidydog_broken.pdf");
        fs::write(&tmp, b"this is not a valid PDF at all").unwrap();

        let reader = SidecarReader::new("python3", script);
        let result = reader.read_content(&tmp, ContentBudget::default());
        assert!(result.is_err(), "broken PDF must return Err; got {result:?}");
        fs::remove_file(&tmp).ok();
    }

    /// 존재하지 않는 파일도 Err 반환 → read_level 업그레이드 안 됨.
    #[test]
    fn missing_file_returns_err() {
        let Some(script) = sidecar_path() else { return; };
        let absent = PathBuf::from("/tmp/tidydog_absent_xyz_123456.txt");
        let reader = SidecarReader::new("python3", script);
        let result = reader.read_content(&absent, ContentBudget::default());
        assert!(result.is_err(), "missing file must return Err");
    }
}
