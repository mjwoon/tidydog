import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { RuleChange, SummaryCard } from "../types";

interface Props {
  ruleChange: RuleChange;
  root:       string;
  onApplied:  () => void;
  onCancel:   () => void;
}

function kindBadgeStyle(kind: SummaryCard["kind"]): React.CSSProperties {
  if (kind === "add")    return { background: "var(--primary-soft)", color: "var(--primary)" };
  if (kind === "delete") return { background: "var(--caution-soft)", color: "var(--caution)" };
  return { background: "var(--surface-2)", color: "var(--muted)" };
}

function kindLabel(kind: SummaryCard["kind"]) {
  if (kind === "add")    return "추가";
  if (kind === "delete") return "삭제";
  return "수정";
}

export function RuleReview({ ruleChange, root, onApplied, onCancel }: Props) {
  const [showDiff, setShowDiff] = useState(false);
  const [applying, setApplying] = useState(false);
  const [error, setError]       = useState<string | null>(null);

  async function handleApply() {
    setApplying(true);
    setError(null);
    try {
      await invoke("apply_rule_change", {
        ruleChangeId: ruleChange.rule_change_id,
        root,
      });
      onApplied();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setApplying(false);
    }
  }

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
        width: 580, maxWidth: "92vw",
        maxHeight: "82vh",
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
            <div style={{ fontWeight: 700, fontSize: 16 }}>ORGANIZER 규칙 변경 제안</div>
            <div style={{ marginTop: 4, fontSize: 13, color: "var(--muted)" }}>
              검토 후 승인하면 ORGANIZER.md에 반영됩니다.
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

        {/* summary cards */}
        <div style={{ flex: 1, overflowY: "auto", padding: "12px 16px" }}>
          {ruleChange.summary_cards.length > 0 ? (
            ruleChange.summary_cards.map((card, i) => (
              <div key={i} style={{
                padding: "10px 12px",
                borderRadius: "var(--r-sm)",
                marginBottom: 8,
                background: "var(--surface-2)",
                fontSize: 13,
              }}>
                <div style={{ display: "flex", alignItems: "center", gap: 8, marginBottom: 6 }}>
                  <span style={{
                    ...kindBadgeStyle(card.kind),
                    padding: "2px 8px", borderRadius: 999,
                    fontSize: 11.5, fontWeight: 600,
                  }}>
                    {kindLabel(card.kind)}
                  </span>
                  <span style={{ color: "var(--ink)" }}>{card.description}</span>
                </div>
                {card.rule_line && (
                  <div style={{
                    fontFamily: "var(--mono)", fontSize: 12,
                    color: "var(--primary)", marginTop: 2,
                    padding: "3px 6px", background: "var(--primary-soft)",
                    borderRadius: "var(--r-sm)",
                  }}>
                    {card.rule_line}
                  </div>
                )}
              </div>
            ))
          ) : (
            <div style={{ padding: 16, color: "var(--muted)", fontSize: 13, textAlign: "center" }}>
              변경 내용이 없습니다.
            </div>
          )}

          {/* diff 토글 */}
          <div style={{ marginTop: 8 }}>
            <button
              onClick={() => setShowDiff((v) => !v)}
              style={{
                background: "none", border: "none", cursor: "pointer",
                color: "var(--muted)", fontSize: 12.5, padding: "4px 0",
                fontFamily: "var(--ui)",
              }}
            >
              {showDiff ? "▲ diff 숨기기" : "▼ diff 보기"}
            </button>
            {showDiff && (
              <pre style={{
                marginTop: 8, padding: "10px 12px",
                background: "var(--surface-2)",
                borderRadius: "var(--r-sm)",
                fontSize: 12, fontFamily: "var(--mono)",
                overflowX: "auto", whiteSpace: "pre-wrap",
                color: "var(--ink)", lineHeight: 1.6,
              }}>
                {ruleChange.diff || "(diff 없음)"}
              </pre>
            )}
          </div>

          {error && (
            <div style={{
              marginTop: 10, padding: "8px 12px",
              background: "var(--caution-soft)",
              borderRadius: "var(--r-sm)",
              color: "var(--caution)", fontSize: 13,
            }}>
              {error}
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
            disabled={applying}
            style={{
              padding: "9px 18px", borderRadius: "var(--r-sm)",
              border: "1px solid var(--line)", background: "var(--surface)",
              cursor: applying ? "not-allowed" : "pointer",
              fontSize: 13.5, fontWeight: 600,
              fontFamily: "var(--ui)", color: "var(--ink)",
            }}
          >
            취소
          </button>
          <button
            onClick={handleApply}
            disabled={applying}
            style={{
              padding: "9px 20px", borderRadius: "var(--r-sm)",
              border: "none",
              background: "var(--primary)",
              color: "#fff",
              cursor: applying ? "not-allowed" : "pointer",
              opacity: applying ? 0.7 : 1,
              fontSize: 13.5, fontWeight: 700,
              fontFamily: "var(--ui)",
            }}
          >
            {applying ? "적용 중…" : "규칙 적용"}
          </button>
        </div>
      </div>
    </div>
  );
}
