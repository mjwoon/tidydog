import { useState, useRef, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import {
  FileNode, ExecResult, AgentState,
  ChatMessage, ProposedPlan, ApiMessage, ChatResponse, RuleChange,
} from "./types";
import { TreeNode } from "./components/TreeNode";
import { Topbar } from "./components/Topbar";
import { DogMascot } from "./components/DogMascot";
import { PlanReview, PlanSummary } from "./components/PlanReview";
import { RuleReview } from "./components/RuleReview";
import { SettingsModal } from "./components/SettingsModal";
import { Chat } from "./components/Chat";

let msgSeq = 0;
function nextId() { return String(++msgSeq); }

export default function App() {
  const [folderPath, setFolderPath] = useState<string | null>(null);
  const [tree, setTree] = useState<FileNode | null>(null);
  const [scanning, setScanning] = useState(false);

  // Phase 4: 챗 상태
  const [messages, setMessages]       = useState<ChatMessage[]>([]);
  const [apiHistory, setApiHistory]   = useState<ApiMessage[]>([]);
  const [agentState, setAgentState]   = useState<AgentState>("idle");
  const [inputText, setInputText]     = useState("");
  const inputRef                       = useRef<HTMLInputElement>(null);

  // N1: AI 콘텐츠 처리 동의 상태
  const [hasConsent, setHasConsent] = useState<boolean | null>(null);

  useEffect(() => {
    invoke<boolean>("get_ai_consent")
      .then(setHasConsent)
      .catch(() => setHasConsent(false));
  }, []);

  // 플랜 모달
  const [pendingPlan, setPendingPlan] = useState<PlanSummary | null>(null);
  const [, setLastResult]   = useState<ExecResult | null>(null);

  // 규칙 변경 모달
  const [pendingRuleChange, setPendingRuleChange] = useState<RuleChange | null>(null);

  // 설정 모달
  const [showSettings, setShowSettings] = useState(false);

  // ── 폴더 스캔 ──────────────────────────────────────────────────────────────

  async function selectAndScan() {
    const selected = await open({ directory: true, multiple: false });
    if (typeof selected !== "string") return;
    setFolderPath(selected);
    setScanning(true);
    setLastResult(null);
    try {
      const result = await invoke<FileNode>("scan_directory", { root: selected, maxDepth: 10 });
      setTree(result);
    } catch (err) {
      console.error("scan_directory failed:", err);
    } finally {
      setScanning(false);
    }
  }

  // ── 플랜 승인/취소 ─────────────────────────────────────────────────────────

  function handleExecuted(result: ExecResult) {
    setPendingPlan(null);
    setLastResult(result);
    setAgentState("idle");

    const agentMsg: ChatMessage = {
      id:   nextId(),
      role: "agent",
      text: `완료 — 이동 ${result.moved}개 · 격리 ${result.staged}개 · 리네임 ${result.renamed}개`,
    };
    setMessages((prev) => [...prev, agentMsg]);

    if (folderPath) {
      invoke<FileNode>("scan_directory", { root: folderPath, maxDepth: 10 })
        .then(setTree)
        .catch(console.error);
    }
  }

  function openPlanModal(plan: ProposedPlan) {
    setPendingPlan({
      plan_id:    plan.plan_id,
      risk_score: plan.risk_score,
      preview:    plan.preview as "auto" | "standard" | "full_review",
      op_count:   plan.op_count,
      ops:        plan.ops,
    });
  }

  function openRuleModal(rc: RuleChange) {
    setPendingRuleChange(rc);
  }

  function handleRuleApplied() {
    setPendingRuleChange(null);
    setAgentState("idle");
    const agentMsg: ChatMessage = {
      id:   nextId(),
      role: "agent",
      text: "ORGANIZER 규칙이 업데이트되었습니다. 다음 정리부터 새 규칙이 적용됩니다.",
    };
    setMessages((prev) => [...prev, agentMsg]);
  }

  // ── 챗 입력 전송 ──────────────────────────────────────────────────────────

  async function handleSend() {
    const text = inputText.trim();
    if (!text || agentState === "thinking") return;
    setInputText("");

    const isUndo = text.startsWith("/undo");

    // 사용자 버블 추가.
    const userMsg: ChatMessage = { id: nextId(), role: "user", text };
    setMessages((prev) => [...prev, userMsg]);
    setAgentState("thinking");

    try {
      const response = await invoke<ChatResponse>("chat", {
        message:            text,
        history:            apiHistory,
        userTriggeredUndo:  isUndo,
        root:               folderPath ?? "",
      });

      // 대화 히스토리 업데이트.
      const newHistory: ApiMessage[] = [
        ...apiHistory,
        { role: "user",      content: text },
        { role: "assistant", content: response.message ?? response.partial_message ?? "" },
      ];
      setApiHistory(newHistory);

      // 응답 종류별 처리.
      switch (response.kind) {
        case "text": {
          const agentMsg: ChatMessage = {
            id:   nextId(),
            role: "agent",
            text: response.message ?? "",
          };
          setMessages((prev) => [...prev, agentMsg]);
          setAgentState("idle");
          break;
        }

        case "plan_proposed": {
          const plan: ProposedPlan = {
            plan_id:     response.plan_id!,
            op_count:    response.op_count!,
            move_count:  response.move_count!,
            stage_count: response.stage_count!,
            risk_score:  response.risk_score!,
            preview:     response.preview!,
            ops:         response.ops ?? [],
          };
          const agentMsg: ChatMessage = {
            id:   nextId(),
            role: "agent",
            text: `정리 플랜을 만들었습니다. 총 ${plan.op_count}개 작업 (이동 ${plan.move_count} · 격리 ${plan.stage_count}).`,
            plan,
          };
          setMessages((prev) => [...prev, agentMsg]);
          setAgentState("done");
          break;
        }

        case "step_limit_reached": {
          const agentMsg: ChatMessage = {
            id:   nextId(),
            role: "agent",
            text: `(최대 단계 도달) ${response.partial_message ?? ""}`,
          };
          setMessages((prev) => [...prev, agentMsg]);
          setAgentState("idle");
          break;
        }

        case "undo_completed": {
          const agentMsg: ChatMessage = {
            id:   nextId(),
            role: "agent",
            text: `플랜 ${response.plan_id}을(를) 되돌렸습니다.`,
          };
          setMessages((prev) => [...prev, agentMsg]);
          setAgentState("idle");
          if (folderPath) {
            invoke<FileNode>("scan_directory", { root: folderPath, maxDepth: 10 })
              .then(setTree)
              .catch(console.error);
          }
          break;
        }

        case "rule_proposed": {
          const rc: RuleChange = {
            rule_change_id: response.rule_change_id!,
            summary_cards:  response.summary_cards ?? [],
            diff:           response.diff ?? "",
            before:         response.before ?? "",
            after:          response.after ?? "",
          };
          const agentMsg: ChatMessage = {
            id:         nextId(),
            role:       "agent",
            text:       "ORGANIZER 규칙 변경을 제안했습니다. 아래 내용을 검토해 주세요.",
            ruleChange: rc,
          };
          setMessages((prev) => [...prev, agentMsg]);
          setAgentState("done");
          break;
        }
      }
    } catch (err: unknown) {
      const errText = err instanceof Error ? err.message : String(err);
      // API 키 미설정 에러이면 설정 모달 자동 오픈.
      const isApiKeyError = errText.includes("API 키") || errText.includes("api_key") || errText.includes("set_api_key");
      const agentMsg: ChatMessage = {
        id:      nextId(),
        role:    "agent",
        text:    isApiKeyError
          ? "API 키가 설정되지 않았습니다. 설정(⚙)에서 Anthropic API 키를 입력해 주세요."
          : errText,
        isError: true,
      };
      setMessages((prev) => [...prev, agentMsg]);
      setAgentState("idle");
      if (isApiKeyError) setShowSettings(true);
    }
  }

  function handleKeyDown(e: React.KeyboardEvent<HTMLInputElement>) {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      handleSend();
    }
  }

  async function handleGrantConsent() {
    await invoke("set_ai_consent", { granted: true });
    setHasConsent(true);
  }

  // ── 마스코트 상태 ─────────────────────────────────────────────────────────

  function mascotState(): string {
    if (agentState === "thinking") return "생각 중";
    if (agentState === "done")     return "검토 중";
    if (scanning)                  return "스캔 중";
    return "대기 중";
  }

  // ── 렌더 ──────────────────────────────────────────────────────────────────

  return (
    <>
      <Topbar folderPath={folderPath} onFolderSelect={selectAndScan} onSettingsOpen={() => setShowSettings(true)} />
      <div className="shell">
        <aside className="sidebar">
          <div className="side-head">폴더 트리</div>
          <div className="tree">
            {scanning && <div className="scan-hint">스캔 중…</div>}
            {!scanning && !tree && (
              <div className="scan-hint">폴더를 선택하면 트리를 표시합니다.</div>
            )}
            {!scanning && tree && <TreeNode node={tree} depth={0} />}
          </div>
          <div className="side-foot">
            <button className="add-btn" onClick={selectAndScan}>＋ 폴더 추가</button>
          </div>
        </aside>

        <main className="main">
          <div className="chat" style={{ overflowY: "auto", flex: 1 }}>
            <Chat
              messages={messages}
              loading={agentState === "thinking"}
              onPlanOpen={openPlanModal}
              onRuleOpen={openRuleModal}
              folderPath={folderPath}
            />
          </div>

          <div className="composer">
            {hasConsent === false && folderPath && (
              <div style={{
                display: "flex",
                alignItems: "center",
                gap: 10,
                padding: "8px 14px",
                background: "var(--caution-soft)",
                borderTop: "1px solid var(--line)",
                fontSize: 12.5,
              }}>
                <span style={{ flex: 1, color: "var(--caution)" }}>
                  파일 내용 분석에는 AI 처리 동의가 필요합니다.
                </span>
                <button
                  onClick={handleGrantConsent}
                  style={{
                    padding: "4px 12px",
                    borderRadius: "var(--r-sm)",
                    border: "1px solid var(--caution)",
                    background: "transparent",
                    color: "var(--caution)",
                    cursor: "pointer",
                    fontSize: 12,
                    fontWeight: 600,
                    fontFamily: "var(--ui)",
                    whiteSpace: "nowrap",
                  }}
                >
                  동의하기
                </button>
              </div>
            )}
            <div className="input-bar">
              <input
                ref={inputRef}
                value={inputText}
                onChange={(e) => setInputText(e.target.value)}
                onKeyDown={handleKeyDown}
                placeholder={
                  !folderPath
                    ? "폴더를 먼저 선택하세요"
                    : agentState === "thinking"
                      ? "응답 대기 중…"
                      : "무엇을 정리할까요? (/undo 로 되돌리기)"
                }
                disabled={!folderPath || agentState === "thinking"}
              />
              <button
                className="send-btn"
                onClick={handleSend}
                disabled={!folderPath || agentState === "thinking" || !inputText.trim()}
              >
                ↑
              </button>
            </div>
          </div>
          <DogMascot state={mascotState()} />
        </main>
      </div>

      {pendingPlan && folderPath && (
        <PlanReview
          plan={pendingPlan}
          root={folderPath}
          onExecuted={handleExecuted}
          onCancel={() => { setPendingPlan(null); setAgentState("idle"); }}
        />
      )}

      {showSettings && (
        <SettingsModal onClose={() => setShowSettings(false)} />
      )}

      {pendingRuleChange && folderPath && (
        <RuleReview
          ruleChange={pendingRuleChange}
          root={folderPath}
          onApplied={handleRuleApplied}
          onCancel={() => { setPendingRuleChange(null); setAgentState("idle"); }}
        />
      )}
    </>
  );
}
