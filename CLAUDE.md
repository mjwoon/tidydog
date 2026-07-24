# TidyDog

크로스플랫폼 AI 파일정리 데스크톱 에이전트.
Tauri 2 + Rust + React/TS/Tailwind v4 + Python 사이드카 + SQLite.
환경: macOS, npm. 대화 언어: 한국어.

현재 세션 상태·미해결 버그·다음 작업은 `docs/HANDOFF.md`를 읽어라.

## 안전 불변식 (절대 위반 금지)

이 앱은 사용자의 실제 파일을 움직인다. 아래는 협상 대상이 아니다.

- **I1 — execute는 에이전트 툴 카탈로그에 없다.** LLM이 파일 실행을 스스로 호출할 수 없다.
  회귀 테스트 `i1_execute_not_in_tool_catalog`가 이를 단언한다. 이 테스트를 삭제·완화하지 마라.
- **I2 — undo는 사용자 트리거(`/undo`) 전용.** 에이전트가 자동으로 되돌리지 않는다. sentinel 테스트로 증명됨.
- **게이트 — propose → confirm → execute.** 사용자 승인 없이 파일이 움직이는 경로는 존재하면 안 된다.
- **영구 삭제 없음.** 삭제/제거/중복정리 의도는 전부 `stage`(격리)로 표현한다.
  staging 디렉터리로 보내고 복원 가능하게 한다. `delete`/`trash` op를 만들지 마라.
- **C4 — 덮어쓰기 없음.** 목적지 충돌 시 rename으로 처리한다.
- **R1–R3 — 규칙 변경은 파일 이동과 동등한 게이트.** `apply_rule_change`도 툴 카탈로그에 없다.
- **N1 — 콘텐츠를 클라우드 LLM에 보내기 전 동의 게이트.** 게이트는 콘텐츠 로드 *이전*에 위치한다.
- **dispatch_tool()이 모든 LLM 호출 경로의 단일 게이트 지점이다.** 우회 경로를 만들지 마라.

## op 스키마 (core 계약 — 임의 변경 금지)

```
op = { op_id, seq, action, content_hash, from, to, conflict, reason }
```

- `action` = `move` | `stage` | `rename` **3종뿐.** 새 action을 추가하지 마라.
  - `move` = 다른 폴더로 이동
  - `stage` = 격리(소프트 삭제) — 삭제·제거·중복본 처리는 전부 이것
  - `rename` = 이름 변경
- `conflict` = `none` | `rename` 2값.
- `display_name`은 데이터 필드가 아니다. FE에서 `basename(from)`으로 파생한다.
- 이 이름들(`action`/`from`/`to`)은 core PlanOp 모델이며 execute·confirm·journal이 전부 공유한다.
  프론트 편의를 위해 리네임하지 마라. 반대로 프론트를 여기에 맞춰라.

## 툴 역할 분리

- **현황 조회·목록 표시** = `list_files` (가벼움, content_hash 없음)
- **플랜 수립**(특히 중복 판정) = `scan_directory`로 content_hash 확보 후 `propose_plan`
- `list_files`에 content_hash를 추가하지 마라. 역할 분리를 유지한다.

## 디자인 토큰

- `--bg #E9EDEB` / `--primary #2C6E63` / `--accent #E3A03A` / `--caution #B07A24`
- **빨강 계열 금지.** 경고·주의는 오커(caution)로.
- 폰트: Pretendard(UI) + IBM Plex Mono(경로·크기·코드). 새 폰트 도입 금지.
- 단일 소스: `design/TidyDog_디자인.md`, `design/mockup_main_shell.html`, `design/mockup_plan_review.html`

## LLM 출력 렌더 안전

챗 버블은 LLM 출력을 렌더한다 = 신뢰할 수 없는 입력이다.

- raw HTML 렌더 금지. `rehype-raw`·`dangerouslySetInnerHTML` 추가 금지.
- 링크는 http/https만 허용. `javascript:`·`data:` 차단.
- 마크다운 파싱은 assistant 메시지에만. 사용자 메시지는 평문.

## 작업 방식

- **설계 결정 먼저, 코드는 그다음.** 안전 계약이 걸린 변경은 사용자와 합의 후 착수.
- 슬라이스 구조: Context / DoD / Safety Properties / Task Order / Constraints / Verification.
- **해피패스 통과 = 완료가 아니다.** 각 슬라이스 후 non-happy-path 갭을 스스로 점검:
  빈 입력, 누락 필드, 조기 리턴, 직렬화 경계 손실.
- **"없음"을 증명하는 테스트는 sentinel 패턴으로.** 새 회귀 테스트를 만들면
  일부러 해당 코드를 깨서 테스트가 실패하는지 확인한 뒤 원복하라. 통과만 확인하면 헛 테스트가 된다.
- 추측하지 마라. 코드를 실제로 열어보고 답하라. 막히면 우회하지 말고 정확한 지점을 보고하라.
- 스코프 크립 주의. 슬라이스에 없던 컴포넌트·기능을 끼워넣지 마라. 커밋을 분리하라.

## 검증 기준

변경 후 항상: `cargo test` / `tsc` 통과 + I1 테스트 통과 + 변경 파일 목록 보고.
