//! 미리보기 수준(C3) 통합 테스트.

use std::path::PathBuf;
use tidydog_core::{preview_level, Action, Conflict, PlanOp, PreviewLevel};

fn p(s: &str) -> PathBuf {
    PathBuf::from(s)
}

fn mv(i: usize) -> PlanOp {
    PlanOp::new(
        Action::Move,
        "h",
        p(&format!("/scope/a{i}.txt")),
        p(&format!("/scope/docs/a{i}.txt")),
    )
}

#[test]
fn empty_is_auto() {
    assert_eq!(preview_level(&[]), PreviewLevel::Auto);
}

#[test]
fn few_moves_standard() {
    let ops: Vec<PlanOp> = (0..3).map(mv).collect();
    assert_eq!(preview_level(&ops), PreviewLevel::Standard);
}

#[test]
fn stage_forces_full_review() {
    let ops = vec![
        mv(0),
        PlanOp::new(Action::Stage, "h", p("/scope/old.dmg"), p("/scope/old.dmg")),
    ];
    assert_eq!(preview_level(&ops), PreviewLevel::FullReview);
}

#[test]
fn many_ops_full_review() {
    let ops: Vec<PlanOp> = (0..10).map(mv).collect();
    assert_eq!(preview_level(&ops), PreviewLevel::FullReview);
}

#[test]
fn conflict_forces_full_review() {
    let mut op = mv(0);
    op.conflict = Conflict::Rename;
    assert_eq!(preview_level(&[op]), PreviewLevel::FullReview);
}
