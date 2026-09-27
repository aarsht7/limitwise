#!/bin/sh
set -eu

mode=${1:-}
doctor_scenario=${LIMITWISE_DOCTOR_SCENARIO:-success}

if [ "$mode" = "--version" ]; then
  if [ "$doctor_scenario" = "malformed_version" ]; then
    printf '%s\n' 'unexpected output'
  else
    printf '%s\n' 'codex-cli 1.2.3'
  fi
  exit 0
fi

if [ "$mode" = "login" ] && [ "${2:-}" = "status" ]; then
  [ "$doctor_scenario" != "logged_out" ] || exit 1
  printf '%s\n' 'Logged in'
  exit 0
fi

if [ "$mode" = "plugin" ] && [ "${2:-}" = "list" ]; then
  if [ "$doctor_scenario" = "plugin_missing" ]; then
    printf '%s\n' '{"installed":[],"available":[]}'
  else
    printf '%s\n' '{"installed":[{"pluginId":"limitwise@limitwise","name":"limitwise","marketplaceName":"limitwise","version":"1.0.0","installed":true,"enabled":true}],"available":[]}'
  fi
  exit 0
fi

if [ "$mode" = "mcp" ] && [ "${2:-}" = "list" ]; then
  if [ "$doctor_scenario" = "mcp_missing" ]; then
    printf '%s\n' 'example'
  else
    printf '%s\n' 'limitwise'
  fi
  exit 0
fi

if [ "$mode" = "app-server" ]; then
  quota_reads=0
  while IFS= read -r request; do
    if [ -n "${LIMITWISE_FAKE_APP_SERVER_REQUESTS_PATH:-}" ]; then
      printf '%s\n' "$request" >> "$LIMITWISE_FAKE_APP_SERVER_REQUESTS_PATH"
    fi
    case "$request" in
      *'"method":"initialize"'*)
        printf '%s\n' '{"id":1,"result":{"serverInfo":{"name":"fake","version":"1"}}}'
        ;;
      *'"method":"account/rateLimits/read"'*)
        quota_reads=$((quota_reads + 1))
        request_id=$(printf '%s' "$request" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
        if [ "$doctor_scenario" = "quota_timeout" ]; then
          continue
        elif [ "$doctor_scenario" = "quota_ambiguous" ]; then
          printf '{"id":%s,"result":{"rateLimits":{"primary":{"usedPercent":10,"windowDurationMins":300,"resetsAt":4102444800},"duplicate":{"usedPercent":11,"windowDurationMins":300,"resetsAt":4102444801},"secondary":{"usedPercent":20,"windowDurationMins":10080,"resetsAt":4102444800}}}}\n' "$request_id"
        else
          used_percent=10
          if [ "${LIMITWISE_FAKE_QUOTA_SCENARIO:-stable}" = "interrupt" ] && [ "$quota_reads" -ge 2 ]; then
            used_percent=95
          fi
          reset_at=${LIMITWISE_FAKE_RESET_AT:-4102444800}
          printf '{"id":%s,"result":{"rateLimits":{"primary":{"usedPercent":%s,"windowDurationMins":300,"resetsAt":%s},"secondary":{"usedPercent":20,"windowDurationMins":10080,"resetsAt":4102444800}}}}\n' "$request_id" "$used_percent" "$reset_at"
        fi
        ;;
      *'"method":"model/list"'*)
        request_id=$(printf '%s' "$request" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
        printf '{"id":%s,"result":{"data":[{"id":"gpt-6-astra","model":"gpt-6-astra","displayName":"GPT-6-Astra","description":"Frontier intelligence for the most demanding work.","defaultReasoningEffort":"low","supportedReasoningEfforts":[{"reasoningEffort":"low","description":"Low"},{"reasoningEffort":"medium","description":"Medium"},{"reasoningEffort":"high","description":"High"},{"reasoningEffort":"xhigh","description":"Extra high"},{"reasoningEffort":"max","description":"Maximum"},{"reasoningEffort":"ultra","description":"Ultra"}],"isDefault":true,"upgrade":null},{"id":"gpt-6-sol","model":"gpt-6-sol","displayName":"GPT-6-Sol","description":"Workhorse model for coding and everyday work.","defaultReasoningEffort":"medium","supportedReasoningEfforts":[{"reasoningEffort":"low","description":"Low"},{"reasoningEffort":"medium","description":"Medium"},{"reasoningEffort":"high","description":"High"},{"reasoningEffort":"xhigh","description":"Extra high"},{"reasoningEffort":"max","description":"Maximum"},{"reasoningEffort":"ultra","description":"Ultra"}],"isDefault":false,"upgrade":null},{"id":"gpt-6-luna","model":"gpt-6-luna","displayName":"GPT-6-Luna","description":"Fast and affordable model for easier tasks.","defaultReasoningEffort":"medium","supportedReasoningEfforts":[{"reasoningEffort":"low","description":"Low"},{"reasoningEffort":"medium","description":"Medium"},{"reasoningEffort":"high","description":"High"},{"reasoningEffort":"xhigh","description":"Extra high"},{"reasoningEffort":"max","description":"Maximum"}],"isDefault":false,"upgrade":null}],"nextCursor":null}}\n' "$request_id"
        ;;
      *'"method":"thread/start"'*)
        request_id=$(printf '%s' "$request" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
        printf '{"id":%s,"result":{"thread":{"id":"fake-plan-thread"}}}\n' "$request_id"
        ;;
      *'"method":"turn/start"'*)
        request_id=$(printf '%s' "$request" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
        plan_json=${LIMITWISE_FAKE_PLAN_JSON:-'"{\"summary\":\"Fixture plan\",\"tasks\":[{\"title\":\"Fixture task\",\"prompt\":\"Execute fixture task\",\"success_criteria\":\"Fixture passes\",\"difficulty\":\"standard\",\"model\":\"gpt-6-sol\",\"effort\":\"medium\"}]}"'}
        printf '{"id":%s,"result":{"turn":{"id":"fake-plan-turn","status":"inProgress"}}}\n' "$request_id"
        printf '{"method":"item/completed","params":{"completedAtMs":1,"threadId":"fake-plan-thread","turnId":"fake-plan-turn","item":{"id":"fake-plan-item","type":"plan","text":%s}}}\n' "$plan_json"
        printf '%s\n' '{"method":"turn/completed","params":{"threadId":"fake-plan-thread","turn":{"id":"fake-plan-turn","status":"completed","items":[]}}}'
        ;;
    esac
  done
  exit 0
fi

if [ "$mode" = "exec" ]; then
  if [ "${2:-}" = "resume" ] && [ "${3:-}" = "--help" ]; then
    printf '%s\n' 'Usage: codex exec resume [SESSION_ID] [PROMPT]'
    exit 0
  fi
  if [ -n "${LIMITWISE_FAKE_ARGS_PATH:-}" ]; then
    printf '%s\n' "$@" >> "$LIMITWISE_FAKE_ARGS_PATH"
  fi
  if [ "${LIMITWISE_FAKE_EMIT_SESSION:-1}" != 0 ]; then
    printf '%s\n' '{"type":"thread.started","thread_id":"fake-session"}'
  fi
  if [ "${LIMITWISE_FAKE_EXEC_SCENARIO:-success}" = "quota_interrupt" ]; then
    trap 'exit 130' INT TERM
    sleep 10
    exit 1
  fi
  printf '%s\n' '{"type":"turn.completed","usage":{"input_tokens":30,"cached_input_tokens":10,"output_tokens":5,"reasoning_output_tokens":2}}'
  [ "${LIMITWISE_FAKE_EXEC_SCENARIO:-success}" != "fail" ] || exit 1
  exit 0
fi

exit 2
