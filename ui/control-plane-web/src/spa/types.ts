export type JsonRecord = Record<string, unknown>;

export type ChatMessage = {
  role: "user" | "assistant";
  content: string;
};

export type ServiceHealth = {
  name?: string;
  ok?: boolean;
  status?: number;
};

export type GoalRow = {
  goal_id?: string;
  id?: string;
  title?: string;
  objective?: string;
  status?: string;
  percent_done?: number;
  open_tasks?: number;
  blocked_tasks?: number;
  failed_tasks?: number;
  updated_at?: string;
};

export type TaskRow = {
  task_id?: string;
  id?: string;
  parent_task_id?: string | null;
  title?: string;
  color?: ColorRef | null;
  role?: string;
  purpose?: string;
  purpose_kind?: string;
  status?: string;
  current_prompt?: string | null;
  prompt?: string | null;
  result?: unknown;
  payload_json?: JsonRecord;
  raw_task?: JsonRecord;
};

export type ColorRef = {
  key?: string;
  label?: string;
  hex?: string;
  meaning?: string;
};

export type Overview = {
  generated_at?: string;
  services?: ServiceHealth[];
  goals?: unknown;
  agents?: unknown;
  plans?: unknown;
  approvals?: unknown;
  runner_status?: unknown;
  human_threads?: unknown;
  follow_ups?: unknown;
};

export type GoalSnapshot = {
  goal_id?: string;
  goal_store_goal?: unknown;
  workflow_progress?: unknown;
  workflow_status?: unknown;
  tasks?: unknown;
  approvals?: unknown;
  checkpoints?: unknown;
  agent_activity?: TaskRow[];
};

export type ChatResponse = {
  provider?: string;
  model?: string | null;
  assistant?: string;
  drafts?: JsonRecord;
};
