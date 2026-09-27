import { describe, expect, it } from "vitest";
import {
  batchEditTasks,
  batchStartTimeWarning,
  canonicalJson,
  defaultRunAt,
  eligibleAttemptAction,
  estimatePayload,
  freezePayload,
  newDraft,
  runAtFromParts,
  runAtForTimezone,
  runAtParts,
  schedulePayload,
  validateComposer,
  validateSimpleComposer,
} from "./domain";
import type { ComposerBudget, DraftTask } from "./types";

const percentage: ComposerBudget = {
  mode: "percentage",
  weeklyCap: "1",
  tokenCap: "100000",
  fiveHourCap: "5",
};

function tasks(count = 1): DraftTask[] {
  return Array.from({ length: count }, (_, index) => ({
    ...newDraft(),
    title: `Task ${index + 1}`,
    prompt: `Prompt ${index + 1}`,
    success_criteria: `Done ${index + 1}`,
    cwd: "/tmp/project",
    run_at: "2099-01-01T10:00:00+01:00",
    timezone: "Europe/Paris",
  }));
}

describe("linear composer contract", () => {
  it("prefills run_at one minute ahead with an explicit local offset", () => {
    const now = new Date("2026-09-26T18:00:00.000Z");
    const runAt = defaultRunAt(now);
    expect(runAt).toMatch(/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{3}[+-]\d{2}:\d{2}$/);
    expect(Date.parse(runAt) - now.getTime()).toBe(60_000);
    expect(newDraft().run_at).not.toBe("");
  });

  it("converts human date and 24-hour time fields to RFC3339 in the selected timezone", () => {
    expect(runAtParts("2099-07-01T16:45:00+02:00")).toEqual({ date: "2099-07-01", time: "16:45" });
    expect(runAtFromParts("2099-07-01", "16:45", "Europe/Paris", "fallback")).toBe("2099-07-01T16:45:00+02:00");
    expect(runAtFromParts("2099-01-01", "16:45", "Europe/Paris", "fallback")).toBe("2099-01-01T16:45:00+01:00");
    expect(runAtFromParts("", "16:45", "Europe/Paris", "fallback")).toBe("fallback");
    expect(runAtForTimezone("2099-07-01T16:45:00+02:00", "UTC")).toBe("2099-07-01T14:45:00+00:00");
  });

  it("maps one task to run_at and three tasks to after_previous only", () => {
    const one = schedulePayload(tasks(1), percentage) as { tasks: Array<Record<string, unknown>> };
    expect(one.tasks[0]).toHaveProperty("run_at");
    expect(one.tasks[0]).not.toHaveProperty("after_previous");
    expect(one.tasks[0]).toMatchObject({ permission_profile: "restricted" });

    const three = schedulePayload(tasks(3), percentage) as { tasks: Array<Record<string, unknown>> };
    expect(three.tasks[0]).toHaveProperty("run_at");
    expect(three.tasks[1]).toMatchObject({ after_previous: true });
    expect(three.tasks[2]).toMatchObject({ after_previous: true });
    expect(three.tasks[1]).not.toHaveProperty("run_at");
    expect(three.tasks[2]).not.toHaveProperty("continue_from_task_id");
  });

  it("defaults copied and fresh drafts safely while preserving explicit networked selection", () => {
    const restricted = newDraft();
    expect(restricted.permission_profile).toBe("restricted");
    const networked = newDraft({ ...tasks(1)[0], permission_profile: "networked" });
    expect(networked.permission_profile).toBe("networked");
    const payload = schedulePayload([networked], percentage) as { tasks: Array<Record<string, unknown>> };
    expect(payload.tasks[0]).toMatchObject({ permission_profile: "networked" });
  });

  it("reordering changes chain order without branch, join, or cycle fields", () => {
    const reordered = tasks(3).reverse();
    const payload = schedulePayload(reordered, percentage) as { tasks: Array<Record<string, unknown>> };
    expect(payload.tasks.map((task) => task.title)).toEqual(["Task 3", "Task 2", "Task 1"]);
    expect(canonicalJson(payload)).not.toContain("edges");
    expect(canonicalJson(payload)).not.toContain("depends_on_task_id");
  });

  it("duplicates a task with a new local identity", () => {
    const original = tasks(1)[0];
    const duplicate = newDraft(original);
    expect(duplicate.localId).not.toBe(original.localId);
    expect(duplicate.prompt).toBe(original.prompt);
  });

  it("preserves existing task IDs and adds new tasks in one edited batch payload", () => {
    const edited = tasks(2);
    edited[0].taskId = "task-existing";
    edited[1].taskId = undefined;
    const payload = batchEditTasks(edited);
    expect(payload[0]).toMatchObject({ task_id: "task-existing", run_at: edited[0].run_at });
    expect(payload[1]).not.toHaveProperty("task_id");
    expect(payload[1]).not.toHaveProperty("run_at");
  });

  it("warns that a passed batch start time must change before save", () => {
    const edited = tasks(1);
    edited[0].run_at = "2026-09-27T09:00:00+02:00";
    expect(batchStartTimeWarning(edited, Date.parse("2026-09-27T09:00:01+02:00"))).toBe(
      "The batch start time has passed. Change it to a future time before saving changes.",
    );
    expect(batchStartTimeWarning(edited, Date.parse("2026-09-27T08:59:59+02:00"))).toBeNull();
  });

  it("rejects invalid time, cwd, percentage, token, and five-hour caps", () => {
    const invalid = tasks(1);
    invalid[0].cwd = "relative";
    invalid[0].run_at = "tomorrow";
    const errors = validateComposer(invalid, { ...percentage, weeklyCap: "0", fiveHourCap: "101" });
    expect(errors.join(" ")).toContain("cwd must be absolute");
    expect(errors.join(" ")).toContain("RFC3339");
    expect(errors.join(" ")).toContain("Weekly cap");
    expect(errors.join(" ")).toContain("Five-hour cap");
    expect(validateComposer(tasks(1), { ...percentage, mode: "tokens", tokenCap: "1.5" }).join(" ")).toContain("integer");
  });

  it("simple mode requires tasks, project directory, weekly limit, and future run time", () => {
    const draft = newDraft();
    draft.title = "Fix UI";
    draft.prompt = "Correct the empty state";
    draft.run_at = "2099-01-01T10:00:00+00:00";
    draft.cwd = "/tmp/project";
    const chain = [draft, { ...newDraft(draft), title: "Verify UI", prompt: "Run focused tests" }];
    expect(validateSimpleComposer(chain, percentage)).toEqual([]);
    expect(validateSimpleComposer([{ ...draft, prompt: "" }], percentage)).toContain("Task 1: prompt is required.");
    expect(validateSimpleComposer([{ ...draft, cwd: "relative" }], percentage).join(" ")).toContain("project directory");
    expect(validateSimpleComposer([draft], { ...percentage, weeklyCap: "0" }).join(" ")).toContain("Weekly cap");
  });

  it("creates a stable idempotency key from canonical frozen payload", async () => {
    const first = await freezePayload(schedulePayload(tasks(3), percentage));
    const second = await freezePayload(schedulePayload(tasks(3).map((task) => ({ ...task })), percentage));
    expect(first.idempotency_key).toBe(second.idempotency_key);
    expect(first.idempotency_key).toMatch(/^ui-[a-f0-9]{64}$/);
    expect(Object.isFrozen(first)).toBe(true);
    expect(Object.isFrozen(first.tasks)).toBe(true);
  });

  it("sends only estimator-supported fields before confirmation", () => {
    const estimate = estimatePayload(tasks(1), percentage);
    expect(estimate).not.toHaveProperty("five_hour_cap_percent");
    expect(estimate.tasks[0]).toEqual({
      title: "Task 1",
      difficulty: "standard",
      model: "gpt-6-sol",
      effort: "medium",
    });
  });

  it("exposes exactly one retry action only for eligible terminal statuses", () => {
    for (const status of ["quota_interrupted", "quota_skipped"] as const) {
      expect(eligibleAttemptAction(status)).toEqual({
        kind: "quota_resume",
        label: "Continue after quota reset",
      });
    }
    expect(eligibleAttemptAction("quota_skipped", "quota telemetry unavailable: missing Codex")).toEqual({
      kind: "retry",
      label: "Retry as new run",
    });
    for (const status of ["failed", "blocked", "missed", "cancelled"] as const) {
      expect(eligibleAttemptAction(status)).toEqual({
        kind: "retry",
        label: "Retry as new run",
      });
    }
    for (const status of ["completed", "scheduled", "running"] as const) {
      expect(eligibleAttemptAction(status)).toBeNull();
    }
  });
});
