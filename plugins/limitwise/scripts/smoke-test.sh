#!/bin/sh
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
smoke_root=$(mktemp -d /tmp/limitwise-smoke.XXXXXX)
trap 'rm -rf "$smoke_root"' EXIT HUP INT TERM
run_at=$(date -u -d '+2 seconds' '+%Y-%m-%dT%H:%M:%S+00:00')
fake_args="$smoke_root/codex-args.txt"
fake_path="$root/tests/fixtures/bin:$PATH"

cargo build --quiet --manifest-path "$root/Cargo.toml"
chmod 0755 "$root/tests/fixtures/fake-codex.sh" "$root/tests/fixtures/bin/systemctl"

env LIMITWISE_HOME="$smoke_root/home" XDG_DATA_HOME="$smoke_root/data" PATH="$fake_path" \
  "$root/target/debug/limitwise" setup >/dev/null
test -x "$smoke_root/data/limitwise/bin/limitwise"
test -f "$smoke_root/home/.config/systemd/user/limitwise.service"
if command -v systemd-analyze >/dev/null 2>&1; then
  if ! systemd-analyze verify "$smoke_root/home/.config/systemd/user/limitwise.service" >"$smoke_root/systemd-verify.log" 2>&1; then
    sed -n '1,160p' "$smoke_root/systemd-verify.log" >&2
    exit 1
  fi
fi

printf '%s\n' \
  '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"smoke","version":"1"}}}' \
  "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"schedule_batch\",\"arguments\":{\"idempotency_key\":\"integration-smoke\",\"budget_mode\":\"percentage\",\"weekly_cap_percent\":1,\"tasks\":[{\"title\":\"smoke-create\",\"prompt\":\"return success\",\"success_criteria\":\"fake completes\",\"cwd\":\"$root\",\"run_at\":\"$run_at\",\"timezone\":\"UTC\",\"difficulty\":\"standard\"},{\"title\":\"smoke-follow-up\",\"prompt\":\"return success again\",\"success_criteria\":\"fake completes after first task\",\"cwd\":\"$root\",\"after_previous\":true,\"timezone\":\"UTC\",\"difficulty\":\"simple\"}]}}}" \
  "{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"tools/call\",\"params\":{\"name\":\"schedule_batch\",\"arguments\":{\"idempotency_key\":\"token-smoke\",\"budget_mode\":\"tokens\",\"token_cap\":35,\"tasks\":[{\"title\":\"token-first\",\"prompt\":\"return success\",\"success_criteria\":\"fake completes\",\"cwd\":\"$root\",\"run_at\":\"$run_at\",\"timezone\":\"UTC\",\"difficulty\":\"simple\"},{\"title\":\"token-skipped\",\"prompt\":\"must not run\",\"success_criteria\":\"token cap prevents launch\",\"cwd\":\"$root\",\"after_previous\":true,\"timezone\":\"UTC\",\"difficulty\":\"simple\"}]}}}" \
  | env LIMITWISE_HOME="$smoke_root/home" XDG_DATA_HOME="$smoke_root/data" "$root/target/debug/limitwise" mcp >/dev/null

sleep 3
env LIMITWISE_HOME="$smoke_root/home" XDG_DATA_HOME="$smoke_root/data" \
  LIMITWISE_CODEX_PATH="$root/tests/fixtures/fake-codex.sh" \
  LIMITWISE_FAKE_ARGS_PATH="$fake_args" \
  LIMITWISE_POLL_SECONDS=1 \
  "$root/target/debug/limitwise" daemon --once

test -f "$fake_args"
grep -q 'gpt-5.6-terra' "$fake_args"
grep -q 'gpt-5.6-luna' "$fake_args"
grep -q 'model_reasoning_effort="medium"' "$fake_args"
grep -q 'approval_policy="never"' "$fake_args"
grep -q 'workspace-write' "$fake_args"
grep -q 'keep final summaries terse' "$fake_args"
grep -q 'do not use network or external apps' "$fake_args"
grep -q 'fake completes after first task' "$fake_args"

task_output=$(printf '%s\n' \
  '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"list_tasks","arguments":{}}}' \
  | env LIMITWISE_HOME="$smoke_root/home" XDG_DATA_HOME="$smoke_root/data" "$root/target/debug/limitwise" mcp \
)
printf '%s\n' "$task_output" | grep -q '"depends_on_task_id":"task-'
completed_count=$(printf '%s\n' "$task_output" | grep -o '"status":"completed"' | wc -l)
test "$completed_count" -eq 3
quota_skipped_count=$(printf '%s\n' "$task_output" | grep -o '"status":"quota_skipped"' | wc -l)
test "$quota_skipped_count" -eq 1

stats_output=$(printf '%s\n' \
  '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"task_usage_stats","arguments":{}}}' \
  | env LIMITWISE_HOME="$smoke_root/home" XDG_DATA_HOME="$smoke_root/data" "$root/target/debug/limitwise" mcp \
)
printf '%s\n' "$stats_output" | grep -q '"run_count":4'
printf '%s\n' "$stats_output" | grep -q '"tokens_used":105'
printf '%s\n' "$stats_output" | grep -q '"tokens_used":0'

estimate_output=$(printf '%s\n' \
  '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"estimate_batch_usage","arguments":{"budget_mode":"tokens","token_cap":10,"tasks":[{"title":"predicted-small-task","difficulty":"simple"}]}}}' \
  | env LIMITWISE_HOME="$smoke_root/home" XDG_DATA_HOME="$smoke_root/data" "$root/target/debug/limitwise" mcp \
)
printf '%s\n' "$estimate_output" | grep -q '"likely_tokens":35'
printf '%s\n' "$estimate_output" | grep -q '"conservative_tokens":35'
printf '%s\n' "$estimate_output" | grep -q '"level":"likely_insufficient"'

env LIMITWISE_HOME="$smoke_root/home" XDG_DATA_HOME="$smoke_root/data" PATH="$fake_path" \
  "$root/target/debug/limitwise" uninstall >/dev/null
test ! -e "$smoke_root/data/limitwise/bin/limitwise"
test ! -e "$smoke_root/home/.config/systemd/user/limitwise.service"
test -f "$smoke_root/data/limitwise/limitwise.sqlite3"

echo "LimitWise fake Codex smoke test passed"
