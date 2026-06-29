/// AppGuard — macOS C2 블랙리스트를 주입한 ScopeGuard 팩토리.
use std::path::PathBuf;
use tidydog_core::ScopeGuard;

/// macOS 시스템·앱 데이터 블랙리스트.
/// 사용자가 실수로 시스템 폴더를 스캔 루트로 선택해도 이 경로 아래 파일은 조작 불가.
pub fn make_guard(roots: Vec<PathBuf>) -> ScopeGuard {
    let home = dirs_home();
    let blacklist = vec![
        PathBuf::from("/System"),
        PathBuf::from("/usr"),
        PathBuf::from("/bin"),
        PathBuf::from("/sbin"),
        PathBuf::from("/private"),
        PathBuf::from("/Library"),
        home.join("Library"),
        home.join(".Trash"),
        home.join(".ssh"),
        home.join(".gnupg"),
    ];
    ScopeGuard::new(roots, blacklist)
}

fn dirs_home() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/tmp"))
}
