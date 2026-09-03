#!/bin/sh
set -eu

mode=${1:-}
if [ "$mode" = "app-server" ]; then
  while IFS= read -r request; do
    case "$request" in
      *'"method":"initialize"'*)
        printf '%s\n' '{"id":1,"result":{"serverInfo":{"name":"fake","version":"1"}}}'
        ;;
      *'"method":"account/rateLimits/read"'*)
        request_id=$(printf '%s' "$request" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
        printf '{"id":%s,"result":{"rateLimits":{"primary":{"usedPercent":10,"windowDurationMins":300,"resetsAt":4102444800},"secondary":{"usedPercent":20,"windowDurationMins":10080,"resetsAt":4102444800}}}}\n' "$request_id"
        ;;
    esac
  done
  exit 0
fi

if [ "$mode" = "exec" ]; then
  if [ -n "${LIMITWISE_FAKE_ARGS_PATH:-}" ]; then
    printf '%s\n' "$@" >> "$LIMITWISE_FAKE_ARGS_PATH"
  fi
  printf '%s\n' '{"type":"thread.started","thread_id":"fake-session"}'
  printf '%s\n' '{"type":"turn.completed","usage":{"input_tokens":30,"cached_input_tokens":10,"output_tokens":5,"reasoning_output_tokens":2}}'
  exit 0
fi

exit 2
