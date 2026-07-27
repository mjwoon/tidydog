//! 게이트 — propose → confirm → execute → undo 오케스트레이션.
//! 안전 명세 §3·§5·§6의 구현. **파일을 바꾸는 유일한 경로.**
//!
//! 구조적 안전 보장:
//! - execute 는 Confirmed + user_confirmed 가 아니면 거부한다.
//! - execute 직전 모든 op의 from/to 를 가드로 재검증(TOCTOU)하고, 하나라도 거부되면 무변경.
//! - 영구삭제 함수가 없다. 삭제는 Stage(휴지통/staging)뿐.
//! - undo 는 최근 UNDO_WINDOW(5)개 실행 플랜으로 제한.

use crate::model::*;
use crate::ops::{FileOps, Store};
use crate::risk::{preview_level, risk_score};
use crate::scope::Guard;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// undo 가능 범위(C5): 최근 5개 실행 플랜.
pub const UNDO_WINDOW: usize = 5;

/// 게이트 거부/실패 사유.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateError {
    NoSuchPlan,
    /// confirm은 Proposed 상태에서만.
    NotProposed,
    /// execute는 Confirmed 상태에서만.
    NotConfirmed,
    /// execute에는 사용자 확인 플래그가 필요.
    NotUserConfirmed,
    /// undo는 Executed 상태에서만.
    NotExecuted,
    /// 최근 5개 밖.
    OutOfUndoWindow,
    /// 스코프 가드 거부 — 이 시점까지 아무것도 바뀌지 않았다.
    ScopeDenied { op_id: String, denial: Denial },
    /// 파일 작업 실패 (스코프 검증 이전 단계, 아무것도 안 바뀜).
    FileOp { op_id: String, message: String },
    /// 부분 실행: 앞선 op들은 완료됐고 undo 가능. 멈춘 op의 journal intent는
    /// completed_at=NULL로 남아 recover_inflight가 정리한다.
    PartialExecute {
        failed_op_id: String,
        message: String,
        /// 정상 완료된 op 수.
        completed: usize,
    },
}

/// execute 결과 요약.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ExecOutcome {
    pub plan_id: String,
    pub moved: usize,
    pub staged: usize,
    pub renamed: usize,
    pub entries: Vec<u64>,
}

/// 안전 코어 엔진. 가드·스토어·파일작업을 트레이트로 주입받는다.
pub struct Engine<G: Guard, S: Store, F: FileOps> {
    pub guard: G,
    pub store: S,
    pub fileops: F,
    seq_counter: u64,
}

impl<G: Guard, S: Store, F: FileOps> Engine<G, S, F> {
    pub fn new(guard: G, store: S, fileops: F) -> Self {
        Self {
            guard,
            store,
            fileops,
            seq_counter: 0,
        }
    }

    /// 플랜 제안. **아무것도 바꾸지 않는다.** id·seq 부여 + 미리보기/위험도 계산 후 저장.
    pub fn propose(
        &mut self,
        mut ops: Vec<PlanOp>,
        scheme: Option<String>,
        rule_summary: Option<String>,
    ) -> Plan {
        let plan_id = self.gen_id("plan");
        for (i, op) in ops.iter_mut().enumerate() {
            op.seq = i as u32;
            op.op_id = format!("{plan_id}-{i}");
        }
        let plan = Plan {
            plan_id: plan_id.clone(),
            created_at: now(),
            status: PlanStatus::Proposed,
            risk_score: risk_score(&ops),
            preview: preview_level(&ops),
            scheme,
            rule_summary,
            ops,
        };
        self.store.save_plan(plan.clone());
        plan
    }

    /// 사용자 검토 후 확정. Proposed → Confirmed.
    pub fn confirm(&mut self, plan_id: &str) -> Result<(), GateError> {
        let plan = self.store.get_plan(plan_id).ok_or(GateError::NoSuchPlan)?;
        if plan.status != PlanStatus::Proposed {
            return Err(GateError::NotProposed);
        }
        self.store.set_plan_status(plan_id, PlanStatus::Confirmed);
        Ok(())
    }

    /// 실행 — 파일을 바꾸는 유일한 경로. Confirmed + user_confirmed 필수.
    pub fn execute(
        &mut self,
        plan_id: &str,
        user_confirmed: bool,
    ) -> Result<ExecOutcome, GateError> {
        let plan = self.store.get_plan(plan_id).ok_or(GateError::NoSuchPlan)?;
        if plan.status != PlanStatus::Confirmed {
            return Err(GateError::NotConfirmed);
        }
        if !user_confirmed {
            return Err(GateError::NotUserConfirmed);
        }

        // ── 1단계: 전 op 가드 재검증(TOCTOU). 하나라도 거부되면 무변경. ──
        for op in &plan.ops {
            self.guard
                .check(&op.from)
                .map_err(|denial| GateError::ScopeDenied {
                    op_id: op.op_id.clone(),
                    denial,
                })?;
            // 이동/리네임의 목적지도 스코프 안이어야 한다. 격리(Stage)의 목적지는 staging(스코프 밖, 어댑터 관할)이라 제외.
            if op.action != Action::Stage {
                self.guard
                    .check(&op.to)
                    .map_err(|denial| GateError::ScopeDenied {
                        op_id: op.op_id.clone(),
                        denial,
                    })?;
            }
        }

        // ── 2단계: S2 journal-first + op 단위 내구성 ──
        //
        // 각 op: [intent INSERT(autocommit)] → [파일 작업] → [complete UPDATE(autocommit)]
        // 파일 작업 실패 시: intent는 completed_at=NULL로 DB에 남고(recover_inflight 대상),
        //   앞선 완료된 op들은 내구적으로 살아있어 undo 가능.
        // 단일 트랜잭션으로 감싸지 않으므로 각 SQL은 autocommit으로 즉시 내구화된다.
        let mut outcome = ExecOutcome {
            plan_id: plan_id.to_string(),
            ..Default::default()
        };
        for op in &plan.ops {
            let ts = now();
            let done_count = outcome.moved + outcome.renamed + outcome.staged;
            match op.action {
                Action::Move | Action::Rename => {
                    let (final_to, renamed) = self.resolve_conflict(&op.to);
                    // move 가 새로 만들 목적지 폴더 = undo 시 되돌릴 대상(파일 작업 전 계산).
                    let created_dirs = self.dirs_to_create(&final_to);
                    // 1) 의도 기록(completed_at=None) — autocommit으로 즉시 내구화
                    let entry_id = self.store.append_journal(JournalEntry {
                        entry_id: 0,
                        plan_id: plan_id.to_string(),
                        op_id: op.op_id.clone(),
                        action: op.action,
                        content_hash: op.content_hash.clone(),
                        from: op.from.clone(),
                        to: final_to.clone(),
                        executed_at: ts,
                        completed_at: None,
                        undoable: true,
                        undone_at: None,
                        created_dirs,
                    });
                    // 2) 파일 작업
                    if let Err(e) = self.fileops.move_file(&op.from, &final_to) {
                        // intent는 DB에 남아 recover_inflight가 정리함.
                        // 앞선 op들은 이미 내구화 → plan을 executed로 표시해 undo 가능하게 함.
                        self.store.set_plan_status(plan_id, PlanStatus::Executed);
                        return Err(GateError::PartialExecute {
                            failed_op_id: op.op_id.clone(),
                            message: e.to_string(),
                            completed: done_count,
                        });
                    }
                    // 3) 완료 확정 — autocommit
                    self.store.complete_journal(entry_id, now());
                    outcome.entries.push(entry_id);
                    if renamed || op.action == Action::Rename {
                        outcome.renamed += 1;
                    } else {
                        outcome.moved += 1;
                    }
                }
                Action::Stage => {
                    // 1) 의도 기록(completed_at=None)
                    let entry_id = self.store.append_journal(JournalEntry {
                        entry_id: 0,
                        plan_id: plan_id.to_string(),
                        op_id: op.op_id.clone(),
                        action: Action::Stage,
                        content_hash: op.content_hash.clone(),
                        from: op.from.clone(),
                        to: op.from.clone(),
                        executed_at: ts,
                        completed_at: None,
                        undoable: true,
                        undone_at: None,
                        // stage 는 사용자 트리에 폴더를 만들지 않는다(staging_dir 로 이동).
                        created_dirs: Vec::new(),
                    });
                    // 2) 파일 작업
                    let staged = match self.fileops.stage_file(&op.from, &op.content_hash) {
                        Err(e) => {
                            self.store.set_plan_status(plan_id, PlanStatus::Executed);
                            return Err(GateError::PartialExecute {
                                failed_op_id: op.op_id.clone(),
                                message: e.to_string(),
                                completed: done_count,
                            });
                        }
                        Ok(p) => p,
                    };
                    // 3) staged_path 확정 + 완료 기록
                    self.store.update_journal_to(entry_id, staged);
                    self.store.complete_journal(entry_id, now());
                    outcome.entries.push(entry_id);
                    outcome.staged += 1;
                }
            }
        }

        self.store.set_plan_status(plan_id, PlanStatus::Executed);
        Ok(outcome)
    }

    /// 되돌리기. Executed + 최근 UNDO_WINDOW 안일 때만. 이동은 역실행, 격리는 복원.
    pub fn undo(&mut self, plan_id: &str) -> Result<(), GateError> {
        let plan = self.store.get_plan(plan_id).ok_or(GateError::NoSuchPlan)?;
        if plan.status != PlanStatus::Executed {
            return Err(GateError::NotExecuted);
        }
        let recent = self.store.recent_executed_plan_ids(UNDO_WINDOW);
        if !recent.iter().any(|p| p == plan_id) {
            return Err(GateError::OutOfUndoWindow);
        }

        // 실행의 역순으로 되돌린다.
        let mut entries = self.store.journal_for_plan(plan_id);
        entries.sort_by(|a, b| b.entry_id.cmp(&a.entry_id));
        for entry in entries {
            if entry.undone_at.is_some() {
                continue;
            }
            // completed_at=None = in-flight(파일 작업 미완료) → 되돌릴 파일 없음. 건너뜀.
            if entry.completed_at.is_none() {
                continue;
            }
            match entry.action {
                Action::Move | Action::Rename => {
                    self.fileops
                        .move_file(&entry.to, &entry.from)
                        .map_err(|e| GateError::FileOp {
                            op_id: entry.op_id.clone(),
                            message: e.to_string(),
                        })?;
                    // execute 가 만든 빈 디렉터리 제거(깊은 것부터). 비어 있지 않으면
                    // remove_empty_dir 이 Err → 무시(다른 파일이 남았으면 보존, 안전).
                    for dir in &entry.created_dirs {
                        let _ = self.fileops.remove_empty_dir(dir);
                    }
                }
                Action::Stage => {
                    self.fileops
                        .restore_file(&entry.content_hash, &entry.from)
                        .map_err(|e| GateError::FileOp {
                            op_id: entry.op_id.clone(),
                            message: e.to_string(),
                        })?;
                }
            }
            self.store.mark_undone(entry.entry_id, now());
        }
        self.store.set_plan_status(plan_id, PlanStatus::Undone);
        Ok(())
    }

    /// move 목적지의 상위 디렉터리 중 아직 없는 것들을 깊은 것부터 반환.
    /// create_dir_all 이 새로 만들 폴더 = undo 시 되돌릴 대상. 이미 있는 폴더는 제외한다.
    fn dirs_to_create(&self, to: &Path) -> Vec<PathBuf> {
        let mut created = Vec::new();
        let mut cur = to.parent();
        while let Some(dir) = cur {
            if self.fileops.exists(dir) {
                break;
            }
            created.push(dir.to_path_buf());
            cur = dir.parent();
        }
        created
    }

    /// 목적지 충돌 해소: 이미 존재하면 "name (n).ext"로. 덮어쓰기 없음.
    fn resolve_conflict(&self, to: &Path) -> (PathBuf, bool) {
        if !self.fileops.exists(to) {
            return (to.to_path_buf(), false);
        }
        let parent = to.parent().map(|p| p.to_path_buf()).unwrap_or_default();
        let stem = to
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        let ext = to.extension().map(|e| e.to_string_lossy().to_string());
        for n in 1.. {
            let name = match &ext {
                Some(e) => format!("{stem} ({n}).{e}"),
                None => format!("{stem} ({n})"),
            };
            let candidate = parent.join(name);
            if !self.fileops.exists(&candidate) {
                return (candidate, true);
            }
        }
        unreachable!()
    }

    fn gen_id(&mut self, prefix: &str) -> String {
        self.seq_counter += 1;
        format!("{prefix}-{}-{}", now_nanos(), self.seq_counter)
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn now_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}
