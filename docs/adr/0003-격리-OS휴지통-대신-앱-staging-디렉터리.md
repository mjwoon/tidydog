# ADR-0003 — 격리(소프트 삭제) = OS 휴지통 → 앱 관리 staging 디렉터리

| 항목 | 내용 |
|------|------|
| **상태** | 수용됨 (Phase 2 구현, 2026-06-29) |
| **결정자** | 프로젝트 오너 (macOS 크레이트 제한 확인 후) |

---

## Context

### 원래 의도

`Action::Stage`(소프트 삭제)의 초기 설계는 파일을 **OS 휴지통**으로 보내는 것이었다. `trash` 크레이트(v5)가 이를 지원한다고 알려져 있었다.

### 발견된 제한

`trash` 크레이트 소스 코드를 직접 확인한 결과:

```rust
// trash/src/lib.rs (실제 크레이트 소스)
#[cfg(all(unix, not(target_os = "macos"), not(target_os = "ios")))]
pub mod os_limited {
    pub fn restore_all<I>(...) { ... }
}
```

`restore_all`(프로그램적 복원)이 `#[cfg(not(target_os = "macos"))]` 조건부 컴파일이다. **macOS에서는 이 함수가 존재하지 않는다.** macOS 휴지통은 Finder 수준의 복원만 지원하며, 프로그램에서 "특정 항목을 원래 위치로"를 할 수 있는 공개 API가 없다.

TidyDog의 `undo` 기능이 `Action::Stage`를 역실행하려면 프로그램이 파일을 정확히 원위치로 돌려놓아야 한다. OS 휴지통 경로로는 이 약속을 지킬 수 없다.

---

## Decision

**`Action::Stage`의 목적지를 OS 휴지통에서 앱 관리 staging 디렉터리로 변경한다.**

구현:
- 경로: `app_data_dir/staging/{content_hash}`
- 충돌(동일 hash가 이미 있음): `{content_hash}_{unix_timestamp}` 형태로 저장
- `staging` DB 테이블로 `content_hash → staged_path` 매핑 추적
- `restore_file(content_hash, to)`: exact match → `{hash}_*` prefix 탐색 순으로 복원

영구 퍼지(30일 후 실제 삭제)는 `staging.purge_after` 컬럼을 예약해 두고 P5로 연기한다. 현재 staging 파일은 사용자가 수동으로 `app_data_dir/staging/`을 지우기 전까지 보존된다.

OS 휴지통으로의 전송은 제거된다. `trash` 크레이트 의존성은 Cargo.toml에 남아 있으나 현재 코드에서 호출되지 않는다.

---

## Consequences

**긍정**

- undo의 Stage 역실행이 macOS에서 안정적으로 동작한다.  
  → `src-tauri/tests/fileops_integration.rs::stage_and_restore_roundtrip`으로 검증: 실제 파일 stage → restore 후 내용 일치 확인.
- staging 경로가 앱 통제 하에 있어 복원 경로가 명확하다 (`staging` 테이블로 추적).
- 데이터모델·안전명세 문서의 "삭제=staging" 의미가 명확해진다.

**비용**

- **디스크 공간**: staging 파일이 `app_data_dir/staging/`에 쌓인다. purge 로직이 없는 현재는 수동 정리 필요.
- **purge 책임**: 언제 실제로 삭제할지를 앱이 결정해야 한다 (OS 휴지통은 사용자가 직접 비운다). P5 예정.
- **문서 의미 변경**: 기존에 "휴지통으로 보낸다"고 표현한 모든 곳을 "앱 staging으로 격리"로 수정해야 한다 (`docs/안전명세.md` 반영 완료).

**관련 코드 경로 및 테스트**

- `src-tauri/src/fileops.rs::FsFileOps::stage_file` — staging 디렉터리로 이동
- `src-tauri/src/fileops.rs::FsFileOps::restore_file` — staging에서 원위치 복원
- `src-tauri/src/db.rs` — `staging` 테이블 스키마
- `src-tauri/tests/fileops_integration.rs::stage_and_restore_roundtrip` — 왕복 검증
- `tests/gate.rs::stage_is_soft_delete_and_restorable` — 게이트 레벨 soft delete + undo 검증
