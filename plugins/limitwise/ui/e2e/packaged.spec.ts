import { expect, test, type Page } from "@playwright/test";
import { spawn, spawnSync, type ChildProcessWithoutNullStreams } from "node:child_process";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";

const binary = process.env.LIMITWISE_UI_BINARY;

async function fillDateTime(page: Page, label: string, instant = new Date(Date.now() + 300_000)) {
  const date = `${instant.getFullYear()}-${String(instant.getMonth() + 1).padStart(2, "0")}-${String(instant.getDate()).padStart(2, "0")}`;
  const time = `${String(instant.getHours()).padStart(2, "0")}:${String(instant.getMinutes()).padStart(2, "0")}`;
  await page.getByLabel(`${label} date`).fill(date);
  await page.getByLabel(`${label} time (24-hour)`).fill(time);
}

test.describe("packaged LimitWise UI", () => {
  test.skip(!binary, "LIMITWISE_UI_BINARY is required for packaged-binary E2E");

  test("serves embedded assets, bootstraps once, and shuts down without leaking bearer", async ({ page }) => {
    const home = await mkdtemp(path.join(tmpdir(), "limitwise-packaged-ui-"));
    const fixtureCodex = path.resolve("../tests/fixtures/fake-codex.sh");
    let child: ChildProcessWithoutNullStreams | undefined;
    try {
      child = spawn(binary!, ["ui", "--no-open"], {
        env: {
          ...process.env,
          LIMITWISE_HOME: home,
          XDG_DATA_HOME: home,
          LIMITWISE_CODEX_PATH: fixtureCodex,
          LIMITWISE_DIRECTORY_PICKER_PATH: "/bin/pwd",
          TZ: "UTC",
        },
        stdio: ["ignore", "pipe", "pipe"],
      }) as ChildProcessWithoutNullStreams;
      let stdout = "";
      let stderr = "";
      child.stdout.on("data", (chunk) => { stdout += String(chunk); });
      child.stderr.on("data", (chunk) => { stderr += String(chunk); });
      const launchUrl = await waitForLaunchUrl(child, () => stdout);

      await page.goto(launchUrl);
      await expect(page.getByRole("heading", { name: "LimitWise" })).toBeVisible();
      await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
      await page.getByRole("button", { name: "Use light theme" }).click();
      await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
      await page.reload();
      await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
      await expect(page.getByText("Five-hour used")).toBeVisible();
      await expect(page.getByText("No tasks match this filter")).toBeVisible();
      await page.getByRole("button", { name: "composer" }).click();
      await expect(page.getByLabel("Task name")).toBeVisible();
      await expect(page.getByLabel("Project directory")).not.toHaveValue("");
      await page.getByRole("button", { name: "Choose folder…" }).click();
      await expect(page.getByText(/Project directory selected:/)).toBeVisible();
      await expect(page.getByLabel("Weekly percentage points")).toHaveValue("1");
      await page.getByLabel("Task name").fill("Migrate settings");
      await page.getByLabel("Task prompt").fill("Refactor the settings architecture");
      await fillDateTime(page, "Run at");
      await page.getByRole("button", { name: "Plan with GPT" }).click();
      await expect(page.getByText(/Planner: gpt-6-sol\/high in Codex Plan mode/)).toBeVisible();
      await expect(page.getByText(/gpt-6-sol\/medium\/standard/).first()).toBeVisible();
      await expect.poll(() => page.evaluate(() => location.hash)).toBe("");
      const bearer = await page.evaluate(() => sessionStorage.getItem("limitwise-bearer") ?? "");
      expect(bearer).toMatch(/^[a-f0-9]{64}$/);
      expect(stdout).not.toContain(bearer);
      expect(stderr).not.toContain(bearer);

      const secondBootstrap = await page.request.post(`${new URL(launchUrl).origin}/api/v1/bootstrap`, {
        headers: { Origin: new URL(launchUrl).origin, "Content-Type": "application/json" },
        data: { bootstrap: new URL(launchUrl).hash.replace("#bootstrap=", "") },
      });
      expect(secondBootstrap.status()).toBe(401);

      const exit = waitForExit(child);
      child.kill("SIGTERM");
      await expect(exit).resolves.toBe(0);
      child = undefined;
    } finally {
      if (child && child.exitCode === null) child.kill("SIGTERM");
      await rm(home, { recursive: true, force: true });
    }
  });

  test("groups and edits a real chained batch", async ({ page }) => {
    const home = await mkdtemp(path.join(tmpdir(), "limitwise-packaged-pages-"));
    const fixtureCodex = path.resolve("../tests/fixtures/fake-codex.sh");
    const environment: NodeJS.ProcessEnv = {
      ...process.env,
      LIMITWISE_HOME: home,
      XDG_DATA_HOME: home,
      LIMITWISE_CODEX_PATH: fixtureCodex,
      TZ: "UTC",
    };
    let child: ChildProcessWithoutNullStreams | undefined;
    try {
      const tasks = Array.from({ length: 21 }, (_, index) => ({
        title: `Paged task ${String(index + 1).padStart(2, "0")}`,
        prompt: "remain scheduled",
        success_criteria: "appears once",
        cwd: path.resolve(".."),
        ...(index === 0
          ? { run_at: new Date(Date.now() + 300_000).toISOString() }
          : { after_previous: true }),
        timezone: "UTC",
        difficulty: "simple",
      }));
      const created = mcpCall(binary!, environment, "schedule_batch", {
        idempotency_key: "packaged-pages",
        budget_mode: "percentage",
        weekly_cap_percent: 1,
        tasks,
      });

      child = spawn(binary!, ["ui", "--no-open"], {
        env: environment,
        stdio: ["ignore", "pipe", "pipe"],
      }) as ChildProcessWithoutNullStreams;
      let stdout = "";
      child.stdout.on("data", (chunk) => { stdout += String(chunk); });
      const launchUrl = await waitForLaunchUrl(child, () => stdout);
      await page.goto(launchUrl);
      await expect(page.getByText("Page 1 of 1 · 1 batches")).toBeVisible();
      await expect(page.locator(".task-row")).toHaveCount(21);
      await page.getByLabel("Sort").selectOption("title_ascending");
      await expect(page.getByText(/^1\. Paged task 01$/)).toBeVisible();
      await expect(page.getByText(/^21\. Paged task 21$/)).toBeVisible();
      await page.getByRole("button", { name: "Edit batch…" }).click();
      await expect(page.getByText("Batch editor · start paused")).toBeVisible();
      await page.getByRole("tab", { name: "21. Paged task 21" }).click();
      await page.getByRole("button", { name: "Remove task" }).click();
      await page.getByRole("button", { name: "Save batch changes" }).click();
      await expect(page.getByText("Batch changes saved. The edited chain remains in one batch.")).toBeVisible();
      await expect(page.locator(".task-row")).toHaveCount(20);
      await expect(page.getByText("Paged task 21", { exact: true })).toHaveCount(0);
      const archived = mcpCall(binary!, environment, "get_task_status", { task_id: created.tasks[20].id });
      expect(archived.task.status).toBe("cancelled");
      expect(archived.runs).toHaveLength(0);

      const exit = waitForExit(child);
      child.kill("SIGTERM");
      await expect(exit).resolves.toBe(0);
      child = undefined;
    } finally {
      if (child && child.exitCode === null) child.kill("SIGTERM");
      await rm(home, { recursive: true, force: true });
    }
  });

  test("runs retry and quota-resume task-detail flows against the embedded backend", async ({ page }) => {
    const home = await mkdtemp(path.join(tmpdir(), "limitwise-packaged-c04-"));
    const fixtureCodex = path.resolve("../tests/fixtures/fake-codex.sh");
    const environment: NodeJS.ProcessEnv = {
      ...process.env,
      LIMITWISE_HOME: home,
      XDG_DATA_HOME: home,
      LIMITWISE_CODEX_PATH: fixtureCodex,
      LIMITWISE_POLL_SECONDS: "1",
      TZ: "UTC",
    };
    let child: ChildProcessWithoutNullStreams | undefined;
    try {
      const failedSource = mcpCall(binary!, environment, "schedule_batch", {
        idempotency_key: "packaged-c04-failed-source",
        budget_mode: "percentage",
        weekly_cap_percent: 1,
        tasks: [{
          title: "Packaged failed task",
          prompt: "fail once",
          success_criteria: "retry succeeds",
          cwd: path.resolve(".."),
          run_at: new Date(Date.now() + 1_000).toISOString(),
          timezone: "UTC",
          difficulty: "simple",
        }],
      }).tasks[0];
      await new Promise((resolve) => setTimeout(resolve, 1_500));
      runBinary(binary!, ["daemon", "--once"], {
        ...environment,
        LIMITWISE_FAKE_EXEC_SCENARIO: "fail",
      });

      const resetAt = Math.floor(Date.now() / 1_000) + 10;
      const quotaSource = mcpCall(binary!, environment, "schedule_batch", {
        idempotency_key: "packaged-c04-quota-source",
        budget_mode: "percentage",
        weekly_cap_percent: 1,
        tasks: [{
          title: "Packaged quota task",
          prompt: "interrupt once",
          success_criteria: "resume succeeds",
          cwd: path.resolve(".."),
          run_at: new Date(Date.now() + 1_000).toISOString(),
          timezone: "UTC",
          difficulty: "standard",
        }],
      }).tasks[0];
      await new Promise((resolve) => setTimeout(resolve, 1_500));
      runBinary(binary!, ["daemon", "--once"], {
        ...environment,
        LIMITWISE_FAKE_EXEC_SCENARIO: "quota_interrupt",
        LIMITWISE_FAKE_QUOTA_SCENARIO: "interrupt",
        LIMITWISE_FAKE_RESET_AT: String(resetAt),
      });

      child = spawn(binary!, ["ui", "--no-open"], {
        env: environment,
        stdio: ["ignore", "pipe", "pipe"],
      }) as ChildProcessWithoutNullStreams;
      let stdout = "";
      child.stdout.on("data", (chunk) => { stdout += String(chunk); });
      const launchUrl = await waitForLaunchUrl(child, () => stdout);
      await page.goto(launchUrl);

      await page.getByRole("button", { name: /^1\. Packaged failed task\b/ }).click();
      await page.getByRole("button", { name: "Retry as new run" }).click();
      await fillDateTime(page, "Retry start");
      await page.getByRole("button", { name: "Review new attempt" }).click();
      await page.getByRole("button", { name: "Confirm Retry as new run" }).dblclick();
      await expect(page.getByText(/Created attempt task-/)).toBeVisible();

      await page.getByRole("button", { name: /^1\. Packaged quota task\b/ }).click();
      await page.getByRole("button", { name: "Continue after quota reset" }).click();
      await expect(page.getByLabel("Retry start date")).toHaveCount(0);
      await page.getByRole("button", { name: "Review new attempt" }).click();
      await page.getByRole("button", { name: "Confirm Continue after quota reset" }).click();
      await expect(page.getByText(/Created attempt task-/)).toBeVisible();

      const failedStatus = mcpCall(binary!, environment, "get_task_status", { task_id: failedSource.id });
      const quotaStatus = mcpCall(binary!, environment, "get_task_status", { task_id: quotaSource.id });
      expect(failedStatus.task.status).toBe("failed");
      expect(failedStatus.lineage).toHaveLength(2);
      expect(quotaStatus.task.status).toBe("quota_interrupted");
      expect(quotaStatus.lineage).toHaveLength(2);

      const exit = waitForExit(child);
      child.kill("SIGTERM");
      await expect(exit).resolves.toBe(0);
      child = undefined;
    } finally {
      if (child && child.exitCode === null) child.kill("SIGTERM");
      await rm(home, { recursive: true, force: true });
    }
  });

  test("offers fresh retry when quota telemetry captured no snapshot", async ({ page }) => {
    const home = await mkdtemp(path.join(tmpdir(), "limitwise-packaged-no-snapshot-"));
    const fixtureCodex = path.resolve("../tests/fixtures/fake-codex.sh");
    const environment: NodeJS.ProcessEnv = {
      ...process.env,
      LIMITWISE_HOME: home,
      XDG_DATA_HOME: home,
      LIMITWISE_CODEX_PATH: fixtureCodex,
      TZ: "UTC",
    };
    let child: ChildProcessWithoutNullStreams | undefined;
    try {
      const task = mcpCall(binary!, environment, "schedule_batch", {
        idempotency_key: "packaged-no-snapshot",
        budget_mode: "percentage",
        weekly_cap_percent: 1,
        tasks: [{
          title: "Packaged unavailable telemetry task",
          prompt: "retry after repairing Codex",
          success_criteria: "fresh retry is offered",
          cwd: path.resolve(".."),
          run_at: new Date(Date.now() + 1_000).toISOString(),
          timezone: "UTC",
          difficulty: "simple",
        }],
      }).tasks[0];
      await new Promise((resolve) => setTimeout(resolve, 1_500));
      runBinary(binary!, ["daemon", "--once"], {
        ...environment,
        LIMITWISE_CODEX_PATH: path.join(home, "missing-codex"),
      });
      expect(mcpCall(binary!, environment, "get_task_status", { task_id: task.id }).task.status).toBe("quota_skipped");

      child = spawn(binary!, ["ui", "--no-open"], {
        env: environment,
        stdio: ["ignore", "pipe", "pipe"],
      }) as ChildProcessWithoutNullStreams;
      let stdout = "";
      child.stdout.on("data", (chunk) => { stdout += String(chunk); });
      const launchUrl = await waitForLaunchUrl(child, () => stdout);
      await page.goto(launchUrl);
      await page.getByRole("button", { name: /^1\. Packaged unavailable telemetry task\b/ }).click();
      await expect(page.getByRole("button", { name: "Continue after quota reset" })).toHaveCount(0);
      await page.getByRole("button", { name: "Retry as new run" }).click();
      await expect(page.getByLabel("Retry start date")).toBeVisible();

      const exit = waitForExit(child);
      child.kill("SIGTERM");
      await expect(exit).resolves.toBe(0);
      child = undefined;
    } finally {
      if (child && child.exitCode === null) child.kill("SIGTERM");
      await rm(home, { recursive: true, force: true });
    }
  });

  test("stops a running Codex process and exposes retry", async ({ page }) => {
    const home = await mkdtemp(path.join(tmpdir(), "limitwise-packaged-stop-"));
    const fixtureCodex = path.resolve("../tests/fixtures/fake-codex.sh");
    const environment: NodeJS.ProcessEnv = {
      ...process.env,
      LIMITWISE_HOME: home,
      XDG_DATA_HOME: home,
      LIMITWISE_CODEX_PATH: fixtureCodex,
      LIMITWISE_FAKE_EXEC_SCENARIO: "quota_interrupt",
      LIMITWISE_POLL_SECONDS: "1",
      TZ: "UTC",
    };
    let ui: ChildProcessWithoutNullStreams | undefined;
    let daemon: ChildProcessWithoutNullStreams | undefined;
    try {
      const task = mcpCall(binary!, environment, "schedule_batch", {
        idempotency_key: "packaged-running-stop",
        budget_mode: "percentage",
        weekly_cap_percent: 1,
        tasks: [{
          title: "Packaged running task",
          prompt: "wait until stopped",
          success_criteria: "becomes cancelled",
          cwd: path.resolve(".."),
          run_at: new Date(Date.now() + 1_000).toISOString(),
          timezone: "UTC",
          difficulty: "simple",
        }],
      }).tasks[0];
      await new Promise((resolve) => setTimeout(resolve, 1_500));
      daemon = spawn(binary!, ["daemon", "--once"], {
        env: environment,
        stdio: ["ignore", "pipe", "pipe"],
      }) as ChildProcessWithoutNullStreams;
      await waitForTaskStatus(binary!, environment, task.id, "running");

      ui = spawn(binary!, ["ui", "--no-open"], {
        env: environment,
        stdio: ["ignore", "pipe", "pipe"],
      }) as ChildProcessWithoutNullStreams;
      let stdout = "";
      ui.stdout.on("data", (chunk) => { stdout += String(chunk); });
      const launchUrl = await waitForLaunchUrl(ui, () => stdout);
      await page.goto(launchUrl);
      await page.getByRole("button", { name: "operations" }).click();
      await page.getByText("Packaged running task", { exact: true }).click();
      page.on("dialog", (dialog) => dialog.accept());
      await page.getByRole("button", { name: "Stop running task…" }).click();
      await expect(page.getByText(/Stop requested/)).toBeVisible();
      await waitForTaskStatus(binary!, environment, task.id, "cancelled");
      await expect(page.getByRole("button", { name: "Retry as new run" })).toBeVisible({ timeout: 5_000 });
      await expect(waitForExit(daemon)).resolves.toBe(0);
      daemon = undefined;

      const status = mcpCall(binary!, environment, "get_task_status", { task_id: task.id });
      expect(status.task.status).toBe("cancelled");
      expect(status.runs[0].status).toBe("cancelled");

      const exit = waitForExit(ui);
      ui.kill("SIGTERM");
      await expect(exit).resolves.toBe(0);
      ui = undefined;
    } finally {
      if (ui && ui.exitCode === null) ui.kill("SIGTERM");
      if (daemon && daemon.exitCode === null) daemon.kill("SIGTERM");
      await rm(home, { recursive: true, force: true });
    }
  });
});

function mcpCall(binary: string, environment: NodeJS.ProcessEnv, name: string, args: Record<string, unknown>) {
  const request = JSON.stringify({ jsonrpc: "2.0", id: 1, method: "tools/call", params: { name, arguments: args } });
  const result = spawnSync(binary, ["mcp"], { env: environment, input: `${request}\n`, encoding: "utf8" });
  if (result.status !== 0) throw new Error(`MCP ${name} exited ${result.status}: ${result.stderr}`);
  const response = JSON.parse(result.stdout);
  if (response.result?.isError) throw new Error(`MCP ${name} failed: ${response.result.content?.[0]?.text}`);
  return response.result.structuredContent;
}

function runBinary(binary: string, args: string[], environment: NodeJS.ProcessEnv) {
  const result = spawnSync(binary, args, { env: environment, encoding: "utf8" });
  if (result.status !== 0) throw new Error(`${args.join(" ")} exited ${result.status}: ${result.stderr}`);
}

async function waitForLaunchUrl(
  child: ChildProcessWithoutNullStreams,
  output: () => string,
): Promise<string> {
  const deadline = Date.now() + 10_000;
  while (Date.now() < deadline) {
    const match = output().match(/LimitWise UI: (http:\/\/127\.0\.0\.1:\d+\/#bootstrap=[a-f0-9]{64})/);
    if (match) return match[1];
    if (child.exitCode !== null) throw new Error(`limitwise ui exited ${child.exitCode} before publishing a URL`);
    await new Promise((resolve) => setTimeout(resolve, 25));
  }
  throw new Error("timed out waiting for the packaged UI launch URL");
}

function waitForExit(child: ChildProcessWithoutNullStreams): Promise<number | null> {
  if (child.exitCode !== null) return Promise.resolve(child.exitCode);
  return new Promise((resolve) => child.once("exit", resolve));
}

async function waitForTaskStatus(
  executable: string,
  environment: NodeJS.ProcessEnv,
  taskId: string,
  expected: string,
) {
  const deadline = Date.now() + 10_000;
  while (Date.now() < deadline) {
    const status = mcpCall(executable, environment, "get_task_status", { task_id: taskId });
    if (status.task.status === expected) return;
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  throw new Error(`timed out waiting for task ${taskId} to become ${expected}`);
}
