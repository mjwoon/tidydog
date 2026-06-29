//! 부수효과 트레이트. 코어 로직은 실제 FS/DB가 아니라 이 인터페이스에만 의존한다.
//! → Tauri/rusqlite/trash 없이도 게이트 로직을 단위 테스트할 수 있다.

use crate::model::{JournalEntry, Plan, PlanStatus};
use std::io;
use std::path::{Path, PathBuf};

/// 실제 파일 작업. 어댑터가 std::fs / trash 크레이트로 구현.
pub trait FileOps {
    /// 이동/리네임. 같은 볼륨이면 원자적, 다른 볼륨이면 copy→검증→delete(어댑터 책임).
    fn move_file(&mut self, from: &Path, to: &Path) -> io::Result<()>;
    /// 격리(소프트 삭제). staged 경로를 반환. 영구삭제는 존재하지 않는다.
    fn stage_file(&mut self, from: &Path, content_hash: &str) -> io::Result<PathBuf>;
    /// staging에서 원위치로 복원(undo).
    fn restore_file(&mut self, content_hash: &str, to: &Path) -> io::Result<()>;
    /// 경로 존재 여부(충돌 해소용).
    fn exists(&self, p: &Path) -> bool;
}

/// 영속성. 어댑터가 SQLite로 구현.
pub trait Store {
    fn save_plan(&mut self, plan: Plan);
    fn get_plan(&self, plan_id: &str) -> Option<Plan>;
    fn set_plan_status(&mut self, plan_id: &str, status: PlanStatus);

    /// journal append-only. 부여된 entry_id 반환.
    fn append_journal(&mut self, entry: JournalEntry) -> u64;
    fn journal_for_plan(&self, plan_id: &str) -> Vec<JournalEntry>;
    fn mark_undone(&mut self, entry_id: u64, ts: u64);

    /// 최근 실행된 플랜 id를 새 것부터(limit개). undo 범위(C5: 5개) 판정용.
    fn recent_executed_plan_ids(&self, limit: usize) -> Vec<String>;

    /// journal-first(S2): 파일 작업 완료 후 completed_at을 확정한다.
    fn complete_journal(&mut self, entry_id: u64, ts: u64);

    /// Stage op의 실제 staged_path 확정 (stage_file 반환값으로 덮어씀).
    fn update_journal_to(&mut self, entry_id: u64, to: std::path::PathBuf);

    /// 크래시 복원(S3): completed_at IS NULL인 in-flight 항목 목록.
    fn inflight_entries(&self) -> Vec<JournalEntry>;
}
