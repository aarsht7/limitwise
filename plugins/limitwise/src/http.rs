use crate::app::ApplicationService;
use crate::planner::PlanBatchInput;
use crate::prediction::EstimateBatchInput;
use crate::store::{
    BatchEditInput, BatchEditSessionInput, ConfirmedRetryTaskInput, RetryTaskOptions,
    ScheduleBatchInput, TaskUpdate,
};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{ErrorKind, Read, Write};
use std::net::{Ipv4Addr, SocketAddrV4, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

const INDEX_HTML: &[u8] = include_bytes!("../ui/dist/index.html");
const APP_JS: &[u8] = include_bytes!("../ui/dist/assets/app.js");
const APP_CSS: &[u8] = include_bytes!("../ui/dist/assets/app.css");
const MAX_HEADER_BYTES: usize = 32 * 1024;
const MAX_BODY_BYTES: usize = 64 * 1024;
const IO_TIMEOUT: Duration = Duration::from_secs(5);

static STOP_REQUESTED: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct UiOptions {
    no_open: bool,
    port: Option<u16>,
}

struct UiState {
    origin: String,
    authority: String,
    bearer: String,
    bootstrap: Mutex<Option<String>>,
    application: ApplicationService,
    launch_directory: String,
}

#[derive(Debug)]
struct Request {
    method: String,
    target: String,
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
}

#[derive(Debug)]
struct Response {
    status: u16,
    content_type: &'static str,
    body: Vec<u8>,
    cache_control: &'static str,
}

#[derive(Debug)]
struct ApiError {
    status: u16,
    code: &'static str,
    message: String,
    fields: Map<String, Value>,
}

#[derive(Deserialize)]
struct BootstrapInput {
    bootstrap: String,
}

#[derive(Deserialize)]
struct SetupInput {
    confirmed: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DirectoryPickerInput {
    confirmed: bool,
}

#[derive(Deserialize)]
struct HttpEstimateInput {
    #[serde(flatten)]
    estimate: EstimateBatchInput,
    #[serde(default)]
    schedule: Option<ScheduleBatchInput>,
}

pub fn run_cli(arguments: Vec<String>) -> Result<(), String> {
    let options = parse_options(arguments)?;
    let requested_port = options.port.unwrap_or(0);
    let listener = TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, requested_port))
        .map_err(|error| {
            format!("cannot bind LimitWise UI to 127.0.0.1:{requested_port}: {error}")
        })?;
    listener
        .set_nonblocking(true)
        .map_err(|error| format!("cannot configure LimitWise UI listener: {error}"))?;
    let address = listener
        .local_addr()
        .map_err(|error| format!("cannot read LimitWise UI address: {error}"))?;
    if !address.ip().is_loopback() {
        return Err("LimitWise UI refused a non-loopback listener".to_string());
    }

    let bearer = random_secret()?;
    let bootstrap = random_secret()?;
    let origin = format!("http://127.0.0.1:{}", address.port());
    let authority = format!("127.0.0.1:{}", address.port());
    let launch_url = format!("{origin}/#bootstrap={bootstrap}");
    let launch_directory = canonical_directory(
        &std::env::current_dir()
            .map_err(|error| format!("cannot determine the UI launch directory: {error}"))?,
    )?;
    let state = Arc::new(UiState {
        origin,
        authority,
        bearer,
        bootstrap: Mutex::new(Some(bootstrap)),
        application: ApplicationService,
        launch_directory,
    });

    STOP_REQUESTED.store(false, Ordering::SeqCst);
    install_signal_handlers();
    if options.no_open {
        println!("LimitWise UI: {launch_url}");
    } else if let Err(error) = open_browser(&launch_url) {
        eprintln!("LimitWise could not open a browser: {error}");
        println!("LimitWise UI: {launch_url}");
    }

    while !STOP_REQUESTED.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((stream, peer)) => {
                if !peer.ip().is_loopback() {
                    continue;
                }
                handle_connection(stream, &state);
            }
            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(25));
            }
            Err(error) => return Err(format!("LimitWise UI listener failed: {error}")),
        }
    }
    Ok(())
}

fn parse_options(arguments: Vec<String>) -> Result<UiOptions, String> {
    let mut options = UiOptions {
        no_open: false,
        port: None,
    };
    let mut arguments = arguments.into_iter();
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--no-open" if !options.no_open => options.no_open = true,
            "--port" if options.port.is_none() => {
                let value = arguments
                    .next()
                    .ok_or_else(|| "--port requires a port from 1 to 65535".to_string())?;
                let port = value
                    .parse::<u16>()
                    .ok()
                    .filter(|port| *port != 0)
                    .ok_or_else(|| "--port requires a port from 1 to 65535".to_string())?;
                options.port = Some(port);
            }
            _ => return Err("usage: limitwise ui [--no-open] [--port <port>]".to_string()),
        }
    }
    Ok(options)
}

fn handle_connection(mut stream: TcpStream, state: &UiState) {
    let _ = stream.set_read_timeout(Some(IO_TIMEOUT));
    let _ = stream.set_write_timeout(Some(IO_TIMEOUT));
    let response = match read_request(&mut stream) {
        Ok(request) => dispatch(request, state),
        Err(error) => error_response(error),
    };
    let _ = write_response(&mut stream, response);
}

fn read_request(stream: &mut TcpStream) -> Result<Request, ApiError> {
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 4096];
    let header_end = loop {
        let count = stream.read(&mut chunk).map_err(|_| {
            ApiError::new(400, "invalid_request", "Could not read the HTTP request")
        })?;
        if count == 0 {
            return Err(ApiError::new(
                400,
                "invalid_request",
                "HTTP request ended before its headers",
            ));
        }
        bytes.extend_from_slice(&chunk[..count]);
        if let Some(position) = find_header_end(&bytes) {
            break position;
        }
        if bytes.len() > MAX_HEADER_BYTES {
            return Err(ApiError::new(
                431,
                "headers_too_large",
                "HTTP headers exceed the 32768-byte limit",
            ));
        }
    };
    if header_end > MAX_HEADER_BYTES {
        return Err(ApiError::new(
            431,
            "headers_too_large",
            "HTTP headers exceed the 32768-byte limit",
        ));
    }
    let header_text = std::str::from_utf8(&bytes[..header_end])
        .map_err(|_| ApiError::new(400, "invalid_request", "HTTP headers must be valid UTF-8"))?;
    let mut lines = header_text.split("\r\n");
    let request_line = lines
        .next()
        .ok_or_else(|| ApiError::new(400, "invalid_request", "Missing HTTP request line"))?;
    let mut request_parts = request_line.split_whitespace();
    let method = request_parts.next().unwrap_or("").to_string();
    let target = request_parts.next().unwrap_or("").to_string();
    let version = request_parts.next().unwrap_or("");
    if method.is_empty()
        || !target.starts_with('/')
        || version != "HTTP/1.1"
        || request_parts.next().is_some()
    {
        return Err(ApiError::new(
            400,
            "invalid_request",
            "Malformed HTTP request line",
        ));
    }
    let mut headers = BTreeMap::new();
    for line in lines.filter(|line| !line.is_empty()) {
        let (name, value) = line
            .split_once(':')
            .ok_or_else(|| ApiError::new(400, "invalid_request", "Malformed HTTP header"))?;
        headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
    }
    if headers.contains_key("transfer-encoding") {
        return Err(ApiError::new(
            400,
            "unsupported_transfer_encoding",
            "Chunked request bodies are not supported",
        ));
    }
    let content_length = headers
        .get("content-length")
        .map(|value| value.parse::<usize>())
        .transpose()
        .map_err(|_| ApiError::new(400, "invalid_request", "Content-Length must be an integer"))?
        .unwrap_or(0);
    if content_length > MAX_BODY_BYTES {
        return Err(ApiError::new(
            413,
            "payload_too_large",
            "JSON request body exceeds the 65536-byte limit",
        ));
    }
    let body_start = header_end + 4;
    while bytes.len() < body_start + content_length {
        let count = stream.read(&mut chunk).map_err(|_| {
            ApiError::new(
                400,
                "invalid_request",
                "Could not read the HTTP request body",
            )
        })?;
        if count == 0 {
            return Err(ApiError::new(
                400,
                "invalid_request",
                "HTTP request body ended before Content-Length",
            ));
        }
        bytes.extend_from_slice(&chunk[..count]);
    }
    Ok(Request {
        method,
        target,
        headers,
        body: bytes[body_start..body_start + content_length].to_vec(),
    })
}

fn dispatch(request: Request, state: &UiState) -> Response {
    if request.target.starts_with("/api/v1/") {
        return match dispatch_api(&request, state) {
            Ok(value) => json_response(200, value),
            Err(error) => error_response(error),
        };
    }
    match (
        request.method.as_str(),
        request.target.split('?').next().unwrap_or(""),
    ) {
        ("GET", "/") | ("GET", "/index.html") => {
            asset_response("text/html; charset=utf-8", INDEX_HTML)
        }
        ("GET", "/assets/app.js") => asset_response("text/javascript; charset=utf-8", APP_JS),
        ("GET", "/assets/app.css") => asset_response("text/css; charset=utf-8", APP_CSS),
        _ => error_response(ApiError::new(
            404,
            "route_not_found",
            "The requested route does not exist",
        )),
    }
}

fn dispatch_api(request: &Request, state: &UiState) -> Result<Value, ApiError> {
    if request.body.len() > MAX_BODY_BYTES {
        return Err(ApiError::new(
            413,
            "payload_too_large",
            "JSON request body exceeds the 65536-byte limit",
        ));
    }
    require_same_origin(request, state)?;
    require_json_content_type(request)?;
    let (path, query) = request
        .target
        .split_once('?')
        .map_or((request.target.as_str(), ""), |(path, query)| (path, query));

    if request.method == "POST" && path == "/api/v1/bootstrap" {
        let input: BootstrapInput = parse_json(&request.body)?;
        let mut bootstrap = state
            .bootstrap
            .lock()
            .map_err(|_| ApiError::new(500, "internal_error", "Bootstrap state is unavailable"))?;
        let accepted = bootstrap
            .as_deref()
            .map(|expected| constant_time_equal(expected, &input.bootstrap))
            .unwrap_or(false);
        if !accepted {
            return Err(ApiError::new(
                401,
                "invalid_bootstrap",
                "The one-time browser bootstrap is invalid or already used",
            ));
        }
        *bootstrap = None;
        return Ok(json!({"token": state.bearer}));
    }

    require_bearer(request, state)?;
    match (request.method.as_str(), path) {
        ("GET", "/api/v1/models") => serde_json::to_value(state.application.models())
            .map_err(|error| domain_error("models", error.to_string())),
        ("GET", "/api/v1/usage") => serialize_result(state.application.usage(), "usage"),
        ("GET", "/api/v1/tasks") => {
            let status = query_value(query, "status")?;
            let page = query_value(query, "page")?;
            let sort = query_value(query, "sort")?;
            if page.is_some() || sort.is_some() {
                let page = page.as_deref().unwrap_or("1").parse::<i64>().map_err(|_| {
                    ApiError::field(
                        422,
                        "validation_failed",
                        "The operation was rejected",
                        "page",
                        "must be an integer at least 1",
                    )
                })?;
                serialize_result(
                    state.application.list_task_page(
                        status.as_deref(),
                        sort.as_deref().unwrap_or("newest"),
                        page,
                    ),
                    "list_task_page",
                )
            } else {
                serialize_result(
                    state.application.list_tasks(status.as_deref()),
                    "list_tasks",
                )
            }
        }
        ("GET", "/api/v1/stats") => serialize_result(state.application.task_usage_stats(), "stats"),
        ("GET", "/api/v1/diagnostics") => {
            serialize_result(state.application.diagnostics(), "diagnostics")
        }
        ("GET", "/api/v1/project-directory") => Ok(json!({"cwd": state.launch_directory})),
        ("POST", "/api/v1/project-directory") => {
            let input: DirectoryPickerInput = parse_json(&request.body)?;
            if !input.confirmed {
                return Err(ApiError::field(
                    422,
                    "confirmation_required",
                    "Explicit confirmation is required before opening the folder picker",
                    "confirmed",
                    "must be true",
                ));
            }
            pick_project_directory(&state.launch_directory)
                .map(|cwd| json!({"cwd": cwd}))
                .map_err(project_picker_error)
        }
        ("POST", "/api/v1/estimates") => {
            let input: HttpEstimateInput = parse_json(&request.body)?;
            if let Some(schedule) = input.schedule.as_ref() {
                state
                    .application
                    .validate_batch(schedule)
                    .map_err(|error| domain_error("estimate", error))?;
            }
            serialize_result(state.application.estimate_batch(input.estimate), "estimate")
        }
        ("POST", "/api/v1/plan") => {
            let input: PlanBatchInput = parse_json(&request.body)?;
            serialize_result(state.application.plan_tasks(input), "plan")
        }
        ("POST", "/api/v1/batches") => {
            let input: ScheduleBatchInput = parse_json(&request.body)?;
            serialize_result(state.application.schedule_batch(input), "schedule")
        }
        ("GET", "/api/v1/batches") => {
            let status = query_value(query, "status")?;
            let page = query_value(query, "page")?
                .unwrap_or_else(|| "1".to_string())
                .parse::<i64>()
                .map_err(|_| {
                    ApiError::field(
                        422,
                        "validation_failed",
                        "The operation was rejected",
                        "page",
                        "must be an integer at least 1",
                    )
                })?;
            let sort = query_value(query, "sort")?.unwrap_or_else(|| "newest".to_string());
            serialize_result(
                state
                    .application
                    .list_batch_page(status.as_deref(), &sort, page),
                "list_batch_page",
            )
        }
        ("POST", "/api/v1/service/setup") => {
            let input: SetupInput = parse_json(&request.body)?;
            if !input.confirmed {
                return Err(ApiError::field(
                    422,
                    "confirmation_required",
                    "Explicit confirmation is required before service setup",
                    "confirmed",
                    "must be true",
                ));
            }
            state
                .application
                .setup_service()
                .map(|message| json!({"message":message}))
                .map_err(|error| domain_error("setup_service", error))
        }
        _ if path.starts_with("/api/v1/batches/") => dispatch_batch_route(request, path, state),
        _ => dispatch_task_route(request, path, state),
    }
}

fn dispatch_batch_route(request: &Request, path: &str, state: &UiState) -> Result<Value, ApiError> {
    let remainder = path.strip_prefix("/api/v1/batches/").ok_or_else(|| {
        ApiError::new(
            404,
            "route_not_found",
            "The requested API route does not exist",
        )
    })?;
    let (raw_id, action) = if let Some(value) = remainder.strip_suffix("/edit-session/heartbeat") {
        (value, Some("heartbeat"))
    } else if let Some(value) = remainder.strip_suffix("/edit-session/cancel") {
        (value, Some("cancel"))
    } else if let Some(value) = remainder.strip_suffix("/edit-session") {
        (value, Some("begin"))
    } else {
        (remainder, None)
    };
    if raw_id.is_empty() || raw_id.contains('/') {
        return Err(ApiError::new(
            404,
            "route_not_found",
            "The requested API route does not exist",
        ));
    }
    let batch_id = percent_decode(raw_id)?;
    match (request.method.as_str(), action) {
        ("POST", Some("begin")) => {
            let _: Value = parse_json(&request.body)?;
            serialize_result(
                state.application.begin_batch_edit(&batch_id),
                "begin_batch_edit",
            )
        }
        ("POST", Some("heartbeat")) => {
            let input: BatchEditSessionInput = parse_json(&request.body)?;
            state
                .application
                .heartbeat_batch_edit(&batch_id, &input.edit_session_id)
                .map(|expires_at| json!({"expires_at": expires_at}))
                .map_err(|error| domain_error("heartbeat_batch_edit", error))
        }
        ("POST", Some("cancel")) => {
            let input: BatchEditSessionInput = parse_json(&request.body)?;
            state
                .application
                .cancel_batch_edit(&batch_id, &input.edit_session_id)
                .map(|()| json!({"cancelled": true}))
                .map_err(|error| domain_error("cancel_batch_edit", error))
        }
        ("PUT", None) => {
            let input: BatchEditInput = parse_json(&request.body)?;
            serialize_result(
                state.application.update_batch(&batch_id, input),
                "update_batch",
            )
        }
        ("POST", _) | ("PUT", Some(_)) => Err(ApiError::new(
            404,
            "route_not_found",
            "The requested API route does not exist",
        )),
        _ => Err(ApiError::new(
            405,
            "method_not_allowed",
            "This route does not allow that HTTP method",
        )),
    }
}

fn dispatch_task_route(request: &Request, path: &str, state: &UiState) -> Result<Value, ApiError> {
    let remainder = path.strip_prefix("/api/v1/tasks/").ok_or_else(|| {
        ApiError::new(
            404,
            "route_not_found",
            "The requested API route does not exist",
        )
    })?;
    let (raw_id, action) = if let Some(value) = remainder.strip_suffix("/retry-preview") {
        (value, Some("retry-preview"))
    } else if let Some(value) = remainder.strip_suffix("/retry") {
        (value, Some("retry"))
    } else if let Some(value) = remainder.strip_suffix("/cancel") {
        (value, Some("cancel"))
    } else if let Some(value) = remainder.strip_suffix("/stop") {
        (value, Some("stop"))
    } else {
        (remainder, None)
    };
    if raw_id.is_empty() || raw_id.contains('/') {
        return Err(ApiError::new(
            404,
            "route_not_found",
            "The requested API route does not exist",
        ));
    }
    let task_id = percent_decode(raw_id)?;
    if let Some(action) = action {
        if request.method != "POST" {
            return Err(ApiError::new(
                405,
                "method_not_allowed",
                "This route does not allow that HTTP method",
            ));
        }
        return match action {
            "retry-preview" => {
                let input: RetryTaskOptions = parse_json(&request.body)?;
                serialize_result(
                    state.application.retry_preview(&task_id, input),
                    "retry_preview",
                )
            }
            "retry" => {
                let input: ConfirmedRetryTaskInput = parse_json(&request.body)?;
                serialize_result(state.application.retry_task(&task_id, input), "retry")
            }
            "cancel" => {
                let _: Value = parse_json(&request.body)?;
                serialize_result(state.application.cancel_task(&task_id), "cancel")
            }
            "stop" => {
                let _: Value = parse_json(&request.body)?;
                serialize_result(state.application.stop_task(&task_id), "stop")
            }
            _ => unreachable!("known task action"),
        };
    }
    match request.method.as_str() {
        "GET" => {
            let status = state
                .application
                .task_status(&task_id)
                .map_err(|error| domain_error("task_status", error))?;
            let mut value = serde_json::to_value(status).map_err(serialization_error)?;
            remove_private_run_fields(&mut value);
            Ok(value)
        }
        "PATCH" => {
            let update: TaskUpdate = parse_json(&request.body)?;
            serialize_result(state.application.update_task(&task_id, update), "update")
        }
        "DELETE" => serialize_result(state.application.archive_task(&task_id), "archive"),
        _ => Err(ApiError::new(
            405,
            "method_not_allowed",
            "This route does not allow that HTTP method",
        )),
    }
}

fn require_same_origin(request: &Request, state: &UiState) -> Result<(), ApiError> {
    if request.headers.get("host").map(String::as_str) != Some(state.authority.as_str()) {
        return Err(ApiError::new(
            403,
            "origin_rejected",
            "Host does not match the loopback UI origin",
        ));
    }
    let origin_matches = request
        .headers
        .get("origin")
        .map(|origin| origin == &state.origin)
        .unwrap_or(false);
    let browser_same_origin = !request.headers.contains_key("origin")
        && request
            .headers
            .get("sec-fetch-site")
            .map(|value| value.eq_ignore_ascii_case("same-origin"))
            .unwrap_or(false);
    if !origin_matches && !browser_same_origin {
        return Err(ApiError::new(
            403,
            "origin_rejected",
            "API requests must come from the same loopback UI origin",
        ));
    }
    Ok(())
}

fn require_json_content_type(request: &Request) -> Result<(), ApiError> {
    let json = request
        .headers
        .get("content-type")
        .and_then(|value| value.split(';').next())
        .map(str::trim)
        .map(|value| value.eq_ignore_ascii_case("application/json"))
        .unwrap_or(false);
    if json {
        Ok(())
    } else {
        Err(ApiError::new(
            415,
            "unsupported_media_type",
            "API requests require Content-Type: application/json",
        ))
    }
}

fn require_bearer(request: &Request, state: &UiState) -> Result<(), ApiError> {
    let provided = request
        .headers
        .get("authorization")
        .and_then(|value| value.strip_prefix("Bearer "));
    if provided
        .map(|provided| constant_time_equal(provided, &state.bearer))
        .unwrap_or(false)
    {
        Ok(())
    } else {
        Err(ApiError::new(
            401,
            "unauthorized",
            "A valid per-launch bearer token is required",
        ))
    }
}

fn parse_json<T: for<'de> Deserialize<'de>>(body: &[u8]) -> Result<T, ApiError> {
    serde_json::from_slice(body).map_err(|error| {
        ApiError::field(
            400,
            "invalid_json",
            "Request body is not valid JSON for this endpoint",
            "body",
            &error.to_string(),
        )
    })
}

fn serialize_result<T: serde::Serialize>(
    result: Result<T, String>,
    operation: &'static str,
) -> Result<Value, ApiError> {
    let value = result.map_err(|error| domain_error(operation, error))?;
    serde_json::to_value(value).map_err(serialization_error)
}

fn serialization_error(_error: serde_json::Error) -> ApiError {
    ApiError::new(
        500,
        "serialization_failed",
        "Could not serialize the API response",
    )
}

fn domain_error(operation: &'static str, error: String) -> ApiError {
    if error == "task not found" {
        return ApiError::new(404, "task_not_found", "Task was not found");
    }
    if matches!(
        operation,
        "schedule"
            | "estimate"
            | "update"
            | "cancel"
            | "stop"
            | "archive"
            | "plan"
            | "retry_preview"
            | "retry"
            | "list_task_page"
    ) {
        return ApiError::field(
            422,
            "validation_failed",
            "The operation was rejected",
            "request",
            &error,
        );
    }
    ApiError::field(
        503,
        "operation_unavailable",
        "The requested local operation is unavailable",
        "operation",
        operation,
    )
}

fn project_picker_error(error: String) -> ApiError {
    let cancelled = error == "folder selection was cancelled";
    ApiError::field(
        if cancelled { 422 } else { 503 },
        if cancelled {
            "selection_cancelled"
        } else {
            "operation_unavailable"
        },
        if cancelled {
            "Folder selection was cancelled"
        } else {
            "The native folder picker is unavailable"
        },
        "picker",
        &error,
    )
}

fn remove_private_run_fields(value: &mut Value) {
    if let Some(runs) = value.get_mut("runs").and_then(Value::as_array_mut) {
        for run in runs {
            if let Some(run) = run.as_object_mut() {
                run.remove("usage_before_json");
                run.remove("usage_after_json");
                run.remove("session_id");
            }
        }
    }
}

fn query_value(query: &str, name: &str) -> Result<Option<String>, ApiError> {
    for part in query.split('&').filter(|part| !part.is_empty()) {
        let (key, value) = part.split_once('=').unwrap_or((part, ""));
        if percent_decode(key)? == name {
            let decoded = percent_decode(value)?;
            return Ok((!decoded.trim().is_empty()).then_some(decoded));
        }
    }
    Ok(None)
}

fn percent_decode(value: &str) -> Result<String, ApiError> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'%' if index + 2 < bytes.len() => {
                let high = hex_value(bytes[index + 1]);
                let low = hex_value(bytes[index + 2]);
                let (Some(high), Some(low)) = (high, low) else {
                    return Err(ApiError::new(
                        400,
                        "invalid_url_encoding",
                        "URL contains invalid percent encoding",
                    ));
                };
                decoded.push((high << 4) | low);
                index += 3;
            }
            b'%' => {
                return Err(ApiError::new(
                    400,
                    "invalid_url_encoding",
                    "URL contains invalid percent encoding",
                ))
            }
            byte => {
                decoded.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8(decoded).map_err(|_| {
        ApiError::new(
            400,
            "invalid_url_encoding",
            "URL path must decode to valid UTF-8",
        )
    })
}

fn hex_value(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn random_secret() -> Result<String, String> {
    let mut bytes = [0_u8; 32];
    File::open("/dev/urandom")
        .and_then(|mut file| file.read_exact(&mut bytes))
        .map_err(|error| format!("cannot obtain operating-system randomness: {error}"))?;
    let mut encoded = String::with_capacity(64);
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for byte in bytes {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    Ok(encoded)
}

fn constant_time_equal(left: &str, right: &str) -> bool {
    let left = left.as_bytes();
    let right = right.as_bytes();
    let mut different = left.len() ^ right.len();
    let length = left.len().max(right.len());
    for index in 0..length {
        different |= usize::from(
            left.get(index).copied().unwrap_or(0) ^ right.get(index).copied().unwrap_or(0),
        );
    }
    different == 0
}

fn open_browser(url: &str) -> Result<(), String> {
    let mut command = if cfg!(target_os = "macos") {
        Command::new("open")
    } else if cfg!(target_os = "linux") {
        Command::new("xdg-open")
    } else {
        return Err("automatic browser opening is supported only on Linux and macOS".to_string());
    };
    command
        .arg(url)
        .spawn()
        .map(|_| ())
        .map_err(|error| error.to_string())
}

fn pick_project_directory(start: &str) -> Result<String, String> {
    if let Some(path) = std::env::var_os("LIMITWISE_DIRECTORY_PICKER_PATH") {
        return selected_directory(Command::new(path).output());
    }
    if cfg!(target_os = "macos") {
        return selected_directory(
            Command::new("osascript")
                .args([
                    "-e",
                    "POSIX path of (choose folder with prompt \"Select project directory\")",
                ])
                .output(),
        );
    }
    if cfg!(target_os = "linux") {
        let zenity = Command::new("zenity")
            .args([
                "--file-selection",
                "--directory",
                "--title=Select project directory",
                &format!("--filename={start}/"),
            ])
            .output();
        if !matches!(&zenity, Err(error) if error.kind() == ErrorKind::NotFound) {
            return selected_directory(zenity);
        }
        return selected_directory(
            Command::new("kdialog")
                .args([
                    "--getexistingdirectory",
                    start,
                    "--title",
                    "Select project directory",
                ])
                .output(),
        );
    }
    Err("folder selection is supported only on Linux and macOS".to_string())
}

fn selected_directory(output: std::io::Result<std::process::Output>) -> Result<String, String> {
    let output = output.map_err(|error| {
        if error.kind() == ErrorKind::NotFound {
            "no supported folder picker is installed (install zenity or kdialog)".to_string()
        } else {
            format!("cannot open the folder picker: {error}")
        }
    })?;
    if !output.status.success() {
        return Err(if output.status.code() == Some(1) {
            "folder selection was cancelled".to_string()
        } else {
            "folder picker failed".to_string()
        });
    }
    let selected = String::from_utf8(output.stdout)
        .map_err(|_| "folder picker returned an invalid path".to_string())?;
    canonical_directory(Path::new(selected.trim()))
}

fn canonical_directory(path: &Path) -> Result<String, String> {
    if !path.is_absolute() {
        return Err("project directory must be absolute".to_string());
    }
    let canonical: PathBuf = std::fs::canonicalize(path)
        .map_err(|error| format!("cannot access project directory: {error}"))?;
    if !canonical.is_dir() {
        return Err("project directory must be an existing directory".to_string());
    }
    Ok(canonical.to_string_lossy().into_owned())
}

extern "C" fn request_stop(_signal: i32) {
    STOP_REQUESTED.store(true, Ordering::SeqCst);
}

fn install_signal_handlers() {
    unsafe {
        libc::signal(
            libc::SIGINT,
            request_stop as *const () as libc::sighandler_t,
        );
        libc::signal(
            libc::SIGTERM,
            request_stop as *const () as libc::sighandler_t,
        );
    }
}

fn find_header_end(bytes: &[u8]) -> Option<usize> {
    bytes.windows(4).position(|window| window == b"\r\n\r\n")
}

fn asset_response(content_type: &'static str, body: &'static [u8]) -> Response {
    Response {
        status: 200,
        content_type,
        body: body.to_vec(),
        cache_control: "no-cache",
    }
}

fn json_response(status: u16, value: Value) -> Response {
    Response {
        status,
        content_type: "application/json; charset=utf-8",
        body: serde_json::to_vec(&value).unwrap_or_else(|_| {
            b"{\"error\":{\"code\":\"serialization_failed\",\"message\":\"Could not serialize the API response\",\"fields\":{}}}".to_vec()
        }),
        cache_control: "no-store",
    }
}

fn error_response(error: ApiError) -> Response {
    json_response(
        error.status,
        json!({"error":{"code":error.code,"message":error.message,"fields":error.fields}}),
    )
}

fn write_response(stream: &mut TcpStream, response: Response) -> Result<(), String> {
    let reason = match response.status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        413 => "Payload Too Large",
        415 => "Unsupported Media Type",
        431 => "Request Header Fields Too Large",
        422 => "Unprocessable Content",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        _ => "Error",
    };
    let headers = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nCache-Control: {}\r\nConnection: close\r\nContent-Security-Policy: default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; connect-src 'self'; img-src 'self' data:; frame-ancestors 'none'; base-uri 'none'; form-action 'self'\r\nReferrer-Policy: no-referrer\r\nX-Content-Type-Options: nosniff\r\nX-Frame-Options: DENY\r\n\r\n",
        response.status,
        reason,
        response.content_type,
        response.body.len(),
        response.cache_control,
    );
    stream
        .write_all(headers.as_bytes())
        .and_then(|()| stream.write_all(&response.body))
        .and_then(|()| stream.flush())
        .map_err(|error| error.to_string())
}

impl ApiError {
    fn new(status: u16, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
            fields: Map::new(),
        }
    }

    fn field(
        status: u16,
        code: &'static str,
        message: impl Into<String>,
        field: &str,
        detail: &str,
    ) -> Self {
        let mut fields = Map::new();
        fields.insert(field.to_string(), Value::String(detail.to_string()));
        Self {
            status,
            code,
            message: message.into(),
            fields,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    static ENVIRONMENT_LOCK: Mutex<()> = Mutex::new(());

    struct EnvironmentGuard {
        root: std::path::PathBuf,
        limitwise_home: Option<std::ffi::OsString>,
        xdg_data_home: Option<std::ffi::OsString>,
        codex_path: Option<std::ffi::OsString>,
    }

    impl EnvironmentGuard {
        fn isolated() -> Self {
            let root = std::env::temp_dir().join(format!(
                "limitwise-http-c04-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir_all(&root).unwrap();
            let limitwise_home = std::env::var_os("LIMITWISE_HOME");
            let xdg_data_home = std::env::var_os("XDG_DATA_HOME");
            let codex_path = std::env::var_os("LIMITWISE_CODEX_PATH");
            let fake_codex = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/fake-codex.sh");
            unsafe {
                std::env::set_var("LIMITWISE_HOME", root.join("home"));
                std::env::set_var("XDG_DATA_HOME", root.join("data"));
                std::env::set_var("LIMITWISE_CODEX_PATH", fake_codex);
            }
            Self {
                root,
                limitwise_home,
                xdg_data_home,
                codex_path,
            }
        }
    }

    impl Drop for EnvironmentGuard {
        fn drop(&mut self) {
            unsafe {
                match &self.limitwise_home {
                    Some(value) => std::env::set_var("LIMITWISE_HOME", value),
                    None => std::env::remove_var("LIMITWISE_HOME"),
                }
                match &self.xdg_data_home {
                    Some(value) => std::env::set_var("XDG_DATA_HOME", value),
                    None => std::env::remove_var("XDG_DATA_HOME"),
                }
                match &self.codex_path {
                    Some(value) => std::env::set_var("LIMITWISE_CODEX_PATH", value),
                    None => std::env::remove_var("LIMITWISE_CODEX_PATH"),
                }
            }
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    fn state() -> UiState {
        UiState {
            origin: "http://127.0.0.1:43121".to_string(),
            authority: "127.0.0.1:43121".to_string(),
            bearer: "bearer-secret".to_string(),
            bootstrap: Mutex::new(Some("bootstrap-secret".to_string())),
            application: ApplicationService,
            launch_directory: "/tmp".to_string(),
        }
    }

    fn api_request(method: &str, target: &str, body: &[u8]) -> Request {
        Request {
            method: method.to_string(),
            target: target.to_string(),
            headers: BTreeMap::from([
                ("host".to_string(), "127.0.0.1:43121".to_string()),
                ("origin".to_string(), "http://127.0.0.1:43121".to_string()),
                ("content-type".to_string(), "application/json".to_string()),
                (
                    "authorization".to_string(),
                    "Bearer bearer-secret".to_string(),
                ),
            ]),
            body: body.to_vec(),
        }
    }

    #[test]
    fn parses_ui_flags_and_rejects_invalid_ports() {
        assert_eq!(
            parse_options(vec!["--no-open".into(), "--port".into(), "43121".into()]).unwrap(),
            UiOptions {
                no_open: true,
                port: Some(43121)
            }
        );
        assert!(parse_options(vec!["--port".into(), "0".into()]).is_err());
        assert!(parse_options(vec!["--port".into(), "not-a-port".into()]).is_err());
        assert!(parse_options(vec!["--bind".into(), "0.0.0.0".into()]).is_err());
    }

    #[test]
    fn secrets_use_256_bits_of_os_randomness() {
        let secret = random_secret().unwrap();
        assert_eq!(secret.len(), 64);
        assert!(secret.bytes().all(|byte| byte.is_ascii_hexdigit()));
    }

    #[test]
    fn authenticated_models_route_exposes_per_model_efforts() {
        let catalog = dispatch_api(&api_request("GET", "/api/v1/models", b""), &state()).unwrap();
        assert_eq!(catalog.get("source"), Some(&json!("bundled")));
        let astra = catalog
            .get("models")
            .and_then(Value::as_array)
            .unwrap()
            .iter()
            .find(|model| model.get("model") == Some(&json!("gpt-6-astra")))
            .unwrap();
        assert!(astra
            .get("supportedReasoningEfforts")
            .and_then(Value::as_array)
            .unwrap()
            .iter()
            .any(|effort| effort.get("reasoningEffort") == Some(&json!("ultra"))));
    }

    #[test]
    fn api_rejects_missing_or_wrong_bearer_hostile_origin_and_content_type() {
        let state = state();
        let mut missing = api_request("GET", "/api/v1/unknown", b"");
        missing.headers.remove("authorization");
        assert_eq!(
            dispatch_api(&missing, &state).unwrap_err().code,
            "unauthorized"
        );

        let mut wrong = api_request("GET", "/api/v1/unknown", b"");
        wrong
            .headers
            .insert("authorization".into(), "Bearer wrong".into());
        assert_eq!(
            dispatch_api(&wrong, &state).unwrap_err().code,
            "unauthorized"
        );

        let mut hostile = api_request("GET", "/api/v1/unknown", b"");
        hostile
            .headers
            .insert("origin".into(), "https://attacker.invalid".into());
        assert_eq!(
            dispatch_api(&hostile, &state).unwrap_err().code,
            "origin_rejected"
        );

        let mut content = api_request("GET", "/api/v1/unknown", b"");
        content.headers.remove("content-type");
        assert_eq!(
            dispatch_api(&content, &state).unwrap_err().code,
            "unsupported_media_type"
        );
    }

    #[test]
    fn bootstrap_is_same_origin_and_single_use() {
        let state = state();
        let request = api_request(
            "POST",
            "/api/v1/bootstrap",
            br#"{"bootstrap":"bootstrap-secret"}"#,
        );
        assert_eq!(
            dispatch_api(&request, &state).unwrap().get("token"),
            Some(&json!("bearer-secret"))
        );
        assert_eq!(
            dispatch_api(&request, &state).unwrap_err().code,
            "invalid_bootstrap"
        );
    }

    #[test]
    fn api_has_stable_errors_for_malformed_oversized_and_unknown_requests() {
        let state = state();
        let malformed = api_request("POST", "/api/v1/estimates", b"{");
        assert_eq!(
            dispatch_api(&malformed, &state).unwrap_err().code,
            "invalid_json"
        );
        let oversized = api_request("POST", "/api/v1/estimates", &vec![b'x'; MAX_BODY_BYTES + 1]);
        assert_eq!(
            dispatch_api(&oversized, &state).unwrap_err().code,
            "payload_too_large"
        );
        let unknown = api_request("GET", "/api/v1/unknown", b"");
        assert_eq!(
            dispatch_api(&unknown, &state).unwrap_err().code,
            "route_not_found"
        );
    }

    #[test]
    fn project_directory_defaults_to_launch_directory_and_picker_needs_confirmation() {
        let state = state();
        let default = dispatch_api(
            &api_request("GET", "/api/v1/project-directory", b""),
            &state,
        )
        .unwrap();
        assert_eq!(default.get("cwd"), Some(&json!("/tmp")));
        let rejected = dispatch_api(
            &api_request(
                "POST",
                "/api/v1/project-directory",
                br#"{"confirmed":false}"#,
            ),
            &state,
        )
        .unwrap_err();
        assert_eq!(rejected.code, "confirmation_required");
        let canonical_tmp = std::fs::canonicalize("/tmp")
            .unwrap()
            .to_string_lossy()
            .into_owned();
        assert_eq!(
            canonical_directory(Path::new("/tmp")).unwrap(),
            canonical_tmp
        );
    }

    #[test]
    fn direct_http_types_reject_permission_bypass_fields() {
        let run_at = (chrono::Utc::now() + chrono::Duration::hours(2)).to_rfc3339();
        let base = json!({
            "idempotency_key": "http-profile-bypass",
            "budget_mode": "percentage",
            "weekly_cap_percent": 1,
            "tasks": [{
                "title": "safe",
                "prompt": "work",
                "cwd": "/tmp",
                "run_at": run_at,
                "timezone": "UTC",
                "difficulty": "simple",
                "permission_profile": "danger-full-access"
            }]
        });
        assert_eq!(
            parse_json::<ScheduleBatchInput>(&serde_json::to_vec(&base).unwrap())
                .unwrap_err()
                .code,
            "invalid_json"
        );
        assert!(parse_json::<TaskUpdate>(
            br#"{"permission_profile":"networked","sandbox":"danger-full-access"}"#
        )
        .is_err());
        assert!(parse_json::<RetryTaskOptions>(
            br#"{"budget_mode":"tokens","token_cap":1,"approval_policy":"on-request"}"#
        )
        .is_err());
        assert!(parse_json::<ConfirmedRetryTaskInput>(
            br#"{"idempotency_key":"retry-bypass","confirmed":true,"budget_mode":"tokens","token_cap":1,"sandbox":"danger-full-access"}"#
        )
        .is_err());
        let retry = parse_json::<ConfirmedRetryTaskInput>(
            br#"{"idempotency_key":"retry-networked","confirmed":true,"budget_mode":"tokens","token_cap":1,"permission_profile":"networked","networked_confirmed":true}"#,
        )
        .unwrap();
        assert_eq!(
            retry.options.permission_profile,
            Some(crate::model::PermissionProfile::Networked)
        );
        assert!(retry.options.networked_confirmed);
    }

    #[test]
    fn task_detail_removes_private_usage_and_session_fields() {
        let mut value = json!({"runs":[{
            "usage_before_json":"secret",
            "usage_after_json":"secret",
            "session_id":"session",
            "transcript_path":"/private/transcript.jsonl",
            "error":"failed"
        }]});
        remove_private_run_fields(&mut value);
        assert!(value.pointer("/runs/0/usage_before_json").is_none());
        assert!(value.pointer("/runs/0/usage_after_json").is_none());
        assert!(value.pointer("/runs/0/session_id").is_none());
        assert_eq!(
            value.pointer("/runs/0/transcript_path"),
            Some(&json!("/private/transcript.jsonl"))
        );
    }

    #[test]
    fn paginated_task_list_and_running_stop_route_use_store_state() {
        let _environment_lock = ENVIRONMENT_LOCK.lock().unwrap();
        let _environment = EnvironmentGuard::isolated();
        let run_at = (chrono::Utc::now() + chrono::Duration::hours(2)).to_rfc3339();
        let mut store = crate::store::Store::open().unwrap();
        let task = store
            .schedule_batch(crate::store::ScheduleBatchInput {
                idempotency_key: "http-page-stop".into(),
                budget_mode: Some(crate::store::BudgetMode::Percentage),
                weekly_cap_percent: Some(1.0),
                token_cap: None,
                cap_percent: None,
                five_hour_cap_percent: None,
                networked_confirmed: false,
                tasks: vec![crate::store::TaskDraft {
                    title: "Newest task".into(),
                    prompt: "wait".into(),
                    success_criteria: "stop requested".into(),
                    cwd: "/tmp".into(),
                    run_at: Some(run_at),
                    after_previous: false,
                    continue_from_task_id: None,
                    timezone: Some("UTC".into()),
                    difficulty: crate::model::Difficulty::Simple,
                    model: None,
                    effort: None,
                    permission_profile: crate::model::PermissionProfile::Restricted,
                }],
            })
            .unwrap()
            .tasks
            .remove(0);
        assert!(store.claim_task(&task.id).unwrap());
        drop(store);

        let page = dispatch_api(
            &api_request("GET", "/api/v1/tasks?page=1&sort=newest", b""),
            &state(),
        )
        .unwrap();
        assert_eq!(page.get("page"), Some(&json!(1)));
        assert_eq!(page.get("page_size"), Some(&json!(20)));
        assert_eq!(page.pointer("/items/0/id"), Some(&json!(task.id)));

        let stopped = dispatch_api(
            &api_request("POST", &format!("/api/v1/tasks/{}/stop", task.id), b"{}"),
            &state(),
        )
        .unwrap();
        assert_eq!(stopped.get("status"), Some(&json!("running")));
        assert!(crate::store::Store::open()
            .unwrap()
            .task_stop_requested(&task.id)
            .unwrap());
    }

    #[test]
    fn grouped_batch_routes_hold_and_atomically_update_pending_work() {
        let _environment_lock = ENVIRONMENT_LOCK.lock().unwrap();
        let _environment = EnvironmentGuard::isolated();
        let run_at = (chrono::Utc::now() + chrono::Duration::hours(2)).to_rfc3339();
        let mut store = crate::store::Store::open().unwrap();
        let created = store
            .schedule_batch(crate::store::ScheduleBatchInput {
                idempotency_key: "http-batch-edit".into(),
                budget_mode: Some(crate::store::BudgetMode::Percentage),
                weekly_cap_percent: Some(1.0),
                token_cap: None,
                cap_percent: None,
                five_hour_cap_percent: None,
                networked_confirmed: false,
                tasks: vec![crate::store::TaskDraft {
                    title: "Editable task".into(),
                    prompt: "wait".into(),
                    success_criteria: "updated".into(),
                    cwd: "/tmp".into(),
                    run_at: Some(run_at),
                    after_previous: false,
                    continue_from_task_id: None,
                    timezone: Some("UTC".into()),
                    difficulty: crate::model::Difficulty::Standard,
                    model: None,
                    effort: None,
                    permission_profile: crate::model::PermissionProfile::Restricted,
                }],
            })
            .unwrap();
        drop(store);

        let page = dispatch_api(
            &api_request("GET", "/api/v1/batches?page=1&sort=newest", b""),
            &state(),
        )
        .unwrap();
        assert_eq!(page.get("total"), Some(&json!(1)));
        assert_eq!(
            page.pointer("/items/0/tasks/0/id"),
            Some(&json!(created.tasks[0].id))
        );

        let edit_path = format!("/api/v1/batches/{}/edit-session", created.batch.id);
        let session = dispatch_api(&api_request("POST", &edit_path, b"{}"), &state()).unwrap();
        let edit_session_id = session
            .get("edit_session_id")
            .and_then(Value::as_str)
            .unwrap();
        assert!(!crate::store::Store::open()
            .unwrap()
            .claim_task(&created.tasks[0].id)
            .unwrap());

        let update = json!({
            "edit_session_id": edit_session_id,
            "tasks": [{
                "task_id": created.tasks[0].id,
                "title": "Edited over HTTP",
                "prompt": created.tasks[0].prompt,
                "success_criteria": created.tasks[0].success_criteria,
                "cwd": "/tmp",
                "run_at": (chrono::Utc::now() + chrono::Duration::hours(3)).to_rfc3339(),
                "timezone": "UTC",
                "difficulty": "standard",
                "model": created.tasks[0].model,
                "effort": created.tasks[0].effort,
                "permission_profile": "restricted"
            }]
        });
        let updated = dispatch_api(
            &api_request(
                "PUT",
                &format!("/api/v1/batches/{}", created.batch.id),
                &serde_json::to_vec(&update).unwrap(),
            ),
            &state(),
        )
        .unwrap();
        assert_eq!(
            updated.pointer("/tasks/0/title"),
            Some(&json!("Edited over HTTP"))
        );
        assert_eq!(updated.get("editing"), Some(&json!(false)));
    }

    #[test]
    fn simple_plan_and_archive_routes_keep_task_history() {
        let _environment_lock = ENVIRONMENT_LOCK.lock().unwrap();
        let _environment = EnvironmentGuard::isolated();
        let planned = dispatch_api(
            &api_request(
                "POST",
                "/api/v1/plan",
                br#"{"tasks":[{"title":"Migrate settings","prompt":"Refactor the architecture"}],"cwd":"/tmp","weekly_cap_percent":3,"planner_model":"gpt-5.6-sol"}"#,
            ),
            &state(),
        )
        .unwrap();
        assert_eq!(planned.get("planner_model"), Some(&json!("gpt-5.6-sol")));
        assert_eq!(planned.get("planner_effort"), Some(&json!("high")));
        assert_eq!(planned.get("weekly_cap_percent"), Some(&json!(3.0)));
        let canonical_tmp = std::fs::canonicalize("/tmp")
            .unwrap()
            .to_string_lossy()
            .into_owned();
        assert_eq!(planned.get("cwd"), Some(&json!(canonical_tmp)));
        assert_eq!(planned.pointer("/tasks/0/model"), Some(&json!("gpt-6-sol")));
        assert_eq!(planned.pointer("/tasks/0/effort"), Some(&json!("medium")));
        assert_eq!(
            planned.get("permission_profile"),
            Some(&json!("restricted"))
        );

        let run_at = (chrono::Utc::now() + chrono::Duration::hours(2)).to_rfc3339();
        let mut store = crate::store::Store::open().unwrap();
        let task = store
            .schedule_batch(crate::store::ScheduleBatchInput {
                idempotency_key: "http-archive".into(),
                budget_mode: Some(crate::store::BudgetMode::Percentage),
                weekly_cap_percent: Some(1.0),
                token_cap: None,
                cap_percent: None,
                five_hour_cap_percent: None,
                networked_confirmed: false,
                tasks: vec![crate::store::TaskDraft {
                    title: "Archive me".into(),
                    prompt: "Keep history".into(),
                    success_criteria: String::new(),
                    cwd: "/tmp".into(),
                    run_at: Some(run_at),
                    after_previous: false,
                    continue_from_task_id: None,
                    timezone: Some("UTC".into()),
                    difficulty: crate::model::Difficulty::Simple,
                    model: None,
                    effort: None,
                    permission_profile: crate::model::PermissionProfile::Restricted,
                }],
            })
            .unwrap()
            .tasks
            .remove(0);
        drop(store);

        let archived = dispatch_api(
            &api_request("DELETE", &format!("/api/v1/tasks/{}", task.id), b""),
            &state(),
        )
        .unwrap();
        assert_eq!(archived.get("status"), Some(&json!("cancelled")));
        let page = dispatch_api(
            &api_request("GET", "/api/v1/tasks?page=1&sort=newest", b""),
            &state(),
        )
        .unwrap();
        assert_eq!(page.get("total"), Some(&json!(0)));
        assert!(crate::store::Store::open()
            .unwrap()
            .task_status(&task.id)
            .unwrap()
            .is_some());
    }

    #[test]
    fn retry_routes_are_post_only_and_share_authenticated_json_boundary() {
        let state = state();
        for path in [
            "/api/v1/tasks/source/retry-preview",
            "/api/v1/tasks/source/retry",
        ] {
            let get = api_request("GET", path, b"");
            assert_eq!(
                dispatch_api(&get, &state).unwrap_err().code,
                "method_not_allowed"
            );
            let malformed = api_request("POST", path, b"{");
            assert_eq!(
                dispatch_api(&malformed, &state).unwrap_err().code,
                "invalid_json"
            );
            let mut unauthenticated = api_request("POST", path, b"{}");
            unauthenticated.headers.remove("authorization");
            assert_eq!(
                dispatch_api(&unauthenticated, &state).unwrap_err().code,
                "unauthorized"
            );
        }
    }

    #[test]
    fn retry_http_and_mcp_success_payloads_are_equivalent() {
        let _environment_lock = ENVIRONMENT_LOCK.lock().unwrap();
        let _environment = EnvironmentGuard::isolated();
        let run_at = (chrono::Utc::now() + chrono::Duration::hours(2)).to_rfc3339();
        let mut store = crate::store::Store::open().unwrap();
        let source = store
            .schedule_batch(crate::store::ScheduleBatchInput {
                idempotency_key: "http-mcp-retry-source".into(),
                budget_mode: Some(crate::store::BudgetMode::Percentage),
                weekly_cap_percent: Some(1.0),
                token_cap: None,
                cap_percent: None,
                five_hour_cap_percent: None,
                networked_confirmed: false,
                tasks: vec![crate::store::TaskDraft {
                    title: "transport equivalence".into(),
                    prompt: "retry through either transport".into(),
                    success_criteria: "same preview".into(),
                    cwd: "/tmp".into(),
                    run_at: Some(run_at.clone()),
                    after_previous: false,
                    continue_from_task_id: None,
                    timezone: Some("UTC".into()),
                    difficulty: crate::model::Difficulty::Simple,
                    model: None,
                    effort: None,
                    permission_profile: crate::model::PermissionProfile::Restricted,
                }],
            })
            .unwrap()
            .tasks
            .remove(0);
        store
            .set_status(&source.id, "failed", Some("fixture"))
            .unwrap();
        drop(store);

        let options = json!({
            "budget_mode":"percentage",
            "weekly_cap_percent":2,
            "run_at":run_at,
            "timezone":"UTC",
            "model":"gpt-5.6-luna",
            "effort":"low"
        });
        let body = serde_json::to_vec(&options).unwrap();
        let mut http = dispatch_api(
            &api_request(
                "POST",
                &format!("/api/v1/tasks/{}/retry-preview", source.id),
                &body,
            ),
            &state(),
        )
        .unwrap();
        let mut arguments = options;
        arguments
            .as_object_mut()
            .unwrap()
            .insert("task_id".into(), json!(source.id));
        let mcp = crate::mcp::call_tool("preview_retry_task", arguments).unwrap();
        let mut mcp = mcp.get("structuredContent").unwrap().clone();
        http.pointer_mut("/estimate")
            .unwrap()
            .as_object_mut()
            .unwrap()
            .remove("generated_at");
        mcp.pointer_mut("/estimate")
            .unwrap()
            .as_object_mut()
            .unwrap()
            .remove("generated_at");
        assert_eq!(http, mcp);
    }

    #[test]
    fn embedded_asset_routes_are_fixed_and_contain_no_launch_secret() {
        let state = state();
        let index = dispatch(
            Request {
                method: "GET".to_string(),
                target: "/".to_string(),
                headers: BTreeMap::new(),
                body: Vec::new(),
            },
            &state,
        );
        assert_eq!(index.status, 200);
        assert_eq!(index.content_type, "text/html; charset=utf-8");
        assert!(index.body.windows(4).any(|window| window == b"root"));
        assert!(!index
            .body
            .windows(state.bearer.len())
            .any(|window| window == state.bearer.as_bytes()));

        let arbitrary = dispatch(
            Request {
                method: "GET".to_string(),
                target: "/../../etc/passwd".to_string(),
                headers: BTreeMap::new(),
                body: Vec::new(),
            },
            &state,
        );
        assert_eq!(arbitrary.status, 404);
    }
}
