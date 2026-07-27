//! # tidydog-core
//!
//! TidyDog 안전 코어. **Tauri·DB·파일시스템에 비의존하는 순수 Rust 라이브러리**(외부 의존성 0).
//! 스코프 가드와 propose→confirm→execute→undo 게이트 로직을 담고,
//! 부수효과(FS/DB)는 트레이트로 분리해 단위 테스트가 된다.
//!
//! 실제 앱은 `Store`/`FileOps`/`Guard`를 어댑터로 채우고 `Engine`을 Tauri command로 감싼다.

pub mod content;
pub mod gate;
pub mod model;
pub mod ops;
pub mod risk;
pub mod scope;
pub mod summary;

pub use content::{ContentBudget, ContentChunk, ContentReader, ReaderError};
pub use gate::{Engine, ExecOutcome, GateError, UNDO_WINDOW};
pub use model::{Action, Conflict, Denial, JournalEntry, Plan, PlanOp, PlanStatus, PreviewLevel};
pub use ops::{FileOps, Store};
pub use risk::{preview_level, risk_score};
pub use scope::{Guard, ScopeGuard};
pub use summary::{SummaryError, SummaryHints, SummaryResult, Summarizer};

/// 테스트 및 어댑터 개발용 인메모리 fake. 프로덕션에서 쓰지 말 것.
pub mod testing {
    use crate::model::{JournalEntry, Plan, PlanStatus};
    use crate::ops::{FileOps, Store};
    use std::collections::{HashMap, HashSet};
    use std::io;
    use std::path::{Path, PathBuf};

    /// 인메모리 Store.
    #[derive(Default)]
    pub struct FakeStore {
        plans: HashMap<String, Plan>,
        /// 실행 순서 기록(undo 윈도우 판정용). 새 것이 뒤.
        executed_order: Vec<String>,
        journal: Vec<JournalEntry>,
        next_entry: u64,
    }

    impl Store for FakeStore {
        fn save_plan(&mut self, plan: Plan) {
            self.plans.insert(plan.plan_id.clone(), plan);
        }
        fn get_plan(&self, plan_id: &str) -> Option<Plan> {
            self.plans.get(plan_id).cloned()
        }
        fn set_plan_status(&mut self, plan_id: &str, status: PlanStatus) {
            if let Some(p) = self.plans.get_mut(plan_id) {
                p.status = status;
            }
            if status == PlanStatus::Executed {
                self.executed_order.retain(|id| id != plan_id);
                self.executed_order.push(plan_id.to_string());
            }
        }
        fn append_journal(&mut self, mut entry: JournalEntry) -> u64 {
            self.next_entry += 1;
            entry.entry_id = self.next_entry;
            let id = entry.entry_id;
            self.journal.push(entry);
            id
        }
        fn journal_for_plan(&self, plan_id: &str) -> Vec<JournalEntry> {
            self.journal
                .iter()
                .filter(|e| e.plan_id == plan_id)
                .cloned()
                .collect()
        }
        fn mark_undone(&mut self, entry_id: u64, ts: u64) {
            if let Some(e) = self.journal.iter_mut().find(|e| e.entry_id == entry_id) {
                e.undone_at = Some(ts);
            }
        }
        fn recent_executed_plan_ids(&self, limit: usize) -> Vec<String> {
            self.executed_order
                .iter()
                .rev()
                .take(limit)
                .cloned()
                .collect()
        }
        fn complete_journal(&mut self, entry_id: u64, ts: u64) {
            if let Some(e) = self.journal.iter_mut().find(|e| e.entry_id == entry_id) {
                e.completed_at = Some(ts);
            }
        }
        fn update_journal_to(&mut self, entry_id: u64, to: std::path::PathBuf) {
            if let Some(e) = self.journal.iter_mut().find(|e| e.entry_id == entry_id) {
                e.to = to;
            }
        }
        fn inflight_entries(&self) -> Vec<JournalEntry> {
            self.journal
                .iter()
                .filter(|e| e.completed_at.is_none())
                .cloned()
                .collect()
        }
    }

    /// 인메모리 FileOps. 실제 디스크 대신 경로 집합과 staging 맵으로 시뮬레이트.
    #[derive(Default)]
    pub struct FakeFileOps {
        pub present: HashSet<PathBuf>,
        /// content_hash -> 격리 시점의 원본 경로(복원 대상).
        pub staged: HashMap<String, PathBuf>,
        pub staging_dir: PathBuf,
        /// Some(n): n번째 move_file 호출(0-indexed)에서 io::Error를 주입.
        pub fail_move_at: Option<usize>,
        move_call_count: usize,
    }

    impl FakeFileOps {
        pub fn with_files<I: IntoIterator<Item = PathBuf>>(files: I) -> Self {
            FakeFileOps {
                present: files.into_iter().collect(),
                staged: HashMap::new(),
                staging_dir: PathBuf::from("/__staging__"),
                fail_move_at: None,
                move_call_count: 0,
            }
        }

        pub fn with_fail_at(mut self, idx: usize) -> Self {
            self.fail_move_at = Some(idx);
            self
        }
    }

    impl FileOps for FakeFileOps {
        fn move_file(&mut self, from: &Path, to: &Path) -> io::Result<()> {
            let call = self.move_call_count;
            self.move_call_count += 1;
            if self.fail_move_at == Some(call) {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    format!("injected failure at move #{call}"),
                ));
            }
            if !self.present.remove(from) {
                return Err(io::Error::new(io::ErrorKind::NotFound, "source missing"));
            }
            self.present.insert(to.to_path_buf());
            // create_dir_all 시뮬레이션: 목적지 상위 폴더들을 present 에 등록한다.
            let mut cur = to.parent();
            while let Some(dir) = cur {
                self.present.insert(dir.to_path_buf());
                cur = dir.parent();
            }
            Ok(())
        }
        fn stage_file(&mut self, from: &Path, content_hash: &str) -> io::Result<PathBuf> {
            if !self.present.remove(from) {
                return Err(io::Error::new(io::ErrorKind::NotFound, "source missing"));
            }
            self.staged
                .insert(content_hash.to_string(), from.to_path_buf());
            let staged_path = self.staging_dir.join(content_hash);
            self.present.insert(staged_path.clone());
            Ok(staged_path)
        }
        fn restore_file(&mut self, content_hash: &str, to: &Path) -> io::Result<()> {
            match self.staged.remove(content_hash) {
                Some(_orig) => {
                    let staged_path = self.staging_dir.join(content_hash);
                    self.present.remove(&staged_path);
                    self.present.insert(to.to_path_buf());
                    Ok(())
                }
                None => Err(io::Error::new(io::ErrorKind::NotFound, "not staged")),
            }
        }
        fn exists(&self, p: &Path) -> bool {
            self.present.contains(p)
        }
        fn remove_empty_dir(&mut self, dir: &Path) -> io::Result<()> {
            // 비어 있음 = present 에 dir 하위 경로가 없음(dir 자신 제외).
            let non_empty = self
                .present
                .iter()
                .any(|pp| pp != dir && pp.starts_with(dir));
            if non_empty {
                return Err(io::Error::new(io::ErrorKind::Other, "directory not empty"));
            }
            self.present.remove(dir);
            Ok(())
        }
    }
}
