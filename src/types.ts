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

// execute_plan Tauri 커맨드 응답 — 성공/부분실패 판별 유니온.
// 부분실패(GateError::PartialExecute)는 moved/staged/renamed 대신
// completed/failed_op/error 를 담는다. `partial` 로 분기해야 undefined 오보를 막는다.
export type ExecPlanResponse =
  | { partial: false; plan_id: string; moved: number; staged: number; renamed: number }
  | { partial: true; plan_id: string; completed: number; failed_op: string; error: string };

// ── Phase 4: 챗 타입 ─────────────────────────────────────────────────────────

export type AgentState = "idle" | "thinking" | "done";

export interface ChatMessage {
  id:          string;
  role:        "user" | "agent";
  text:        string;
  plan?:       ProposedPlan;
  ruleChange?: RuleChange;
  isError?:    boolean;
  canRetry?:   boolean;
}

export interface PlanOp {
  op_id:        string;
  seq:          number;
  action:       "move" | "stage" | "rename";
  content_hash: string;
  from:         string;
  to:           string;
  conflict:     "none" | "rename";
  reason?:      string;
}

export interface ProposedPlan {
  plan_id:     string;
  op_count:    number;
  move_count:  number;
  stage_count: number;
  risk_score:  number;
  preview:     string;
  ops:         PlanOp[];
}

export interface SummaryCard {
  kind:        "add" | "modify" | "delete";
  description: string;
  rule_line?:  string;
}

export interface RuleChange {
  rule_change_id: string;
  summary_cards:  SummaryCard[];
  diff:           string;
  before:         string;
  after:          string;
}

// Claude API 형식 대화 히스토리 (chat 커맨드에 전달).
export type ApiMessage = {
  role:    "user" | "assistant";
  content: string | unknown[];
};

// chat Tauri 커맨드 응답 — AgentResult 의 serde 직렬화.
export interface ChatResponse {
  kind:             "text" | "plan_proposed" | "step_limit_reached" | "undo_completed" | "rule_proposed";
  message?:         string;
  partial_message?: string;
  plan_id?:         string;
  op_count?:        number;
  move_count?:      number;
  stage_count?:     number;
  risk_score?:      number;
  preview?:         string;
  ops?:             PlanOp[];
  // rule_proposed 필드
  rule_change_id?:  string;
  summary_cards?:   SummaryCard[];
  diff?:            string;
  before?:          string;
  after?:           string;
}
