export interface FileNode {
  name: string;
  path: string;
  is_dir: boolean;
  size?: number;
  ext?: string;
  children: FileNode[];
}

export interface ExecResult {
  moved: number;
  staged: number;
  renamed: number;
}

// ── Phase 4: 챗 타입 ─────────────────────────────────────────────────────────

export type AgentState = "idle" | "thinking" | "done";

export interface ChatMessage {
  id:        string;
  role:      "user" | "agent";
  text:      string;
  plan?:     ProposedPlan;
  isError?:  boolean;
}

export interface ProposedPlan {
  plan_id:     string;
  op_count:    number;
  move_count:  number;
  stage_count: number;
  risk_score:  number;
  preview:     string;
}

// Claude API 형식 대화 히스토리 (chat 커맨드에 전달).
export type ApiMessage = {
  role:    "user" | "assistant";
  content: string | unknown[];
};

// chat Tauri 커맨드 응답 — AgentResult 의 serde 직렬화.
export interface ChatResponse {
  kind:             "text" | "plan_proposed" | "step_limit_reached" | "undo_completed";
  message?:         string;   // Text, StepLimitReached
  partial_message?: string;   // StepLimitReached
  plan_id?:         string;   // PlanProposed, UndoCompleted
  op_count?:        number;
  move_count?:      number;
  stage_count?:     number;
  risk_score?:      number;
  preview?:         string;
}
