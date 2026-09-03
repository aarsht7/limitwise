mod config;
mod mcp;
mod model;
mod prediction;
mod scheduler;
mod service;
mod store;
mod transcript;
mod usage;

use std::env;

fn main() {
    if let Err(error) = run() {
        eprintln!("limitwise: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let mut args = env::args().skip(1);
    match args.next().as_deref() {
        Some("mcp") => mcp::serve(),
        Some("daemon") => scheduler::daemon(args.any(|arg| arg == "--once")),
        Some("setup") => service::setup().map(|message| println!("{message}")),
        Some("uninstall") => {
            let purge = args.any(|arg| arg == "--purge");
            service::uninstall(purge).map(|message| println!("{message}"))
        }
        Some("usage") => {
            let snapshot = usage::UsageClient::default().fetch()?;
            println!(
                "{}",
                serde_json::to_string_pretty(&snapshot).map_err(|e| e.to_string())?
            );
            Ok(())
        }
        Some("stats") => {
            let stats = store::Store::open()?
                .task_usage_stats(store::now_epoch(), &config::system_timezone())?;
            println!(
                "{}",
                serde_json::to_string_pretty(&stats).map_err(|e| e.to_string())?
            );
            Ok(())
        }
        Some("help") | Some("--help") | Some("-h") | None => {
            println!(
                "LimitWise {}\n\nCOMPATIBILITY:\n  Tested only on Linux x86-64. macOS, including Apple Silicon, and other architectures are untested.\n\nUSAGE:\n  limitwise mcp\n  limitwise daemon [--once]\n  limitwise setup\n  limitwise uninstall [--purge]\n  limitwise usage\n  limitwise stats",
                env!("CARGO_PKG_VERSION")
            );
            Ok(())
        }
        Some(other) => Err(format!("unknown command '{other}'")),
    }
}
