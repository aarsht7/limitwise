mod app;
mod config;
mod diagnostic;
mod http;
mod mcp;
mod model;
mod planner;
mod prediction;
mod scheduler;
mod service;
mod store;
mod transcript;
mod usage;

use std::env;
use std::fmt;
use std::io;

fn main() {
    match run() {
        Ok(0) => {}
        Ok(code) => std::process::exit(code),
        Err(error) => {
            eprintln!("limitwise: {error}");
            std::process::exit(error.exit_code);
        }
    }
}

#[derive(Debug)]
struct CliError {
    message: String,
    exit_code: i32,
}

impl CliError {
    fn doctor(message: String) -> Self {
        Self {
            message,
            exit_code: 2,
        }
    }
}

impl From<String> for CliError {
    fn from(message: String) -> Self {
        Self {
            message,
            exit_code: 1,
        }
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

fn run() -> Result<i32, CliError> {
    let mut args = env::args().skip(1);
    let application = app::ApplicationService;
    match args.next().as_deref() {
        Some("mcp") => mcp::serve().map(|()| 0).map_err(Into::into),
        Some("daemon") => scheduler::daemon(args.any(|arg| arg == "--once"))
            .map(|()| 0)
            .map_err(Into::into),
        Some("setup") => application
            .setup_service()
            .map(|message| {
                println!("{message}");
                0
            })
            .map_err(Into::into),
        Some("uninstall") => {
            let purge = args.any(|arg| arg == "--purge");
            service::uninstall(purge)
                .map(|message| {
                    println!("{message}");
                    0
                })
                .map_err(Into::into)
        }
        Some("usage") => {
            let snapshot = application.usage()?;
            println!(
                "{}",
                serde_json::to_string_pretty(&snapshot).map_err(|e| e.to_string())?
            );
            Ok(0)
        }
        Some("stats") => {
            let stats = application.task_usage_stats()?;
            println!(
                "{}",
                serde_json::to_string_pretty(&stats).map_err(|e| e.to_string())?
            );
            Ok(0)
        }
        Some("doctor") => {
            let arguments = args.collect::<Vec<_>>();
            let mut stdout = io::stdout().lock();
            diagnostic::run_cli(&arguments, &mut stdout).map_err(CliError::doctor)
        }
        Some("ui") => http::run_cli(args.collect())
            .map(|()| 0)
            .map_err(Into::into),
        Some("--version") | Some("-V") => {
            println!("limitwise {}", env!("CARGO_PKG_VERSION"));
            Ok(0)
        }
        Some("help") | Some("--help") | Some("-h") | None => {
            println!(
                "LimitWise {}\n\nCOMPATIBILITY:\n  Tested only on Linux x86-64. macOS, including Apple Silicon, and other architectures are untested.\n\nUSAGE:\n  limitwise mcp\n  limitwise daemon [--once]\n  limitwise setup\n  limitwise uninstall [--purge]\n  limitwise usage\n  limitwise stats\n  limitwise doctor [--json]\n  limitwise ui [--no-open] [--port <port>]",
                env!("CARGO_PKG_VERSION")
            );
            Ok(0)
        }
        Some(other) => Err(format!("unknown command '{other}'").into()),
    }
}
