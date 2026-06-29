//! 스코프 가드 통합 테스트 — 실제 임시 파일시스템 사용.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use tidydog_core::{Denial, Guard, ScopeGuard};

static COUNTER: AtomicU32 = AtomicU32::new(0);

/// 고유한 임시 작업 디렉터리 생성.
fn tmp_workspace() -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let base = std::env::temp_dir().join(format!("tidydog_scope_{}_{}", std::process::id(), n));
    fs::create_dir_all(&base).unwrap();
    base
}

#[test]
fn in_scope_file_allowed() {
    let ws = tmp_workspace();
    let scope = ws.join("scope");
    fs::create_dir_all(&scope).unwrap();
    let f = scope.join("a.txt");
    fs::write(&f, b"hi").unwrap();

    let guard = ScopeGuard::new(vec![scope.clone()], vec![]);
    assert!(guard.check(&f).is_ok());
}

#[test]
fn traversal_escape_denied() {
    let ws = tmp_workspace();
    let scope = ws.join("scope");
    fs::create_dir_all(&scope).unwrap();
    // 스코프 밖 비밀 파일
    let secret = ws.join("secret.txt");
    fs::write(&secret, b"x").unwrap();

    let guard = ScopeGuard::new(vec![scope.clone()], vec![]);
    // scope/../secret.txt 로 빠져나가려는 시도
    let attempt = scope.join("..").join("secret.txt");
    assert_eq!(guard.check(&attempt), Err(Denial::OutsideScope));
}

#[test]
fn symlink_escape_denied() {
    let ws = tmp_workspace();
    let scope = ws.join("scope");
    let outside = ws.join("outside");
    fs::create_dir_all(&scope).unwrap();
    fs::create_dir_all(&outside).unwrap();
    let target = outside.join("target.txt");
    fs::write(&target, b"x").unwrap();
    let link = scope.join("link.txt");
    std::os::unix::fs::symlink(&target, &link).unwrap();

    let guard = ScopeGuard::new(vec![scope.clone()], vec![]);
    // 심볼릭 링크는 canonicalize로 스코프 밖 실경로로 해석 → 거부
    assert_eq!(guard.check(&link), Err(Denial::OutsideScope));
}

#[test]
fn dotfile_denied() {
    let ws = tmp_workspace();
    let scope = ws.join("scope");
    fs::create_dir_all(&scope).unwrap();
    let dotf = scope.join(".secret");
    fs::write(&dotf, b"x").unwrap();

    let guard = ScopeGuard::new(vec![scope.clone()], vec![]);
    assert_eq!(guard.check(&dotf), Err(Denial::Hidden));
}

#[test]
fn node_modules_denied() {
    let ws = tmp_workspace();
    let scope = ws.join("scope");
    let nm = scope.join("node_modules");
    fs::create_dir_all(&nm).unwrap();
    let f = nm.join("lib.js");
    fs::write(&f, b"x").unwrap();

    let guard = ScopeGuard::new(vec![scope.clone()], vec![]);
    assert_eq!(guard.check(&f), Err(Denial::Hidden));
}

#[test]
fn blacklist_denied() {
    let ws = tmp_workspace();
    let scope = ws.join("scope");
    let sys = scope.join("system");
    fs::create_dir_all(&sys).unwrap();
    let f = sys.join("important.conf");
    fs::write(&f, b"x").unwrap();

    // system 디렉터리를 블랙리스트로 주입
    let guard = ScopeGuard::new(vec![scope.clone()], vec![sys.clone()]);
    assert_eq!(guard.check(&f), Err(Denial::Blacklisted));
}
