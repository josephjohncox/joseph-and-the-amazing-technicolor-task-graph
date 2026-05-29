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

export type ComposedGoalSnapshot = {
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
  plan_id?: string | null;
  goal_id?: string | null;
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
  plan_id?: string | null;
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

export type OperatorWorkspaceSnapshot = {
  generated_at?: string;
  selected_goal_id?: string;
  goals?: OperatorGoalSummary[];
  selected_goal?: OperatorGoalDetail | null;
  actions?: OperatorAction[];
  events?: OperatorEvent[];
  worker_runs?: OperatorWorkerRun[];
  evidence?: OperatorEvidence[];
  services?: ServiceHealth[];
  runners?: unknown;
  event_sources?: unknown;
  human_threads?: unknown;
  config?: JsonRecord;
};

export type OperatorGoalSummary = {
  goal_id?: string;
  id?: string;
  title?: string;
  objective?: string;
  status?: string;
  percent_done?: number;
  open_tasks?: number;
  blocked_tasks?: number;
  failed_tasks?: number;
  satisfied?: boolean;
  updated_at?: string;
};

export type OperatorGoalDetail = {
  summary?: OperatorGoalSummary;
  progress?: unknown;
  graph?: unknown;
  tasks?: TaskRow[];
  actions?: OperatorAction[];
  evidence?: OperatorEvidence[];
  snapshot?: ComposedGoalSnapshot;
};

export type OperatorAction = {
  action_id?: string;
  kind?: string;
  plan_id?: string;
  goal_id?: string;
  task_id?: string | null;
  title?: string;
  question?: string;
  status?: string;
  allowed_resolutions?: string[];
  approval?: JsonRecord | null;
  thunk?: JsonRecord | null;
  payload_json?: JsonRecord;
};

export type PlanPhase =
  | "asking"
  | "drafting_plan"
  | "drafting_goals"
  | "accepting"
  | "executing"
  | "reviewing"
  | "satisfied"
  | "cancelled";

export type PlanAction = {
  action_id?: string;
  plan_id?: string;
  goal_id?: string | null;
  task_id?: string | null;
  kind?: string;
  title?: string;
  reason?: string;
  allowed_actions?: string[];
  required_fields?: string[];
  status?: string;
  evidence_refs?: JsonRecord[];
};

export type PlanSummary = {
  plan_id?: string;
  id?: string;
  source_plan_id?: string | null;
  title?: string;
  objective?: string;
  status?: string;
  phase?: PlanPhase | string;
  mode?: string;
  version?: number;
  subgoal_count?: number;
  initial_task_count?: number;
  open_question_count?: number;
  action_item_count?: number;
  compiled_goal_id?: string | null;
  updated_at?: string | null;
  actions?: PlanAction[];
};

export type PlanListResponse = {
  generated_at?: string;
  plans?: PlanSummary[];
  data?: PlanSummary[];
  source?: JsonRecord;
};

export type PlanDetailResponse = {
  generated_at?: string;
  found?: boolean;
  plan_id?: string;
  plan?: PlanSummary & JsonRecord;
  phase?: PlanPhase | string;
  actions?: PlanAction[];
  source?: JsonRecord;
};

export type OperatorEvent = {
  event_id?: string;
  event_type?: string;
  goal_id?: string | null;
  task_id?: string | null;
  title?: string;
  detail?: string;
  created_at?: string | null;
  payload_json?: JsonRecord;
};

export type OperatorEvidence = {
  evidence_id?: string;
  goal_id?: string;
  task_id?: string | null;
  title?: string;
  uri?: string | null;
  checkpoint?: JsonRecord | null;
  created_at?: string | null;
  payload_json?: JsonRecord;
};

export type OperatorWorkerRun = {
  run_id?: string;
  goal_id?: string;
  task_id?: string | null;
  worker?: string;
  status?: string;
  summary?: string;
  started_at?: string | null;
  finished_at?: string | null;
  payload_json?: JsonRecord;
};
