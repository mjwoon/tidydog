//! 데이터 타입. 코어 전반에서 쓰는 순수 값 타입(외부 의존성 0).
//! DB/JSON 직렬화는 어댑터 책임 — 여기엔 매핑용 문자열 변환만 둔다.

use std::path::PathBuf;

/// 파일에 가할 수 있는 작업. **영구삭제는 의도적으로 존재하지 않는다.**
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// 다른 위치로 이동.
    Move,
    /// 격리(소프트 삭제). 어댑터가 OS 휴지통/staging으로 보낸다. 복원 가능.
    Stage,
    /// 같은 폴더 내 이름 변경(충돌 해소 포함).
    Rename,
}

impl Action {
    /// 데이터 모델의 action 컬럼 값('move'/'stage'/'rename')으로 매핑.
    pub fn as_str(&self) -> &'static str {
        match self {
            Action::Move => "move",
            Action::Stage => "stage",
            Action::Rename => "rename",
        }
    }
}

/// 플랜 수명 상태. plans.status CHECK 와 1:1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanStatus {
    Proposed,
    Confirmed,
    Executed,
    Undone,
    Expired,
}

impl PlanStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            PlanStatus::Proposed => "proposed",
            PlanStatus::Confirmed => "confirmed",
            PlanStatus::Executed => "executed",
            PlanStatus::Undone => "undone",
            PlanStatus::Expired => "expired",
        }
    }
}

/// 목적지 충돌 처리 방식. **덮어쓰기 없음(C4)** — 충돌은 항상 rename.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Conflict {
    None,
    Rename,
}

impl Conflict {
    pub fn as_str(&self) -> &'static str {
        match self {
            Conflict::None => "none",
            Conflict::Rename => "rename",
        }
    }
}

/// 미리보기(승인) 수준 — C3. 위험할수록 더 강한 검토를 요구.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewLevel {
    /// 저위험. 자동 적용 후보(현재 제품에선 여전히 게이트를 거침).
    Auto,
    /// 표준 확인.
    Standard,
    /// 풀 리뷰 — 격리 포함/대량/충돌 등. 사용자가 행 단위로 봐야 함.
    FullReview,
}

/// 플랜의 단일 오퍼레이션. 실행 전엔 "의도"일 뿐 아무것도 바꾸지 않는다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanOp {
    pub op_id: String,
    pub seq: u32,
    pub action: Action,
    pub content_hash: String,
    pub from: PathBuf,
    pub to: PathBuf,
    pub conflict: Conflict,
    pub reason: Option<String>,
}

impl PlanOp {
    /// op_id 없이 의도만 만들 때 사용(Engine::propose가 id·seq를 채운다).
    pub fn new(action: Action, content_hash: impl Into<String>, from: PathBuf, to: PathBuf) -> Self {
        PlanOp {
            op_id: String::new(),
            seq: 0,
            action,
            content_hash: content_hash.into(),
            from,
            to,
            conflict: Conflict::None,
            reason: None,
        }
    }

    pub fn with_reason(mut self, reason: impl Into<String>) -> Self {
        self.reason = Some(reason.into());
        self
    }
}

/// 제안된 오퍼레이션 세트.
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    pub plan_id: String,
    pub created_at: u64,
    pub status: PlanStatus,
    pub risk_score: f64,
    pub preview: PreviewLevel,
    pub scheme: Option<String>,
    pub rule_summary: Option<String>,
    pub ops: Vec<PlanOp>,
}

/// 실제 실행된 작업 기록. append-only(불변식 3). undo·감사용.
/// `completed_at`가 None이면 in-flight(파일 작업 전 기록된 의도).
/// 크래시 복원은 completed_at IS NULL 항목을 FS와 대조해 정리한다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JournalEntry {
    pub entry_id: u64,
    pub plan_id: String,
    pub op_id: String,
    pub action: Action,
    pub content_hash: String,
    pub from: PathBuf,
    pub to: PathBuf,
    pub executed_at: u64,
    /// 파일 작업 완료 확인 시각. None = in-flight (S2 journal-first).
    pub completed_at: Option<u64>,
    pub undoable: bool,
    pub undone_at: Option<u64>,
}

/// 스코프 가드 거부 사유.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Denial {
    /// 화이트리스트(허용 루트) 밖.
    OutsideScope,
    /// 블랙리스트(시스템/앱 데이터 등) 안.
    Blacklisted,
    /// 숨김 경로(dotfile, node_modules 등).
    Hidden,
    /// 존재하지 않거나 해석 불가.
    NotFound,
}
