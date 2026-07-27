/// FsFileOps — tidydog_core::FileOps 트레이트를 std::fs + 커스텀 staging 디렉터리로 구현.
///
/// Stage는 OS 휴지통 대신 앱 데이터 디렉터리 내 `staging/` 폴더를 사용한다.
/// 이유: macOS의 trash 크레이트는 복원 API(`restore_all`)를 지원하지 않으므로(os_limited는
/// Linux/Windows 전용) undo가 구조적으로 불가능. staging 디렉터리 방식으로 복원을 보장한다.
/// 사용자가 명시적으로 "버리기"를 선택하면 그 시점에 OS 휴지통으로 이동한다(purge).
use blake3::Hasher;
use std::io;
use std::path::{Path, PathBuf};
use tidydog_core::FileOps;

pub struct FsFileOps {
    pub staging_dir: PathBuf,
}

impl FsFileOps {
    pub fn new(staging_dir: PathBuf) -> Self {
        FsFileOps { staging_dir }
    }
}

impl FileOps for FsFileOps {
    fn move_file(&mut self, from: &Path, to: &Path) -> io::Result<()> {
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // 같은 볼륨이면 atomic rename.
        match std::fs::rename(from, to) {
            Ok(()) => Ok(()),
            Err(_) => {
                // cross-device: copy → BLAKE3 검증 → delete (S5).
                std::fs::copy(from, to)?;
                let src_hash = blake3_file(from)?;
                let dst_hash = blake3_file(to)?;
                if src_hash != dst_hash {
                    let _ = std::fs::remove_file(to);
                    return Err(io::Error::new(
                        io::ErrorKind::Other,
                        format!(
                            "cross-volume copy integrity failed: src={src_hash} dst={dst_hash}"
                        ),
                    ));
                }
                std::fs::remove_file(from)
            }
        }
    }

    fn stage_file(&mut self, from: &Path, content_hash: &str) -> io::Result<PathBuf> {
        // staging/ 하위에 content_hash 이름으로 이동. 영구삭제 아님.
        std::fs::create_dir_all(&self.staging_dir)?;
        let staged = self.staging_dir.join(content_hash);
        // 동명 staging 파일이 이미 있으면 덮어쓰기 방지(리네임으로 회피).
        let staged = if staged.exists() {
            self.staging_dir.join(format!("{}_{}", content_hash, now_secs()))
        } else {
            staged
        };
        std::fs::rename(from, &staged).or_else(|_e| -> io::Result<()> {
            // cross-device fallback
            std::fs::copy(from, &staged)?;
            std::fs::remove_file(from)?;
            Ok(())
        })?;
        Ok(staged)
    }

    fn restore_file(&mut self, content_hash: &str, to: &Path) -> io::Result<()> {
        // staging 디렉터리에서 정확한 경로를 찾는다.
        let staged = self.staging_dir.join(content_hash);
        if !staged.exists() {
            // hash_timestamp 형식으로 저장된 경우 glob 검색.
            let prefix = format!("{content_hash}_");
            let candidate = std::fs::read_dir(&self.staging_dir)
                .ok()
                .and_then(|mut d| {
                    d.find_map(|e| {
                        let e = e.ok()?;
                        let name = e.file_name();
                        let s = name.to_string_lossy();
                        if s.starts_with(&prefix) {
                            Some(e.path())
                        } else {
                            None
                        }
                    })
                });
            let staged = candidate.ok_or_else(|| {
                io::Error::new(io::ErrorKind::NotFound, "staged file not found")
            })?;
            return self.move_file_simple(&staged, to);
        }
        self.move_file_simple(&staged, to)
    }

    fn exists(&self, p: &Path) -> bool {
        p.exists()
    }

    fn remove_empty_dir(&mut self, dir: &Path) -> io::Result<()> {
        // std::fs::remove_dir 는 빈 디렉터리만 제거하고, 비어 있지 않으면 Err 를 낸다
        // → 재귀 삭제 없음(사용자 데이터 보호). undo 는 이 Err 를 무시(보존)한다.
        std::fs::remove_dir(dir)
    }
}

impl FsFileOps {
    fn move_file_simple(&self, from: &Path, to: &Path) -> io::Result<()> {
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::rename(from, to).or_else(|_| {
            std::fs::copy(from, to)?;
            std::fs::remove_file(from)?;
            Ok(())
        })
    }
}

/// BLAKE3 파일 해시 (hex string).
fn blake3_file(path: &Path) -> io::Result<String> {
    use std::io::Read;
    let mut f = std::fs::File::open(path)?;
    let mut hasher = Hasher::new();
    let mut buf = [0u8; 65536];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize().to_hex().to_string())
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
