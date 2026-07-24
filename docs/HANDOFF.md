# TidyDog 인계 — p4(에이전트 루프) 마무리 시점

작성 시점: 2026-07-06. 이 문서는 claude.ai 대화에서 Claude Code로 작업을 넘기며 쓴 것이다.
불변식·계약은 `CLAUDE.md`에 있다. 이 문서는 **지금 상태와 다음 할 일**이다.

---

## 1. 어디까지 왔나

repo 명명 기준 **p0**(스캐폴딩) ~ **p5b**(ORGANIZER 규칙 학습)까지 구현 완료.
이 문서가 하나로 묶어 부른 "에이전트 루프 + 규칙 학습"은 repo에서 두 슬라이스로 나뉜다:
**p4 = 에이전트 루프 + 챗 UI**(commit 2ebe1c7), **p5b = ORGANIZER 규칙 학습**
(commit 8e8f906, plan `docs/superpowers/plans/2026-07-02-phase5b-rule-learning.md`).
현재 **p4 육안 확인 중**(§4 — PlanReview/빈 플랜)이며, 확인이 끝나면 p4 공식 클로즈.
(참고: repo에는 p2=안전코어+어댑터, p3=콘텐츠 파이프라인+N1, n2=API 키 슬라이스도 있다.)

동작 확인된 것:
- 폴더 선택 → 스캔 → SQLite 영속화 → React 트리 렌더
- 안전 코어 통합, propose→confirm→execute 게이트, op 단위 journal-first 내구성, 크래시 복구
- Python 사이드카 콘텐츠 읽기 + LLM 요약(N1 동의 게이트, content_hash 캐싱)
- 에이전트 루프 멀티스텝 툴 오케스트레이션, ORGANIZER.md 규칙 학습(제안/승인 게이트)
- 챗에서 활성 대상 폴더 인식 → 현황 응답 → `propose_plan` 호출 → 플랜 카드 렌더

테스트: Rust 56 passed / 0 failed / 1 ignored, TypeScript 0 errors.

---

## 2. 최근 세션에서 확정한 결정

시간순. 각 결정의 근거까지 적어둔다 — 나중에 뒤집으려 할 때 이유를 알아야 하니까.

1. **op 필드 네이밍 = core 계약 유지(`action`/`from`/`to`).**
   초안 스펙은 `op_type`/`src`/`dst`였으나 실제 core PlanOp가 우선.
   안전 크레이트를 프론트 편의로 리네임하지 않는다.

2. **conflict 렌더 = 독립 오커 배지.**
   action 배지에 인라인으로 붙이지 않고 별도 pill로 분리.
   목적: 충돌 이동을 깨끗한 이동으로 착각해 승인하는 것 방지.

3. **빈 플랜 = 빈 상태 + 실행 버튼 숨김.**
   `op_count === 0`이면 "제안할 이동이 없습니다", 취소 버튼은 "닫기"로. no-op 실행 방지.
   `op_count > 0`인데 ops가 없으면 방어적 폴백(= 전달 체인 회귀 신호).

4. **전달 체인 회귀 잠금.**
   `plan_proposed_variant_carries_ops_end_to_end` 테스트 추가.
   variant 레벨 + 직렬화 경계 두 곳 단언. 일부러 끊어서 FAILED 확인 후 원복 완료.
   (과거에 이 체인 8곳이 끊겨 PlanReview가 플레이스홀더로 폴백한 사고가 있었다.)

5. **삭제 의도 → `stage`(격리) 매핑.**
   에이전트가 중복본에 "제거"를 시도했으나 스키마에 delete가 없어 `propose_plan`을
   호출하지 못하고 산문 서술로 빠지는 버그가 있었다.
   해결: SYSTEM_PROMPT에 매핑 명시 + 툴 스키마 action 값별 description 부여.
   **새 op action을 추가하지 않고** 기존 stage를 재사용 — staging 인프라·journal·undo가 이미 다룸.
   (휴지통 `trash` op 정식 추가는 별도 슬라이스 후보. 지금은 하지 않는다.)

6. **content_hash 확보 = `scan_directory` 경유 강제.**
   `list_files`에는 content_hash가 없어 중복 판정 불가.
   `list_files`에 해시를 싣는 대신 툴 역할을 분리:
   현황=list_files(가벼움) / 플랜=scan_directory(해시 포함).

7. **챗 버블 마크다운 렌더 도입(react-markdown).**
   에이전트 응답이 구조적 마크다운이라 렌더가 유리. raw HTML 차단·링크 스킴 화이트리스트 적용.

8. **활성 대상 경로 관통.**
   FE pill의 활성 경로가 `run_agent_loop` 시스템 컨텍스트까지 전달되지 않아
   에이전트가 매번 경로를 되묻던 버그 해결. 대상 폴더는 DB 우선(재스캔 안 함).

---

## 3. 미해결 버그 (다음 작업 후보)

### ✅ B1 (해결됨 — commit 1bc34da) — "완료 — 이동 undefined개 · 격리 undefined개 · 리네임 undefined개"

**(b) confirm 게이트 위반 = 코드로 배제됨.** "완료" emit은 `App.tsx handleExecuted` 한 곳뿐이고,
그 경로는 `PlanReview` "실행" 버튼 → `confirm_plan`(승인 게이트) → `execute_plan` 성공 후로만 도달한다.
자동·에이전트 경로 없음(I1 회귀테스트 `i1_execute_not_in_tool_catalog`가 execute/execute_plan을 툴
카탈로그에서 배제). 사용자가 승인하지 않으면 이 메시지는 물리적으로 못 뜬다. ※ 옛 추정 "승인 UI
미경유"는 코드와 모순 — 첫 op에서 실패하면 파일이 거의 안 옮겨져 "트리 미변경"으로 보였을 뿐, 실행은 시도됐다.

**근본 원인 = 부분실패 분기 미처리.** `execute_plan`은 성공 시 `moved/staged/renamed`를, `PartialExecute`
시 `completed/failed_op/error`를 반환하는데, 프론트가 `partial`을 무시하고 무조건 `onExecuted`를 불러
undefined 카운트 + "완료" 오보를 냈다(즉 부분실패를 성공으로 표시).

**수정:** `types.ts`에 `ExecPlanResponse` 판별 유니온 도입(→ `partial` 분기 강제, undefined 접근 컴파일
차단 = FE 하니스 없는 대신 타입 회귀 가드). `PlanReview`는 partial이면 모달 유지 + caution 배너 +
[되돌리기(`undo_plan`, I2 사용자 트리거)][닫기]. `App`에 `handleUndone`/`handlePartialClose`(메시지 +
트리 재스캔) 추가. 검증: `tsc` OK, 백엔드 미변경(cargo 56 passed).

**남은 한계:** partial 경로의 *런타임* 동작은 미검증(op이 실행 중간 실패해야 트리거 — 수동 재현 난이도 높음).
성공 경로·빈 플랜 육안 확인은 §4에 포함.

### 🟡 B2 — 플랜에 move op가 0개

이전 턴에서 에이전트는 "files (1) 폴더의 개발 파일 → 개발 폴더로 이동"을 언급했으나,
실제 플랜은 격리 5 · 이동 0으로 나왔다. LLM이 이동 건을 빠뜨렸거나 stage로 뭉갠 것으로 보임.
**PlanReview 모달을 열어 5개 op의 실제 from→to를 봐야 판단 가능.**
버그가 아니라 LLM 판단 편차일 수 있음. 우선순위 낮음.

---

## 4. 남은 육안 확인 (p4 클로즈 조건)

`npm run tauri dev`로 확인:

1. **PlanReview 모달 렌더** — "정리 플랜 보기 →" 클릭 시
   - 5개 op가 stage 배지(오커)로 뜨는가
   - 그룹 헤더 카운트가 그룹 내 실제 행 수와 일치하는가
   - 긴 절대경로에서 from→to 모노 행이 레이아웃을 깨지 않는가
2. **빈 플랜** — 이미 정돈된/빈 폴더로 플랜 요청 시
   "제안할 이동이 없습니다" + 실행 버튼 숨김 + 취소가 "닫기"로

※ conflict 배지 실렌더는 현재 확인 불가 — 아래 P-1 참조.

---

## 5. 추적 중인 미결 항목

| ID | 항목 | 메모 |
|---|---|---|
| **P-1** | **충돌 탐지 미구현** | `Conflict::Rename`을 실제로 산출하는 로직이 어디에도 없다. `store.rs`는 DB 역직렬화일 뿐. 즉 목적지에 동명 파일이 있어도 전부 `none`으로 흐른다. C4("덮어쓰기 없음")가 코드로 강제되는지 확인 필요. execute가 `to`에 쓰기 전 동명 검사 → rename 결정하는 지점이 필요하다. **렌더는 이미 대비됨.** → P5 후보 |
| **P-2** | **LLM 호출 에러 핸들링** | 529 overloaded 발생 시 날 JSON을 사용자에게 그대로 노출하고 재시도가 없다. 필요: 제한된 지수 백오프 재시도(2~3회) + 사용자향 메시지 변환 + 재시도 버튼. request_id는 로그에 보존. → P5 또는 별도 슬라이스 |
| **P-3** | **P3–P4 ADR + 워크로그 미작성** | P2 결정은 ADR 0001–0005로 기록됨. P3–P4 ADR과 워크로그는 프롬프트만 있고 실행 안 됨. 위 §2의 결정 8개도 여기 포함시킬 것 |
| **P-4** | **사이드카 번들링** | `resolve_sidecar`의 3-candidate 경로는 dev 전용(소스 트리 기준). 번들된 .app에서 실패한다. → P6 |
| **P-5** | **Python 런타임 provisioning** | 사이드카 vs 네이티브 Rust 파서 — ADR 미작성 |
| **P-6** | **SettingsModal 온보딩** | 5-b 슬라이스 중 스코프 크립으로 들어왔던 것. 사용자 온보딩 용도로 P6 또는 별도 슬라이스 |
| **P-7** | **hwp 파서 확장** | P5 후보 |
| **P-8** | **휴지통 `trash` op 정식 추가** | 현재는 stage로 대체 중. OS 휴지통 vs 앱 staging 결정과 함께 별도 슬라이스. core action 추가 = 안전 표면 확대이므로 신중히 |

---

## 6. 다음 단계 제안

1. ~~**B1 진단**~~ → **해결됨 (commit 1bc34da)**. (b) 게이트 위반 배제 + 부분실패 분기 수정.
2. **남은 육안 확인 2건** → p4 공식 클로즈  ← **현재 최우선**
3. **P-3 ADR + 워크로그 작성** — 결정 근거가 아직 살아있을 때
4. 그다음 데몬+캘린더+마스코트 슬라이스(repo에 아직 plan·commit 없음 — 번호 미정;
   repo의 p5b는 이미 규칙학습이 점유하므로 "Phase 5"로 부르지 말 것) 또는 P-1/P-2 신뢰성 작업
