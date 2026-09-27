use crate::app::ApplicationService;
#[cfg(test)]
use crate::diagnostic;
use crate::prediction::EstimateBatchInput;
use crate::store::{ConfirmedRetryTaskInput, RetryTaskOptions, ScheduleBatchInput, TaskUpdate};
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

pub(crate) fn call_tool(name: &str, arguments: Value) -> Result<Value, String> {
    let application = ApplicationService;
    let value = match name {
        "model_catalog" => serde_json::to_value(application.models()).map_err(|e| e.to_string())?,
        "diagnostics_snapshot" => {
            serde_json::to_value(application.diagnostics()?).map_err(|e| e.to_string())?
        }
        "usage_snapshot" => {
            serde_json::to_value(application.usage()?).map_err(|e| e.to_string())?
        }
        "setup_service" => json!({"message": application.setup_service()?}),
        "schedule_batch" => {
            let input: ScheduleBatchInput =
                serde_json::from_value(arguments).map_err(|e| e.to_string())?;
            serde_json::to_value(application.schedule_batch(input)?).map_err(|e| e.to_string())?
        }
        "preview_retry_task" => {
            let task_id = required_string(&arguments, "task_id")?.to_string();
            let mut options = arguments;
            options
                .as_object_mut()
                .ok_or_else(|| "arguments must be an object".to_string())?
                .remove("task_id");
            let input: RetryTaskOptions =
                serde_json::from_value(options).map_err(|e| e.to_string())?;
            serde_json::to_value(application.retry_preview(&task_id, input)?)
                .map_err(|e| e.to_string())?
        }
        "retry_task" => {
            let task_id = required_string(&arguments, "task_id")?.to_string();
            let mut input_value = arguments;
            input_value
                .as_object_mut()
                .ok_or_else(|| "arguments must be an object".to_string())?
                .remove("task_id");
            let input: ConfirmedRetryTaskInput =
                serde_json::from_value(input_value).map_err(|e| e.to_string())?;
            serde_json::to_value(application.retry_task(&task_id, input)?)
                .map_err(|e| e.to_string())?
        }
        "list_tasks" => {
            let status = arguments.get("status").and_then(Value::as_str);
            serde_json::to_value(application.list_tasks(status)?).map_err(|e| e.to_string())?
        }
        "get_task_status" => {
            let task_id = required_string(&arguments, "task_id")?;
            serde_json::to_value(application.task_status(task_id)?).map_err(|e| e.to_string())?
        }
        "task_usage_stats" => {
            serde_json::to_value(application.task_usage_stats()?).map_err(|e| e.to_string())?
        }
        "estimate_batch_usage" => {
            let input: EstimateBatchInput =
                serde_json::from_value(arguments).map_err(|e| e.to_string())?;
            serde_json::to_value(application.estimate_batch(input)?).map_err(|e| e.to_string())?
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
            serde_json::to_value(application.update_task(&task_id, update)?)
                .map_err(|e| e.to_string())?
        }
        "cancel_task" => {
            let task_id = required_string(&arguments, "task_id")?;
            serde_json::to_value(application.cancel_task(task_id)?).map_err(|e| e.to_string())?
        }
        _ => return Err(format!("unknown tool '{name}'")),
    };
    tool_result(value)
}

pub(crate) fn tool_result(value: Value) -> Result<Value, String> {
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
    let catalog = ApplicationService.models();
    let model_names = catalog
        .models
        .iter()
        .map(|model| model.model.clone())
        .collect::<Vec<_>>();
    let mut effort_names = catalog
        .models
        .iter()
        .flat_map(|model| {
            model
                .supported_reasoning_efforts
                .iter()
                .map(|effort| effort.reasoning_effort.clone())
        })
        .collect::<Vec<_>>();
    effort_names.sort();
    effort_names.dedup();
    let model_schema = json!({
        "type":"string",
        "enum":model_names,
        "description":"A visible model returned by Codex model/list."
    });
    let effort_schema = json!({
        "type":"string",
        "enum":effort_names,
        "description":"Must be supported by the selected model according to Codex model/list."
    });
    vec![
        tool("model_catalog", "Read the visible models and per-model reasoning efforts reported by the installed Codex app-server.", json!({"type":"object","properties":{},"additionalProperties":false}), true),
        json!({
            "name": "diagnostics_snapshot",
            "description": "Read a redacted, non-mutating snapshot of LimitWise, Codex, plugin, storage, service, and log health.",
            "inputSchema": {"type":"object","properties":{},"additionalProperties":false},
            "annotations": {
                "readOnlyHint": true,
                "destructiveHint": false,
                "idempotentHint": true,
                "openWorldHint": false
            }
        }),
        tool("usage_snapshot", "Read current weekly and optional rolling five-hour Codex usage. Missing five-hour telemetry is returned as null; ambiguous or missing weekly telemetry fails closed.", json!({"type":"object","properties":{},"additionalProperties":false}), true),
        tool("setup_service", "Install and start the native user background service. Requires explicit approval. LimitWise is tested only on Linux x86-64; macOS, including Apple Silicon, is untested.", json!({"type":"object","properties":{},"additionalProperties":false}), false),
        tool("schedule_batch", "Create a confirmed one-off, sequential, or quota-reset continuation batch with a percentage or token budget and optional five-hour cap. Never call while in Plan mode.", json!({
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
                "five_hour_cap_percent":{"type":"number","exclusiveMinimum":0,"maximum":100,"description":"Optional maximum percentage points this batch may consume in each provider 5-hour window. The global 10% reserve still applies."},
                "networked_confirmed":{"type":"boolean","description":"Separate explicit acknowledgement required when any task selects permission_profile=networked."},
                "tasks":{"type":"array","minItems":1,"items":{"type":"object","additionalProperties":false,
                    "required":["title","prompt","cwd","difficulty"],
                    "oneOf":[
                        {"required":["run_at"]},
                        {"required":["after_previous"],"properties":{"after_previous":{"const":true}}},
                        {"required":["continue_from_task_id"]}
                    ],
                    "properties":{
                        "title":{"type":"string"}, "prompt":{"type":"string"}, "success_criteria":{"type":"string"},
                        "cwd":{"type":"string","description":"Absolute existing project directory"},
                        "run_at":{"type":"string","description":"Exact future local timestamp in RFC3339 form with explicit UTC offset. Required unless after_previous is true; relative times are not accepted."},
                        "after_previous":{"type":"boolean","default":false,"description":"When true, omit run_at and start only after the immediately preceding task completes successfully."},
                        "continue_from_task_id":{"type":"string","minLength":1,"description":"Continue quota-limited unfinished work from this quota_interrupted or quota_skipped task after its provider 5-hour reset. Omit run_at and after_previous."},
                        "timezone":{"type":"string","description":"IANA timezone; defaults to system timezone"},
                        "difficulty":{"type":"string","enum":["simple","standard","complex","exceptional"]},
                        "model":model_schema.clone(),
                        "effort":effort_schema.clone()
                        ,"permission_profile":{"type":"string","enum":["restricted","networked"],"default":"restricted","description":"restricted keeps network and web search off; networked enables both while workspace-write, apps disabled, and approval never remain fixed."}
                    }
                }}
            }
        }), false),
        tool("preview_retry_task", "Read the single eligible retry or quota-resume plan, copied specification, fresh budget, execution/reset time, session behavior, attempt number, and current local estimate without writing.", retry_schema(false, &model_schema, &effort_schema), true),
        tool("retry_task", "Create one explicitly confirmed, idempotent retry or quota-resume attempt without changing predecessor history. Never call while in Plan mode.", retry_schema(true, &model_schema, &effort_schema), false),
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
                        "model":model_schema.clone(),
                        "effort":effort_schema.clone()
                    }
                }}
            }
        }), true),
        tool("update_task", "Update a task that has not started.", json!({"type":"object","required":["task_id","changes"],"additionalProperties":false,"properties":{"task_id":{"type":"string"},"changes":{"type":"object","additionalProperties":false,"properties":{"title":{"type":"string"},"prompt":{"type":"string"},"success_criteria":{"type":"string"},"cwd":{"type":"string"},"run_at":{"type":"string"},"timezone":{"type":"string"},"difficulty":{"type":"string","enum":["simple","standard","complex","exceptional"]},"model":model_schema.clone(),"effort":effort_schema.clone(),"permission_profile":{"type":"string","enum":["restricted","networked"]},"networked_confirmed":{"type":"boolean","description":"Separate explicit acknowledgement required when selecting networked."}}}}}), false),
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
            "idempotentHint": matches!(name, "schedule_batch" | "setup_service" | "retry_task"),
            "openWorldHint": false
        }
    })
}

fn retry_schema(confirm: bool, model_schema: &Value, effort_schema: &Value) -> Value {
    let mut required = vec!["task_id", "budget_mode"];
    if confirm {
        required.extend(["idempotency_key", "confirmed"]);
    }
    let mut schema = json!({
        "type":"object",
        "required":required,
        "additionalProperties":false,
        "oneOf":[
            {"properties":{"budget_mode":{"const":"percentage"}},"required":["weekly_cap_percent"],"not":{"required":["token_cap"]}},
            {"properties":{"budget_mode":{"const":"tokens"}},"required":["token_cap"],"not":{"required":["weekly_cap_percent"]}}
        ],
        "properties":{
            "task_id":{"type":"string","minLength":1},
            "budget_mode":{"type":"string","enum":["percentage","tokens"]},
            "weekly_cap_percent":{"type":"number","exclusiveMinimum":0,"maximum":100},
            "token_cap":{"type":"integer","minimum":1,"maximum":1000000000},
                "five_hour_cap_percent":{"type":"number","exclusiveMinimum":0,"maximum":100,"description":"Optional attempt allowance; the global 10% rolling five-hour reserve applies whenever telemetry is available."},
            "run_at":{"type":"string","description":"Required future RFC3339 time for an ordinary retry; omit for quota resume because the provider reset is authoritative."},
            "timezone":{"type":"string","description":"IANA timezone; defaults to the source task timezone."},
            "model":model_schema.clone(),
            "effort":effort_schema.clone()
            ,"permission_profile":{"type":"string","enum":["restricted","networked"],"description":"Omit to inherit the source task profile."}
            ,"networked_confirmed":{"type":"boolean","description":"Separate explicit acknowledgement required to create a networked retry or resume."}
        }
    });
    if confirm {
        let properties = schema
            .get_mut("properties")
            .and_then(Value::as_object_mut)
            .expect("retry properties");
        properties.insert(
            "idempotency_key".to_string(),
            json!({"type":"string","minLength":1}),
        );
        properties.insert("confirmed".to_string(), json!({"const":true}));
    }
    schema
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
            "model_catalog",
            "diagnostics_snapshot",
            "usage_snapshot",
            "setup_service",
            "schedule_batch",
            "preview_retry_task",
            "retry_task",
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
    fn diagnostics_schema_is_read_only_idempotent_and_closed_world() {
        let diagnostics = tool_definitions()
            .into_iter()
            .find(|value| value.get("name").and_then(Value::as_str) == Some("diagnostics_snapshot"))
            .unwrap();
        assert_eq!(
            diagnostics.pointer("/inputSchema/additionalProperties"),
            Some(&json!(false))
        );
        assert_eq!(
            diagnostics.pointer("/annotations/readOnlyHint"),
            Some(&json!(true))
        );
        assert_eq!(
            diagnostics.pointer("/annotations/destructiveHint"),
            Some(&json!(false))
        );
        assert_eq!(
            diagnostics.pointer("/annotations/idempotentHint"),
            Some(&json!(true))
        );
        assert_eq!(
            diagnostics.pointer("/annotations/openWorldHint"),
            Some(&json!(false))
        );
    }

    #[test]
    fn diagnostics_mcp_result_matches_cli_json_contract() {
        let report = diagnostic::DiagnosticReport {
            schema_version: diagnostic::DIAGNOSTIC_SCHEMA_VERSION.to_string(),
            overall: diagnostic::DiagnosticStatus::Warn,
            generated_at: 1_700_000_000,
            platform: diagnostic::DiagnosticPlatform {
                os: "linux".to_string(),
                architecture: "x86_64".to_string(),
            },
            checks: vec![diagnostic::DiagnosticCheck {
                id: "service.state".to_string(),
                status: diagnostic::DiagnosticStatus::Warn,
                summary: "Optional background service is not installed".to_string(),
                evidence: Vec::new(),
                remediation: None,
            }],
        };
        let mut cli = Vec::new();
        diagnostic::write_json_report(&report, &mut cli).unwrap();
        let cli_value: Value = serde_json::from_slice(&cli).unwrap();
        let mcp = tool_result(serde_json::to_value(&report).unwrap()).unwrap();
        assert_eq!(mcp.get("structuredContent"), Some(&cli_value));
        assert_eq!(
            serde_json::from_str::<Value>(
                mcp.pointer("/content/0/text").unwrap().as_str().unwrap()
            )
            .unwrap(),
            cli_value
        );
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
        assert!(schema
            .pointer("/properties/five_hour_cap_percent")
            .is_some());
        assert!(schema.pointer("/properties/cap_percent").is_none());
        assert!(schema
            .pointer("/properties/tasks/items/properties/after_previous")
            .is_some());
        assert!(schema
            .pointer("/properties/tasks/items/properties/continue_from_task_id")
            .is_some());
        assert_eq!(
            schema.pointer("/properties/tasks/items/properties/permission_profile/enum"),
            Some(&json!(["restricted", "networked"]))
        );
        assert!(schema.pointer("/properties/networked_confirmed").is_some());
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

    #[test]
    fn schemas_and_catalog_expose_current_models_and_extended_efforts() {
        let definitions = tool_definitions();
        let schedule = definitions
            .iter()
            .find(|value| value.get("name").and_then(Value::as_str) == Some("schedule_batch"))
            .unwrap();
        let models = schedule
            .pointer("/inputSchema/properties/tasks/items/properties/model/enum")
            .and_then(Value::as_array)
            .unwrap();
        let efforts = schedule
            .pointer("/inputSchema/properties/tasks/items/properties/effort/enum")
            .and_then(Value::as_array)
            .unwrap();
        assert!(models.contains(&json!("gpt-6-astra")));
        assert!(models.contains(&json!("gpt-6-sol")));
        assert!(models.contains(&json!("gpt-6-luna")));
        assert!(efforts.contains(&json!("max")));
        assert!(efforts.contains(&json!("ultra")));

        let result = call_tool("model_catalog", json!({})).unwrap();
        assert_eq!(
            result.pointer("/structuredContent/models/0/model"),
            Some(&json!("gpt-6-astra"))
        );
    }

    #[test]
    fn retry_schemas_split_read_only_preview_from_idempotent_confirmation() {
        let definitions = tool_definitions();
        let preview = definitions
            .iter()
            .find(|value| value.get("name").and_then(Value::as_str) == Some("preview_retry_task"))
            .unwrap();
        let confirm = definitions
            .iter()
            .find(|value| value.get("name").and_then(Value::as_str) == Some("retry_task"))
            .unwrap();
        assert_eq!(
            preview.pointer("/annotations/readOnlyHint"),
            Some(&json!(true))
        );
        assert_eq!(
            confirm.pointer("/annotations/readOnlyHint"),
            Some(&json!(false))
        );
        assert_eq!(
            confirm.pointer("/annotations/idempotentHint"),
            Some(&json!(true))
        );
        assert!(preview
            .pointer("/inputSchema/properties/idempotency_key")
            .is_none());
        assert_eq!(
            confirm.pointer("/inputSchema/properties/confirmed/const"),
            Some(&json!(true))
        );
        assert_eq!(
            confirm.pointer("/inputSchema/additionalProperties"),
            Some(&json!(false))
        );
        assert!(preview
            .pointer("/inputSchema/properties/permission_profile")
            .is_some());
        assert!(confirm
            .pointer("/inputSchema/properties/networked_confirmed")
            .is_some());
    }

    #[test]
    fn direct_mcp_calls_reject_permission_injection_before_writes() {
        let error = call_tool(
            "schedule_batch",
            json!({
                "idempotency_key":"mcp-profile-injection",
                "budget_mode":"tokens",
                "token_cap":1,
                "tasks":[{
                    "title":"unsafe",
                    "prompt":"unsafe",
                    "cwd":"/tmp",
                    "run_at":"2099-01-01T00:00:00+00:00",
                    "timezone":"UTC",
                    "difficulty":"simple",
                    "permission_profile":"networked --sandbox danger-full-access"
                }]
            }),
        )
        .unwrap_err();
        assert!(error.contains("unknown variant"));

        let error = call_tool(
            "schedule_batch",
            json!({
                "idempotency_key":"mcp-sandbox-injection",
                "budget_mode":"tokens",
                "token_cap":1,
                "sandbox":"danger-full-access",
                "tasks":[]
            }),
        )
        .unwrap_err();
        assert!(error.contains("unknown field"));
    }
}
