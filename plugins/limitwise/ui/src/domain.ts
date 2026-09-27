import type { BatchEditTask, ComposerBudget, DraftTask, Task, TaskStatusName } from "./types";

export const ACTIVE_STATUSES = new Set(["scheduled", "running"]);
export const ALL_STATUSES = [
  "scheduled",
  "running",
  "completed",
  "failed",
  "blocked",
  "missed",
  "cancelled",
  "quota_skipped",
  "quota_interrupted",
] as const;

interface SchedulePayload extends Record<string, unknown> {
  budget_mode: ComposerBudget["mode"];
  tasks: Array<Record<string, unknown>>;
  weekly_cap_percent?: number;
  token_cap?: number;
  five_hour_cap_percent?: number;
}

export function eligibleAttemptAction(status: TaskStatusName, lastError?: string | null) {
  if (status === "quota_interrupted" || status === "quota_skipped") {
    if (lastError?.startsWith("quota telemetry unavailable:")) {
      return { kind: "retry" as const, label: "Retry as new run" };
    }
    return { kind: "quota_resume" as const, label: "Continue after quota reset" };
  }
  if (["failed", "blocked", "missed", "cancelled"].includes(status)) {
    return { kind: "retry" as const, label: "Retry as new run" };
  }
  return null;
}

export function defaultRunAt(now = new Date()): string {
  const scheduled = new Date(now.getTime() + 60_000);
  const offsetMinutes = -scheduled.getTimezoneOffset();
  const localTime = new Date(scheduled.getTime() + offsetMinutes * 60_000).toISOString().slice(0, -1);
  return `${localTime}${formatOffset(offsetMinutes)}`;
}

export function runAtParts(runAt: string): { date: string; time: string } {
  const match = runAt.match(/^(\d{4}-\d{2}-\d{2})T(\d{2}:\d{2})/);
  return match ? { date: match[1], time: match[2] } : { date: "", time: "" };
}

export function runAtFromParts(date: string, time: string, timezone: string, fallback: string): string {
  if (!/^\d{4}-\d{2}-\d{2}$/.test(date) || !/^\d{2}:\d{2}$/.test(time)) return fallback;
  const [year, month, day] = date.split("-").map(Number);
  const [hour, minute] = time.split(":").map(Number);
  const wallTime = Date.UTC(year, month - 1, day, hour, minute);
  let instant = wallTime;
  try {
    const formatter = new Intl.DateTimeFormat("en-CA", {
      timeZone: timezone,
      year: "numeric",
      month: "2-digit",
      day: "2-digit",
      hour: "2-digit",
      minute: "2-digit",
      hourCycle: "h23",
    });
    for (let attempt = 0; attempt < 2; attempt += 1) {
      const parts = Object.fromEntries(
        formatter.formatToParts(new Date(instant)).map((part) => [part.type, part.value]),
      );
      const representedWallTime = Date.UTC(
        Number(parts.year),
        Number(parts.month) - 1,
        Number(parts.day),
        Number(parts.hour),
        Number(parts.minute),
      );
      instant += wallTime - representedWallTime;
    }
  } catch {
    const local = new Date(year, month - 1, day, hour, minute);
    instant = local.getTime();
  }
  const offsetMinutes = Math.round((wallTime - instant) / 60_000);
  return `${date}T${time}:00${formatOffset(offsetMinutes)}`;
}

export function runAtForTimezone(runAt: string, timezone: string): string {
  const instant = new Date(runAt);
  if (!Number.isFinite(instant.getTime())) return runAt;
  try {
    const parts = Object.fromEntries(
      new Intl.DateTimeFormat("en-CA", {
        timeZone: timezone,
        year: "numeric",
        month: "2-digit",
        day: "2-digit",
        hour: "2-digit",
        minute: "2-digit",
        hourCycle: "h23",
      }).formatToParts(instant).map((part) => [part.type, part.value]),
    );
    const date = `${parts.year}-${parts.month}-${parts.day}`;
    const time = `${parts.hour}:${parts.minute}`;
    return runAtFromParts(date, time, timezone, runAt);
  } catch {
    return runAt;
  }
}

function formatOffset(offsetMinutes: number): string {
  const sign = offsetMinutes >= 0 ? "+" : "-";
  const hours = String(Math.floor(Math.abs(offsetMinutes) / 60)).padStart(2, "0");
  const minutes = String(Math.abs(offsetMinutes) % 60).padStart(2, "0");
  return `${sign}${hours}:${minutes}`;
}

export function newDraft(copy?: DraftTask): DraftTask {
  return {
    localId: crypto.randomUUID(),
    title: copy ? `${copy.title} copy` : "",
    prompt: copy?.prompt ?? "",
    success_criteria: copy?.success_criteria ?? "",
    cwd: copy?.cwd ?? "",
    run_at: copy?.run_at ?? defaultRunAt(),
    continue_from_task_id: copy?.continue_from_task_id ?? "",
    timezone: copy?.timezone ?? Intl.DateTimeFormat().resolvedOptions().timeZone ?? "UTC",
    difficulty: copy?.difficulty ?? "standard",
    model: copy?.model ?? "gpt-6-sol",
    effort: copy?.effort ?? "medium",
    permission_profile: copy?.permission_profile ?? "restricted",
  };
}

export function validateComposer(tasks: DraftTask[], budget: ComposerBudget): string[] {
  const errors: string[] = [];
  if (!tasks.length) errors.push("Add at least one task.");
  tasks.forEach((task, index) => {
    if (!task.title.trim()) errors.push(`Task ${index + 1}: title is required.`);
    if (!task.prompt.trim()) errors.push(`Task ${index + 1}: prompt is required.`);
    if (!task.cwd.startsWith("/")) errors.push(`Task ${index + 1}: cwd must be absolute.`);
    if (!task.timezone.trim()) errors.push(`Task ${index + 1}: timezone is required.`);
  });
  const first = tasks[0];
  if (first && !first.continue_from_task_id.trim()) {
    if (!/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}(:\d{2}(\.\d+)?)?(Z|[+-]\d{2}:\d{2})$/.test(first.run_at)) {
      errors.push("Task 1: run_at must be RFC3339 with an explicit UTC offset.");
    } else if (Date.parse(first.run_at) <= Date.now()) {
      errors.push("Task 1: run_at must be in the future.");
    }
  }
  if (budget.mode === "percentage") {
    validateNumber(budget.weeklyCap, 0, 100, "Weekly cap", errors);
  } else {
    validateInteger(budget.tokenCap, 1, 1_000_000_000, "Token cap", errors);
  }
  if (budget.fiveHourCap.trim()) {
    validateNumber(budget.fiveHourCap, 0, 100, "Five-hour cap", errors);
  }
  return errors;
}

export function validateSimpleComposer(tasks: DraftTask[], budget: ComposerBudget): string[] {
  const errors: string[] = [];
  if (!tasks.length) errors.push("Add at least one task.");
  tasks.forEach((task, index) => {
    if (!task.title.trim()) errors.push(`Task ${index + 1}: title is required.`);
    if (!task.prompt.trim()) errors.push(`Task ${index + 1}: prompt is required.`);
    if (!task.cwd.startsWith("/")) errors.push(`Task ${index + 1}: project directory must be absolute.`);
  });
  const first = tasks[0];
  if (!first) return errors;
  if (!/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}(:\d{2}(\.\d+)?)?(Z|[+-]\d{2}:\d{2})$/.test(first.run_at)) {
    errors.push("Run time must be RFC3339 with an explicit UTC offset.");
  } else if (Date.parse(first.run_at) <= Date.now()) {
    errors.push("Run time must be in the future.");
  }
  validateNumber(budget.weeklyCap, 0, 100, "Weekly cap", errors);
  return errors;
}

export function schedulePayload(tasks: DraftTask[], budget: ComposerBudget) {
  const payload: SchedulePayload = {
    budget_mode: budget.mode,
    tasks: tasks.map((task, index) => ({
      title: task.title.trim(),
      prompt: task.prompt,
      success_criteria: task.success_criteria,
      cwd: task.cwd,
      ...(index === 0
        ? task.continue_from_task_id.trim()
          ? { continue_from_task_id: task.continue_from_task_id.trim() }
          : { run_at: task.run_at }
        : { after_previous: true }),
      timezone: task.timezone,
      difficulty: task.difficulty,
      model: task.model,
      effort: task.effort,
      permission_profile: task.permission_profile,
    })),
  };
  if (budget.mode === "percentage") payload.weekly_cap_percent = Number(budget.weeklyCap);
  else payload.token_cap = Number(budget.tokenCap);
  if (budget.fiveHourCap.trim()) payload.five_hour_cap_percent = Number(budget.fiveHourCap);
  return payload;
}

export function draftsForBatchEdit(tasks: Task[]): DraftTask[] {
  return [...tasks]
    .sort((left, right) => left.position - right.position)
    .map((task, index) => ({
      localId: task.id,
      taskId: task.id,
      title: task.title,
      prompt: task.prompt,
      success_criteria: task.success_criteria,
      cwd: task.cwd,
      run_at: index === 0 ? task.run_at_iso : tasks[0]?.run_at_iso ?? task.run_at_iso,
      continue_from_task_id: "",
      timezone: task.timezone,
      difficulty: task.difficulty,
      model: task.model,
      effort: task.effort,
      permission_profile: task.permission_profile,
    }));
}

export function batchEditTasks(tasks: DraftTask[]): BatchEditTask[] {
  return tasks.map((task, index) => ({
    ...(task.taskId ? { task_id: task.taskId } : {}),
    title: task.title.trim(),
    prompt: task.prompt,
    success_criteria: task.success_criteria,
    cwd: task.cwd,
    ...(index === 0 ? { run_at: task.run_at } : {}),
    timezone: task.timezone,
    difficulty: task.difficulty,
    model: task.model,
    effort: task.effort,
    permission_profile: task.permission_profile,
  }));
}

export function batchStartTimeWarning(tasks: DraftTask[], now = Date.now()): string | null {
  const runAt = tasks[0]?.run_at;
  if (runAt && Number.isFinite(Date.parse(runAt)) && Date.parse(runAt) <= now) {
    return "The batch start time has passed. Change it to a future time before saving changes.";
  }
  return null;
}

export function estimatePayload(tasks: DraftTask[], budget: ComposerBudget) {
  return {
    budget_mode: budget.mode,
    ...(budget.mode === "percentage"
      ? { weekly_cap_percent: Number(budget.weeklyCap) }
      : { token_cap: Number(budget.tokenCap) }),
    tasks: tasks.map(({ title, difficulty, model, effort }) => ({
      title: title.trim(),
      difficulty,
      model,
      effort,
    })),
  };
}

export async function freezePayload<T extends Record<string, unknown>>(payload: T) {
  const canonical = canonicalJson(payload);
  const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(canonical));
  const idempotency = Array.from(new Uint8Array(digest), (byte) => byte.toString(16).padStart(2, "0")).join("");
  return deepFreeze({ idempotency_key: `ui-${idempotency}`, ...payload });
}

export function canonicalJson(value: unknown): string {
  if (Array.isArray(value)) return `[${value.map(canonicalJson).join(",")}]`;
  if (value && typeof value === "object") {
    const object = value as Record<string, unknown>;
    return `{${Object.keys(object)
      .sort()
      .map((key) => `${JSON.stringify(key)}:${canonicalJson(object[key])}`)
      .join(",")}}`;
  }
  return JSON.stringify(value) ?? "null";
}

function deepFreeze<T>(value: T): T {
  if (value && typeof value === "object") {
    Object.freeze(value);
    Object.values(value).forEach(deepFreeze);
  }
  return value;
}

function validateNumber(value: string, exclusiveMin: number, max: number, label: string, errors: string[]) {
  const parsed = Number(value);
  if (!Number.isFinite(parsed) || parsed <= exclusiveMin || parsed > max) {
    errors.push(`${label} must be greater than ${exclusiveMin} and at most ${max}.`);
  }
}

function validateInteger(value: string, min: number, max: number, label: string, errors: string[]) {
  const parsed = Number(value);
  if (!Number.isInteger(parsed) || parsed < min || parsed > max) {
    errors.push(`${label} must be an integer from ${min} to ${max}.`);
  }
}
