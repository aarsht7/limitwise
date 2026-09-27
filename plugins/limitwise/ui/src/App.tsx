import React, { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  Background,
  Controls,
  ReactFlow,
  applyNodeChanges,
  type Connection,
  type Edge,
  type Node,
  type NodeChange,
} from "@xyflow/react";
import { ApiError, api, bootstrapSession } from "./api";
import {
  ACTIVE_STATUSES,
  ALL_STATUSES,
  batchEditTasks,
  batchStartTimeWarning,
  defaultRunAt,
  draftsForBatchEdit,
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
import type {
  ArchiveTaskResult,
  AttemptCreateResult,
  BatchEditSession,
  BatchGroup,
  BatchPage,
  ComposerBudget,
  DiagnosticReport,
  DraftTask,
  Effort,
  Model,
  ModelCatalog,
  PermissionProfile,
  PlannedBatch,
  RetryTaskOptions,
  RetryTaskPreview,
  Task,
  TaskDetail,
  TaskPage,
  TaskSort,
  TaskStatusName,
  UsageSnapshot,
  UsageStats,
} from "./types";

type Tab = "dashboard" | "composer" | "operations";
type ViewState = "loading" | "ready" | "empty" | "stale" | "error";
type Theme = "dark" | "light";
type ComposerMode = "simple" | "advanced";

const THEME_STORAGE_KEY = "limitwise-theme";

interface NodeData extends Record<string, unknown> {
  label: string;
  status: string;
}

interface DashboardData {
  usage: UsageSnapshot;
  batches: BatchPage;
  stats: UsageStats;
  diagnostics: DiagnosticReport;
}

export function App() {
  const [token, setToken] = useState("");
  const [sessionError, setSessionError] = useState("");
  const [tab, setTab] = useState<Tab>("dashboard");
  const [refresh, setRefresh] = useState(0);
  const [theme, setTheme] = useState<Theme>(readTheme);
  const [catalog, setCatalog] = useState<ModelCatalog | null>(null);
  const [catalogError, setCatalogError] = useState("");

  useEffect(() => {
    document.documentElement.dataset.theme = theme;
    document.documentElement.style.colorScheme = theme;
    window.localStorage.setItem(THEME_STORAGE_KEY, theme);
  }, [theme]);

  useEffect(() => {
    void bootstrapSession().then(setToken).catch((error: unknown) => {
      setSessionError(error instanceof Error ? error.message : "Could not authenticate the local UI.");
    });
  }, []);

  useEffect(() => {
    if (!token) return;
    void api<ModelCatalog>(token, "/models")
      .then((value) => {
        setCatalog(value);
        setCatalogError("");
      })
      .catch((error: unknown) => {
        setCatalogError(error instanceof Error ? error.message : "Could not load Codex models.");
      });
  }, [token]);

  if (sessionError) {
    return <StatePanel state="error" title="Local session unavailable" detail={sessionError} />;
  }
  if (!token) {
    return <StatePanel state="loading" title="Connecting to LimitWise" />;
  }
  if (!catalog && catalogError) {
    return <StatePanel state="error" title="Codex model catalog unavailable" detail={catalogError} />;
  }
  if (!catalog) {
    return <StatePanel state="loading" title="Loading Codex models and reasoning efforts" />;
  }

  const changed = () => setRefresh((value) => value + 1);

  return (
    <div className="app-shell">
      <header className="topbar">
        <div className="brand-lockup">
          <span className="brand-mark" aria-hidden="true">L</span>
          <div>
            <h1>LimitWise</h1>
            <span>Local task scheduler</span>
          </div>
        </div>
        <div className="topbar-actions">
          <nav aria-label="Primary">
            {(["dashboard", "composer", "operations"] as const).map((item) => (
              <button
                className={tab === item ? "nav-button active" : "nav-button"}
                key={item}
                onClick={() => setTab(item)}
              >
                {item}
              </button>
            ))}
          </nav>
          <div className="theme-switch" role="group" aria-label="Color theme">
            <button
              className={theme === "dark" ? "active" : ""}
              aria-pressed={theme === "dark"}
              aria-label="Use dark theme"
              title="Dark theme"
              onClick={() => setTheme("dark")}
            >
              Dark
            </button>
            <button
              className={theme === "light" ? "active" : ""}
              aria-pressed={theme === "light"}
              aria-label="Use light theme"
              title="Light theme"
              onClick={() => setTheme("light")}
            >
              Light
            </button>
          </div>
        </div>
      </header>
      <main>
        {catalog.warning && <StatePanel state="stale" title="Using bundled model catalog" detail={catalog.warning} />}
        {tab === "dashboard" && <Dashboard token={token} refresh={refresh} catalog={catalog} />}
        {tab === "composer" && (
          <Composer
            token={token}
            catalog={catalog}
            onScheduled={() => {
              changed();
              setTab("dashboard");
            }}
          />
        )}
        {tab === "operations" && (
          <Operations
            token={token}
            catalog={catalog}
            refresh={refresh}
            onChanged={changed}
          />
        )}
      </main>
      <footer>Loopback only · bearer authenticated · transcript contents are never served</footer>
    </div>
  );
}

function readTheme(): Theme {
  if (typeof window === "undefined") return "dark";
  return window.localStorage.getItem(THEME_STORAGE_KEY) === "light" ? "light" : "dark";
}

export function StatePanel({
  state,
  title,
  detail,
}: {
  state: ViewState;
  title: string;
  detail?: string;
}) {
  return (
    <section className={`state-panel ${state}`} data-state={state}>
      <strong>{title}</strong>
      {detail && <p>{detail}</p>}
    </section>
  );
}

export function StatusBadge({ status }: { status: TaskStatusName | string }) {
  return <span className={`status status-${status}`}>{status.replaceAll("_", " ")}</span>;
}

function Dashboard({ token, refresh, catalog }: { token: string; refresh: number; catalog: ModelCatalog }) {
  const [data, setData] = useState<DashboardData | null>(null);
  const [error, setError] = useState("");
  const [filter, setFilter] = useState("all");
  const [sort, setSort] = useState<TaskSort>("newest");
  const [page, setPage] = useState(1);
  const [selected, setSelected] = useState("");
  const [detail, setDetail] = useState<TaskDetail | null>(null);
  const [detailError, setDetailError] = useState("");
  const [notice, setNotice] = useState("");
  const [editSession, setEditSession] = useState<BatchEditSession | null>(null);
  const [editPending, setEditPending] = useState(false);

  const load = useCallback(async () => {
    try {
      const [usage, batches, stats, diagnostics] = await Promise.all([
        api<UsageSnapshot>(token, "/usage"),
        api<BatchPage>(token, batchPagePath(page, sort, filter)),
        api<UsageStats>(token, "/stats"),
        api<DiagnosticReport>(token, "/diagnostics"),
      ]);
      setData({ usage, batches, stats, diagnostics });
      setError("");
    } catch (reason) {
      setError(message(reason));
    }
  }, [filter, page, sort, token]);

  useEffect(() => {
    void load();
  }, [load, refresh]);

  const hasActiveTasks = data?.batches.items.some((batch) =>
    batch.tasks.some((task) => ACTIVE_STATUSES.has(task.status))) ?? false;
  useEffect(() => {
    if (!hasActiveTasks) return;
    const poll = () => {
      if (document.visibilityState === "visible") void load();
    };
    const timer = window.setInterval(poll, 5_000);
    document.addEventListener("visibilitychange", poll);
    return () => {
      window.clearInterval(timer);
      document.removeEventListener("visibilitychange", poll);
    };
  }, [hasActiveTasks, load]);

  useEffect(() => {
    if (!selected) {
      setDetail(null);
      return;
    }
    setDetail(null);
    setDetailError("");
    void api<TaskDetail>(token, `/tasks/${encodeURIComponent(selected)}`)
      .then(setDetail)
      .catch((reason) => setDetailError(message(reason)));
  }, [selected, token, refresh]);

  if (!data && !error) return <StatePanel state="loading" title="Loading local scheduler state" />;
  if (!data && error) return <StatePanel state="error" title="Dashboard unavailable" detail={error} />;
  if (!data) return null;
  const stale = Date.now() / 1000 - data.usage.captured_at > 60;
  const batches = data.batches.items;

  const beginEdit = async (group: BatchGroup) => {
    setEditPending(true);
    setError("");
    try {
      const session = await api<BatchEditSession>(token, `/batches/${encodeURIComponent(group.batch.id)}/edit-session`, {
        method: "POST",
        body: "{}",
      });
      setSelected("");
      setDetail(null);
      setEditSession(session);
    } catch (reason) {
      setError(message(reason));
    } finally {
      setEditPending(false);
    }
  };

  return (
    <div className="stack">
      {error && <StatePanel state="error" title="Refresh failed; showing last local snapshot" detail={error} />}
      {notice && <StatePanel state="ready" title={notice} />}
      {stale && (
        <StatePanel
          state="stale"
          title="Quota telemetry is stale"
          detail="Scheduling remains blocked until a fresh quota snapshot is available."
        />
      )}
      {data.usage.warnings?.map((warning) => (
        <StatePanel
          key={warning}
          state="stale"
          title="Five-hour quota telemetry is unavailable"
          detail={warning}
        />
      ))}
      <section className="card-grid" aria-label="Quota and health">
        <Metric
          label="Five-hour used"
          value={data.usage.five_hour ? `${data.usage.five_hour.used_percent.toFixed(1)}%` : "Unavailable"}
          note={data.usage.five_hour ? `resets ${formatTime(data.usage.five_hour.resets_at)}` : "weekly/token budget remains active"}
        />
        <Metric
          label="Weekly used"
          value={`${data.usage.weekly.used_percent.toFixed(1)}%`}
          note={`resets ${formatTime(data.usage.weekly.resets_at)}`}
        />
        <Metric
          label="Diagnostics"
          value={data.diagnostics.overall}
          note={`${data.diagnostics.checks.length} checks`}
        />
        <Metric
          label="Seven-day usage"
          value={`${data.stats.last_week.tokens_used.toLocaleString()} tokens`}
          note={`${data.stats.last_week.run_count} runs · ${data.stats.last_week.tokens_unavailable_runs} unavailable`}
        />
      </section>
      <section className="panel">
        <div className="section-heading">
          <div>
            <span className="eyebrow">Local records</span>
            <h2>Task batches</h2>
          </div>
          <div className="list-controls">
            <label>
              Status
              <select value={filter} onChange={(event) => { setFilter(event.target.value); setPage(1); setSelected(""); }}>
                <option value="all">all</option>
                {ALL_STATUSES.map((status) => (
                  <option key={status}>{status}</option>
                ))}
              </select>
            </label>
            <SortSelect value={sort} onChange={(value) => { setSort(value); setPage(1); setSelected(""); }} />
          </div>
        </div>
        {!batches.length ? (
          <StatePanel state="empty" title="No tasks match this filter" />
        ) : (
          <div className="task-layout">
            <div className="batch-list">
              {batches.map((group) => (
                <article className="batch-group" key={group.batch.id}>
                  <div className="batch-heading">
                    <span>
                      <strong>Batch · {group.tasks.length} task{group.tasks.length === 1 ? "" : "s"}</strong>
                      <small>{group.batch.id}</small>
                    </span>
                    {group.editing ? (
                      <span className="editing-badge">Editing — start paused</span>
                    ) : group.editable ? (
                      <button onClick={() => void beginEdit(group)} disabled={editPending || Boolean(editSession)}>
                        Edit batch…
                      </button>
                    ) : null}
                  </div>
                  <div className="task-list batch-tasks">
                    {group.tasks.map((task, index) => (
                      <button
                        className={selected === task.id ? "task-row selected" : "task-row"}
                        key={task.id}
                        onClick={() => { setEditSession(null); setSelected(task.id); }}
                      >
                        <span>
                          <strong>{index + 1}. {task.title}</strong>
                          <small>{index === 0 ? formatTime(task.run_at) : "after previous task"}</small>
                        </span>
                        <StatusBadge status={task.status} />
                      </button>
                    ))}
                  </div>
                </article>
              ))}
              <Pagination value={data.batches} noun="batches" onChange={(value) => { setPage(value); setSelected(""); setEditSession(null); }} />
            </div>
            {editSession ? (
              <BatchEditor
                key={editSession.edit_session_id}
                token={token}
                catalog={catalog}
                session={editSession}
                onClose={(saved) => {
                  setEditSession(null);
                  if (saved) setNotice("Batch changes saved. The edited chain remains in one batch.");
                  void load();
                }}
              />
            ) : (
              <TaskDetailPanel
                detail={detail}
                error={detailError}
                selected={selected}
                token={token}
                catalog={catalog}
                onCreated={() => {
                  void load();
                  void api<TaskDetail>(token, `/tasks/${encodeURIComponent(selected)}`)
                    .then(setDetail)
                    .catch((reason) => setDetailError(message(reason)));
                }}
                onArchived={(result) => {
                  setSelected("");
                  setDetail(null);
                  setNotice(`Task removed from lists. ${result.preserved_runs} run record${result.preserved_runs === 1 ? "" : "s"} preserved.`);
                  void load();
                }}
              />
            )}
          </div>
        )}
      </section>
    </div>
  );
}

function BatchEditor({
  token,
  catalog,
  session,
  onClose,
}: {
  token: string;
  catalog: ModelCatalog;
  session: BatchEditSession;
  onClose: (saved: boolean) => void;
}) {
  const [tasks, setTasks] = useState<DraftTask[]>(() => draftsForBatchEdit(session.tasks));
  const [selected, setSelected] = useState(0);
  const [error, setError] = useState("");
  const [pending, setPending] = useState(false);
  const [networkedAcknowledged, setNetworkedAcknowledged] = useState(false);
  const [clock, setClock] = useState(Date.now());
  const released = useRef(false);
  const batchId = session.batch.id;
  const editSessionId = session.edit_session_id;

  useEffect(() => {
    const heartbeat = () => {
      setClock(Date.now());
      void api<{ expires_at: number }>(token, `/batches/${encodeURIComponent(batchId)}/edit-session/heartbeat`, {
        method: "POST",
        body: JSON.stringify({ edit_session_id: editSessionId }),
      }).catch((reason) => setError(message(reason)));
    };
    const timer = window.setInterval(heartbeat, 15_000);
    return () => {
      window.clearInterval(timer);
      if (!released.current) {
        void api(token, `/batches/${encodeURIComponent(batchId)}/edit-session/cancel`, {
          method: "POST",
          body: JSON.stringify({ edit_session_id: editSessionId }),
        }).catch(() => undefined);
      }
    };
  }, [batchId, editSessionId, token]);

  const commitTasks = (next: DraftTask[], nextSelected = selected) => {
    setTasks(next);
    setSelected(Math.min(nextSelected, next.length - 1));
    setError("");
    setNetworkedAcknowledged(false);
  };
  const updateTask = (field: keyof DraftTask, value: string) => {
    commitTasks(tasks.map((task, index) => index === selected ? { ...task, [field]: value } : task));
  };
  const updateTaskModel = (value: string) => {
    commitTasks(tasks.map((task, index) =>
      index === selected ? { ...task, ...routeSelection(catalog, value, task.effort) } : task
    ));
  };
  const add = () => {
    const next = [...tasks, newDraft(tasks.at(-1))];
    commitTasks(next, next.length - 1);
  };
  const remove = () => {
    if (tasks.length === 1) {
      setError("A batch must contain at least one task.");
      return;
    }
    commitTasks(tasks.filter((_, index) => index !== selected));
  };
  const move = (offset: number) => {
    const destination = selected + offset;
    if (destination < 0 || destination >= tasks.length) return;
    const next = [...tasks];
    [next[selected], next[destination]] = [next[destination], next[selected]];
    commitTasks(next, destination);
  };
  const cancel = async () => {
    setPending(true);
    try {
      await api(token, `/batches/${encodeURIComponent(batchId)}/edit-session/cancel`, {
        method: "POST",
        body: JSON.stringify({ edit_session_id: editSessionId }),
      });
      released.current = true;
      onClose(false);
    } catch (reason) {
      const detail = message(reason);
      if (detail.includes("not active") || detail.includes("expired")) {
        released.current = true;
        onClose(false);
      } else {
        setError(detail);
      }
    } finally {
      setPending(false);
    }
  };
  const save = async () => {
    const passed = batchStartTimeWarning(tasks);
    if (passed) {
      setError(passed);
      return;
    }
    const budget: ComposerBudget = {
      mode: session.batch.budget_mode,
      weeklyCap: String(session.batch.weekly_cap_percent ?? 1),
      tokenCap: String(session.batch.token_cap ?? 1),
      fiveHourCap: session.batch.five_hour_cap_percent == null ? "" : String(session.batch.five_hour_cap_percent),
    };
    const validation = validateComposer(tasks, budget);
    if (validation.length) {
      setError(validation.join(" "));
      return;
    }
    setPending(true);
    setError("");
    try {
      const usesNetwork = tasks.some((task) => task.permission_profile === "networked");
      await api<BatchGroup>(token, `/batches/${encodeURIComponent(batchId)}`, {
        method: "PUT",
        body: JSON.stringify({
          edit_session_id: editSessionId,
          tasks: batchEditTasks(tasks),
          ...(usesNetwork ? { networked_confirmed: networkedAcknowledged } : {}),
        }),
      });
      released.current = true;
      onClose(true);
    } catch (reason) {
      setError(message(reason));
    } finally {
      setPending(false);
    }
  };

  const task = tasks[selected];
  const passedWarning = batchStartTimeWarning(tasks, clock);
  const usesNetwork = tasks.some((item) => item.permission_profile === "networked");
  return (
    <section className="panel batch-editor">
      <div className="section-heading">
        <div>
          <span className="eyebrow">Batch editor · start paused</span>
          <h3>Edit task chain</h3>
          <p>The first task cannot start while this editor stays connected. Saving replaces this pending linear chain atomically.</p>
        </div>
        <code>{batchId}</code>
      </div>
      {passedWarning && <StatePanel state="stale" title={passedWarning} />}
      {error && <StatePanel state="error" title="Batch changes were not saved" detail={error} />}
      <div className="batch-editor-tasks" role="tablist" aria-label="Tasks in batch">
        {tasks.map((item, index) => (
          <button
            role="tab"
            aria-selected={selected === index}
            className={selected === index ? "selected" : ""}
            key={item.localId}
            onClick={() => setSelected(index)}
            disabled={pending}
          >
            {index + 1}. {item.title || "Untitled task"}
          </button>
        ))}
      </div>
      <div className="button-row">
        <button onClick={add} disabled={pending}>Add task</button>
        <button onClick={remove} disabled={pending}>Remove task</button>
        <button onClick={() => move(-1)} disabled={pending || selected === 0}>Move earlier</button>
        <button onClick={() => move(1)} disabled={pending || selected === tasks.length - 1}>Move later</button>
      </div>
      {task && (
        <fieldset className="inspector embedded-inspector" disabled={pending}>
          <legend>Task {selected + 1}</legend>
          <Field label="Title" value={task.title} onChange={(value) => updateTask("title", value)} />
          <TextArea label="Prompt" value={task.prompt} onChange={(value) => updateTask("prompt", value)} />
          <TextArea label="Success criteria" value={task.success_criteria} onChange={(value) => updateTask("success_criteria", value)} />
          <Field label="Absolute cwd" value={task.cwd} onChange={(value) => updateTask("cwd", value)} />
          {selected === 0 ? (
            <DateTimeField label="Batch start" value={task.run_at} timezone={task.timezone} onChange={(value) => updateTask("run_at", value)} />
          ) : <p className="locked-value">Runs after the previous task succeeds.</p>}
          <Field label="IANA timezone" value={task.timezone} onChange={(value) => updateTask("timezone", value)} />
          <Select label="Difficulty" value={task.difficulty} options={["simple", "standard", "complex", "exceptional"]} onChange={(value) => updateTask("difficulty", value)} />
          <Select label="Model" value={task.model} options={modelOptions(catalog)} onChange={updateTaskModel} />
          <Select label="Effort" value={task.effort} options={effortOptions(catalog, task.model)} onChange={(value) => updateTask("effort", value)} />
          <Select label="Permission profile" value={task.permission_profile} options={["restricted", "networked"]} onChange={(value) => updateTask("permission_profile", value)} />
          <p className="muted"><PermissionRisk profile={task.permission_profile} /></p>
        </fieldset>
      )}
      {usesNetwork && (
        <label className="risk-acknowledgement">
          <input type="checkbox" checked={networkedAcknowledged} onChange={(event) => setNetworkedAcknowledged(event.target.checked)} />
          I explicitly acknowledge that networked tasks may access the network and use web search. External apps, interactive approval, danger-full-access, and non-workspace-write sandboxes remain prohibited.
        </label>
      )}
      <div className="button-row">
        <button onClick={() => void cancel()} disabled={pending}>Discard changes</button>
        <button className="primary" onClick={() => void save()} disabled={pending || Boolean(passedWarning) || (usesNetwork && !networkedAcknowledged)}>
          {pending ? "Saving…" : "Save batch changes"}
        </button>
      </div>
    </section>
  );
}

function Metric({ label, value, note }: { label: string; value: string; note: string }) {
  return (
    <article className="metric-card">
      <span>{label}</span>
      <strong>{value}</strong>
      <small>{note}</small>
    </article>
  );
}

function TaskDetailPanel({
  detail,
  error,
  selected,
  token,
  catalog,
  onCreated,
  onArchived,
}: {
  detail: TaskDetail | null;
  error: string;
  selected: string;
  token: string;
  catalog: ModelCatalog;
  onCreated: () => void;
  onArchived: (result: ArchiveTaskResult) => void;
}) {
  if (!selected) return <StatePanel state="empty" title="Select a task for details" />;
  if (error) return <StatePanel state="error" title="Task detail unavailable" detail={error} />;
  if (!detail) return <StatePanel state="loading" title="Loading task detail" />;
  const { task, batch, runs } = detail;
  const archive = async () => {
    const pending = task.status === "scheduled" ? " This also cancels the pending task." : "";
    if (!window.confirm(`Remove “${task.title}” from task lists?${pending} Run metadata stays stored.`)) return;
    try {
      const result = await api<ArchiveTaskResult>(token, `/tasks/${encodeURIComponent(task.id)}`, { method: "DELETE" });
      onArchived(result);
    } catch (reason) {
      window.alert(message(reason));
    }
  };
  return (
    <article className="detail-card">
      <div className="section-heading">
        <h3>{task.title}</h3>
        <StatusBadge status={task.status} />
      </div>
      <dl className="detail-grid">
        <dt>ID</dt><dd>{task.id}</dd>
        <dt>Project</dt><dd>{task.cwd}</dd>
        <dt>Schedule</dt><dd>{task.run_at_iso} ({task.timezone})</dd>
        <dt>Route</dt><dd>{task.model} · {task.effort} · {task.difficulty}</dd>
        <dt>Permission profile</dt><dd>{task.permission_profile} · <PermissionRisk profile={task.permission_profile} /></dd>
        <dt>Dependency</dt><dd>{task.depends_on_task_id ?? "none"} · {task.dependency_type}</dd>
        <dt>Attempt</dt><dd>#{task.attempt_number} · {task.attempt_kind ?? "original"} · source {task.source_task_id ?? "none"}</dd>
        <dt>Budget</dt><dd>{batch.budget_mode === "tokens" ? `${batch.token_cap} tokens` : `${batch.weekly_cap_percent}% weekly`} · five-hour {batch.five_hour_cap_percent ?? "global reserve only"}</dd>
        <dt>Prompt</dt><dd className="prewrap">{task.prompt}</dd>
        <dt>Success</dt><dd className="prewrap">{task.success_criteria || "not specified"}</dd>
        <dt>Last error</dt><dd>{task.last_error ?? "none"}</dd>
      </dl>
      <h4>Runs</h4>
      {!runs.length ? <p className="muted">No runs yet.</p> : runs.map((run) => (
        <div className="run-row" key={run.id}>
          <StatusBadge status={run.status} />
          <span>{formatTime(run.started_at)} · {run.tokens_used ?? "tokens unavailable"} · {task.permission_profile}</span>
          <span>Transcript path: {run.transcript_path ?? "unavailable"}</span>
          {run.error && <span className="error-text">{run.error}</span>}
        </div>
      ))}
      <h4>Attempt lineage</h4>
      <div className="lineage-list">
        {detail.lineage.map((item) => (
          <div className={item.id === task.id ? "lineage-row current" : "lineage-row"} key={item.id}>
            <span>#{item.attempt_number} · {item.attempt_kind ?? "original"} · {item.permission_profile}</span>
            <code>{item.id}</code>
            <StatusBadge status={item.status} />
          </div>
        ))}
      </div>
      <RetryAttemptPanel token={token} source={task} catalog={catalog} onCreated={onCreated} />
      <button className="danger secondary-action" onClick={() => void archive()} disabled={task.status === "running"} title={task.status === "running" ? "Stop this task before removing it" : undefined}>
        Remove from task lists…
      </button>
      <p className="privacy-note">Transcript contents and Codex session identifiers are not exposed by the UI API.</p>
    </article>
  );
}

function RetryAttemptPanel({
  token,
  source,
  catalog,
  onCreated,
}: {
  token: string;
  source: Task;
  catalog: ModelCatalog;
  onCreated: () => void;
}) {
  const action = eligibleAttemptAction(source.status, source.last_error);
  const initialRoute = routeSelection(catalog, source.model, source.effort);
  const [open, setOpen] = useState(false);
  const [budgetMode, setBudgetMode] = useState<ComposerBudget["mode"]>("percentage");
  const [weeklyCap, setWeeklyCap] = useState("1");
  const [tokenCap, setTokenCap] = useState("100000");
  const [fiveHourCap, setFiveHourCap] = useState("");
  const [runAt, setRunAt] = useState(() => defaultRunAt());
  const [model, setModel] = useState<Model>(initialRoute.model);
  const [effort, setEffort] = useState<Effort>(initialRoute.effort);
  const [permissionProfile, setPermissionProfile] = useState<PermissionProfile>(source.permission_profile);
  const [networkedAcknowledged, setNetworkedAcknowledged] = useState(false);
  const [preview, setPreview] = useState<RetryTaskPreview | null>(null);
  const [idempotencyKey, setIdempotencyKey] = useState("");
  const [created, setCreated] = useState<AttemptCreateResult | null>(null);
  const [error, setError] = useState("");
  const [pending, setPending] = useState(false);
  const pendingRef = useRef(false);

  useEffect(() => {
    setOpen(false);
    setPreview(null);
    setCreated(null);
    setError("");
    const route = routeSelection(catalog, source.model, source.effort);
    setModel(route.model);
    setEffort(route.effort);
    setPermissionProfile(source.permission_profile);
    setNetworkedAcknowledged(false);
    setRunAt(defaultRunAt());
  }, [catalog, source.id, source.model, source.effort]);

  const changeModel = (value: string) => {
    const route = routeSelection(catalog, value, effort);
    setModel(route.model);
    setEffort(route.effort);
  };

  if (!action) return null;

  const options = (): RetryTaskOptions => ({
    budget_mode: budgetMode,
    ...(budgetMode === "percentage"
      ? { weekly_cap_percent: Number(weeklyCap) }
      : { token_cap: Number(tokenCap) }),
    ...(fiveHourCap.trim() ? { five_hour_cap_percent: Number(fiveHourCap) } : {}),
    ...(action.kind === "retry" ? { run_at: runAt } : {}),
    timezone: source.timezone,
    model,
    effort,
    permission_profile: permissionProfile,
  });

  const review = async () => {
    setPending(true);
    setError("");
    setCreated(null);
    try {
      const result = await api<RetryTaskPreview>(
        token,
        `/tasks/${encodeURIComponent(source.id)}/retry-preview`,
        { method: "POST", body: JSON.stringify(options()) },
      );
      setPreview(result);
      setIdempotencyKey(`ui-attempt-${crypto.randomUUID()}`);
    } catch (reason) {
      setError(message(reason));
    } finally {
      setPending(false);
    }
  };

  const confirm = async () => {
    if (!preview || !idempotencyKey || pendingRef.current) return;
    pendingRef.current = true;
    setPending(true);
    setError("");
    try {
      const result = await api<AttemptCreateResult>(
        token,
        `/tasks/${encodeURIComponent(source.id)}/retry`,
        {
          method: "POST",
          body: JSON.stringify({
            ...options(),
            idempotency_key: idempotencyKey,
            confirmed: true,
            ...(permissionProfile === "networked" ? { networked_confirmed: networkedAcknowledged } : {}),
          }),
        },
      );
      setCreated(result);
      onCreated();
    } catch (reason) {
      setError(message(reason));
    } finally {
      pendingRef.current = false;
      setPending(false);
    }
  };

  return (
    <section className="attempt-card">
      {!open ? (
        <button className="primary" onClick={() => setOpen(true)}>{action.label}</button>
      ) : (
        <div className="stack">
          <div className="section-heading">
            <div>
              <span className="eyebrow">New immutable attempt</span>
              <h4>{action.label}</h4>
            </div>
            <button onClick={() => { setOpen(false); setPreview(null); setError(""); }} disabled={pending}>Close</button>
          </div>
          {error && <StatePanel state="error" title="Attempt rejected" detail={error} />}
          {created && (
            <StatePanel
              state="ready"
              title={`Created attempt ${created.task.id}`}
              detail={`Source ${source.id} remains unchanged. ${created.idempotent_replay ? "Duplicate confirmation returned the existing attempt." : "A fresh batch and task were created."}`}
            />
          )}
          {!preview ? (
            <>
              <div className="attempt-fields">
                <Select label="Fresh budget mode" value={budgetMode} options={["percentage", "tokens"]} onChange={(value) => setBudgetMode(value as ComposerBudget["mode"])} disabled={pending} />
                {budgetMode === "percentage" ? (
                  <Field label="Fresh weekly percentage points" value={weeklyCap} onChange={setWeeklyCap} disabled={pending} />
                ) : (
                  <Field label="Fresh total token cap" value={tokenCap} onChange={setTokenCap} disabled={pending} />
                )}
                <Field label="Optional five-hour percentage points" value={fiveHourCap} onChange={setFiveHourCap} disabled={pending} />
                {action.kind === "retry" && <DateTimeField label="Retry start" value={runAt} timezone={source.timezone} onChange={setRunAt} disabled={pending} />}
                <Select label="Model" value={model} options={modelOptions(catalog)} onChange={changeModel} disabled={pending} />
                <Select label="Effort" value={effort} options={effortOptions(catalog, model)} onChange={(value) => setEffort(value as Effort)} disabled={pending} />
                <Select label="Permission profile" value={permissionProfile} options={["restricted", "networked"]} onChange={(value) => { setPermissionProfile(value as PermissionProfile); setNetworkedAcknowledged(false); }} disabled={pending} />
              </div>
              <p className="muted"><PermissionRisk profile={permissionProfile} /> Retry defaults to the source profile ({source.permission_profile}) and may be changed here.</p>
              <p className="muted">The global rolling five-hour reserve remains 10% whenever telemetry is available. Preview does not write.</p>
              <button className="primary" onClick={() => void review()} disabled={pending}>
                {pending ? "Preparing read-only preview…" : "Review new attempt"}
              </button>
            </>
          ) : (
            <div className="confirmation">
              <p>Attempt #{preview.attempt_number} · {preview.attempt_kind} · runs {preview.run_at_iso}.</p>
              <p>Provider reset: {preview.provider_reset_at ? formatTime(preview.provider_reset_at) : "not applicable"}.</p>
              <p>Session behavior: {preview.resume_mode.replaceAll("_", " ")}.</p>
              <p>Copied fields: {preview.task.title} · {preview.task.cwd} · {preview.task.difficulty} · {preview.task.model}/{preview.task.effort} · {preview.task.permission_profile}.</p>
              <p><PermissionRisk profile={preview.task.permission_profile} /></p>
              <p>Fresh budget: {preview.budget.budget_mode === "tokens" ? `${preview.budget.token_cap} tokens` : `${preview.budget.weekly_cap_percent}% weekly`} · five-hour {preview.budget.five_hour_cap_percent ?? "global reserve only"}.</p>
              <p>Current estimate: {JSON.stringify(preview.estimate)}</p>
              <p>Confirmation creates a new task and batch. Source task and run history remain immutable.</p>
              {preview.task.permission_profile === "networked" && (
                <label className="risk-acknowledgement">
                  <input type="checkbox" checked={networkedAcknowledged} onChange={(event) => setNetworkedAcknowledged(event.target.checked)} />
                  I explicitly acknowledge that this task may access the network and use web search. External apps, interactive approval, danger-full-access, and non-workspace-write sandboxes remain prohibited.
                </label>
              )}
              <div className="button-row">
                <button onClick={() => { setPreview(null); setCreated(null); }} disabled={pending}>Back</button>
                <button className="primary" onClick={() => void confirm()} disabled={pending || Boolean(created) || (preview.task.permission_profile === "networked" && !networkedAcknowledged)}>
                  {pending ? "Creating once…" : created ? "Attempt created" : `Confirm ${action.label}`}
                </button>
              </div>
            </div>
          )}
        </div>
      )}
    </section>
  );
}

function Composer({
  token,
  catalog,
  onScheduled,
}: {
  token: string;
  catalog: ModelCatalog;
  onScheduled: () => void;
}) {
  const initialTasks = useMemo(() => {
    const draft = newDraft();
    return [{ ...draft, ...routeSelection(catalog, draft.model, draft.effort) }];
  }, [catalog]);
  const [mode, setMode] = useState<ComposerMode>("simple");
  const [tasks, setTasks] = useState<DraftTask[]>(initialTasks);
  const [nodes, setNodes] = useState<Array<Node<NodeData>>>(() => nodesFor(initialTasks));
  const [selected, setSelected] = useState(0);
  const [budget, setBudget] = useState<ComposerBudget>({
    mode: "percentage",
    weeklyCap: "1",
    tokenCap: "100000",
    fiveHourCap: "",
  });
  const [plannerModel, setPlannerModel] = useState<Model>(() => defaultModel(catalog, "gpt-6-sol"));
  const [notice, setNotice] = useState("");
  const [errors, setErrors] = useState<string[]>([]);
  const [review, setReview] = useState<{
    payload: Readonly<Record<string, unknown>>;
    estimate: Record<string, unknown>;
    usage: UsageSnapshot;
    tasks: DraftTask[];
    plan?: PlannedBatch;
  } | null>(null);
  const [pending, setPending] = useState(false);
  const [projectLoading, setProjectLoading] = useState(true);
  const [pickingDirectory, setPickingDirectory] = useState(false);
  const [networkedAcknowledged, setNetworkedAcknowledged] = useState(false);
  const pendingRef = useRef(false);
  const locked = pending || pickingDirectory || projectLoading || review !== null;
  const edges: Edge[] = useMemo(
    () => tasks.slice(1).map((task, index) => ({ id: `edge-${task.localId}`, source: tasks[index].localId, target: task.localId, animated: true })),
    [tasks],
  );

  useEffect(() => {
    let active = true;
    api<{ cwd: string }>(token, "/project-directory")
      .then(({ cwd }) => {
        if (active) setTasks((current) => current.map((item) => item.cwd ? item : { ...item, cwd }));
      })
      .catch((reason) => {
        if (active) setErrors([`Project directory is unavailable: ${message(reason)}`]);
      })
      .finally(() => {
        if (active) setProjectLoading(false);
      });
    return () => { active = false; };
  }, [token]);

  const commitTasks = (next: DraftTask[], nextSelected?: number) => {
    if (locked) return;
    setTasks(next);
    setNodes((current) => nodesFor(next, current));
    setSelected((value) => Math.min(nextSelected ?? value, next.length - 1));
    setReview(null);
    setNetworkedAcknowledged(false);
  };
  const updateTask = (field: keyof DraftTask, value: string) => {
    commitTasks(tasks.map((task, index) => index === selected ? { ...task, [field]: value } : task));
  };
  const updateTaskModel = (value: string) => {
    commitTasks(tasks.map((task, index) =>
      index === selected ? { ...task, ...routeSelection(catalog, value, task.effort) } : task
    ));
  };
  const updateProjectDirectory = (cwd: string) => {
    commitTasks(tasks.map((item) => ({ ...item, cwd })));
  };
  const browseProjectDirectory = async () => {
    if (locked) return;
    setPickingDirectory(true);
    try {
      const selected = await api<{ cwd: string }>(token, "/project-directory", {
        method: "POST",
        body: JSON.stringify({ confirmed: true }),
      });
      setTasks((current) => current.map((item) => ({ ...item, cwd: selected.cwd })));
      setNotice(`Project directory selected: ${selected.cwd}`);
      setErrors([]);
    } catch (reason) {
      setErrors([`Folder selection failed: ${message(reason)}`]);
    } finally {
      setPickingDirectory(false);
    }
  };
  const add = () => {
    const next = [...tasks, newDraft(tasks.at(-1))];
    commitTasks(next, next.length - 1);
  };
  const duplicate = () => commitTasks(
    [...tasks.slice(0, selected + 1), newDraft(tasks[selected]), ...tasks.slice(selected + 1)],
    selected + 1,
  );
  const remove = () => {
    if (tasks.length === 1) return setNotice("A linear batch needs at least one task.");
    commitTasks(tasks.filter((_, index) => index !== selected));
  };
  const move = (offset: number) => {
    const destination = selected + offset;
    if (destination < 0 || destination >= tasks.length) return;
    const next = [...tasks];
    [next[selected], next[destination]] = [next[destination], next[selected]];
    setSelected(destination);
    commitTasks(next);
  };
  const nodeChanges = (changes: NodeChange<Node<NodeData>>[]) => {
    if (!locked) setNodes((current) => applyNodeChanges(changes, current));
  };
  const reorderFromCanvas = () => {
    if (locked) return;
    const order = [...nodes].sort((left, right) => left.position.x - right.position.x);
    const byId = new Map(tasks.map((task) => [task.localId, task]));
    commitTasks(order.map((node) => byId.get(node.id)).filter((task): task is DraftTask => Boolean(task)));
  };
  const rejectGraphShape = (_connection?: Connection) => {
    setNotice("LimitWise v1 accepts one linear chain only. Branches, joins, and cycles are rejected.");
  };

  const prepareReview = async () => {
    const validation = mode === "simple"
      ? validateSimpleComposer(tasks, budget)
      : validateComposer(tasks, budget);
    setErrors(validation);
    if (validation.length) return;
    setPending(true);
    try {
      let preparedTasks = tasks;
      let plan: PlannedBatch | undefined;
      if (mode === "simple") {
        plan = await api<PlannedBatch>(token, "/plan", {
          method: "POST",
          body: JSON.stringify({
            tasks: tasks.map(({ title, prompt }) => ({ title: title.trim(), prompt })),
            cwd: tasks[0].cwd,
            weekly_cap_percent: Number(budget.weeklyCap),
            planner_model: plannerModel,
          }),
        });
        preparedTasks = tasks.map((draft, index) => ({
          ...draft,
          title: plan!.tasks[index].title,
          prompt: plan!.tasks[index].prompt,
          success_criteria: plan!.tasks[index].success_criteria,
          cwd: plan!.cwd,
          run_at: index === 0 ? runAtForTimezone(draft.run_at, plan!.timezone) : draft.run_at,
          timezone: plan!.timezone,
          difficulty: plan!.tasks[index].difficulty,
          model: plan!.tasks[index].model,
          effort: plan!.tasks[index].effort,
          permission_profile: draft.permission_profile,
          continue_from_task_id: "",
        }));
        const plannedValidation = validateComposer(preparedTasks, budget);
        if (plannedValidation.length) {
          setErrors(plannedValidation);
          return;
        }
      }
      const usage = await api<UsageSnapshot>(token, "/usage");
      const schedule = schedulePayload(preparedTasks, budget);
      const estimate = await api<Record<string, unknown>>(token, "/estimates", {
        method: "POST",
        body: JSON.stringify({
          ...estimatePayload(preparedTasks, budget),
          schedule: { idempotency_key: "ui-validation-only", ...schedule },
        }),
      });
      const payload = await freezePayload(schedule);
      setReview({ payload, estimate, usage, tasks: preparedTasks, plan });
      setNotice("");
    } catch (reason) {
      setErrors([`Confirmation is blocked: ${message(reason)}`]);
    } finally {
      setPending(false);
    }
  };

  const confirmSchedule = async () => {
    if (!review || pendingRef.current) return;
    pendingRef.current = true;
    setPending(true);
    try {
      const usesNetwork = review.tasks.some((item) => item.permission_profile === "networked");
      await api(token, "/batches", {
        method: "POST",
        body: JSON.stringify({
          ...review.payload,
          ...(usesNetwork ? { networked_confirmed: networkedAcknowledged } : {}),
        }),
      });
      onScheduled();
    } catch (reason) {
      setErrors([message(reason)]);
    } finally {
      pendingRef.current = false;
      setPending(false);
    }
  };

  const task = tasks[selected];
  return (
    <div className="stack">
      <section className="panel composer-heading">
        <div>
          <span className="eyebrow">Task composer</span>
          <h2>{mode === "simple" ? "Simple planned task chain" : "Advanced task chain"}</h2>
          <p>{mode === "simple"
            ? "Build the chain, ask Codex to plan it, then schedule only after you approve the plan."
            : <>Node order is execution order. Only the first node has a time; every later node uses <code>after_previous: true</code>.</>}</p>
        </div>
        <div className="composer-actions">
          <div className="theme-switch mode-switch" role="group" aria-label="Composer mode">
            <button className={mode === "simple" ? "active" : ""} aria-pressed={mode === "simple"} onClick={() => setMode("simple")} disabled={locked}>Simple</button>
            <button className={mode === "advanced" ? "active" : ""} aria-pressed={mode === "advanced"} onClick={() => setMode("advanced")} disabled={locked}>Advanced</button>
          </div>
          <div className="button-row">
            <button onClick={add} disabled={locked}>Add</button>
            <button onClick={duplicate} disabled={locked}>Duplicate</button>
            <button onClick={remove} disabled={locked}>Delete</button>
            <button onClick={() => move(-1)} disabled={locked || selected === 0}>Move left</button>
            <button onClick={() => move(1)} disabled={locked || selected === tasks.length - 1}>Move right</button>
          </div>
        </div>
      </section>
      {notice && <StatePanel state="stale" title={notice} />}
      {errors.length > 0 && (
        <StatePanel state="error" title="Composer needs attention" detail={errors.join(" ")} />
      )}
      <section className="panel project-panel">
        <div>
          <span className="eyebrow">Project</span>
          <h3>Working directory</h3>
        </div>
        <Field
          label="Project directory"
          value={task?.cwd ?? ""}
          onChange={updateProjectDirectory}
          disabled={locked || projectLoading}
        />
        <button onClick={() => void browseProjectDirectory()} disabled={locked || projectLoading}>
          {pickingDirectory ? "Opening folder picker…" : "Choose folder…"}
        </button>
      </section>
      <section className="composer-grid">
        <div className="flow-panel" aria-label="Linear task canvas">
          <ReactFlow
            nodes={nodes}
            edges={edges}
            onNodesChange={nodeChanges}
            onNodeClick={(_, node) => setSelected(tasks.findIndex((item) => item.localId === node.id))}
            onNodeDragStop={reorderFromCanvas}
            onConnect={rejectGraphShape}
            nodesConnectable={!locked}
            nodesDraggable={!locked}
            fitView
          >
            <Background />
            <Controls showInteractive={false} />
          </ReactFlow>
        </div>
        {task && (mode === "simple" ? (
          <fieldset className="panel inspector" disabled={locked}>
            <legend>Task {selected + 1}</legend>
            <Field label="Task name" value={task.title} onChange={(value) => updateTask("title", value)} />
            <TextArea label="Task prompt" value={task.prompt} onChange={(value) => updateTask("prompt", value)} />
            {selected === 0 ? (
              <DateTimeField label="Run at" value={task.run_at} timezone={task.timezone} onChange={(value) => updateTask("run_at", value)} />
            ) : <p className="locked-value">Runs after the previous task succeeds.</p>}
            <p className="muted">Codex plans this step before anything is scheduled.</p>
            <Select
              label="Permission profile"
              value={task.permission_profile}
              options={["restricted", "networked"]}
              onChange={(value) => updateTask("permission_profile", value as PermissionProfile)}
            />
            <p className="muted"><PermissionRisk profile={task.permission_profile} /></p>
          </fieldset>
        ) : (
          <fieldset className="panel inspector" disabled={locked}>
            <legend>Task {selected + 1}</legend>
            <Field label="Title" value={task.title} onChange={(value) => updateTask("title", value)} />
            <TextArea label="Prompt" value={task.prompt} onChange={(value) => updateTask("prompt", value)} />
            <TextArea label="Success criteria" value={task.success_criteria} onChange={(value) => updateTask("success_criteria", value)} />
            {selected === 0 ? (
              <DateTimeField label="Run at" value={task.run_at} timezone={task.timezone} onChange={(value) => updateTask("run_at", value)} />
            ) : <p className="locked-value">after_previous: true</p>}
            <Field label="IANA timezone" value={task.timezone} onChange={(value) => updateTask("timezone", value)} />
            <Select label="Difficulty" value={task.difficulty} options={["simple", "standard", "complex", "exceptional"]} onChange={(value) => updateTask("difficulty", value)} />
            <Select label="Model" value={task.model} options={modelOptions(catalog)} onChange={updateTaskModel} />
            <Select label="Effort" value={task.effort} options={effortOptions(catalog, task.model)} onChange={(value) => updateTask("effort", value as Effort)} />
            <Select label="Permission profile" value={task.permission_profile} options={["restricted", "networked"]} onChange={(value) => updateTask("permission_profile", value as PermissionProfile)} />
            <p className="muted"><PermissionRisk profile={task.permission_profile} /></p>
          </fieldset>
        ))}
      </section>
      {mode === "simple" && <section className="panel planner-note">
        <div>
          <span className="eyebrow">Codex Plan mode</span>
          <h3>Plan first, schedule second</h3>
          <p>The selected model plans with <code>high</code> reasoning and assigns each step a bounded difficulty, execution model, and effort.</p>
          <p>Planning stays read-only. Each task keeps the execution permission profile selected in its inspector.</p>
          <p>Planning itself uses Codex quota. Nothing is scheduled until you review and confirm. When a five-hour quota window is available, the global 10% reserve still applies.</p>
        </div>
        <Select
          label="Planning model"
          value={plannerModel}
          options={modelOptions(catalog)}
          onChange={(value) => setPlannerModel(value as Model)}
          disabled={locked}
        />
      </section>}
      <section className="panel budget-panel">
        <div>
          <span className="eyebrow">Hard limits</span>
          <h3>Required weekly limit</h3>
          <p>Choose the total-weekly percentage points available to this batch. When a five-hour quota window is available, its global 10% reserve still applies.</p>
        </div>
        <Field label="Weekly percentage points" value={budget.weeklyCap} onChange={(weeklyCap) => setBudget({ ...budget, mode: "percentage", weeklyCap, fiveHourCap: "" })} disabled={locked} />
      </section>
      {!review ? (
        <button className="primary action" onClick={() => void prepareReview()} disabled={locked}>
          {pending ? (mode === "simple" ? "Planning with Codex…" : "Checking quota and estimate…") : (mode === "simple" ? "Plan with GPT" : "Review exact payload")}
        </button>
      ) : (
        <section className="panel confirmation">
          <span className="eyebrow">Explicit confirmation</span>
          <h3>Review before scheduling</h3>
          {review.plan && <>
            <p>Planner: {review.plan.planner_model}/{review.plan.planner_effort} in Codex Plan mode. Project: <code>{review.plan.cwd}</code>.</p>
            <p>Weekly execution limit: {review.plan.weekly_cap_percent}%.</p>
            <p>{review.plan.summary}</p>
            <ol className="plan-list">
              {review.plan.tasks.map((item, index) => (
                <li key={`${index}-${item.title}`}>
                  <strong>{item.title}</strong> — {item.prompt}<br />
                  <span className="muted">Done when: {item.success_criteria} · {item.model}/{item.effort}/{item.difficulty}</span>
                </li>
              ))}
            </ol>
          </>}
          <p>Quota: five-hour {review.usage.five_hour ? `${review.usage.five_hour.used_percent}% used` : "unavailable (weekly/token budget remains active)"}; weekly {review.usage.weekly.used_percent}% used.</p>
          {review.usage.warnings?.map((warning) => <p className="muted" key={warning}>Warning: {warning}.</p>)}
          <p>Route: {review.tasks.map((item) => `${item.model}/${item.effort}/${item.difficulty}`).join(" → ")}.</p>
          <p>Permission profiles: {review.tasks.map((item) => item.permission_profile).join(" → ")}.</p>
          {review.tasks.some((item) => item.permission_profile === "networked") ? (
            <>
              <p><PermissionRisk profile="networked" /></p>
              <label className="risk-acknowledgement">
                <input type="checkbox" checked={networkedAcknowledged} onChange={(event) => setNetworkedAcknowledged(event.target.checked)} />
                I explicitly acknowledge that networked tasks may access the network and use web search. External apps, interactive approval, danger-full-access, and non-workspace-write sandboxes remain prohibited.
              </label>
            </>
          ) : <p><PermissionRisk profile="restricted" /></p>}
          <p>Estimate: {JSON.stringify(review.estimate)}</p>
          <pre>{JSON.stringify(review.payload, null, 2)}</pre>
          <div className="button-row">
            <button onClick={() => setReview(null)} disabled={pending}>{review.plan ? "Revise tasks" : "Back to edit"}</button>
            <button className="primary" onClick={() => void confirmSchedule()} disabled={pending || (review.tasks.some((item) => item.permission_profile === "networked") && !networkedAcknowledged)}>
              {pending ? "Scheduling…" : "Confirm and schedule once"}
            </button>
          </div>
        </section>
      )}
    </div>
  );
}

function nodesFor(tasks: DraftTask[], current: Array<Node<NodeData>> = []): Array<Node<NodeData>> {
  const positions = new Map(current.map((node) => [node.id, node.position]));
  return tasks.map((task, index) => ({
    id: task.localId,
    position: positions.get(task.localId) ?? { x: index * 260, y: 80 },
    data: { label: task.title || `Task ${index + 1}`, status: index === 0 ? "scheduled time" : "after previous" },
    className: "flow-node",
  }));
}

function Operations({
  token,
  catalog,
  refresh,
  onChanged,
}: {
  token: string;
  catalog: ModelCatalog;
  refresh: number;
  onChanged: () => void;
}) {
  const [tasks, setTasks] = useState<TaskPage>(emptyTaskPage());
  const [filter, setFilter] = useState("all");
  const [sort, setSort] = useState<TaskSort>("newest");
  const [page, setPage] = useState(1);
  const [selectedId, setSelectedId] = useState("");
  const [selectedTaskIds, setSelectedTaskIds] = useState<string[]>([]);
  const [changes, setChanges] = useState<Record<string, string>>({});
  const [error, setError] = useState("");
  const [messageText, setMessageText] = useState("");
  const [pending, setPending] = useState(false);
  const [updateNetworkedAcknowledged, setUpdateNetworkedAcknowledged] = useState(false);

  const load = useCallback(async () => {
    try {
      setTasks(await api<TaskPage>(token, taskPagePath(page, sort, filter)));
      setError("");
    } catch (reason) {
      setError(message(reason));
    }
  }, [filter, page, sort, token]);
  useEffect(() => { void load(); }, [load, refresh]);
  const hasRunningTask = tasks.items.some((task) => task.status === "running");
  useEffect(() => {
    if (!hasRunningTask) return;
    const timer = window.setInterval(() => { void load(); }, 2_000);
    return () => window.clearInterval(timer);
  }, [hasRunningTask, load]);
  useEffect(() => {
    const removableIds = new Set(tasks.items
      .filter((task) => task.status !== "running")
      .map((task) => task.id));
    setSelectedTaskIds((current) => {
      const next = current.filter((taskId) => removableIds.has(taskId));
      return next.length === current.length ? current : next;
    });
  }, [tasks.items]);
  const selected = tasks.items.find((task) => task.id === selectedId);
  useEffect(() => {
    if (!selected) return;
    const route = routeSelection(catalog, selected.model, selected.effort);
    setChanges({
      title: selected.title,
      prompt: selected.prompt,
      success_criteria: selected.success_criteria,
      cwd: selected.cwd,
      run_at: selected.run_at_iso,
      timezone: selected.timezone,
      difficulty: selected.difficulty,
      model: route.model,
      effort: route.effort,
      permission_profile: selected.permission_profile,
    });
    setUpdateNetworkedAcknowledged(false);
  }, [catalog, selected]);

  const changePendingModel = (value: string) => {
    const route = routeSelection(catalog, value, changes.effort);
    setChanges({ ...changes, model: route.model, effort: route.effort });
  };

  const mutate = async (action: () => Promise<unknown>, success: string, afterSuccess?: () => void) => {
    setPending(true);
    setError("");
    setMessageText("");
    try {
      await action();
      afterSuccess?.();
      await load();
      onChanged();
      setMessageText(success);
    } catch (reason) {
      setError(message(reason));
    } finally {
      setPending(false);
    }
  };

  const update = () => {
    if (!selected) return;
    void mutate(
      () => api(token, `/tasks/${encodeURIComponent(selected.id)}`, {
        method: "PATCH",
        body: JSON.stringify({
          ...changes,
          ...(changes.permission_profile === "networked" ? { networked_confirmed: updateNetworkedAcknowledged } : {}),
        }),
      }),
      "Task updated from current server state.",
    );
  };
  const cancel = () => {
    if (!selected || !window.confirm(`Cancel scheduled task “${selected.title}”?`)) return;
    void mutate(
      () => api(token, `/tasks/${encodeURIComponent(selected.id)}/cancel`, { method: "POST", body: "{}" }),
      "Task cancelled.",
    );
  };
  const stop = () => {
    if (!selected || !window.confirm(`Stop running task “${selected.title}”?`)) return;
    void mutate(
      () => api(token, `/tasks/${encodeURIComponent(selected.id)}/stop`, { method: "POST", body: "{}" }),
      "Stop requested. Status changes to cancelled after Codex exits.",
    );
  };
  const archive = () => {
    if (!selected || selected.status === "running") return;
    const pendingWarning = selected.status === "scheduled" ? " This also cancels the pending task." : "";
    if (!window.confirm(`Remove “${selected.title}” from task lists?${pendingWarning} Run metadata stays stored.`)) return;
    void mutate(
      () => api<ArchiveTaskResult>(token, `/tasks/${encodeURIComponent(selected.id)}`, { method: "DELETE" }),
      "Task removed from dashboard and operations. Run metadata was preserved.",
      () => setSelectedId(""),
    );
  };
  const removableTasks = tasks.items.filter((task) => task.status !== "running");
  const selectedTasks = removableTasks.filter((task) => selectedTaskIds.includes(task.id));
  const allRemovableSelected = removableTasks.length > 0 && selectedTasks.length === removableTasks.length;
  const resetListSelection = () => {
    setSelectedId("");
    setSelectedTaskIds([]);
  };
  const toggleTaskSelection = (taskId: string, checked: boolean) => {
    setSelectedTaskIds((current) => checked
      ? [...new Set([...current, taskId])]
      : current.filter((id) => id !== taskId));
  };
  const toggleAllRemovable = (checked: boolean) => {
    setSelectedTaskIds(checked ? removableTasks.map((task) => task.id) : []);
  };
  const archiveSelected = async () => {
    if (!selectedTasks.length) return;
    const scheduledCount = selectedTasks.filter((task) => task.status === "scheduled").length;
    const scheduledWarning = scheduledCount
      ? ` This also cancels ${scheduledCount} pending task${scheduledCount === 1 ? "" : "s"}.`
      : "";
    if (!window.confirm(`Remove ${selectedTasks.length} selected task${selectedTasks.length === 1 ? "" : "s"} from task lists?${scheduledWarning} Run metadata stays stored.`)) return;

    setPending(true);
    setError("");
    setMessageText("");
    try {
      const results = await Promise.allSettled(selectedTasks.map((task) =>
        api<ArchiveTaskResult>(token, `/tasks/${encodeURIComponent(task.id)}`, { method: "DELETE" })
      ));
      const removedIds = selectedTasks
        .filter((_, index) => results[index].status === "fulfilled")
        .map((task) => task.id);
      const failures = selectedTasks.flatMap((task, index) => {
        const result = results[index];
        return result.status === "rejected" ? [`${task.title}: ${message(result.reason)}`] : [];
      });

      setSelectedTaskIds(selectedTasks
        .filter((_, index) => results[index].status === "rejected")
        .map((task) => task.id));
      if (removedIds.includes(selectedId)) setSelectedId("");
      await load();
      if (removedIds.length) {
        if (!failures.length) onChanged();
        setMessageText(`${removedIds.length} task${removedIds.length === 1 ? "" : "s"} removed from dashboard and operations. Run metadata was preserved.`);
      }
      if (failures.length) {
        setError(`${failures.length} task${failures.length === 1 ? "" : "s"} could not be removed: ${failures.join("; ")}`);
      }
    } finally {
      setPending(false);
    }
  };
  const setup = () => {
    if (!window.confirm("Install or replace the LimitWise user background service?")) return;
    void mutate(
      () => api(token, "/service/setup", { method: "POST", body: JSON.stringify({ confirmed: true }) }),
      "Background service setup completed.",
    );
  };

  return (
    <div className="stack">
      <section className="panel section-heading">
        <div>
          <span className="eyebrow">Operations</span>
          <h2>Manage existing tasks</h2>
          <p>Pending tasks can be edited or cancelled. Running tasks can be stopped, then retried as a new run.</p>
        </div>
        <button onClick={setup} disabled={pending}>Set up daemon…</button>
      </section>
      {error && <StatePanel state="error" title="Operation rejected" detail={error} />}
      {messageText && <StatePanel state="ready" title={messageText} />}
      {!tasks.items.length ? <StatePanel state="empty" title="No local tasks" /> : (
        <section className="operations-grid">
          <div className="panel stack">
            <div className="list-controls">
              <label>Status<select value={filter} onChange={(event) => { setFilter(event.target.value); setPage(1); resetListSelection(); }}>
                <option value="all">all</option>
                {ALL_STATUSES.map((status) => <option key={status}>{status}</option>)}
              </select></label>
              <SortSelect value={sort} onChange={(value) => { setSort(value); setPage(1); resetListSelection(); }} />
            </div>
            <div className="bulk-actions">
              <label className="bulk-select-all">
                <input
                  type="checkbox"
                  aria-label="Select all removable tasks on this page"
                  checked={allRemovableSelected}
                  onChange={(event) => toggleAllRemovable(event.target.checked)}
                  disabled={pending || !removableTasks.length}
                />
                Select all removable
              </label>
              <button className="danger" onClick={() => void archiveSelected()} disabled={pending || !selectedTasks.length}>
                {pending ? "Removing selected tasks…" : `Remove selected (${selectedTasks.length})…`}
              </button>
            </div>
            <div className="task-list">
              {tasks.items.map((task) => (
                <div className="operations-task-row" key={task.id}>
                  <label
                    className="task-select"
                    title={task.status === "running" ? "Stop this task before removing it" : `Select ${task.title} for removal`}
                  >
                    <input
                      type="checkbox"
                      aria-label={`Select ${task.title} for removal`}
                      checked={selectedTaskIds.includes(task.id)}
                      onChange={(event) => toggleTaskSelection(task.id, event.target.checked)}
                      disabled={pending || task.status === "running"}
                    />
                  </label>
                  <button className={selectedId === task.id ? "task-row selected" : "task-row"} onClick={() => setSelectedId(task.id)} disabled={pending}>
                    <span><strong>{task.title}</strong><small>{task.id}</small></span>
                    <StatusBadge status={task.status} />
                  </button>
                </div>
              ))}
            </div>
            <Pagination value={tasks} onChange={(value) => { setPage(value); resetListSelection(); }} />
          </div>
          {selected?.status === "scheduled" ? (
            <fieldset className="panel inspector" disabled={pending || selected.status !== "scheduled"}>
              <legend>Edit pending task</legend>
              <Field label="Title" value={changes.title ?? ""} onChange={(value) => setChanges({ ...changes, title: value })} />
              <TextArea label="Prompt" value={changes.prompt ?? ""} onChange={(value) => setChanges({ ...changes, prompt: value })} />
              <TextArea label="Success criteria" value={changes.success_criteria ?? ""} onChange={(value) => setChanges({ ...changes, success_criteria: value })} />
              <Field label="Absolute cwd" value={changes.cwd ?? ""} onChange={(value) => setChanges({ ...changes, cwd: value })} />
              <DateTimeField
                label="Run at"
                value={changes.run_at ?? ""}
                timezone={changes.timezone ?? selected.timezone}
                onChange={(value) => setChanges({ ...changes, run_at: value })}
                disabled={Boolean(selected.depends_on_task_id)}
              />
              <Field label="IANA timezone" value={changes.timezone ?? ""} onChange={(value) => setChanges({ ...changes, timezone: value })} />
              <Select label="Difficulty" value={changes.difficulty ?? "standard"} options={["simple", "standard", "complex", "exceptional"]} onChange={(value) => setChanges({ ...changes, difficulty: value })} />
              <Select label="Model" value={changes.model ?? defaultModel(catalog, "gpt-6-sol")} options={modelOptions(catalog)} onChange={changePendingModel} />
              <Select label="Effort" value={changes.effort ?? "medium"} options={effortOptions(catalog, changes.model)} onChange={(value) => setChanges({ ...changes, effort: value })} />
              <Select label="Permission profile" value={changes.permission_profile ?? "restricted"} options={["restricted", "networked"]} onChange={(value) => { setChanges({ ...changes, permission_profile: value }); setUpdateNetworkedAcknowledged(false); }} />
              <p className="muted"><PermissionRisk profile={(changes.permission_profile ?? "restricted") as PermissionProfile} /></p>
              {changes.permission_profile === "networked" && (
                <label className="risk-acknowledgement">
                  <input type="checkbox" checked={updateNetworkedAcknowledged} onChange={(event) => setUpdateNetworkedAcknowledged(event.target.checked)} />
                  I explicitly acknowledge that this task may access the network and use web search. External apps, interactive approval, danger-full-access, and non-workspace-write sandboxes remain prohibited.
                </label>
              )}
              <div className="button-row">
                <button className="primary" onClick={update} disabled={changes.permission_profile === "networked" && !updateNetworkedAcknowledged}>Save current pending task</button>
                <button className="danger" onClick={cancel}>Cancel task…</button>
                <button className="danger" onClick={archive}>Remove from task lists…</button>
              </div>
            </fieldset>
          ) : selected ? (
            <section className="panel inspector">
              <div className="section-heading">
                <div><span className="eyebrow">Immutable task</span><h3>{selected.title}</h3></div>
                <StatusBadge status={selected.status} />
              </div>
              <p className="muted">{selected.id}</p>
              {selected.status === "running" && (
                <>
                  <p className="muted">Pause is unavailable: Codex exec has no safe persistent pause. Stop ends this attempt; retry becomes available after cancellation.</p>
                  <button className="danger" onClick={stop} disabled={pending}>Stop running task…</button>
                </>
              )}
              <RetryAttemptPanel token={token} source={selected} catalog={catalog} onCreated={() => { void load(); onChanged(); }} />
              <button className="danger secondary-action" onClick={archive} disabled={pending || selected.status === "running"} title={selected.status === "running" ? "Stop this task before removing it" : undefined}>
                Remove from task lists…
              </button>
            </section>
          ) : <StatePanel state="empty" title="Select a task to manage" />}
        </section>
      )}
    </div>
  );
}

function SortSelect({ value, onChange }: { value: TaskSort; onChange: (value: TaskSort) => void }) {
  return (
    <label>
      Sort
      <select value={value} onChange={(event) => onChange(event.target.value as TaskSort)}>
        <option value="newest">Created: new to old</option>
        <option value="oldest">Created: old to new</option>
        <option value="run_at_newest">Run time: new to old</option>
        <option value="run_at_oldest">Run time: old to new</option>
        <option value="title_ascending">Title: A to Z</option>
        <option value="title_descending">Title: Z to A</option>
      </select>
    </label>
  );
}

function Pagination({
  value,
  noun = "tasks",
  onChange,
}: {
  value: Pick<TaskPage, "page" | "total_pages" | "total">;
  noun?: string;
  onChange: (page: number) => void;
}) {
  return (
    <nav className="pagination" aria-label={`${noun} pages`}>
      <button onClick={() => onChange(value.page - 1)} disabled={value.page <= 1}>Previous</button>
      <span>Page {value.page} of {value.total_pages} · {value.total} {noun}</span>
      <button onClick={() => onChange(value.page + 1)} disabled={value.page >= value.total_pages}>Next</button>
    </nav>
  );
}

function emptyTaskPage(): TaskPage {
  return { items: [], page: 1, page_size: 20, total: 0, total_pages: 1, sort: "newest" };
}

function taskPagePath(page: number, sort: TaskSort, filter: string) {
  const query = new URLSearchParams({ page: String(page), sort });
  if (filter !== "all") query.set("status", filter);
  return `/tasks?${query}`;
}

function batchPagePath(page: number, sort: TaskSort, filter: string) {
  const query = new URLSearchParams({ page: String(page), sort });
  if (filter !== "all") query.set("status", filter);
  return `/batches?${query}`;
}

export function modelOptions(catalog: ModelCatalog): string[] {
  return catalog.models.map((model) => model.model);
}

export function effortOptions(catalog: ModelCatalog, modelName?: string): string[] {
  const selected = catalog.models.find((model) => model.model === modelName)
    ?? catalog.models.find((model) => model.isDefault)
    ?? catalog.models[0];
  return selected?.supportedReasoningEfforts.map((effort) => effort.reasoningEffort) ?? [];
}

function defaultModel(catalog: ModelCatalog, preferred: string): string {
  return catalog.models.find((model) => model.model === preferred)?.model
    ?? catalog.models.find((model) => model.isDefault)?.model
    ?? catalog.models[0]?.model
    ?? preferred;
}

export function routeSelection(catalog: ModelCatalog, requestedModel: string, requestedEffort?: string) {
  const model = defaultModel(catalog, requestedModel);
  const selected = catalog.models.find((candidate) => candidate.model === model);
  const supported = selected?.supportedReasoningEfforts.map((effort) => effort.reasoningEffort) ?? [];
  const effort = requestedEffort && supported.includes(requestedEffort)
    ? requestedEffort
    : supported.includes(selected?.defaultReasoningEffort ?? "")
      ? selected!.defaultReasoningEffort
      : supported[0] ?? requestedEffort ?? "medium";
  return { model, effort };
}

function Field({
  label,
  value,
  onChange,
  disabled,
}: {
  label: string;
  value: string;
  onChange: (value: string) => void;
  disabled?: boolean;
}) {
  return <label>{label}<input value={value} disabled={disabled} onChange={(event) => onChange(event.target.value)} /></label>;
}

function DateTimeField({
  label,
  value,
  timezone,
  onChange,
  disabled,
}: {
  label: string;
  value: string;
  timezone: string;
  onChange: (value: string) => void;
  disabled?: boolean;
}) {
  const parts = runAtParts(value);
  const change = (date: string, time: string) => onChange(runAtFromParts(date, time, timezone, value));
  return (
    <fieldset className="date-time-field" disabled={disabled}>
      <legend>{label}</legend>
      <div className="date-time-inputs">
        <label>
          Date
          <input
            type="date"
            aria-label={`${label} date`}
            value={parts.date}
            onChange={(event) => change(event.target.value, parts.time)}
          />
        </label>
        <label>
          Time (24-hour)
          <input
            type="time"
            lang="en-GB"
            step="60"
            aria-label={`${label} time (24-hour)`}
            value={parts.time}
            onChange={(event) => change(parts.date, event.target.value)}
          />
        </label>
      </div>
      <small>{timezone || "Local timezone"}</small>
    </fieldset>
  );
}

function TextArea({ label, value, onChange }: { label: string; value: string; onChange: (value: string) => void }) {
  return <label>{label}<textarea value={value} rows={4} onChange={(event) => onChange(event.target.value)} /></label>;
}

function Select({
  label,
  value,
  options,
  onChange,
  disabled,
}: {
  label: string;
  value: string;
  options: readonly string[];
  onChange: (value: string) => void;
  disabled?: boolean;
}) {
  return (
    <label>{label}<select value={value} disabled={disabled} onChange={(event) => onChange(event.target.value)}>
      {options.map((option) => <option key={option}>{option}</option>)}
    </select></label>
  );
}

function formatTime(epoch: number) {
  return new Date(epoch * 1000).toLocaleString();
}

function PermissionRisk({ profile }: { profile: PermissionProfile }) {
  return profile === "networked"
    ? <>Network and web search are enabled. Access remains limited to <code>workspace-write</code>; external apps, interactive approval, and <code>danger-full-access</code> remain prohibited.</>
    : <>Network and web search are disabled. Access is limited to <code>workspace-write</code>; external apps and interactive approval are disabled.</>;
}

function message(reason: unknown) {
  if (reason instanceof ApiError) {
    const fields = Object.values(reason.fields).join("; ");
    return fields ? `${reason.message}: ${fields}` : reason.message;
  }
  return reason instanceof Error ? reason.message : "Unknown local error";
}
