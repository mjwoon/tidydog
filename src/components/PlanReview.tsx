import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { PlanOp, ExecPlanResponse } from "../types";

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
  onUndone: () => void;
  onPartialClose: (info: { completed: number; failed_op: string; error: string }) => void;
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
  if (a === "rename") return { background: "var(--caution-soft)", color: "var(--caution)" };
  return { background: "var(--primary-soft)", color: "var(--primary)" }; // move
}

function shortPath(p: string) {
  return p.replace(/^\/Users\/[^/]+/, "~");
}

function basename(p: string) {
  const parts = p.split("/").filter(Boolean);
  return parts[parts.length - 1] ?? p;
}

function dirname(p: string) {
  const idx = p.lastIndexOf("/");
  return idx > 0 ? p.slice(0, idx) : p;
}

// root 접두를 제거해 상대 목적지로 표시. 실패 시 홈 축약으로 폴백.
function relTo(p: string, root: string) {
  if (root && p.startsWith(root)) {
    const rel = p.slice(root.length).replace(/^\/+/, "");
    return rel || basename(p);
  }
  return shortPath(p);
}

interface OpGroup {
  key:     string;   // 그룹 식별자
  dest:    string;   // 헤더에 표시할 목적지 경로
  isStage: boolean;
  ops:     PlanOp[];
}

// 목적지 디렉터리 기준으로 op 를 묶는다. 격리(stage)는 별도 그룹.
function groupOps(ops: PlanOp[], root: string): OpGroup[] {
  const map = new Map<string, OpGroup>();
  for (const op of ops) {
    const isStage = op.action === "stage";
    const destDir = isStage ? "격리" : relTo(dirname(op.to), root);
    const key = isStage ? "__stage__" : destDir;
    let g = map.get(key);
    if (!g) {
      g = { key, dest: destDir, isStage, ops: [] };
      map.set(key, g);
    }
    g.ops.push(op);
  }
  // 이동/리네임 그룹 먼저, 격리 그룹은 마지막.
  return [...map.values()].sort((a, b) => Number(a.isStage) - Number(b.isStage));
}

export function PlanReview({ plan, root, onExecuted, onUndone, onPartialClose, onCancel }: Props) {
  // 부분실패(PartialExecute) 시 모달을 유지하고 배너로 알린 뒤 되돌리기/닫기를 제공.
  const [partialFail, setPartialFail] = useState<{ completed: number; failed_op: string; error: string } | null>(null);
  const [undoing, setUndoing] = useState(false);

  async function handleExecute() {
    try {
      await invoke("confirm_plan", { planId: plan.plan_id });
      const result = await invoke<ExecPlanResponse>("execute_plan", { planId: plan.plan_id, root });
      if (result.partial) {
        // 일부 op만 완료되고 중단됨. moved/staged/renamed 가 없으므로 "완료"로 오보하면 안 된다.
        setPartialFail({ completed: result.completed, failed_op: result.failed_op, error: result.error });
      } else {
        onExecuted({ moved: result.moved, staged: result.staged, renamed: result.renamed });
      }
    } catch (err) {
      console.error("execute_plan failed:", err);
      alert(`실행 실패: ${err}`);
    }
  }

  // I2: undo 는 사용자 트리거 전용. 이 버튼 클릭이 사용자 트리거이며 에이전트 자동 되돌림이 아니다.
  async function handleUndo() {
    setUndoing(true);
    try {
      await invoke("undo_plan", { planId: plan.plan_id });
      onUndone();
    } catch (err) {
      console.error("undo_plan failed:", err);
      alert(`되돌리기 실패: ${err}`);
      setUndoing(false);
    }
  }

  const riskPct = Math.round(plan.risk_score * 100);
  const isHighRisk = plan.risk_score >= 0.5;
  const isEmpty = plan.op_count === 0 || !plan.ops || plan.ops.length === 0;

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

        {/* ops list — 목적지 디렉터리별 그루핑 */}
        <div style={{ flex: 1, overflowY: "auto", padding: "8px 0" }}>
          {plan.ops && plan.ops.length > 0 ? (
            groupOps(plan.ops, root).map((group, gi) => (
              <div key={group.key}>
                {gi > 0 && <div style={{ height: 1, background: "var(--line)", margin: "6px 22px" }} />}
                <div style={{ padding: "8px 22px" }}>
                  {/* group head */}
                  <div style={{
                    display: "flex", alignItems: "center", gap: 8,
                    fontSize: 12.5, fontWeight: 700, padding: "8px 2px",
                    color: group.isStage ? "var(--caution)" : "var(--muted)",
                  }}>
                    <span style={{
                      fontFamily: "var(--mono)", fontSize: 12, fontWeight: 500,
                      color: group.isStage ? "var(--caution)" : "var(--ink)",
                    }}>
                      {group.isStage ? "격리" : `${group.dest}/`}
                    </span>
                    {group.isStage && (
                      <span style={{ fontWeight: 500, opacity: 0.85 }}>
                        · 삭제하지 않고 따로 보관해요
                      </span>
                    )}
                    <span style={{ marginLeft: "auto", fontWeight: 600 }}>{group.ops.length}</span>
                  </div>

                  {/* ops */}
                  {group.ops.map((op) => (
                    <div key={op.op_id || `${op.from}->${op.to}`} style={{
                      display: "flex", alignItems: "flex-start", gap: 12,
                      padding: "11px 12px", borderRadius: "var(--r-sm)",
                    }}>
                      <span style={{ fontSize: 17, lineHeight: 1.4, flex: "none" }}>📄</span>
                      <div style={{ flex: 1, minWidth: 0 }}>
                        <div style={{ fontSize: 14, fontWeight: 600 }}>
                          {basename(op.from)}
                          {op.reason && (
                            <span style={{ fontWeight: 400, color: "var(--muted)", fontSize: 13 }}>
                              {" · "}{op.reason}
                            </span>
                          )}
                        </div>
                        <div style={{
                          marginTop: 5, display: "flex", alignItems: "center", gap: 9,
                          fontFamily: "var(--mono)", fontSize: 12.5,
                        }}>
                          <span style={{ color: "var(--muted)" }}>{relTo(dirname(op.from), root)}</span>
                          {op.action !== "stage" && (
                            <>
                              <span style={{ color: group.isStage ? "var(--caution)" : "var(--primary)", fontWeight: 600 }}>→</span>
                              <span style={{ color: "var(--ink)" }}>{relTo(dirname(op.to), root)}</span>
                            </>
                          )}
                        </div>
                      </div>
                      <div style={{ flex: "none", display: "flex", flexDirection: "column", gap: 4, alignItems: "flex-end", alignSelf: "center" }}>
                        <span style={{
                          ...actionBadgeStyle(op.action),
                          fontSize: 11.5, fontWeight: 700,
                          padding: "4px 9px", borderRadius: 7,
                        }}>
                          {actionLabel(op.action)}
                        </span>
                        {op.conflict !== "none" && (
                          <span
                            title="같은 이름의 파일이 이미 있어 덮어쓰지 않고 이름을 바꿔 옮깁니다"
                            style={{
                              background: "var(--caution-soft)", color: "var(--caution)",
                              fontSize: 11, fontWeight: 700,
                              padding: "3px 8px", borderRadius: 7, whiteSpace: "nowrap",
                            }}
                          >
                            ⚠ 이름충돌
                          </span>
                        )}
                      </div>
                    </div>
                  ))}
                </div>
              </div>
            ))
          ) : (
            <div style={{ padding: "40px 24px", color: "var(--muted)", fontSize: 13.5, textAlign: "center", lineHeight: 1.6 }}>
              {plan.op_count === 0 ? (
                <>
                  <div style={{ fontSize: 15, fontWeight: 600, color: "var(--ink)", marginBottom: 4 }}>
                    제안할 이동이 없습니다
                  </div>
                  <div>지금 정리 위치가 이미 적절해 옮길 파일이 없어요.</div>
                </>
              ) : (
                // 방어적 폴백: op_count>0인데 ops가 안 온 경우(전달 체인 회귀 신호).
                `${plan.op_count}개 작업 준비됨`
              )}
            </div>
          )}
        </div>

        {/* 부분실패 배너 — moved/staged 없이 completed/failed_op/error 만 온다 */}
        {partialFail && (
          <div style={{
            margin: "0 24px 4px", padding: "12px 14px",
            background: "var(--caution-soft)", borderRadius: "var(--r-sm)",
            fontSize: 13, lineHeight: 1.55,
          }}>
            <div style={{ fontWeight: 700, color: "var(--caution)" }}>일부만 처리되고 중단됐어요</div>
            <div style={{ marginTop: 4, color: "var(--ink)" }}>
              {partialFail.completed}개 처리 후{" "}
              <span style={{ fontFamily: "var(--mono)", fontSize: 12 }}>{partialFail.failed_op}</span>
              에서 실패: {partialFail.error}
            </div>
            <div style={{ marginTop: 4, color: "var(--muted)" }}>
              처리된 항목은 되돌리거나 그대로 닫을 수 있어요.
            </div>
          </div>
        )}

        {/* footer */}
        <div style={{
          padding: "16px 24px",
          borderTop: "1px solid var(--line)",
          display: "flex", gap: 10, justifyContent: "flex-end",
        }}>
          {partialFail ? (
            <>
              <button
                onClick={() => onPartialClose(partialFail)}
                style={{
                  padding: "9px 18px", borderRadius: "var(--r-sm)",
                  border: "1px solid var(--line)", background: "var(--surface)",
                  cursor: "pointer", fontSize: 13.5, fontWeight: 600,
                  fontFamily: "var(--ui)", color: "var(--ink)",
                }}
              >
                닫기
              </button>
              <button
                onClick={handleUndo}
                disabled={undoing}
                style={{
                  padding: "9px 20px", borderRadius: "var(--r-sm)",
                  border: "none", background: "var(--primary)", color: "#fff",
                  cursor: undoing ? "default" : "pointer", fontSize: 13.5, fontWeight: 700,
                  fontFamily: "var(--ui)", opacity: undoing ? 0.7 : 1,
                }}
              >
                {undoing ? "되돌리는 중…" : "되돌리기"}
              </button>
            </>
          ) : (
            <>
              <button
                onClick={onCancel}
                style={{
                  padding: "9px 18px", borderRadius: "var(--r-sm)",
                  border: "1px solid var(--line)", background: "var(--surface)",
                  cursor: "pointer", fontSize: 13.5, fontWeight: 600,
                  fontFamily: "var(--ui)", color: "var(--ink)",
                }}
              >
                {isEmpty ? "닫기" : "취소"}
              </button>
              {!isEmpty && (
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
              )}
            </>
          )}
        </div>
      </div>
    </div>
  );
}
