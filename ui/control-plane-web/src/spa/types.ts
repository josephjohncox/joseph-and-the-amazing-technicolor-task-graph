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
  event_sources?: unknown;
  recent_events?: unknown;
  triggers?: unknown;
};

export type GoalSnapshot = {
  goal_id?: string;
  goal_store_goal?: unknown;
  workflow_progress?: unknown;
  workflow_compute_graph?: ComputeGraphSnapshot | { data?: ComputeGraphSnapshot } | unknown;
  workflow_status?: unknown;
  tasks?: unknown;
  approvals?: unknown;
  checkpoints?: unknown;
  agent_activity?: TaskRow[];
};

export type WaitRef = {
  kind?: string;
  reference?: string;
};

export type ComputeGraphNode = {
  id?: string;
  kind?: string;
  label?: string;
  status?: string;
  task_id?: string | null;
  thunk_id?: string | null;
  continuation_id?: string | null;
  requested_input?: string | null;
  wait_ref?: WaitRef | null;
};

export type ComputeGraphEdge = {
  from?: string;
  to?: string;
  kind?: string;
};

export type ComputeGraphSnapshot = {
  goal_id?: string;
  nodes?: ComputeGraphNode[];
  edges?: ComputeGraphEdge[];
  open_thunks?: number;
  runnable_tasks?: string[];
  waiting_tasks?: string[];
};

export type ChatResponse = {
  provider?: string;
  model?: string | null;
  assistant?: string;
  drafts?: JsonRecord;
  draft_refs?: JsonRecord;
  draft_summary?: JsonRecord;
  session_id?: string;
  run_id?: string;
  chat_log?: JsonRecord;
  chat_backend?: JsonRecord;
  model_params?: JsonRecord;
  context?: JsonRecord;
  raw_model_response?: string;
  chat_run?: ChatRunTrace;
};

export type ChatRunTrace = {
  run_id?: string;
  found?: boolean;
  session_id?: string;
  goal_id?: string | null;
  mode?: string;
  status?: string;
  stage?: string;
  started_at?: string;
  updated_at?: string;
  finished_at?: string;
  elapsed_ms?: number;
  backend?: JsonRecord;
  model_params?: JsonRecord;
  chat_log?: JsonRecord;
  error?: string;
  steps?: Array<{ stage?: string; at?: string; detail?: JsonRecord }>;
};
