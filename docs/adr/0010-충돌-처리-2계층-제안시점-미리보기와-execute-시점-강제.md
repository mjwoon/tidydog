# ADR-0010 — 충돌 처리 2계층: 제안 시점 미리보기 + execute 시점 강제

| 항목 | 내용 |
|------|------|
| **상태** | 수용됨 (P-1, 2026-07) |
| **결정자** | 프로젝트 오너 |
| **관련** | ADR-0002(op 단위 execute), CLAUDE.md C4(덮어쓰기 없음) |

---

## Context

op 스키마의 `conflict` 필드(`none | rename`)는 목적지에 동명 파일이 있을 때 PlanReview에서
"이름충돌" 배지를 띄우기 위한 것이다. 그러나 P-1 이전에는 `PlanOp::new`가 항상
`Conflict::None`으로 두었고, 이를 채우는 로직이 어디에도 없었다 → 배지는 영구 휴면.

한편 실제 안전(덮어쓰기 없음, C4)은 이미 **execute 시점**에 보장되고 있었다.
`gate.rs::resolve_conflict`가 파일을 쓰기 직전 `fileops.exists(to)`를 검사해 목적지가 차 있으면
`name (n).ext`로 리네임한다. 즉 "충돌을 안전하게 처리"하는 코드는 있었으나, "충돌을 사용자에게
미리 보여주는" 코드가 없었다.

충돌을 어디서 판정할 것인가에 두 가지 선택이 있었다.

**단일 계층(execute만)** — 지금처럼 execute에서만 처리. 사용자는 승인 시점에 충돌을 모른다.

**2계층(제안 + execute)** — 제안 시점에도 미리보기로 판정해 배지를 띄우고, execute는 그대로
권위 있는 강제를 유지.

---

## Decision

**충돌을 두 계층에서 다룬다. 제안 시점은 UX 미리보기(비권위), execute 시점은 실제 강제(권위).**

**제안 시점 (미리보기, 비권위)**
- `Engine::propose`가 각 op(Move/Rename)의 `fileops.exists(op.to)`를 검사해 차 있으면
  `conflict = Rename`을 채운다.
- Stage는 제외한다 — `to`가 `from` placeholder라 검사하면 항상 오탐한다.
- 목적: PlanReview에서 사용자가 "이 이동은 이름이 바뀐다"를 승인 전에 인지 → 깨끗한 이동으로
  착각해 승인하는 것 방지(§ 인계 §2-2). `risk_score`도 이 값을 소비해 위험도에 반영된다.

**execute 시점 (강제, 권위)**
- `resolve_conflict`가 쓰기 직전 다시 `exists(to)`를 검사해 실제로 리네임한다(C4).
- 제안과 execute 사이에 FS 상태가 바뀔 수 있으므로 **execute의 재검사가 최종 판정**이다.
  제안 시점 `conflict` 값은 신뢰의 근거가 아니라 표시용이다.

**같은 플랜 내 op 간 충돌**(두 op가 같은 목적지를 노림)은 제안 시점 미리보기 범위 밖이다.
execute가 op를 순차 실행하며 `resolve_conflict`로 안전하게 처리한다.

---

## Consequences

**긍정**

- 사용자가 승인 전에 충돌을 인지한다. "덮어쓰는 줄 알았는데 리네임됐다" 같은 놀람이 준다.
- 안전(C4)은 execute의 권위 있는 재검사로 유지된다 — 제안 시점 값이 틀려도(경합) 위험 없음.
- 판정 기준(`fileops.exists`)이 두 계층에서 동일해 일관적이다.

**비용**

- 충돌 정보가 두 곳에서 계산된다(제안·execute). 단, 제안은 표시용/비권위임을 명확히 함으로써
  "왜 두 번 검사하나"의 혼란을 이 ADR로 봉인한다.
- 제안과 execute 사이 경합 시 배지가 실제와 어긋날 수 있다(드묾, 안전엔 무해).

**관련 코드 경로**

- `crates/tidydog-core/src/gate.rs::propose` — 제안 시점 conflict 채움(Move/Rename, exists 검사).
- `crates/tidydog-core/src/gate.rs::resolve_conflict` — execute 시점 리네임(C4, 권위).
- `crates/tidydog-core/src/risk.rs` — `conflict == Rename` 소비(위험도).
- `src/components/PlanReview.tsx` — `conflict !== "none"` 배지(오커, 기존).
- 테스트: `propose_flags_conflict_when_dest_exists` 외 2건, `conflict_renamed_not_overwritten`(execute).
- 커밋: `91e41ab`.
