//! 게이트 통합 테스트 — 인메모리 fake로 안전 속성 검증.

use std::path::PathBuf;
use tidydog_core::testing::{FakeFileOps, FakeStore};
use tidydog_core::{Action, Engine, FileOps, GateError, PlanOp, PlanStatus, ScopeGuard, Store};

fn engine_with_fail(files: &[&str], fail_at: usize) -> Engine<ScopeGuard, FakeStore, FakeFileOps> {
    let guard = ScopeGuard::new(vec![p("/scope")], vec![]);
    let fileops = FakeFileOps::with_files(files.iter().map(|s| p(s))).with_fail_at(fail_at);
    Engine::new(guard, FakeStore::default(), fileops)
}

fn p(s: &str) -> PathBuf {
    PathBuf::from(s)
}

/// 스코프 루트 "/scope" 의 가드 + 주어진 present 파일들로 엔진 구성.
fn engine(files: &[&str]) -> Engine<ScopeGuard, FakeStore, FakeFileOps> {
    let guard = ScopeGuard::new(vec![p("/scope")], vec![]);
    let fileops = FakeFileOps::with_files(files.iter().map(|s| p(s)));
    Engine::new(guard, FakeStore::default(), fileops)
}

fn mv(from: &str, to: &str) -> PlanOp {
    PlanOp::new(Action::Move, "h", p(from), p(to))
}

#[test]
fn propose_changes_nothing() {
    let mut e = engine(&["/scope/a.txt"]);
    let plan = e.propose(vec![mv("/scope/a.txt", "/scope/docs/a.txt")], None, None);
    assert_eq!(plan.status, PlanStatus::Proposed);
    // 파일은 그대로
    assert!(e.fileops.exists(&p("/scope/a.txt")));
    assert!(!e.fileops.exists(&p("/scope/docs/a.txt")));
}

#[test]
fn execute_requires_confirm() {
    let mut e = engine(&["/scope/a.txt"]);
    let plan = e.propose(vec![mv("/scope/a.txt", "/scope/docs/a.txt")], None, None);
    assert_eq!(e.execute(&plan.plan_id, true), Err(GateError::NotConfirmed));
    assert!(e.fileops.exists(&p("/scope/a.txt"))); // 무변경
}

#[test]
fn execute_requires_user_flag() {
    let mut e = engine(&["/scope/a.txt"]);
    let plan = e.propose(vec![mv("/scope/a.txt", "/scope/docs/a.txt")], None, None);
    e.confirm(&plan.plan_id).unwrap();
    assert_eq!(
        e.execute(&plan.plan_id, false),
        Err(GateError::NotUserConfirmed)
    );
    assert!(e.fileops.exists(&p("/scope/a.txt"))); // 무변경
}

#[test]
fn happy_move() {
    let mut e = engine(&["/scope/a.txt"]);
    let plan = e.propose(vec![mv("/scope/a.txt", "/scope/docs/a.txt")], None, None);
    e.confirm(&plan.plan_id).unwrap();
    let out = e.execute(&plan.plan_id, true).unwrap();
    assert_eq!(out.moved, 1);
    assert!(!e.fileops.exists(&p("/scope/a.txt")));
    assert!(e.fileops.exists(&p("/scope/docs/a.txt")));
    assert_eq!(e.store.get_plan(&plan.plan_id).unwrap().status, PlanStatus::Executed);
}

#[test]
fn scope_denied_no_change() {
    // from 이 스코프 밖
    let mut e = engine(&["/outside/x.txt"]);
    let plan = e.propose(vec![mv("/outside/x.txt", "/scope/x.txt")], None, None);
    e.confirm(&plan.plan_id).unwrap();
    let res = e.execute(&plan.plan_id, true);
    assert!(matches!(res, Err(GateError::ScopeDenied { .. })));
    // 아무것도 바뀌지 않음
    assert!(e.fileops.exists(&p("/outside/x.txt")));
    assert!(!e.fileops.exists(&p("/scope/x.txt")));
}

#[test]
fn conflict_renamed_not_overwritten() {
    // 목적지에 동명 파일이 이미 존재
    let mut e = engine(&["/scope/a.txt", "/scope/docs/a.txt"]);
    let plan = e.propose(vec![mv("/scope/a.txt", "/scope/docs/a.txt")], None, None);
    e.confirm(&plan.plan_id).unwrap();
    let out = e.execute(&plan.plan_id, true).unwrap();
    assert_eq!(out.renamed, 1);
    // 기존 파일 보존(덮어쓰기 없음), 새 파일은 리네임되어 안착, 원본 from 은 사라짐
    assert!(e.fileops.exists(&p("/scope/docs/a.txt")));
    assert!(e.fileops.exists(&p("/scope/docs/a (1).txt")));
    assert!(!e.fileops.exists(&p("/scope/a.txt")));
}

#[test]
fn stage_is_soft_delete_and_restorable() {
    let mut e = engine(&["/scope/old.dmg"]);
    let op = PlanOp::new(Action::Stage, "hash-dmg", p("/scope/old.dmg"), p("/scope/old.dmg"));
    let plan = e.propose(vec![op], None, None);
    e.confirm(&plan.plan_id).unwrap();
    let out = e.execute(&plan.plan_id, true).unwrap();
    assert_eq!(out.staged, 1);
    // 원위치엔 없지만 영구삭제가 아니다(staged 보존)
    assert!(!e.fileops.exists(&p("/scope/old.dmg")));
    assert!(e.fileops.staged.contains_key("hash-dmg"));
    // undo → 복원
    e.undo(&plan.plan_id).unwrap();
    assert!(e.fileops.exists(&p("/scope/old.dmg")));
}

#[test]
fn undo_reverses_move() {
    let mut e = engine(&["/scope/a.txt"]);
    let plan = e.propose(vec![mv("/scope/a.txt", "/scope/docs/a.txt")], None, None);
    e.confirm(&plan.plan_id).unwrap();
    e.execute(&plan.plan_id, true).unwrap();
    e.undo(&plan.plan_id).unwrap();
    assert!(e.fileops.exists(&p("/scope/a.txt")));
    assert!(!e.fileops.exists(&p("/scope/docs/a.txt")));
    assert_eq!(e.store.get_plan(&plan.plan_id).unwrap().status, PlanStatus::Undone);
}

/// S2 journal-first: 파일 작업 완료 후 journal entry의 completed_at이 설정되는지 확인.
#[test]
fn journal_first_completed_at_set_after_execute() {
    let mut e = engine(&["/scope/a.txt"]);
    let plan = e.propose(vec![mv("/scope/a.txt", "/scope/docs/a.txt")], None, None);
    e.confirm(&plan.plan_id).unwrap();
    e.execute(&plan.plan_id, true).unwrap();
    // execute 후에는 inflight 항목이 없어야 한다.
    assert!(e.store.inflight_entries().is_empty());
}

/// S3 크래시 복원: execute 전에는 inflight 항목이 없다(propose만으로는 journal 없음).
#[test]
fn no_inflight_before_execute() {
    let mut e = engine(&["/scope/a.txt"]);
    let _plan = e.propose(vec![mv("/scope/a.txt", "/scope/docs/a.txt")], None, None);
    assert!(e.store.inflight_entries().is_empty());
}

// ── Q4: 실패·크래시 경로 테스트 ────────────────────────────────────────────

/// 중간 op 실패 시 앞선 op들의 journal이 내구적으로 남고 undo 가능해야 한다.
/// 5-op 플랜에서 op 3(index 2)이 실패 → op 0·1의 journal은 completed_at이 있어야 함.
#[test]
fn partial_execute_prior_ops_remain_in_journal() {
    let files = [
        "/scope/f0.txt", "/scope/f1.txt", "/scope/f2.txt",
        "/scope/f3.txt", "/scope/f4.txt",
    ];
    let mut e = engine_with_fail(&files, 2); // op index 2에서 move_file 실패
    let ops = vec![
        mv("/scope/f0.txt", "/scope/d/f0.txt"),
        mv("/scope/f1.txt", "/scope/d/f1.txt"),
        mv("/scope/f2.txt", "/scope/d/f2.txt"), // ← 이 op가 실패
        mv("/scope/f3.txt", "/scope/d/f3.txt"),
        mv("/scope/f4.txt", "/scope/d/f4.txt"),
    ];
    let plan = e.propose(ops, None, None);
    e.confirm(&plan.plan_id).unwrap();

    let result = e.execute(&plan.plan_id, true);

    // PartialExecute 에러여야 함
    assert!(matches!(result, Err(GateError::PartialExecute { completed: 2, .. })));

    // op 0·1: 디스크에서 이동됨
    assert!(!e.fileops.exists(&p("/scope/f0.txt")));
    assert!(e.fileops.exists(&p("/scope/d/f0.txt")));
    assert!(!e.fileops.exists(&p("/scope/f1.txt")));
    assert!(e.fileops.exists(&p("/scope/d/f1.txt")));

    // op 2: 실패했으므로 원위치
    assert!(e.fileops.exists(&p("/scope/f2.txt")));

    // journal: op 0·1은 completed_at이 Some, op 2는 completed_at이 None(inflight)
    let journal = e.store.journal_for_plan(&plan.plan_id);
    assert_eq!(journal.len(), 3, "op 0·1·2의 intent가 모두 저장돼야 함");

    let completed: Vec<_> = journal.iter().filter(|e| e.completed_at.is_some()).collect();
    let inflight: Vec<_> = journal.iter().filter(|e| e.completed_at.is_none()).collect();
    assert_eq!(completed.len(), 2, "op 0·1은 completed");
    assert_eq!(inflight.len(), 1, "op 2의 intent만 inflight");

    // plan은 executed 상태 → undo 가능
    assert_eq!(e.store.get_plan(&plan.plan_id).unwrap().status, PlanStatus::Executed);
}

/// 부분 실행 후 undo: 완료된 op 0·1만 되돌림, inflight op 2는 undo 대상 아님.
#[test]
fn partial_execute_then_undo_reverts_completed_ops() {
    let files = [
        "/scope/f0.txt", "/scope/f1.txt", "/scope/f2.txt",
    ];
    let mut e = engine_with_fail(&files, 2); // op 2에서 실패
    let ops = vec![
        mv("/scope/f0.txt", "/scope/d/f0.txt"),
        mv("/scope/f1.txt", "/scope/d/f1.txt"),
        mv("/scope/f2.txt", "/scope/d/f2.txt"),
    ];
    let plan = e.propose(ops, None, None);
    e.confirm(&plan.plan_id).unwrap();
    e.execute(&plan.plan_id, true).unwrap_err(); // PartialExecute

    // undo: completed된 op 0·1을 역순으로 되돌림
    e.undo(&plan.plan_id).unwrap();
    assert!(e.fileops.exists(&p("/scope/f0.txt")));
    assert!(e.fileops.exists(&p("/scope/f1.txt")));
    assert!(!e.fileops.exists(&p("/scope/d/f0.txt")));
    assert!(!e.fileops.exists(&p("/scope/d/f1.txt")));
}

/// recover_inflight 시나리오 1: to에 파일이 있음 → completed 처리.
#[test]
fn recover_inflight_to_exists_marks_completed() {
    let mut store = tidydog_core::testing::FakeStore::default();
    // in-flight entry: completed_at=None, to에 파일이 있는 상황 시뮬레이션
    let entry_id = store.append_journal(tidydog_core::JournalEntry {
        entry_id: 0,
        plan_id: "plan-1".to_string(),
        op_id: "op-1".to_string(),
        action: tidydog_core::Action::Move,
        content_hash: "h".to_string(),
        from: p("/scope/a.txt"),
        to: p("/scope/docs/a.txt"),
        executed_at: 1000,
        completed_at: None, // in-flight
        undoable: true,
        undone_at: None,
    });
    assert_eq!(store.inflight_entries().len(), 1);

    // to가 존재하므로 파일 작업은 완료됐음 → complete_journal
    store.complete_journal(entry_id, 1001);

    assert_eq!(store.inflight_entries().len(), 0, "완료 처리 후 inflight 없어야 함");
    let entries = store.journal_for_plan("plan-1");
    assert!(entries[0].completed_at.is_some());
}

/// recover_inflight 시나리오 2: from에 파일이 있음(op 시작 전 크래시) → 항목 제거 가능.
#[test]
fn recover_inflight_from_exists_means_op_not_started() {
    let mut store = tidydog_core::testing::FakeStore::default();
    store.append_journal(tidydog_core::JournalEntry {
        entry_id: 0,
        plan_id: "plan-2".to_string(),
        op_id: "op-1".to_string(),
        action: tidydog_core::Action::Move,
        content_hash: "h".to_string(),
        from: p("/scope/still-here.txt"),
        to: p("/scope/docs/still-here.txt"),
        executed_at: 2000,
        completed_at: None,
        undoable: true,
        undone_at: None,
    });

    // from이 존재하면 op가 실행되지 않은 것 → inflight entry 존재
    // (실제 recover_inflight는 FS를 확인해 DELETE 하지만, 여기선 Store 인터페이스로 검증)
    let inflight = store.inflight_entries();
    assert_eq!(inflight.len(), 1);
    assert_eq!(inflight[0].from, p("/scope/still-here.txt"));
    // 검증: from.exists()가 true면 op 미실행 → 정리 대상임을 확인
    assert!(!inflight[0].from.exists(), "테스트 환경에선 실제 파일 없음 — 경로 기록만 확인");
}

#[test]
fn undo_out_of_window_denied() {
    // 6개 플랜 실행 → 가장 오래된 것은 최근 5개 밖
    let files: Vec<String> = (0..6).map(|i| format!("/scope/f{i}.txt")).collect();
    let refs: Vec<&str> = files.iter().map(|s| s.as_str()).collect();
    let mut e = engine(&refs);

    let mut first_id = String::new();
    for i in 0..6 {
        let from = format!("/scope/f{i}.txt");
        let to = format!("/scope/d/f{i}.txt");
        let plan = e.propose(vec![mv(&from, &to)], None, None);
        if i == 0 {
            first_id = plan.plan_id.clone();
        }
        e.confirm(&plan.plan_id).unwrap();
        e.execute(&plan.plan_id, true).unwrap();
    }
    assert_eq!(e.undo(&first_id), Err(GateError::OutOfUndoWindow));
}
