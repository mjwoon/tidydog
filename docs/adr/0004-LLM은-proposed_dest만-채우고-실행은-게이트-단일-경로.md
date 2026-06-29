# ADR-0004 — LLM은 proposed_dest만 채우고 실행은 게이트 단일 경로 유지

| 항목 | 내용 |
|------|------|
| **상태** | 수용됨 (Phase 3 설계, 2026-06-29) |
| **결정자** | 프로젝트 오너 |

---

## Context

Phase 3에서 LLM(Claude API)이 파일 내용을 요약하고 분류 목적지(`proposed_dest`)를 제안하게 된다. 이 시점에 두 가지 설계 방향이 가능했다.

**방향 A — LLM이 실행까지 담당**  
LLM이 "이 파일을 여기로 옮겨라"는 명령을 직접 내리고, 코드가 이를 바로 FS에 적용한다. 자동화가 높아지고 사용자 개입이 줄어든다.

**방향 B — LLM은 제안만, 실행은 기존 게이트**  
LLM 출력은 `files` 테이블의 메타데이터 컬럼(`summary`, `topic_tags`, `language`, `proposed_dest`)을 채우는 데 그친다. 파일을 실제로 움직이는 경로는 여전히 `Engine::execute` 하나뿐이다.

### 왜 방향 A가 위험한가

LLM은 확률적 시스템이다.

1. **출력 불확실성**: 같은 파일을 두 번 보내면 다른 `proposed_dest`가 나올 수 있다.
2. **환각 위험**: LLM이 존재하지 않는 경로나 시스템 디렉터리를 목적지로 제안할 수 있다.
3. **게이트 우회**: LLM → FS 직결 경로가 생기면, ADR-0001에서 구조적으로 보장했던 "파일을 바꾸는 유일한 경로는 Engine::execute"가 무력화된다.
4. **스코프 가드 미적용**: Engine 밖에서 파일을 이동하면 `ScopeGuard` 재검증(TOCTOU), 충돌 리네임(C4), journal-first(S2)가 모두 우회된다.

Phase 3 설계 원칙에 명시적으로 기록됨: "LLM·요약·파서·네트워크 추가 금지(P2 코어)" — P3 기능은 별도 레이어로 추가되나 게이트 우회 금지.

---

## Decision

**LLM 출력의 영향 범위를 `files` 테이블의 읽기 전용 메타데이터 컬럼으로 제한한다.**

LLM이 할 수 있는 것:
- `files.summary` 채우기
- `files.topic_tags` 채우기 (JSON 배열)
- `files.language` 채우기
- `files.proposed_dest` 채우기
- `files.read_level='content'`로 업그레이드

LLM이 할 수 없는 것:
- `Engine::execute` 직접 호출
- `FsFileOps::move_file` 직접 호출
- `FsFileOps::stage_file` 직접 호출
- `plans`, `journal` 테이블 직접 수정

실제 파일 이동 흐름은 동일하다:  
`propose_plan(ops[to=proposed_dest])` → `confirm_plan` → `execute_plan` → `Engine::execute`

`index_file_content` 커맨드와 `derive_proposed_dest` 커맨드는 `files` 테이블만 수정한다.

---

## Consequences

**긍정**

- 확률적 LLM이 비가역 FS 작업 권한을 갖지 않는다. LLM 출력이 잘못돼도 파일은 움직이지 않는다.
- ADR-0001의 안전 구조적 보장이 P3 이후에도 유지된다.
- `ScopeGuard`, journal-first, 충돌 리네임이 LLM 제안 경로에도 그대로 적용된다.
- P4에서 "execute를 LLM 도구로 노출"을 검토할 때 이 ADR이 명시적 게이트 역할을 한다.

**비용**

- 완전 자동화까지 사용자 확인 단계(confirm)가 필수로 남는다. 배치 자동화를 원하는 사용자에게는 마찰이 된다.
- `proposed_dest`와 실제 `to` 경로 사이의 매핑(프론트엔드 책임)이 명시적으로 필요하다.

**관련 코드 경로**

- `src-tauri/src/summarizer.rs` — `CloudSummarizer::summarize()`: `files` 테이블 수정, Engine 호출 없음
- `src-tauri/src/organizer.rs` — `derive_dest()`: 경로 문자열 반환만, FS 조작 없음
- `src-tauri/src/lib.rs::index_file_content` — `files` UPDATE만
- `src-tauri/src/lib.rs::derive_proposed_dest` — `files.proposed_dest` UPDATE만
- `crates/tidydog-core/src/gate.rs` — `Engine::execute` (유일한 실행 경로, 변경 없음)
