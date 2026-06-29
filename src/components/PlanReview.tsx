import { invoke } from "@tauri-apps/api/core";

export interface PlanOp {
  op_id: string;
  seq: number;
  action: "move" | "stage" | "rename";
  content_hash: string;
  from: string;
  to: string;
  conflict: "none" | "rename";
  reason?: string;
}

export interface PlanSummary {
  plan_id: string;
  risk_score: number;
  preview: "auto" | "standard" | "full_review";
  op_count: number;
  ops?: PlanOp[];
}

interface Props {
  plan: PlanSummary;
  root: string;
  onExecuted: (result: { moved: number; staged: number; renamed: number }) => void;
  onCancel: () => void;
}

function actionLabel(a: string) {
  if (a === "move") return "이동";
  if (a === "stage") return "격리";
  if (a === "rename") return "이름 변경";
  return a;
}

function actionBadgeStyle(a: string): React.CSSProperties {
  if (a === "stage") return { background: "var(--caution-soft)", color: "var(--caution)" };
  if (a === "rename") return { background: "var(--primary-soft)", color: "var(--primary)" };
  return { background: "var(--surface-2)", color: "var(--muted)" };
}

function shortPath(p: string) {
  return p.replace(/^\/Users\/[^/]+/, "~");
}

export function PlanReview({ plan, root, onExecuted, onCancel }: Props) {
  async function handleExecute() {
    try {
      await invoke("confirm_plan", { planId: plan.plan_id });
      const result = await invoke<{ plan_id: string; moved: number; staged: number; renamed: number }>(
        "execute_plan",
        { planId: plan.plan_id, root }
      );
      onExecuted({ moved: result.moved, staged: result.staged, renamed: result.renamed });
    } catch (err) {
      console.error("execute_plan failed:", err);
      alert(`실행 실패: ${err}`);
    }
  }

  const riskPct = Math.round(plan.risk_score * 100);
  const isHighRisk = plan.risk_score >= 0.5;

  return (
    <div style={{
      position: "fixed", inset: 0, zIndex: 100,
      background: "rgba(32,39,42,0.45)",
      display: "flex", alignItems: "center", justifyContent: "center",
    }}>
      <div style={{
        background: "var(--surface)",
        borderRadius: "var(--r)",
        boxShadow: "0 4px 32px rgba(20,40,35,.18)",
        width: 560, maxWidth: "90vw",
        maxHeight: "80vh",
        display: "flex", flexDirection: "column",
        overflow: "hidden",
      }}>
        {/* header */}
        <div style={{
          padding: "20px 24px 16px",
          borderBottom: "1px solid var(--line)",
          display: "flex", alignItems: "flex-start", gap: 12,
        }}>
          <div style={{ flex: 1 }}>
            <div style={{ fontWeight: 700, fontSize: 16 }}>플랜 검토</div>
            <div style={{ marginTop: 4, fontSize: 13, color: "var(--muted)" }}>
              {plan.op_count}개 작업 ·{" "}
              <span style={{ color: isHighRisk ? "var(--caution)" : "var(--muted)" }}>
                위험도 {riskPct}%
              </span>
            </div>
          </div>
          <button
            onClick={onCancel}
            style={{
              background: "none", border: "none", cursor: "pointer",
              fontSize: 18, color: "var(--muted)", lineHeight: 1, padding: 4,
            }}
          >×</button>
        </div>

        {/* ops list */}
        <div style={{ flex: 1, overflowY: "auto", padding: "12px 16px" }}>
          {plan.ops && plan.ops.length > 0 ? (
            plan.ops.map((op) => (
              <div key={op.op_id} style={{
                padding: "10px 12px",
                borderRadius: "var(--r-sm)",
                marginBottom: 6,
                background: "var(--surface-2)",
                fontSize: 13,
              }}>
                <div style={{ display: "flex", alignItems: "center", gap: 8, marginBottom: 6 }}>
                  <span style={{
                    ...actionBadgeStyle(op.action),
                    padding: "2px 8px", borderRadius: 999,
                    fontSize: 11.5, fontWeight: 600,
                  }}>
                    {actionLabel(op.action)}
                  </span>
                  {op.conflict === "rename" && (
                    <span style={{
                      background: "var(--caution-soft)", color: "var(--caution)",
                      padding: "2px 8px", borderRadius: 999, fontSize: 11.5, fontWeight: 600,
                    }}>
                      충돌 → 리네임
                    </span>
                  )}
                  {op.reason && (
                    <span style={{ color: "var(--muted)", fontSize: 12 }}>{op.reason}</span>
                  )}
                </div>
                <div style={{ fontFamily: "var(--mono)", fontSize: 12, color: "var(--ink)" }}>
                  {shortPath(op.from)}
                </div>
                {op.action !== "stage" && (
                  <div style={{ fontFamily: "var(--mono)", fontSize: 12, color: "var(--primary)", marginTop: 2 }}>
                    → {shortPath(op.to)}
                  </div>
                )}
              </div>
            ))
          ) : (
            <div style={{ padding: "16px", color: "var(--muted)", fontSize: 13, textAlign: "center" }}>
              {plan.op_count}개 작업 준비됨
            </div>
          )}
        </div>

        {/* footer */}
        <div style={{
          padding: "16px 24px",
          borderTop: "1px solid var(--line)",
          display: "flex", gap: 10, justifyContent: "flex-end",
        }}>
          <button
            onClick={onCancel}
            style={{
              padding: "9px 18px", borderRadius: "var(--r-sm)",
              border: "1px solid var(--line)", background: "var(--surface)",
              cursor: "pointer", fontSize: 13.5, fontWeight: 600,
              fontFamily: "var(--ui)", color: "var(--ink)",
            }}
          >
            취소
          </button>
          <button
            onClick={handleExecute}
            style={{
              padding: "9px 20px", borderRadius: "var(--r-sm)",
              border: "none",
              background: isHighRisk ? "var(--caution)" : "var(--primary)",
              color: "#fff",
              cursor: "pointer", fontSize: 13.5, fontWeight: 700,
              fontFamily: "var(--ui)",
            }}
          >
            {isHighRisk ? "위험 확인 후 실행" : "실행"}
          </button>
        </div>
      </div>
    </div>
  );
}
