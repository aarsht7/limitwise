import { expect, test, type Page } from "@playwright/test";
import { diagnostics, modelCatalog, stats, tasks, usage } from "./fixtures";

async function fixtureApi(page: Page) {
  const calls: Array<{ method: string; path: string; query: string; body: unknown }> = [];
  const createdAttempts = new Map<string, Record<string, unknown>>();
  const batchItems = [...new Set(tasks.map((task) => task.batch_id))].map((batchId) => {
    const batchTasks = tasks.filter((task) => task.batch_id === batchId).sort((left, right) => left.position - right.position);
    return {
      batch: { id: batchId, idempotency_key: `fixture-${batchId}`, budget_mode: "percentage", weekly_cap_percent: 1, token_cap: null, consumed_tokens: 0, five_hour_cap_percent: null, allowance_points: 1, consumed_points: 0 },
      tasks: batchTasks,
      editable: batchTasks.every((task) => task.status === "scheduled"),
      editing: false,
    };
  });
  await page.addInitScript(() => sessionStorage.setItem("limitwise-bearer", "fixture-token"));
  await page.route("**/api/v1/**", async (route) => {
    const request = route.request();
    const url = new URL(request.url());
    const path = url.pathname;
    const body = request.postDataJSON?.() ?? null;
    const bodyObject = body && typeof body === "object" ? body as Record<string, unknown> : {};
    const planInputs = Array.isArray(bodyObject.tasks)
      ? bodyObject.tasks as Array<{ title: string; prompt: string }>
      : [];
    calls.push({ method: request.method(), path, query: url.search, body });
    const detailTask = tasks.find((task) => request.method() === "GET" && path === `/api/v1/tasks/${task.id}`);
    const archivedTask = tasks.find((task) => request.method() === "DELETE" && path === `/api/v1/tasks/${task.id}`);
    const retrySource = tasks.find((task) => path === `/api/v1/tasks/${task.id}/retry-preview`);
    const retryConfirm = tasks.find((task) => path === `/api/v1/tasks/${task.id}/retry`);
    const fixtures: Record<string, unknown> = {
      "/api/v1/models": modelCatalog,
      "/api/v1/usage": usage,
      "/api/v1/tasks": tasks,
      "/api/v1/stats": stats,
      "/api/v1/diagnostics": diagnostics,
      "/api/v1/tasks/task-one": {
        task: tasks[0],
        batch: { id: "batch-one", idempotency_key: "fixture", budget_mode: "percentage", weekly_cap_percent: 1, consumed_tokens: 0, allowance_points: 1, consumed_points: 0 },
        runs: [],
        lineage: [tasks[0]],
      },
      "/api/v1/estimates": { cap_assessment: { level: "tight", message: "Confirm a tight estimate." } },
      "/api/v1/plan": {
        summary: "A bounded two-stage implementation plan.",
        tasks: planInputs.map((task) => ({
          title: task.title,
          prompt: `Planned: ${task.prompt}`,
          success_criteria: `${task.title} is verified`,
          difficulty: "standard",
          model: "gpt-6-sol",
          effort: "medium",
        })),
        planner_model: typeof bodyObject.planner_model === "string" ? bodyObject.planner_model : "gpt-6-sol",
        planner_effort: "high",
        cwd: typeof bodyObject.cwd === "string" ? bodyObject.cwd : "/tmp/project",
        timezone: "UTC",
        permission_profile: "restricted",
        weekly_cap_percent: typeof bodyObject.weekly_cap_percent === "number" ? bodyObject.weekly_cap_percent : 1,
      },
      "/api/v1/project-directory": { cwd: request.method() === "POST" ? "/tmp/picked-project" : "/tmp/project" },
      "/api/v1/service/setup": { message: "service installed" },
      "/api/v1/tasks/task-one/cancel": { ...tasks[0], status: "cancelled" },
      "/api/v1/tasks/task-running/stop": tasks[4],
    };
    const createdAttempt = detailTask ? createdAttempts.get(detailTask.id) : undefined;
    const quotaResume = retrySource?.status.startsWith("quota_") && !retrySource.last_error?.startsWith("quota telemetry unavailable:");
    const taskPage = path === "/api/v1/tasks" && url.searchParams.has("page")
      ? { items: tasks, page: Number(url.searchParams.get("page")), page_size: 20, total: tasks.length, total_pages: 1, sort: url.searchParams.get("sort") ?? "newest" }
      : undefined;
    const batchPage = request.method() === "GET" && path === "/api/v1/batches"
      ? { items: batchItems, page: Number(url.searchParams.get("page") ?? 1), page_size: 20, total: batchItems.length, total_pages: 1, sort: url.searchParams.get("sort") ?? "newest" }
      : undefined;
    const beginBatchEdit = request.method() === "POST" && path === "/api/v1/batches/batch-one/edit-session"
      ? { edit_session_id: "batch-edit-fixture", expires_at: 4_070_908_860, batch: batchItems[0].batch, tasks: batchItems[0].tasks }
      : undefined;
    const response = batchPage
      ?? beginBatchEdit
      ?? (request.method() === "POST" && path.endsWith("/edit-session/heartbeat") ? { expires_at: 4_070_908_860 } : undefined)
      ?? (request.method() === "POST" && path.endsWith("/edit-session/cancel") ? { cancelled: true } : undefined)
      ?? (request.method() === "PUT" && path === "/api/v1/batches/batch-one" ? batchItems[0] : undefined)
      ?? (request.method() === "POST" && path === "/api/v1/batches" ? { batch: { id: "batch-created" }, tasks, idempotent_replay: false } : undefined)
      ?? (archivedTask
      ? { task_id: archivedTask.id, archived_at: 4_070_912_400, status: archivedTask.status === "scheduled" ? "cancelled" : archivedTask.status, preserved_runs: 2 }
      : detailTask
      ? { task: detailTask, batch: { id: detailTask.batch_id, idempotency_key: "fixture", budget_mode: "percentage", weekly_cap_percent: 1, consumed_tokens: 0, allowance_points: 1, consumed_points: 0 }, runs: [], lineage: [detailTask, ...(createdAttempt ? [createdAttempt] : [])] }
      : retrySource
        ? {
            source_task_id: retrySource.id,
            attempt_kind: quotaResume ? "quota_resume" : "retry",
            action_label: quotaResume ? "Continue after quota reset" : "Retry as new run",
            attempt_number: 2,
            run_at: 4_070_912_400,
            run_at_iso: "2099-01-01T01:00:00+00:00",
            provider_reset_at: quotaResume ? 4_070_912_400 : null,
            resume_mode: quotaResume ? "context_fallback" : "fresh_session",
            session_available: false,
            task: { ...retrySource, permission_profile: bodyObject.permission_profile ?? retrySource.permission_profile },
            budget: { budget_mode: "percentage", weekly_cap_percent: 1, five_hour_cap_percent: null },
            estimate: { cap_assessment: { level: "fits", message: "Fixture estimate" } },
          }
        : retryConfirm
          ? (() => {
              const isQuotaResume = retryConfirm.status.startsWith("quota_") && !retryConfirm.last_error?.startsWith("quota telemetry unavailable:");
              const task = { ...retryConfirm, permission_profile: bodyObject.permission_profile ?? retryConfirm.permission_profile, id: `${retryConfirm.id}-attempt`, source_task_id: retryConfirm.id, attempt_kind: isQuotaResume ? "quota_resume" : "retry", attempt_number: 2, status: "scheduled" };
              createdAttempts.set(retryConfirm.id, task);
              return { batch: { id: "batch-attempt" }, task, idempotent_replay: false };
            })()
          : taskPage ?? fixtures[path] ?? {});
    await route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify(response) });
  });
  return calls;
}

test("dashboard renders fixture quota, diagnostics, statuses, and safe task detail", async ({ page }) => {
  const calls = await fixtureApi(page);
  await page.goto("/");
  await expect(page.getByText("Five-hour used")).toBeVisible();
  await expect(page.getByText("20.0%")).toBeVisible();
  await expect(page.getByText("1. Fixture task", { exact: true })).toBeVisible();
  await expect(page.getByText("2. Chained fixture task", { exact: true })).toBeVisible();
  await expect(page.getByText("Page 1 of 1 · 6 batches")).toBeVisible();
  expect(calls.some((call) => call.path === "/api/v1/batches" && call.query.includes("sort=newest"))).toBe(true);
  await page.getByRole("button", { name: /^1\. Fixture task / }).click();
  await expect(page.getByText("Transcript contents and Codex session identifiers are not exposed by the UI API.")).toBeVisible();
});

test("theme defaults to dark and persists the light preference", async ({ page }) => {
  await fixtureApi(page);
  await page.goto("/");
  await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
  await expect(page.getByRole("button", { name: "Use dark theme" })).toHaveAttribute("aria-pressed", "true");

  await page.getByRole("button", { name: "Use light theme" }).click();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
  await expect(page.getByRole("button", { name: "Use light theme" })).toHaveAttribute("aria-pressed", "true");

  await page.reload();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
});

test("task lists sort server-side and use 20-row pages", async ({ page }) => {
  const calls = await fixtureApi(page);
  await page.goto("/");
  await page.getByLabel("Sort").selectOption("title_ascending");
  await expect.poll(() => calls.some((call) => call.path === "/api/v1/batches" && call.query.includes("sort=title_ascending"))).toBe(true);
  await page.getByRole("button", { name: "operations" }).click();
  await expect(page.getByText("Page 1 of 1 · 7 tasks")).toBeVisible();
  await expect(page.getByLabel("Sort")).toHaveValue("newest");
});

test("dashboard batch editor pauses start and can replace tasks in the chain", async ({ page }) => {
  const calls = await fixtureApi(page);
  await page.goto("/");
  await page.getByRole("button", { name: "Edit batch…" }).click();
  await expect(page.getByText("Batch editor · start paused")).toBeVisible();
  await expect(page.getByLabel("Batch start date")).toHaveValue("2099-01-01");
  await expect(page.getByLabel("Batch start time (24-hour)")).toHaveValue("00:00");
  const taskTabHeight = await page.getByRole("tab", { name: "1. Fixture task" }).evaluate((element) => element.getBoundingClientRect().height);
  expect(taskTabHeight).toBeLessThan(60);
  await page.getByRole("tab", { name: "2. Chained fixture task" }).click();
  await page.getByRole("button", { name: "Remove task" }).click();
  await page.getByRole("button", { name: "Add task" }).click();
  await page.getByRole("textbox", { name: "Title", exact: true }).fill("Replacement fixture task");
  await page.getByRole("button", { name: "Save batch changes" }).click();
  await expect(page.getByText("Batch changes saved. The edited chain remains in one batch.")).toBeVisible();
  const update = calls.find((call) => call.method === "PUT" && call.path === "/api/v1/batches/batch-one")?.body as { edit_session_id: string; tasks: Array<Record<string, unknown>> };
  expect(update.edit_session_id).toBe("batch-edit-fixture");
  expect(update.tasks).toHaveLength(2);
  expect(update.tasks[0]).toMatchObject({ task_id: "task-one", run_at: "2099-01-01T00:00:00+00:00" });
  expect(update.tasks[1]).toMatchObject({ title: "Replacement fixture task" });
  expect(update.tasks[1]).not.toHaveProperty("task_id");
  expect(update.tasks[1]).not.toHaveProperty("run_at");
});

test("one-task and reordered three-task chains freeze once with no graph shapes", async ({ page }) => {
  const calls = await fixtureApi(page);
  await page.goto("/");
  await page.getByRole("button", { name: "composer" }).click();
  await page.getByRole("button", { name: "Advanced", exact: true }).click();
  const initialDate = await page.getByLabel("Run at date").inputValue();
  const initialTime = await page.getByLabel("Run at time (24-hour)").inputValue();
  expect(Date.parse(`${initialDate}T${initialTime}`)).toBeGreaterThan(Date.now());
  await page.getByLabel("Title").fill("One");
  await page.getByLabel("Prompt").fill("Do one");
  await expect(page.getByLabel("Project directory")).toHaveValue("/tmp/project");
  await expect(page.getByLabel("Weekly percentage points")).toBeVisible();
  await page.getByLabel("Weekly percentage points").fill("4");
  await page.getByLabel("Run at date").fill("2099-01-01");
  await page.getByLabel("Run at time (24-hour)").fill("10:00");
  await page.getByRole("button", { name: "Add" }).click();
  await page.getByRole("button", { name: "Add" }).click();
  await page.getByRole("button", { name: "Move left" }).click();
  await page.getByRole("button", { name: "Review exact payload" }).click();
  await expect(page.getByText("Explicit confirmation")).toBeVisible();
  await page.getByRole("button", { name: "Confirm and schedule once" }).dblclick();
  expect(calls.filter((call) => call.method === "POST" && call.path === "/api/v1/batches")).toHaveLength(1);
  const scheduled = calls.find((call) => call.method === "POST" && call.path === "/api/v1/batches")?.body as { weekly_cap_percent: number; tasks: Array<Record<string, unknown>> };
  expect(scheduled.weekly_cap_percent).toBe(4);
  expect(scheduled.tasks[0]).toHaveProperty("run_at");
  expect(scheduled.tasks[0]).toMatchObject({ permission_profile: "restricted" });
  expect(scheduled.tasks[1]).toMatchObject({ after_previous: true });
  expect(JSON.stringify(scheduled)).not.toContain("edges");
});

test("networked composer requires distinct acknowledgement and cancelled warning does not schedule", async ({ page }) => {
  const calls = await fixtureApi(page);
  await page.goto("/");
  await page.getByRole("button", { name: "composer" }).click();
  await page.getByRole("button", { name: "Advanced", exact: true }).click();
  await page.getByLabel("Title").fill("Network task");
  await page.getByLabel("Prompt").fill("Fetch public documentation");
  await expect(page.getByLabel("Project directory")).toHaveValue("/tmp/project");
  await page.getByLabel("Run at date").fill("2099-01-01");
  await page.getByLabel("Run at time (24-hour)").fill("10:00");
  await page.getByLabel("Permission profile").selectOption("networked");
  await page.getByRole("button", { name: "Review exact payload" }).click();
  const confirmation = page.getByRole("heading", { name: "Review before scheduling" }).locator("..");
  await expect(confirmation.getByText(/Network and web search are enabled/)).toBeVisible();
  const confirm = page.getByRole("button", { name: "Confirm and schedule once" });
  await expect(confirm).toBeDisabled();
  await page.getByRole("button", { name: "Back to edit" }).click();
  expect(calls.filter((call) => call.method === "POST" && call.path === "/api/v1/batches")).toHaveLength(0);

  await page.getByRole("button", { name: "Review exact payload" }).click();
  await page.getByLabel(/I explicitly acknowledge that networked tasks/).check();
  await confirm.click();
  const scheduled = calls.find((call) => call.method === "POST" && call.path === "/api/v1/batches")?.body as Record<string, unknown>;
  expect(scheduled).toMatchObject({ networked_confirmed: true });
  expect((scheduled.tasks as Array<Record<string, unknown>>)[0]).toMatchObject({ permission_profile: "networked" });
});

test("invalid fields block estimate and scheduling", async ({ page }) => {
  const calls = await fixtureApi(page);
  await page.goto("/");
  await page.getByRole("button", { name: "composer" }).click();
  await page.getByRole("button", { name: "Plan with GPT" }).click();
  await expect(page.getByText("Composer needs attention")).toBeVisible();
  expect(calls.filter((call) => call.path === "/api/v1/estimates")).toHaveLength(0);
  expect(calls.filter((call) => call.method === "POST" && call.path === "/api/v1/batches")).toHaveLength(0);
});

test("simple composer keeps chains and schedules only after Codex plan approval", async ({ page }) => {
  const calls = await fixtureApi(page);
  await page.goto("/");
  await page.getByRole("button", { name: "composer" }).click();
  await expect(page.getByLabel("Task name")).toBeVisible();
  await expect(page.getByLabel("Task prompt")).toBeVisible();
  await expect(page.getByLabel("Run at date")).toBeVisible();
  await expect(page.getByLabel("Run at time (24-hour)")).toBeVisible();
  await expect(page.getByLabel("Project directory")).toHaveValue("/tmp/project");
  await expect(page.getByLabel("Weekly percentage points")).toHaveValue("1");
  await expect(page.getByLabel("Planning model")).toHaveValue("gpt-6-sol");
  await expect(page.getByLabel("Permission profile")).toHaveValue("restricted");
  await page.getByRole("button", { name: "Choose folder…" }).click();
  await expect(page.getByLabel("Project directory")).toHaveValue("/tmp/picked-project");
  await page.getByLabel("Weekly percentage points").fill("3");
  await page.getByLabel("Planning model").selectOption("gpt-6-astra");
  await page.getByLabel("Permission profile").selectOption("networked");
  await page.getByLabel("Task name").fill("Migrate settings");
  await page.getByLabel("Task prompt").fill(`Refactor the settings architecture ${"x".repeat(2_000)}`);
  await page.getByLabel("Run at date").fill("2099-01-01");
  await page.getByLabel("Run at time (24-hour)").fill("10:00");
  await page.getByRole("button", { name: "Add" }).click();
  await page.getByLabel("Task name").fill("Verify settings");
  await page.getByLabel("Task prompt").fill("Run focused settings tests");
  await expect(page.getByText("Runs after the previous task succeeds.")).toBeVisible();
  await expect(page.getByLabel("Permission profile")).toHaveValue("networked");
  await expect(page.getByLabel("Linear task canvas")).toBeVisible();
  await page.getByRole("button", { name: "Plan with GPT" }).click();
  await expect(page.getByText(/Planner: gpt-6-astra\/high in Codex Plan mode/)).toBeVisible();
  await expect(page.getByText("A bounded two-stage implementation plan.")).toBeVisible();
  await expect(page.getByText(/gpt-6-sol\/medium\/standard/).first()).toBeVisible();
  await expect.poll(() => page.evaluate(() => document.documentElement.scrollWidth <= document.documentElement.clientWidth)).toBe(true);
  expect(calls.filter((call) => call.method === "POST" && call.path === "/api/v1/batches")).toHaveLength(0);
  const confirm = page.getByRole("button", { name: "Confirm and schedule once" });
  await expect(confirm).toBeDisabled();
  await page.getByLabel(/I explicitly acknowledge that networked tasks/).check();
  await confirm.click();
  const planRequest = calls.find((call) => call.path === "/api/v1/plan")?.body as { cwd: string; weekly_cap_percent: number; planner_model: string; tasks: Array<Record<string, unknown>> };
  expect(planRequest.tasks).toHaveLength(2);
  expect(planRequest).toMatchObject({ cwd: "/tmp/picked-project", weekly_cap_percent: 3, planner_model: "gpt-6-astra" });
  const scheduled = calls.find((call) => call.method === "POST" && call.path === "/api/v1/batches")?.body as { weekly_cap_percent: number; tasks: Array<Record<string, unknown>> };
  expect(scheduled.weekly_cap_percent).toBe(3);
  expect(scheduled.tasks[0]).toMatchObject({ cwd: "/tmp/picked-project", timezone: "UTC", difficulty: "standard", model: "gpt-6-sol", effort: "medium", permission_profile: "networked" });
  expect(scheduled.tasks[1]).toMatchObject({ after_previous: true, model: "gpt-6-sol", effort: "medium", permission_profile: "networked" });
});

test("operations wait for server response and expose confirmations", async ({ page }) => {
  const calls = await fixtureApi(page);
  page.on("dialog", (dialog) => dialog.accept());
  await page.goto("/");
  await page.getByRole("button", { name: "operations" }).click();
  await page.getByText("Fixture task", { exact: true }).click();
  await page.getByRole("button", { name: "Cancel task…" }).click();
  await expect(page.getByText("Task cancelled.")).toBeVisible();
  await page.getByRole("button", { name: "Set up daemon…" }).click();
  await expect(page.getByText("Background service setup completed.")).toBeVisible();
  await page.getByText("Failed fixture task", { exact: true }).click();
  await expect(page.getByRole("button", { name: "Retry as new run" })).toBeVisible();
  await page.getByRole("button", { name: "Remove from task lists…" }).click();
  await expect(page.getByText(/Run metadata was preserved/)).toBeVisible();
  await page.getByText("Running fixture task", { exact: true }).click();
  await expect(page.getByText(/Pause is unavailable/)).toBeVisible();
  await page.getByRole("button", { name: "Stop running task…" }).click();
  await expect(page.getByText(/Stop requested/)).toBeVisible();
  expect(calls.some((call) => call.path === "/api/v1/tasks/task-one/cancel")).toBe(true);
  expect(calls.some((call) => call.path === "/api/v1/tasks/task-running/stop")).toBe(true);
  expect(calls.some((call) => call.path === "/api/v1/service/setup")).toBe(true);
  expect(calls.some((call) => call.method === "DELETE" && call.path === "/api/v1/tasks/task-retry")).toBe(true);
});

test("operations selects and removes multiple tasks without dashboard checkboxes", async ({ page }) => {
  const calls = await fixtureApi(page);
  page.on("dialog", (dialog) => dialog.accept());
  await page.goto("/");
  await expect(page.getByLabel("Select all removable tasks on this page")).toHaveCount(0);

  await page.getByRole("button", { name: "operations" }).click();
  await page.getByLabel("Select Fixture task for removal").check();
  await page.getByLabel("Select Failed fixture task for removal").check();
  await expect(page.getByRole("button", { name: "Remove selected (2)…" })).toBeEnabled();
  await expect(page.getByLabel("Select Running fixture task for removal")).toBeDisabled();
  await page.getByRole("button", { name: "Remove selected (2)…" }).click();

  await expect(page.getByText("2 tasks removed from dashboard and operations. Run metadata was preserved.")).toBeVisible();
  const deletePaths = calls.filter((call) => call.method === "DELETE").map((call) => call.path);
  expect(deletePaths).toHaveLength(2);
  expect(deletePaths).toEqual(expect.arrayContaining([
    "/api/v1/tasks/task-one",
    "/api/v1/tasks/task-retry",
  ]));
});

test("failed retry previews copied fields and duplicate confirmation creates once", async ({ page }) => {
  const calls = await fixtureApi(page);
  await page.goto("/");
  await page.getByText("Failed fixture task").click();
  await expect(page.getByText("#1 · original · restricted", { exact: true })).toBeVisible();
  await page.getByRole("button", { name: "Retry as new run" }).click();
  const retryDate = await page.getByLabel("Retry start date").inputValue();
  const retryTime = await page.getByLabel("Retry start time (24-hour)").inputValue();
  expect(Date.parse(`${retryDate}T${retryTime}`)).toBeGreaterThan(Date.now());
  await page.getByLabel("Retry start date").fill("2099-01-01");
  await page.getByLabel("Retry start time (24-hour)").fill("01:00");
  await page.getByRole("button", { name: "Review new attempt" }).click();
  await expect(page.getByText(/Attempt #2 · retry/)).toBeVisible();
  await expect(page.getByText(/fresh session/)).toBeVisible();
  await page.getByRole("button", { name: "Confirm Retry as new run" }).dblclick();
  expect(calls.filter((call) => call.path === "/api/v1/tasks/task-retry/retry")).toHaveLength(1);
  await expect(page.getByText("task-retry-attempt", { exact: true })).toBeVisible();
});

test("quota resume shows reset fallback and every ineligible state shows no attempt action", async ({ page }) => {
  await fixtureApi(page);
  await page.goto("/");
  await page.getByText("Quota fixture task").click();
  await page.getByRole("button", { name: "Continue after quota reset" }).click();
  await expect(page.getByLabel("Retry start date")).toHaveCount(0);
  await page.getByRole("button", { name: "Review new attempt" }).click();
  await expect(page.getByText(/context fallback/)).toBeVisible();
  for (const title of ["Completed fixture task", "Fixture task", "Running fixture task"]) {
    await page.getByRole("button", { name: new RegExp(`^1\\. ${title} `) }).click();
    await expect(page.getByRole("button", { name: "Retry as new run" })).toHaveCount(0);
    await expect(page.getByRole("button", { name: "Continue after quota reset" })).toHaveCount(0);
  }
  await page.getByRole("button", { name: /^1\. Unavailable quota task / }).click();
  await expect(page.getByRole("button", { name: "Continue after quota reset" })).toHaveCount(0);
  await page.getByRole("button", { name: "Retry as new run" }).click();
  await expect(page.getByLabel("Retry start date")).toBeVisible();
});

test("retry inherits networked profile and requires its distinct acknowledgement", async ({ page }) => {
  const calls = await fixtureApi(page);
  await page.goto("/");
  await page.getByRole("button", { name: /^1\. Quota fixture task / }).click();
  await page.getByRole("button", { name: "Continue after quota reset" }).click();
  await expect(page.getByLabel("Permission profile")).toHaveValue("networked");
  await page.getByRole("button", { name: "Review new attempt" }).click();
  const confirm = page.getByRole("button", { name: "Confirm Continue after quota reset" });
  await expect(confirm).toBeDisabled();
  await page.getByLabel(/I explicitly acknowledge that this task may access the network/).check();
  await confirm.click();
  const body = calls.find((call) => call.path === "/api/v1/tasks/task-quota/retry")?.body;
  expect(body).toMatchObject({ permission_profile: "networked", networked_confirmed: true });
});

test("retry permits changing inherited networked profile to restricted", async ({ page }) => {
  const calls = await fixtureApi(page);
  await page.goto("/");
  await page.getByRole("button", { name: /^1\. Quota fixture task / }).click();
  await page.getByRole("button", { name: "Continue after quota reset" }).click();
  await page.getByLabel("Permission profile").selectOption("restricted");
  await page.getByRole("button", { name: "Review new attempt" }).click();
  await expect(page.getByText(/restricted/)).toBeVisible();
  await page.getByRole("button", { name: "Confirm Continue after quota reset" }).click();
  const body = calls.find((call) => call.path === "/api/v1/tasks/task-quota/retry")?.body as Record<string, unknown>;
  expect(body).toMatchObject({ permission_profile: "restricted" });
  expect(body).not.toHaveProperty("networked_confirmed");
});
