//! 미리보기 수준 / 위험도 — 안전 명세 §4 (C3).
//! 더 위험한 플랜일수록 더 강한 사용자 검토를 요구한다.

use crate::model::{Action, PlanOp, PreviewLevel};

/// 풀 리뷰로 끌어올리는 op 개수 임계값.
const FULL_REVIEW_COUNT: usize = 10;

/// 오퍼레이션 집합의 미리보기 수준을 계산.
/// - 격리(Stage)가 하나라도 있으면 → 풀 리뷰(소프트 삭제는 항상 행 단위 확인).
/// - 충돌(rename)이 있으면 → 풀 리뷰.
/// - op 개수가 많으면 → 풀 리뷰.
/// - 그 외 소수의 단순 이동 → 표준.
/// - 비었으면 → Auto(보여줄 것 없음).
pub fn preview_level(ops: &[PlanOp]) -> PreviewLevel {
    if ops.is_empty() {
        return PreviewLevel::Auto;
    }
    let has_stage = ops.iter().any(|o| o.action == Action::Stage);
    let has_conflict = ops.iter().any(|o| o.conflict == crate::model::Conflict::Rename);
    if has_stage || has_conflict || ops.len() >= FULL_REVIEW_COUNT {
        PreviewLevel::FullReview
    } else {
        PreviewLevel::Standard
    }
}

/// 0.0~1.0 위험도 점수. 격리 비중이 높을수록 위험.
pub fn risk_score(ops: &[PlanOp]) -> f64 {
    if ops.is_empty() {
        return 0.0;
    }
    let stages = ops.iter().filter(|o| o.action == Action::Stage).count() as f64;
    let conflicts = ops
        .iter()
        .filter(|o| o.conflict == crate::model::Conflict::Rename)
        .count() as f64;
    let n = ops.len() as f64;
    // 격리는 강하게, 충돌은 약하게 가중.
    ((stages * 1.0 + conflicts * 0.4) / n).min(1.0)
}
