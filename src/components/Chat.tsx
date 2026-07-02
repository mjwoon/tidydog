import { useRef, useEffect } from "react";
import { ChatMessage, ProposedPlan } from "../types";

interface Props {
  messages:     ChatMessage[];
  loading:      boolean;
  onPlanOpen:   (plan: ProposedPlan) => void;
  folderPath:   string | null;
}

function shortPath(p: string) {
  return p.replace(/^\/Users\/[^/]+/, "~");
}

function PlanChip({ plan, onOpen }: { plan: ProposedPlan; onOpen: () => void }) {
  return (
    <div style={{
      marginTop: 8,
      background: "var(--primary-soft)",
      border: "1px solid var(--primary)",
      borderRadius: "var(--r-sm)",
      padding: "10px 14px",
      display: "flex",
      alignItems: "center",
      gap: 12,
      fontSize: 13,
    }}>
      <div style={{ flex: 1 }}>
        <span style={{ fontWeight: 600, color: "var(--primary)" }}>정리 플랜 준비됨 </span>
        <span style={{ color: "var(--muted)" }}>
          이동 {plan.move_count}개 · 격리 {plan.stage_count}개
        </span>
      </div>
      <button
        onClick={onOpen}
        style={{
          padding: "5px 14px",
          borderRadius: "var(--r-sm)",
          border: "none",
          background: "var(--primary)",
          color: "#fff",
          cursor: "pointer",
          fontSize: 12.5,
          fontWeight: 700,
          fontFamily: "var(--ui)",
          whiteSpace: "nowrap",
        }}
      >
        정리 플랜 보기 →
      </button>
    </div>
  );
}

function Bubble({ msg, onPlanOpen }: { msg: ChatMessage; onPlanOpen: (p: ProposedPlan) => void }) {
  const isUser = msg.role === "user";
  return (
    <div style={{
      display: "flex",
      flexDirection: "column",
      alignItems: isUser ? "flex-end" : "flex-start",
      marginBottom: 10,
    }}>
      <div
        className={`bubble ${isUser ? "bubble-user" : "bubble-agent"}`}
        style={{
          maxWidth: "80%",
          padding: "9px 13px",
          borderRadius: isUser ? "14px 14px 4px 14px" : "14px 14px 14px 4px",
          background: isUser
            ? "var(--primary)"
            : msg.isError
              ? "var(--caution-soft)"
              : "var(--surface-2)",
          color: isUser
            ? "#fff"
            : msg.isError
              ? "var(--caution)"
              : "var(--ink)",
          fontSize: 14,
          lineHeight: 1.55,
          whiteSpace: "pre-wrap",
          wordBreak: "break-word",
        }}
      >
        {msg.text}
      </div>
      {msg.plan && (
        <div style={{ maxWidth: "80%" }}>
          <PlanChip plan={msg.plan} onOpen={() => onPlanOpen(msg.plan!)} />
        </div>
      )}
    </div>
  );
}

function ThinkingBubble() {
  return (
    <div style={{ display: "flex", alignItems: "flex-start", marginBottom: 10 }}>
      <div style={{
        padding: "9px 14px",
        borderRadius: "14px 14px 14px 4px",
        background: "var(--surface-2)",
        color: "var(--muted)",
        fontSize: 14,
        letterSpacing: 2,
      }}>
        ···
      </div>
    </div>
  );
}

export function Chat({ messages, loading, onPlanOpen, folderPath }: Props) {
  const bottomRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [messages, loading]);

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 2, padding: "16px 20px", height: "100%" }}>
      {messages.length === 0 && !loading && (
        <div className="scan-hint" style={{ marginTop: 32 }}>
          {folderPath
            ? `${shortPath(folderPath)} 폴더가 열려 있습니다. 정리 방법을 말해 주세요.`
            : "왼쪽에서 폴더를 선택하면 TidyDog이 분석을 시작합니다."}
        </div>
      )}
      {messages.map((m) => (
        <Bubble key={m.id} msg={m} onPlanOpen={onPlanOpen} />
      ))}
      {loading && <ThinkingBubble />}
      <div ref={bottomRef} />
    </div>
  );
}
