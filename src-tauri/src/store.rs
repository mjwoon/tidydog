/// SqliteStore — tidydog_core::Store 트레이트를 rusqlite로 구현.
use rusqlite::{Connection, params};
use std::path::PathBuf;
use tidydog_core::{
    Action, Conflict, JournalEntry, Plan, PlanOp, PlanStatus, PreviewLevel, Store,
};

pub struct SqliteStore<'a> {
    pub conn: &'a Connection,
}

impl<'a> SqliteStore<'a> {
    pub fn new(conn: &'a Connection) -> Self {
        SqliteStore { conn }
    }

    /// plan_ops 테이블에서 해당 plan의 ops를 seq 순으로 읽는다(T1 정규화).
    /// 테이블이 없거나 행이 없으면 None → 호출자가 ops_json으로 fallback.
    fn ops_for_plan(&self, plan_id: &str) -> Option<Vec<PlanOp>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT op_id, seq, action, content_hash, from_path, to_path, conflict, reason
                 FROM plan_ops WHERE plan_id = ?1 ORDER BY seq",
            )
            .ok()?;
        let ops: Vec<PlanOp> = stmt
            .query_map(params![plan_id], |row| {
                Ok(PlanOp {
                    op_id: row.get(0)?,
                    seq: row.get::<_, i64>(1)? as u32,
                    action: action_from_str(&row.get::<_, String>(2)?),
                    content_hash: row.get(3)?,
                    from: PathBuf::from(row.get::<_, String>(4)?),
                    to: PathBuf::from(row.get::<_, String>(5)?),
                    conflict: conflict_from_str(&row.get::<_, String>(6)?),
                    reason: row.get(7)?,
                })
            })
            .ok()?
            .filter_map(|r| r.ok())
            .collect();
        if ops.is_empty() { None } else { Some(ops) }
    }
}

fn action_from_str(s: &str) -> Action {
    match s {
        "stage" => Action::Stage,
        "rename" => Action::Rename,
        _ => Action::Move,
    }
}

fn preview_from_str(s: &str) -> PreviewLevel {
    match s {
        "standard" => PreviewLevel::Standard,
        "full_review" => PreviewLevel::FullReview,
        _ => PreviewLevel::Auto,
    }
}

fn status_from_str(s: &str) -> PlanStatus {
    match s {
        "confirmed" => PlanStatus::Confirmed,
        "executed" => PlanStatus::Executed,
        "undone" => PlanStatus::Undone,
        "expired" => PlanStatus::Expired,
        _ => PlanStatus::Proposed,
    }
}

fn conflict_from_str(s: &str) -> Conflict {
    if s == "rename" {
        Conflict::Rename
    } else {
        Conflict::None
    }
}

impl<'a> Store for SqliteStore<'a> {
    fn save_plan(&mut self, plan: Plan) {
        let ops_json = serde_ops(&plan.ops);
        let _ = self.conn.execute(
            "INSERT OR REPLACE INTO plans
             (plan_id, created_at, status, risk_score, preview, scheme, rule_summary, ops_json)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
            params![
                plan.plan_id,
                plan.created_at as i64,
                plan.status.as_str(),
                plan.risk_score,
                preview_as_str(&plan.preview),
                plan.scheme,
                plan.rule_summary,
                ops_json,
            ],
        );
        // T1 정규화: plan_ops 테이블에 개별 op을 기록 (감사 무결성).
        // ops_json은 기존 플랜 호환용으로 유지.
        for op in &plan.ops {
            let _ = self.conn.execute(
                "INSERT OR IGNORE INTO plan_ops
                 (op_id, plan_id, seq, action, content_hash, from_path, to_path, conflict, reason)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                params![
                    op.op_id,
                    plan.plan_id,
                    op.seq as i64,
                    op.action.as_str(),
                    op.content_hash,
                    op.from.to_string_lossy().as_ref(),
                    op.to.to_string_lossy().as_ref(),
                    op.conflict.as_str(),
                    op.reason,
                ],
            );
        }
    }

    fn get_plan(&self, plan_id: &str) -> Option<Plan> {
        let (pid, created_at, status, risk_score, preview, scheme, rule_summary, ops_json) = self
            .conn
            .query_row(
                "SELECT plan_id, created_at, status, risk_score, preview, scheme, rule_summary, ops_json
                 FROM plans WHERE plan_id = ?1",
                params![plan_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, f64>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, Option<String>>(5)?,
                        row.get::<_, Option<String>>(6)?,
                        row.get::<_, String>(7)?,
                    ))
                },
            )
            .ok()?;
        // T1: plan_ops 우선 읽기, 없으면 ops_json fallback (기존 플랜 호환).
        let ops = self
            .ops_for_plan(plan_id)
            .unwrap_or_else(|| deserialize_ops(&ops_json));
        Some(Plan {
            plan_id: pid,
            created_at: created_at as u64,
            status: status_from_str(&status),
            risk_score,
            preview: preview_from_str(&preview),
            scheme,
            rule_summary,
            ops,
        })
    }

    fn set_plan_status(&mut self, plan_id: &str, status: PlanStatus) {
        let _ = self.conn.execute(
            "UPDATE plans SET status = ?1 WHERE plan_id = ?2",
            params![status.as_str(), plan_id],
        );
    }

    fn append_journal(&mut self, entry: JournalEntry) -> u64 {
        let _ = self.conn.execute(
            "INSERT INTO journal
             (plan_id, op_id, action, content_hash, from_path, to_path,
              executed_at, completed_at, undoable, undone_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
            params![
                entry.plan_id,
                entry.op_id,
                entry.action.as_str(),
                entry.content_hash,
                entry.from.to_string_lossy().as_ref(),
                entry.to.to_string_lossy().as_ref(),
                entry.executed_at as i64,
                entry.completed_at.map(|t| t as i64),
                entry.undoable as i64,
                entry.undone_at.map(|t| t as i64),
            ],
        );
        self.conn.last_insert_rowid() as u64
    }

    fn journal_for_plan(&self, plan_id: &str) -> Vec<JournalEntry> {
        let mut stmt = match self.conn.prepare(
            "SELECT entry_id, plan_id, op_id, action, content_hash, from_path, to_path,
                    executed_at, completed_at, undoable, undone_at
             FROM journal WHERE plan_id = ?1 ORDER BY entry_id",
        ) {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        stmt.query_map(params![plan_id], |row| {
            Ok(JournalEntry {
                entry_id: row.get::<_, i64>(0)? as u64,
                plan_id: row.get(1)?,
                op_id: row.get(2)?,
                action: action_from_str(&row.get::<_, String>(3)?),
                content_hash: row.get(4)?,
                from: PathBuf::from(row.get::<_, String>(5)?),
                to: PathBuf::from(row.get::<_, String>(6)?),
                executed_at: row.get::<_, i64>(7)? as u64,
                completed_at: row.get::<_, Option<i64>>(8)?.map(|t| t as u64),
                undoable: row.get::<_, i64>(9)? != 0,
                undone_at: row.get::<_, Option<i64>>(10)?.map(|t| t as u64),
            })
        })
        .ok()
        .map(|rows| rows.filter_map(|r| r.ok()).collect())
        .unwrap_or_default()
    }

    fn mark_undone(&mut self, entry_id: u64, ts: u64) {
        let _ = self.conn.execute(
            "UPDATE journal SET undone_at = ?1 WHERE entry_id = ?2",
            params![ts as i64, entry_id as i64],
        );
    }

    fn recent_executed_plan_ids(&self, limit: usize) -> Vec<String> {
        let mut stmt = match self.conn.prepare(
            "SELECT plan_id FROM plans WHERE status = 'executed'
             ORDER BY rowid DESC LIMIT ?1",
        ) {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        stmt.query_map(params![limit as i64], |row| row.get(0))
            .ok()
            .map(|rows| rows.filter_map(|r| r.ok()).collect())
            .unwrap_or_default()
    }

    fn complete_journal(&mut self, entry_id: u64, ts: u64) {
        let _ = self.conn.execute(
            "UPDATE journal SET completed_at = ?1 WHERE entry_id = ?2",
            params![ts as i64, entry_id as i64],
        );
    }

    fn update_journal_to(&mut self, entry_id: u64, to: std::path::PathBuf) {
        let _ = self.conn.execute(
            "UPDATE journal SET to_path = ?1 WHERE entry_id = ?2",
            params![to.to_string_lossy().as_ref(), entry_id as i64],
        );
    }

    fn inflight_entries(&self) -> Vec<JournalEntry> {
        let mut stmt = match self.conn.prepare(
            "SELECT entry_id, plan_id, op_id, action, content_hash, from_path, to_path,
                    executed_at, completed_at, undoable, undone_at
             FROM journal WHERE completed_at IS NULL ORDER BY entry_id",
        ) {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        stmt.query_map([], |row| {
            Ok(JournalEntry {
                entry_id: row.get::<_, i64>(0)? as u64,
                plan_id: row.get(1)?,
                op_id: row.get(2)?,
                action: action_from_str(&row.get::<_, String>(3)?),
                content_hash: row.get(4)?,
                from: PathBuf::from(row.get::<_, String>(5)?),
                to: PathBuf::from(row.get::<_, String>(6)?),
                executed_at: row.get::<_, i64>(7)? as u64,
                completed_at: row.get::<_, Option<i64>>(8)?.map(|t| t as u64),
                undoable: row.get::<_, i64>(9)? != 0,
                undone_at: row.get::<_, Option<i64>>(10)?.map(|t| t as u64),
            })
        })
        .ok()
        .map(|rows| rows.filter_map(|r| r.ok()).collect())
        .unwrap_or_default()
    }
}

fn preview_as_str(p: &PreviewLevel) -> &'static str {
    match p {
        PreviewLevel::Auto => "auto",
        PreviewLevel::Standard => "standard",
        PreviewLevel::FullReview => "full_review",
    }
}

/// PlanOp 벡터를 간단한 JSON 배열로 직렬화 (외부 의존성 없이 serde_json 사용 가능하나,
/// src-tauri는 이미 serde_json을 가지므로 여기선 수동 포맷).
fn serde_ops(ops: &[PlanOp]) -> String {
    let entries: Vec<String> = ops
        .iter()
        .map(|op| {
            format!(
                r#"{{"op_id":{},"seq":{},"action":{},"content_hash":{},"from":{},"to":{},"conflict":{},"reason":{}}}"#,
                json_str(&op.op_id),
                op.seq,
                json_str(op.action.as_str()),
                json_str(&op.content_hash),
                json_str(&op.from.to_string_lossy()),
                json_str(&op.to.to_string_lossy()),
                json_str(op.conflict.as_str()),
                op.reason.as_deref().map(json_str).unwrap_or_else(|| "null".to_string()),
            )
        })
        .collect();
    format!("[{}]", entries.join(","))
}

fn deserialize_ops(json: &str) -> Vec<PlanOp> {
    // 경량 파서: serde_json 크레이트는 이미 포함되어 있으므로 그대로 활용.
    // JSON 형식은 serde_ops가 보장하므로 파싱 실패 시 빈 벡터 반환.
    let trimmed = json.trim();
    if trimmed == "[]" || trimmed.is_empty() {
        return vec![];
    }
    // serde_json::Value 대신 수동 파싱은 복잡하므로, serde_json 사용 (src-tauri 의존성).
    match serde_json::from_str::<serde_json::Value>(json) {
        Ok(serde_json::Value::Array(arr)) => arr
            .iter()
            .filter_map(|v| {
                let op_id = v["op_id"].as_str()?.to_string();
                let seq = v["seq"].as_u64()? as u32;
                let action = action_from_str(v["action"].as_str()?);
                let content_hash = v["content_hash"].as_str()?.to_string();
                let from = PathBuf::from(v["from"].as_str()?);
                let to = PathBuf::from(v["to"].as_str()?);
                let conflict = conflict_from_str(v["conflict"].as_str().unwrap_or("none"));
                let reason = v["reason"].as_str().map(|s| s.to_string());
                Some(PlanOp { op_id, seq, action, content_hash, from, to, conflict, reason })
            })
            .collect(),
        _ => vec![],
    }
}

fn json_str(s: &str) -> String {
    let escaped = s.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{}\"", escaped)
}
