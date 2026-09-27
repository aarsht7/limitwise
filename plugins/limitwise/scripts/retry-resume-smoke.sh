#!/bin/sh
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
smoke_root=$(mktemp -d /tmp/limitwise-c04-smoke.XXXXXX)
trap 'rm -rf "$smoke_root"' EXIT HUP INT TERM
fake_codex="$root/tests/fixtures/fake-codex.sh"
home_root="$smoke_root/home"
data_root="$smoke_root/data"
args_path="$smoke_root/codex-args.txt"

command -v jq >/dev/null 2>&1 || { printf '%s\n' 'jq is required for retry/resume smoke testing' >&2; exit 1; }

if [ -n "${LIMITWISE_C04_BINARY:-}" ]; then
  binary=$LIMITWISE_C04_BINARY
else
  cargo build --quiet --locked --manifest-path "$root/Cargo.toml"
  binary=$root/target/debug/limitwise
fi
[ -x "$binary" ] || { printf 'binary is not executable: %s\n' "$binary" >&2; exit 1; }

mcp_call() {
  printf '%s\n' "$1" | env LIMITWISE_HOME="$home_root" XDG_DATA_HOME="$data_root" "$binary" mcp
}

task_id_from_list() {
  mcp_call '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"list_tasks","arguments":{}}}' \
    | jq -er '.result.structuredContent[0].id'
}

task_history() {
  mcp_call "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{\"name\":\"get_task_status\",\"arguments\":{\"task_id\":\"$1\"}}}" \
    | jq -cS '.result.structuredContent | {task,runs}'
}

wait_until_epoch() {
  now=$(date -u '+%s')
  if [ "$1" -gt "$now" ]; then
    sleep $((1 + $1 - now))
  fi
}

first_run_at=$(date -u -d '+2 seconds' '+%Y-%m-%dT%H:%M:%S+00:00')
mcp_call "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{\"name\":\"schedule_batch\",\"arguments\":{\"idempotency_key\":\"c04-failure-source\",\"budget_mode\":\"percentage\",\"weekly_cap_percent\":1,\"tasks\":[{\"title\":\"ordinary failure\",\"prompt\":\"fail once\",\"success_criteria\":\"retry succeeds\",\"cwd\":\"$root\",\"run_at\":\"$first_run_at\",\"timezone\":\"UTC\",\"difficulty\":\"simple\"}]}}}" >/dev/null
sleep 3
env LIMITWISE_HOME="$home_root" XDG_DATA_HOME="$data_root" LIMITWISE_CODEX_PATH="$fake_codex" \
  LIMITWISE_FAKE_EXEC_SCENARIO=fail LIMITWISE_POLL_SECONDS=1 "$binary" daemon --once
failure_id=$(task_id_from_list)
test -n "$failure_id"
failure_history=$(task_history "$failure_id")

retry_at=$(date -u -d '+2 seconds' '+%Y-%m-%dT%H:%M:%S+00:00')
retry_arguments="{\"task_id\":\"$failure_id\",\"budget_mode\":\"tokens\",\"token_cap\":1000,\"run_at\":\"$retry_at\",\"timezone\":\"UTC\"}"
mcp_call "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"preview_retry_task\",\"arguments\":$retry_arguments}}" | grep -q '"resume_mode":"fresh_session"'
confirm_arguments=$(printf '%s' "$retry_arguments" | sed 's/}$/,"idempotency_key":"c04-failure-retry","confirmed":true}/')
first_retry=$(mcp_call "{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"tools/call\",\"params\":{\"name\":\"retry_task\",\"arguments\":$confirm_arguments}}")
second_retry=$(mcp_call "{\"jsonrpc\":\"2.0\",\"id\":4,\"method\":\"tools/call\",\"params\":{\"name\":\"retry_task\",\"arguments\":$confirm_arguments}}")
first_retry_id=$(printf '%s' "$first_retry" | jq -er '.result.structuredContent.task.id')
second_retry_id=$(printf '%s' "$second_retry" | jq -er '.result.structuredContent.task.id')
test -n "$first_retry_id"
test "$first_retry_id" = "$second_retry_id"
printf '%s\n' "$second_retry" | grep -q '"idempotent_replay":true'
test "$(task_history "$failure_id")" = "$failure_history"
sleep 3
: >"$args_path"
env LIMITWISE_HOME="$home_root" XDG_DATA_HOME="$data_root" LIMITWISE_CODEX_PATH="$fake_codex" \
  LIMITWISE_FAKE_ARGS_PATH="$args_path" LIMITWISE_POLL_SECONDS=1 "$binary" daemon --once
! grep -qx 'resume' "$args_path"
mcp_call "{\"jsonrpc\":\"2.0\",\"id\":5,\"method\":\"tools/call\",\"params\":{\"name\":\"get_task_status\",\"arguments\":{\"task_id\":\"$first_retry_id\"}}}" \
  | jq -e '.result.structuredContent.task.status == "completed"' >/dev/null
test "$(task_history "$failure_id")" = "$failure_history"

quota_run_at=$(date -u -d '+2 seconds' '+%Y-%m-%dT%H:%M:%S+00:00')
reset_at=$(($(date -u '+%s') + 8))
mcp_call "{\"jsonrpc\":\"2.0\",\"id\":5,\"method\":\"tools/call\",\"params\":{\"name\":\"schedule_batch\",\"arguments\":{\"idempotency_key\":\"c04-quota-source\",\"budget_mode\":\"percentage\",\"weekly_cap_percent\":1,\"tasks\":[{\"title\":\"quota source\",\"prompt\":\"wait for interruption\",\"success_criteria\":\"resume succeeds\",\"cwd\":\"$root\",\"run_at\":\"$quota_run_at\",\"timezone\":\"UTC\",\"difficulty\":\"standard\"}]}}}" >/dev/null
sleep 3
env LIMITWISE_HOME="$home_root" XDG_DATA_HOME="$data_root" LIMITWISE_CODEX_PATH="$fake_codex" \
  LIMITWISE_FAKE_EXEC_SCENARIO=quota_interrupt LIMITWISE_FAKE_QUOTA_SCENARIO=interrupt \
  LIMITWISE_FAKE_RESET_AT="$reset_at" LIMITWISE_POLL_SECONDS=1 "$binary" daemon --once
quota_id=$(mcp_call '{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"list_tasks","arguments":{"status":"quota_interrupted"}}}' \
  | jq -er '.result.structuredContent[0].id')
test -n "$quota_id"
quota_history=$(task_history "$quota_id")
quota_arguments="{\"task_id\":\"$quota_id\",\"budget_mode\":\"percentage\",\"weekly_cap_percent\":1}"
mcp_call "{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"tools/call\",\"params\":{\"name\":\"preview_retry_task\",\"arguments\":$quota_arguments}}" | grep -q '"attempt_kind":"quota_resume"'
quota_confirm=$(printf '%s' "$quota_arguments" | sed 's/}$/,"idempotency_key":"c04-quota-resume","confirmed":true}/')
quota_attempt=$(mcp_call "{\"jsonrpc\":\"2.0\",\"id\":8,\"method\":\"tools/call\",\"params\":{\"name\":\"retry_task\",\"arguments\":$quota_confirm}}")
quota_attempt_id=$(printf '%s' "$quota_attempt" | jq -er '.result.structuredContent.task.id')
env LIMITWISE_HOME="$home_root" XDG_DATA_HOME="$data_root" LIMITWISE_CODEX_PATH="$fake_codex" \
  LIMITWISE_POLL_SECONDS=1 "$binary" daemon --once
mcp_call "{\"jsonrpc\":\"2.0\",\"id\":9,\"method\":\"tools/call\",\"params\":{\"name\":\"get_task_status\",\"arguments\":{\"task_id\":\"$quota_attempt_id\"}}}" \
  | jq -e '.result.structuredContent.task.status == "scheduled"' >/dev/null
wait_until_epoch "$reset_at"
: >"$args_path"
env LIMITWISE_HOME="$home_root" XDG_DATA_HOME="$data_root" LIMITWISE_CODEX_PATH="$fake_codex" \
  LIMITWISE_FAKE_ARGS_PATH="$args_path" LIMITWISE_POLL_SECONDS=1 "$binary" daemon --once
grep -qx 'resume' "$args_path"
grep -qx 'fake-session' "$args_path"

status_output=$(mcp_call "{\"jsonrpc\":\"2.0\",\"id\":10,\"method\":\"tools/call\",\"params\":{\"name\":\"get_task_status\",\"arguments\":{\"task_id\":\"$quota_id\"}}}")
printf '%s\n' "$status_output" | jq -e --arg attempt "$quota_attempt_id" '
  .result.structuredContent.task.status == "quota_interrupted" and
  (.result.structuredContent.lineage | any(.id == $attempt and .attempt_kind == "quota_resume" and .attempt_number == 2))
' >/dev/null
test "$(task_history "$quota_id")" = "$quota_history"

fallback_run_at=$(date -u -d '+2 seconds' '+%Y-%m-%dT%H:%M:%S+00:00')
fallback_reset_at=$(($(date -u '+%s') + 8))
mcp_call "{\"jsonrpc\":\"2.0\",\"id\":11,\"method\":\"tools/call\",\"params\":{\"name\":\"schedule_batch\",\"arguments\":{\"idempotency_key\":\"c04-fallback-source\",\"budget_mode\":\"percentage\",\"weekly_cap_percent\":1,\"tasks\":[{\"title\":\"fallback source\",\"prompt\":\"continue without session\",\"success_criteria\":\"fallback succeeds\",\"cwd\":\"$root\",\"run_at\":\"$fallback_run_at\",\"timezone\":\"UTC\",\"difficulty\":\"standard\"}]}}}" >/dev/null
sleep 3
env LIMITWISE_HOME="$home_root" XDG_DATA_HOME="$data_root" LIMITWISE_CODEX_PATH="$fake_codex" \
  LIMITWISE_FAKE_EXEC_SCENARIO=quota_interrupt LIMITWISE_FAKE_QUOTA_SCENARIO=interrupt \
  LIMITWISE_FAKE_EMIT_SESSION=0 LIMITWISE_FAKE_RESET_AT="$fallback_reset_at" LIMITWISE_POLL_SECONDS=1 "$binary" daemon --once
fallback_id=$(mcp_call '{"jsonrpc":"2.0","id":12,"method":"tools/call","params":{"name":"list_tasks","arguments":{"status":"quota_interrupted"}}}' \
  | jq -er '.result.structuredContent[] | select(.title == "fallback source") | .id')
fallback_arguments="{\"task_id\":\"$fallback_id\",\"budget_mode\":\"percentage\",\"weekly_cap_percent\":1}"
mcp_call "{\"jsonrpc\":\"2.0\",\"id\":13,\"method\":\"tools/call\",\"params\":{\"name\":\"preview_retry_task\",\"arguments\":$fallback_arguments}}" \
  | jq -e '.result.structuredContent.resume_mode == "context_fallback"' >/dev/null
fallback_confirm=$(printf '%s' "$fallback_arguments" | sed 's/}$/,"idempotency_key":"c04-fallback-resume","confirmed":true}/')
fallback_attempt=$(mcp_call "{\"jsonrpc\":\"2.0\",\"id\":14,\"method\":\"tools/call\",\"params\":{\"name\":\"retry_task\",\"arguments\":$fallback_confirm}}")
fallback_attempt_id=$(printf '%s' "$fallback_attempt" | jq -er '.result.structuredContent.task.id')
wait_until_epoch "$fallback_reset_at"
: >"$args_path"
env LIMITWISE_HOME="$home_root" XDG_DATA_HOME="$data_root" LIMITWISE_CODEX_PATH="$fake_codex" \
  LIMITWISE_FAKE_ARGS_PATH="$args_path" LIMITWISE_POLL_SECONDS=1 "$binary" daemon --once
! grep -qx 'resume' "$args_path"
grep -q 'Continuation context:' "$args_path"
grep -q "predecessor task ID: $fallback_id" "$args_path"
mcp_call "{\"jsonrpc\":\"2.0\",\"id\":15,\"method\":\"tools/call\",\"params\":{\"name\":\"get_task_status\",\"arguments\":{\"task_id\":\"$fallback_attempt_id\"}}}" \
  | jq -e '.result.structuredContent.task.status == "completed"' >/dev/null

echo "LimitWise retry/resume smoke test passed"
