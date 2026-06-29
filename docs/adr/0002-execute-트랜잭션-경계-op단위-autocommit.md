# ADR-0002 — execute 트랜잭션 경계: 단일 트랜잭션 → op단위 autocommit

| 항목 | 내용 |
|------|------|
| **상태** | 수용됨 (Phase 2 안전 감사, 2026-06-29) |
| **결정자** | 프로젝트 오너 (감사 후 수정 지시) |

---

## Context

### 초기 구현의 결함

Phase 2 초기 `execute`는 단일 SQLite 트랜잭션으로 모든 op를 감쌌다.

```rust
// 초기 구현 (결함 있음)
let tx = conn.unchecked_transaction()?;
let store = SqliteStore::new(&*tx);
engine.execute(&plan_id, true)?;
tx.commit()?;
```

이 구조에는 근본적인 비대칭이 있다.

**FS 작업은 트랜잭션 밖에 있다.** SQLite 트랜잭션은 DB 변경만 롤백한다. 파일 이동은 롤백되지 않는다. 따라서 5개 op 중 3번째에서 실패하면:

```
파일 상태:  f0 이동됨, f1 이동됨, f2 실패 (변경 없음)
journal:    ROLLBACK → f0, f1의 기록도 사라짐
```

결과: f0, f1이 이동됐지만 journal에 기록이 없어 **undo 경로가 없다.** "항상 되돌릴 수 있다"는 핵심 약속이 깨진다.

### journal-first와의 충돌

S2 journal-first 패턴(파일 작업 전 journal에 intent 기록)을 단일 트랜잭션으로 구현하면, 파일 작업 실패 시 ROLLBACK이 intent 기록도 지운다. journal-first의 의미가 사라진다.

---

## Decision

**execute 루프에서 트랜잭션 래퍼를 제거하고, op 단위 autocommit으로 전환한다.**

각 op의 실행 단위:

```
1. append_journal(completed_at=None)  → autocommit (intent 기록, 즉시 내구화)
2. 파일 작업 (move_file / stage_file)
3. complete_journal(entry_id, ts)     → autocommit (완료 확정, 즉시 내구화)
```

파일 작업 실패 시:
- 앞선 op들의 journal은 이미 autocommit으로 내구화되어 살아 있다.
- 실패한 op의 intent(`completed_at=NULL`)는 DB에 남아 `recover_inflight` 대상이 된다.
- 플랜 상태를 `Executed`로 표시해 undo 가능하게 한다.
- `GateError::PartialExecute { completed, failed_op_id, ... }`를 반환한다.

**포기한 보장**: 플랜 전체의 all-or-nothing 원자성.  
**새로 얻은 보장**: 완료된 op는 반드시 내구적으로 undo 가능하고, 멈춘 지점은 crash recovery로 정리 가능하다.

`undo_plan`은 여전히 단일 트랜잭션을 사용한다. undo는 DB 변경(journal mark)과 파일 역이동이 같은 플랜 내에서 이루어지므로 원자성 확보가 가능하다.

---

## Consequences

**긍정**

- journal-first(S2)와 op단위 내구성(S1)이 구조적으로 일관된다. `completed_at=NULL` intent가 ROLLBACK에 사라지지 않는다.
- 부분 실패 후 undo: 완료된 op들만 역실행, inflight op는 건너뜀.
- 크래시 복원 경로(`recover_inflight`)가 의미를 가진다.

**비용**

- 전체 원자성 상실: 플랜 내 모든 op가 성공하거나 전혀 실행되지 않는 보장이 없다.  
  → 이는 의도된 트레이드오프. FS와 DB의 원자성 결합은 불가능하다.
- 클라이언트가 `PartialExecute`를 별도 처리해야 한다 (`execute_plan` 커맨드가 `{"partial": true}` 반환).

**관련 코드 경로 및 테스트**

- `crates/tidydog-core/src/gate.rs::Engine::execute` — op 루프, autocommit 구조
- `tests/gate.rs::partial_execute_prior_ops_remain_in_journal`  
  → 5-op 플랜 op index 2 실패 시 op 0·1의 journal에 `completed_at`이 있음을 검증
- `tests/gate.rs::partial_execute_then_undo_reverts_completed_ops`  
  → 부분 실패 후 undo가 완료된 op 0·1만 역실행함을 검증
- `tests/gate.rs::journal_first_completed_at_set_after_execute`  
  → 정상 실행 후 inflight 항목이 0임을 검증 (S2)
- `src-tauri/src/lib.rs::execute_plan` — PartialExecute graceful 처리
