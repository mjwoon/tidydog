//! 스코프 가드 — 안전 명세 §2.
//! 모든 경로는 사용을 허락받기 전에 이 검사를 통과해야 한다.
//! 순서: 해석(canonicalize, 심볼릭 링크 따라감) → 화이트리스트(허용 루트) → 블랙리스트 → 숨김.
//!
//! 핵심: 심볼릭 링크나 `..`로 스코프를 빠져나가려는 시도는 canonicalize 후 화이트리스트
//! 검사에서 자동으로 걸린다. 즉 "능력을 정책이 아니라 경로 해석 구조로 제거".

use crate::model::Denial;
use std::path::{Component, Path, PathBuf};

/// 경로 허용 여부를 판정하는 인터페이스. Engine은 이 트레이트에만 의존한다.
pub trait Guard {
    /// 통과하면 해석된 절대경로(canonical)를, 막히면 사유를 반환.
    fn check(&self, path: &Path) -> Result<PathBuf, Denial>;
}

/// 기본 스코프 가드.
/// - `roots`: 허용 루트(화이트리스트). 정리 대상 폴더들. 생성 시 canonical 화.
/// - `blacklist`: 금지 접두(시스템 디렉터리·앱 데이터 등). OS별 목록은 어댑터가 주입.
/// - `allow_hidden`: false면 dotfile/node_modules 등 숨김 성분을 막는다.
#[derive(Debug, Clone)]
pub struct ScopeGuard {
    roots: Vec<PathBuf>,
    blacklist: Vec<PathBuf>,
    allow_hidden: bool,
}

impl ScopeGuard {
    pub fn new(roots: Vec<PathBuf>, blacklist: Vec<PathBuf>) -> Self {
        let roots = roots.iter().map(|r| canonical_or_lexical(r)).collect();
        let blacklist = blacklist.iter().map(|b| canonical_or_lexical(b)).collect();
        ScopeGuard {
            roots,
            blacklist,
            allow_hidden: false,
        }
    }

    pub fn allow_hidden(mut self, yes: bool) -> Self {
        self.allow_hidden = yes;
        self
    }

    /// 존재하지 않는 대상(예: 아직 만들지 않은 목적지)도 해석한다.
    /// 존재하면 fs::canonicalize, 아니면 가장 가까운 존재 조상을 canonical화한 뒤
    /// 나머지 성분을 어휘적으로(.. 팝) 붙인다.
    fn resolve(&self, path: &Path) -> Result<PathBuf, Denial> {
        if path.exists() {
            return std::fs::canonicalize(path).map_err(|_| Denial::NotFound);
        }
        // 가장 가까운 존재 조상 찾기(모두 path 데이터를 가리키는 &Path).
        let mut existing: &Path = path;
        while !existing.exists() {
            existing = match existing.parent() {
                Some(p) => p,
                None => return Err(Denial::NotFound),
            };
        }
        let base = std::fs::canonicalize(existing).map_err(|_| Denial::NotFound)?;
        let tail = path.strip_prefix(existing).unwrap_or(path);
        let mut out = base;
        for comp in tail.components() {
            match comp {
                Component::Normal(c) => out.push(c),
                Component::ParentDir => {
                    out.pop();
                }
                _ => {}
            }
        }
        Ok(out)
    }

    fn hidden_below_root(&self, resolved: &Path) -> bool {
        // resolved 가 속한 root 를 찾아 그 아래 성분만 검사.
        let root = self.roots.iter().find(|r| resolved.starts_with(r));
        let rel = match root {
            Some(r) => resolved.strip_prefix(r).unwrap_or(resolved),
            None => resolved,
        };
        rel.components().any(|c| {
            if let Component::Normal(os) = c {
                let s = os.to_string_lossy();
                s.starts_with('.') || s == "node_modules"
            } else {
                false
            }
        })
    }
}

impl Guard for ScopeGuard {
    fn check(&self, path: &Path) -> Result<PathBuf, Denial> {
        let resolved = self.resolve(path)?;

        // 화이트리스트: 허용 루트 중 하나의 하위여야 한다.
        if !self.roots.iter().any(|r| resolved.starts_with(r)) {
            return Err(Denial::OutsideScope);
        }
        // 블랙리스트: 금지 접두의 하위면 거부.
        if self.blacklist.iter().any(|b| resolved.starts_with(b)) {
            return Err(Denial::Blacklisted);
        }
        // 숨김 성분.
        if !self.allow_hidden && self.hidden_below_root(&resolved) {
            return Err(Denial::Hidden);
        }
        Ok(resolved)
    }
}

/// 존재하면 canonical, 아니면 어휘적 정규화(절대경로 가정).
fn canonical_or_lexical(p: &Path) -> PathBuf {
    if p.exists() {
        std::fs::canonicalize(p).unwrap_or_else(|_| lexical_normalize(p))
    } else {
        lexical_normalize(p)
    }
}

fn lexical_normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in p.components() {
        match comp {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}
