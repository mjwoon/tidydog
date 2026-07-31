# ADR-0007 — 삭제·제거·중복 정리는 stage(격리)로 매핑 (delete/trash op 미도입)

| 항목 | 내용 |
|------|------|
| **상태** | 수용됨 (Phase 4, 2026-07) |
| **결정자** | 프로젝트 오너 |
| **관련** | ADR-0003(격리=앱 staging), ADR-0006(툴 카탈로그 경계) |

---

## Context

op 스키마의 `action`은 `move | stage | rename` 3종뿐이다. `delete`도 `trash`도 없다.

Phase 4 에이전트 루프를 검증하던 중 버그가 드러났다. 사용자가 "중복본 제거", "이거 버려"
같은 **삭제 의도**를 표현하면, 에이전트는 그에 맞는 op를 만들려 했으나 스키마에 delete가
없어 `propose_plan`을 호출하지 못하고 **산문 서술로 빠졌다**("~를 제거하겠습니다"로 끝나고
실제 플랜 없음). 즉 삭제 의도가 실행 가능한 플랜으로 이어지지 못했다.

두 가지 해결 방향이 있었다.

**방향 A — `delete`(또는 `trash`) op를 정식 추가**
사용자 의도를 그대로 표현하는 op를 만든다. 직관적이다.

**방향 B — 삭제 의도를 기존 `stage`(격리)로 매핑**
새 op action을 추가하지 않고, 삭제/제거/중복본 정리 의도를 전부 `stage`로 표현한다.
파일은 앱 staging 디렉터리로 격리되고 복원 가능하다.

---

## Decision

**삭제·제거·버리기·중복본 정리 의도는 전부 `stage`(격리)로 표현한다. 새 op action을
추가하지 않는다.**

- SYSTEM_PROMPT에 명시: "delete 동작은 존재하지 않는다. 삭제/제거/버리기/중복본 정리
  의도는 `stage`(격리)로 표현한다. TidyDog은 파일을 영구 삭제하지 않고 staging으로 보내며
  복원 가능하다."
- `propose_plan` 툴 스키마의 `action` 값별 description에 이 매핑을 박아둔다.
- 기존 stage 인프라(staging 디렉터리, journal, undo/restore)를 그대로 재사용한다.

이는 CLAUDE.md의 안전 불변식 **"영구 삭제 없음"** 을 op 레이어에서 강제하는 결정이다.

### 왜 방향 B인가

1. **비가역 삭제를 구조적으로 배제.** delete/trash op가 없으면, 에이전트가 아무리 "삭제"를
   의도해도 물리적 삭제 코드 경로 자체가 존재하지 않는다.
2. **안전 표면을 넓히지 않음.** 새 core action = 새 실행 분기 = 새 undo 분기 = 새 검증 부담.
   stage 재사용은 이미 검증된 격리·복원 경로를 탄다.
3. **복원 보장.** stage는 파일을 staging 디렉터리로 옮길 뿐이며 `restore_file`로 되돌릴 수
   있다(ADR-0003). "삭제했다"고 표시돼도 데이터는 살아 있다.

---

## Consequences

**긍정**

- 확률적 에이전트가 "삭제"를 의도해도 영구 삭제는 일어나지 않는다. 최악은 격리이며 복원 가능.
- op 스키마·실행·undo 분기가 3종으로 유지되어 검증 표면이 작다.
- 삭제 의도가 실행 가능한 플랜(stage op)으로 이어져 산문-폴백 버그가 해소된다.

**비용**

- 사용자가 "완전히 지웠다"고 기대하면 격리 디렉터리가 쌓인다. 진짜 영구 삭제(OS 휴지통
  이동, purge)는 별도 사용자 명시 동작으로 남는다.
- PlanReview에서 stage op의 목적지 라벨(`.archive/...`)과 실제 staging 경로가 다르다 —
  표시상 혼동 여지(후속 UX 정리 대상).

**관련 코드 경로**

- `src-tauri/src/agent.rs` — SYSTEM_PROMPT의 삭제→stage 매핑, propose_plan 스키마 description.
- `crates/tidydog-core/src/gate.rs::execute` — `Action::Stage` → `stage_file`.
- `src-tauri/src/fileops.rs::stage_file` / `restore_file` — 격리·복원.
- **미결(P-8)** — 정식 `trash` op(OS 휴지통) 도입은 별도 슬라이스로 보류. core action 추가 =
  안전 표면 확대이므로 신중히.
