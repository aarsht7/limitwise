use crate::prediction::{estimate, EstimateBatchInput};
use crate::service;
use crate::store::{now_epoch, ScheduleBatchInput, Store, TaskUpdate};
use crate::usage::UsageClient;
use serde_json::{json, Value};
use std::io::{self, BufRead, Write};

pub fn serve() -> Result<(), String> {
    let stdin = io::stdin();
    let mut stdout = io::stdout().lock();
    for line in stdin.lock().lines() {
        let line = line.map_err(|e| e.to_string())?;
        if line.trim().is_empty() {
            continue;
        }
        let request: Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(error) => {
                write_response(
                    &mut stdout,
                    json!({
                        "jsonrpc": "2.0", "id": null,
                        "error": {"code": -32700, "message": error.to_string()}
                    }),
                )?;
                continue;
            }
        };
        if request.get("id").is_none() {
            continue;
        }
        let response = dispatch(&request);
        write_response(&mut stdout, response)?;
    }
    Ok(())
}

fn dispatch(request: &Value) -> Value {
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    let method = request.get("method").and_then(Value::as_str).unwrap_or("");
    let result = match method {
        "initialize" => Ok(json!({
            "protocolVersion": request.pointer("/params/protocolVersion").cloned().unwrap_or(json!("2025-06-18")),
            "capabilities": {"tools": {"listChanged": false}},
            "serverInfo": {"name": "limitwise", "version": env!("CARGO_PKG_VERSION")}
        })),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({"tools": tool_definitions()})),
        "tools/call" => call_tool(
            request
                .pointer("/params/name")
                .and_then(Value::as_str)
                .unwrap_or(""),
            request
                .pointer("/params/arguments")
                .cloned()
                .unwrap_or_else(|| json!({})),
        ),
        _ => {
            return json!({"jsonrpc":"2.0", "id":id, "error":{"code":-32601,"message":"method not found"}})
        }
    };
    match result {
        Ok(value) => json!({"jsonrpc":"2.0", "id":id, "result":value}),
        Err(error) => json!({"jsonrpc":"2.0", "id":id, "result":{
            "content":[{"type":"text","text":error}], "isError":true
        }}),
    }
}

fn call_tool(name: &str, arguments: Value) -> Result<Value, String> {
    let value = match name {
        "usage_snapshot" => {
            serde_json::to_value(UsageClient::default().fetch()?).map_err(|e| e.to_string())?
        }
        "setup_service" => json!({"message": service::setup()?}),
        "schedule_batch" => {
            let input: ScheduleBatchInput =
                serde_json::from_value(arguments).map_err(|e| e.to_string())?;
            let mut store = Store::open()?;
            serde_json::to_value(store.schedule_batch(input)?).map_err(|e| e.to_string())?
        }
        "list_tasks" => {
            let status = arguments.get("status").and_then(Value::as_str);
            serde_json::to_value(Store::open()?.list_tasks(status)?).map_err(|e| e.to_string())?
        }
        "get_task_status" => {
            let task_id = required_string(&arguments, "task_id")?;
            serde_json::to_value(
                Store::open()?
                    .task_status(task_id)?
                    .ok_or_else(|| "task not found".to_string())?,
            )
            .map_err(|e| e.to_string())?
        }
        "task_usage_stats" => serde_json::to_value(
            Store::open()?.task_usage_stats(now_epoch(), &crate::config::system_timezone())?,
        )
        .map_err(|e| e.to_string())?,
        "estimate_batch_usage" => {
            let input: EstimateBatchInput =
                serde_json::from_value(arguments).map_err(|e| e.to_string())?;
            let store = Store::open()?;
            serde_json::to_value(estimate(&store, input, now_epoch())?)
                .map_err(|e| e.to_string())?
        }
        "update_task" => {
            let task_id = required_string(&arguments, "task_id")?.to_string();
            let update: TaskUpdate = serde_json::from_value(
                arguments
                    .get("changes")
                    .cloned()
                    .ok_or_else(|| "changes is required".to_string())?,
            )
            .map_err(|e| e.to_string())?;
            let mut store = Store::open()?;
            serde_json::to_value(store.update_task(&task_id, update)?).map_err(|e| e.to_string())?
        }
        "cancel_task" => {
            let task_id = required_string(&arguments, "task_id")?;
            serde_json::to_value(Store::open()?.cancel_task(task_id)?).map_err(|e| e.to_string())?
        }
        _ => return Err(format!("unknown tool '{name}'")),
    };
    let text = serde_json::to_string_pretty(&value).map_err(|e| e.to_string())?;
    Ok(json!({"content":[{"type":"text","text":text}], "structuredContent":value}))
}

fn required_string<'a>(value: &'a Value, field: &str) -> Result<&'a str, String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .filter(|v| !v.trim().is_empty())
        .ok_or_else(|| format!("{field} is required"))
}

fn write_response(writer: &mut impl Write, response: Value) -> Result<(), String> {
    serde_json::to_writer(&mut *writer, &response).map_err(|e| e.to_string())?;
    writer.write_all(b"\n").map_err(|e| e.to_string())?;
    writer.flush().map_err(|e| e.to_string())
}

fn tool_definitions() -> Vec<Value> {
    vec![
        tool("usage_snapshot", "Read current rolling five-hour and weekly Codex usage. Fails closed on ambiguous telemetry.", json!({"type":"object","properties":{},"additionalProperties":false}), true),
        tool("setup_service", "Install and start the native user background service. Requires explicit approval. LimitWise is tested only on Linux x86-64; macOS, including Apple Silicon, is untested.", json!({"type":"object","properties":{},"additionalProperties":false}), false),
        tool("schedule_batch", "Create a confirmed one-off or sequential task batch with a percentage or token budget. Never call while in Plan mode.", json!({
            "type":"object", "required":["idempotency_key","budget_mode","tasks"], "additionalProperties":false,
            "oneOf":[
                {"properties":{"budget_mode":{"const":"percentage"}},"required":["weekly_cap_percent"],"not":{"required":["token_cap"]}},
                {"properties":{"budget_mode":{"const":"tokens"}},"required":["token_cap"],"not":{"required":["weekly_cap_percent"]}}
            ],
            "properties":{
                "idempotency_key":{"type":"string","minLength":1},
                "budget_mode":{"type":"string","enum":["percentage","tokens"],"description":"Choose percentage for a per-weekly-window allowance or tokens for one total batch token cap."},
                "weekly_cap_percent":{"type":"number","exclusiveMinimum":0,"maximum":100,"description":"Maximum percentage points from the full weekly limit for this batch in each weekly window; 1 means exactly 1% of the total weekly limit."},
                "token_cap":{"type":"integer","minimum":1,"maximum":1000000000,"description":"Maximum total input plus output tokens for the entire batch. Cached input tokens are included; reasoning tokens are already included in output tokens."},
                "tasks":{"type":"array","minItems":1,"items":{"type":"object","additionalProperties":false,
                    "required":["title","prompt","cwd","difficulty"],
                    "oneOf":[
                        {"required":["run_at"]},
                        {"required":["after_previous"],"properties":{"after_previous":{"const":true}}}
                    ],
                    "properties":{
                        "title":{"type":"string"}, "prompt":{"type":"string"}, "success_criteria":{"type":"string"},
                        "cwd":{"type":"string","description":"Absolute existing project directory"},
                        "run_at":{"type":"string","description":"Exact future local timestamp in RFC3339 form with explicit UTC offset. Required unless after_previous is true; relative times are not accepted."},
                        "after_previous":{"type":"boolean","default":false,"description":"When true, omit run_at and start only after the immediately preceding task completes successfully."},
                        "timezone":{"type":"string","description":"IANA timezone; defaults to system timezone"},
                        "difficulty":{"type":"string","enum":["simple","standard","complex","exceptional"]},
                        "model":{"type":"string","enum":["gpt-5.6-luna","gpt-5.6-terra","gpt-5.6-sol"]},
                        "effort":{"type":"string","enum":["low","medium","high","xhigh"]}
                    }
                }}
            }
        }), false),
        tool("list_tasks", "List scheduled and historical local tasks.", json!({"type":"object","additionalProperties":false,"properties":{"status":{"type":"string"}}}), true),
        tool("get_task_status", "Read one local task and its current status.", json!({"type":"object","required":["task_id"],"additionalProperties":false,"properties":{"task_id":{"type":"string"}}}), true),
        tool("task_usage_stats", "Read stored scheduled-run token totals for rolling year, month, and week windows, daily totals for the last seven local calendar days, and individual runs from the last seven days.", json!({"type":"object","properties":{},"additionalProperties":false}), true),
        tool("estimate_batch_usage", "Estimate likely p50 and conservative p90 token and weekly-percentage usage from comparable completed local runs, then assess the proposed batch cap. Predictions are not guarantees and never change the cap.", json!({
            "type":"object", "required":["budget_mode","tasks"], "additionalProperties":false,
            "oneOf":[
                {"properties":{"budget_mode":{"const":"percentage"}},"required":["weekly_cap_percent"],"not":{"required":["token_cap"]}},
                {"properties":{"budget_mode":{"const":"tokens"}},"required":["token_cap"],"not":{"required":["weekly_cap_percent"]}}
            ],
            "properties":{
                "budget_mode":{"type":"string","enum":["percentage","tokens"]},
                "weekly_cap_percent":{"type":"number","exclusiveMinimum":0,"maximum":100,"description":"Proposed percentage points from the full weekly limit."},
                "token_cap":{"type":"integer","minimum":1,"maximum":1000000000,"description":"Proposed total batch token cap."},
                "tasks":{"type":"array","minItems":1,"items":{"type":"object","additionalProperties":false,
                    "required":["title","difficulty"],
                    "properties":{
                        "title":{"type":"string","minLength":1},
                        "difficulty":{"type":"string","enum":["simple","standard","complex","exceptional"]},
                        "model":{"type":"string","enum":["gpt-5.6-luna","gpt-5.6-terra","gpt-5.6-sol"]},
                        "effort":{"type":"string","enum":["low","medium","high","xhigh"]}
                    }
                }}
            }
        }), true),
        tool("update_task", "Update a task that has not started.", json!({"type":"object","required":["task_id","changes"],"additionalProperties":false,"properties":{"task_id":{"type":"string"},"changes":{"type":"object"}}}), false),
        tool("cancel_task", "Cancel a task that has not started.", json!({"type":"object","required":["task_id"],"additionalProperties":false,"properties":{"task_id":{"type":"string"}}}), false),
    ]
}

fn tool(name: &str, description: &str, input_schema: Value, read_only: bool) -> Value {
    json!({
        "name": name,
        "description": description,
        "inputSchema": input_schema,
        "annotations": {
            "readOnlyHint": read_only,
            "destructiveHint": name == "cancel_task",
            "idempotentHint": matches!(name, "schedule_batch" | "setup_service"),
            "openWorldHint": false
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exposes_required_tools() {
        let definitions = tool_definitions();
        let names: Vec<_> = definitions
            .iter()
            .filter_map(|v| v.get("name").and_then(Value::as_str))
            .collect();
        for required in [
            "usage_snapshot",
            "setup_service",
            "schedule_batch",
            "list_tasks",
            "get_task_status",
            "task_usage_stats",
            "estimate_batch_usage",
            "update_task",
            "cancel_task",
        ] {
            assert!(names.contains(&required));
        }
    }

    #[test]
    fn schedule_schema_exposes_budget_choice_and_supports_chaining() {
        let schedule = tool_definitions()
            .into_iter()
            .find(|value| value.get("name").and_then(Value::as_str) == Some("schedule_batch"))
            .unwrap();
        let schema = schedule.get("inputSchema").unwrap();
        assert!(schema.pointer("/properties/budget_mode").is_some());
        assert!(schema.pointer("/properties/weekly_cap_percent").is_some());
        assert!(schema.pointer("/properties/token_cap").is_some());
        assert!(schema.pointer("/properties/cap_percent").is_none());
        assert!(schema
            .pointer("/properties/tasks/items/properties/after_previous")
            .is_some());
    }

    #[test]
    fn estimate_schema_exposes_routes_and_budget_choice() {
        let estimate = tool_definitions()
            .into_iter()
            .find(|value| value.get("name").and_then(Value::as_str) == Some("estimate_batch_usage"))
            .unwrap();
        let schema = estimate.get("inputSchema").unwrap();
        assert!(schema.pointer("/properties/budget_mode").is_some());
        assert!(schema.pointer("/properties/weekly_cap_percent").is_some());
        assert!(schema.pointer("/properties/token_cap").is_some());
        assert!(schema
            .pointer("/properties/tasks/items/properties/difficulty")
            .is_some());
        assert!(schema
            .pointer("/properties/tasks/items/properties/model")
            .is_some());
        assert!(schema
            .pointer("/properties/tasks/items/properties/effort")
            .is_some());
    }
}
