export type TaskStatusName =
  | "scheduled"
  | "running"
  | "completed"
  | "failed"
  | "blocked"
  | "missed"
  | "cancelled"
  | "quota_skipped"
  | "quota_interrupted";

export interface RateWindow {
  used_percent: number;
  remaining_percent: number;
  duration_minutes: number;
  resets_at: number;
}

export interface UsageSnapshot {
  adapter: string;
  captured_at: number;
  five_hour: RateWindow | null;
  weekly: RateWindow;
  warnings?: string[];
}

export interface Task {
  id: string;
  batch_id: string;
  title: string;
  prompt: string;
  success_criteria: string;
  cwd: string;
  run_at: number;
  run_at_iso: string;
  position: number;
  depends_on_task_id?: string | null;
  dependency_type: string;
  source_task_id?: string | null;
  attempt_kind?: "retry" | "quota_resume" | null;
  attempt_number: number;
  timezone: string;
  difficulty: Difficulty;
  model: Model;
  effort: Effort;
  permission_profile: PermissionProfile;
  status: TaskStatusName;
  created_at: number;
  updated_at: number;
  last_error?: string | null;
}

export type TaskSort =
  | "newest"
  | "oldest"
  | "run_at_newest"
  | "run_at_oldest"
  | "title_ascending"
  | "title_descending";

export interface TaskPage {
  items: Task[];
  page: number;
  page_size: number;
  total: number;
  total_pages: number;
  sort: TaskSort;
}

export interface BatchGroup {
  batch: Batch;
  tasks: Task[];
  editable: boolean;
  editing: boolean;
}

export interface BatchPage {
  items: BatchGroup[];
  page: number;
  page_size: number;
  total: number;
  total_pages: number;
  sort: TaskSort;
}

export interface BatchEditSession {
  edit_session_id: string;
  expires_at: number;
  batch: Batch;
  tasks: Task[];
}

export interface BatchEditTask {
  task_id?: string;
  title: string;
  prompt: string;
  success_criteria: string;
  cwd: string;
  run_at?: string;
  timezone: string;
  difficulty: Difficulty;
  model: Model;
  effort: Effort;
  permission_profile: PermissionProfile;
}

export interface PlannedTask {
  title: string;
  prompt: string;
  success_criteria: string;
  difficulty: Difficulty;
  model: Model;
  effort: Effort;
}

export interface PlannedBatch {
  summary: string;
  tasks: PlannedTask[];
  planner_model: Model;
  planner_effort: Effort;
  cwd: string;
  timezone: string;
  permission_profile: PermissionProfile;
  weekly_cap_percent: number;
}

export interface ArchiveTaskResult {
  task_id: string;
  archived_at: number;
  status: TaskStatusName;
  preserved_runs: number;
}

export interface Batch {
  id: string;
  idempotency_key: string;
  budget_mode: BudgetMode;
  weekly_cap_percent?: number | null;
  token_cap?: number | null;
  consumed_tokens: number;
  five_hour_cap_percent?: number | null;
  allowance_points: number;
  consumed_points: number;
}

export interface RunRecord {
  id: string;
  task_id: string;
  started_at: number;
  finished_at?: number | null;
  status: string;
  transcript_path?: string | null;
  tokens_used?: number | null;
  token_usage_state: string;
  error?: string | null;
}

export interface TaskDetail {
  task: Task;
  batch: Batch;
  runs: RunRecord[];
  lineage: Task[];
}

export interface RetryTaskOptions {
  budget_mode: BudgetMode;
  weekly_cap_percent?: number;
  token_cap?: number;
  five_hour_cap_percent?: number;
  run_at?: string;
  timezone?: string;
  model?: Model;
  effort?: Effort;
  permission_profile?: PermissionProfile;
  networked_confirmed?: boolean;
}

export interface RetryTaskPreview {
  source_task_id: string;
  attempt_kind: "retry" | "quota_resume";
  action_label: string;
  attempt_number: number;
  run_at: number;
  run_at_iso: string;
  provider_reset_at?: number | null;
  resume_mode: "fresh_session" | "session_resume_or_context_fallback" | "context_fallback";
  session_available: boolean;
  task: Pick<Task, "title" | "prompt" | "success_criteria" | "cwd" | "timezone" | "difficulty" | "model" | "effort" | "permission_profile">;
  budget: {
    budget_mode: BudgetMode;
    weekly_cap_percent?: number | null;
    token_cap?: number | null;
    five_hour_cap_percent?: number | null;
  };
  estimate: Record<string, unknown>;
}

export interface AttemptCreateResult {
  batch: Batch;
  task: Task;
  idempotent_replay: boolean;
}

export interface DiagnosticReport {
  schema_version: string;
  overall: "pass" | "warn" | "fail";
  generated_at: number;
  checks: Array<{ id: string; status: "pass" | "warn" | "fail"; summary: string }>;
}

export interface UsageStats {
  generated_at: number;
  timezone: string;
  last_week: { run_count: number; tokens_used: number; tokens_unavailable_runs: number };
  last_month: { run_count: number; tokens_used: number; tokens_unavailable_runs: number };
  last_year: { run_count: number; tokens_used: number; tokens_unavailable_runs: number };
}

export interface ReasoningEffortOption {
  reasoningEffort: string;
  description: string;
}

export interface CodexModel {
  id: string;
  model: string;
  displayName: string;
  description: string;
  defaultReasoningEffort: string;
  supportedReasoningEfforts: ReasoningEffortOption[];
  isDefault: boolean;
  upgrade?: string | null;
}

export interface ModelCatalog {
  source: "codex" | "fallback" | "bundled";
  warning?: string | null;
  models: CodexModel[];
}

export type Difficulty = "simple" | "standard" | "complex" | "exceptional";
export type Model = string;
export type Effort = string;
export type BudgetMode = "percentage" | "tokens";
export type PermissionProfile = "restricted" | "networked";

export interface DraftTask {
  localId: string;
  taskId?: string;
  title: string;
  prompt: string;
  success_criteria: string;
  cwd: string;
  run_at: string;
  continue_from_task_id: string;
  timezone: string;
  difficulty: Difficulty;
  model: Model;
  effort: Effort;
  permission_profile: PermissionProfile;
}

export interface ComposerBudget {
  mode: BudgetMode;
  weeklyCap: string;
  tokenCap: string;
  fiveHourCap: string;
}

export interface ApiErrorBody {
  error: { code: string; message: string; fields: Record<string, string> };
}
