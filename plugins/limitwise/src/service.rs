use crate::config::{home_dir, set_private_dir, Paths};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

pub fn setup() -> Result<String, String> {
    let paths = Paths::discover()?;
    paths.ensure()?;
    let current = std::env::current_exe().map_err(|e| e.to_string())?;
    install_binary(&current, &paths.installed_binary)?;
    if let Some(parent) = paths.installed_binary.parent() {
        set_private_dir(parent)?;
    }

    if cfg!(target_os = "macos") {
        install_launch_agent(&paths)
    } else if cfg!(target_os = "linux") {
        install_systemd_user_service(&paths)
    } else {
        Err("LimitWise setup supports Linux and macOS only".to_string())
    }
}

fn install_binary(source: &Path, destination: &Path) -> Result<(), String> {
    let parent = destination
        .parent()
        .ok_or_else(|| "invalid installed binary path".to_string())?;
    let temporary = parent.join(format!(".limitwise-install-{}", std::process::id()));
    let result = (|| {
        fs::copy(source, &temporary).map_err(|e| e.to_string())?;
        make_executable(&temporary)?;
        fs::rename(&temporary, destination).map_err(|e| e.to_string())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

pub fn uninstall(purge: bool) -> Result<String, String> {
    let paths = Paths::discover()?;
    if cfg!(target_os = "macos") {
        let plist = launch_agent_path()?;
        let domain = format!("gui/{}/io.openai.limitwise", unsafe { libc::getuid() });
        let _ = Command::new("launchctl")
            .args(["bootout", &domain])
            .status();
        remove_if_exists(&plist)?;
    } else if cfg!(target_os = "linux") {
        let _ = Command::new("systemctl")
            .args(["--user", "disable", "--now", "limitwise.service"])
            .status();
        remove_if_exists(&systemd_unit_path()?)?;
        let _ = Command::new("systemctl")
            .args(["--user", "daemon-reload"])
            .status();
    }
    remove_if_exists(&paths.installed_binary)?;
    if purge && paths.data_dir.exists() {
        fs::remove_dir_all(&paths.data_dir).map_err(|e| e.to_string())?;
        Ok("LimitWise service and all local state were removed".to_string())
    } else {
        Ok("LimitWise service was removed; schedules and transcripts were preserved".to_string())
    }
}

fn install_systemd_user_service(paths: &Paths) -> Result<String, String> {
    let unit_path = systemd_unit_path()?;
    let parent = unit_path
        .parent()
        .ok_or_else(|| "invalid systemd path".to_string())?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let unit = systemd_unit(&paths.installed_binary);
    fs::write(&unit_path, unit).map_err(|e| e.to_string())?;
    run_checked(Command::new("systemctl").args(["--user", "daemon-reload"]))?;
    run_checked(Command::new("systemctl").args(["--user", "enable", "limitwise.service"]))?;
    run_checked(Command::new("systemctl").args(["--user", "restart", "limitwise.service"]))?;
    Ok(format!(
        "LimitWise systemd user service installed at {}",
        unit_path.display()
    ))
}

fn install_launch_agent(paths: &Paths) -> Result<String, String> {
    let plist_path = launch_agent_path()?;
    let parent = plist_path
        .parent()
        .ok_or_else(|| "invalid LaunchAgent path".to_string())?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    fs::write(&plist_path, launch_agent_plist(paths)).map_err(|e| e.to_string())?;
    let uid = unsafe { libc::getuid() };
    let domain = format!("gui/{uid}");
    let service = format!("{domain}/io.openai.limitwise");
    let _ = Command::new("launchctl")
        .args(["bootout", &service])
        .status();
    run_checked(Command::new("launchctl").args([
        "bootstrap",
        &domain,
        &plist_path.to_string_lossy(),
    ]))?;
    Ok(format!(
        "WARNING: LimitWise is untested on macOS, including Apple Silicon. LimitWise LaunchAgent installed at {}",
        plist_path.display()
    ))
}

fn systemd_unit(binary: &Path) -> String {
    format!(
        "[Unit]\nDescription=LimitWise quota-aware Codex scheduler\n\n[Service]\nType=simple\nExecStart={} daemon\nRestart=on-failure\nRestartSec=5\nNoNewPrivileges=true\nPrivateTmp=true\n\n[Install]\nWantedBy=default.target\n",
        systemd_escape(binary)
    )
}

fn launch_agent_plist(paths: &Paths) -> String {
    let stdout = paths.logs_dir.join("daemon.stdout.log");
    let stderr = paths.logs_dir.join("daemon.stderr.log");
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\"><dict>\n  <key>Label</key><string>io.openai.limitwise</string>\n  <key>ProgramArguments</key><array><string>{}</string><string>daemon</string></array>\n  <key>RunAtLoad</key><true/>\n  <key>KeepAlive</key><true/>\n  <key>ProcessType</key><string>Background</string>\n  <key>StandardOutPath</key><string>{}</string>\n  <key>StandardErrorPath</key><string>{}</string>\n</dict></plist>\n",
        xml_escape(&paths.installed_binary.to_string_lossy()),
        xml_escape(&stdout.to_string_lossy()),
        xml_escape(&stderr.to_string_lossy())
    )
}

fn systemd_unit_path() -> Result<PathBuf, String> {
    Ok(home_dir()?.join(".config/systemd/user/limitwise.service"))
}

fn launch_agent_path() -> Result<PathBuf, String> {
    Ok(home_dir()?.join("Library/LaunchAgents/io.openai.limitwise.plist"))
}

fn run_checked(command: &mut Command) -> Result<(), String> {
    let display = format!("{command:?}");
    let status = command
        .status()
        .map_err(|e| format!("cannot run {display}: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{display} exited with {status}"))
    }
}

fn remove_if_exists(path: &Path) -> Result<(), String> {
    if path.exists() {
        fs::remove_file(path).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn systemd_escape(path: &Path) -> String {
    let value = path.to_string_lossy();
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(unix)]
fn make_executable(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn binary_install_replaces_existing_file_atomically() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "limitwise-install-test-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&root).unwrap();
        let source = root.join("source");
        let destination = root.join("limitwise");
        fs::write(&source, b"new").unwrap();
        fs::write(&destination, b"old").unwrap();

        install_binary(&source, &destination).unwrap();
        assert_eq!(fs::read(&destination).unwrap(), b"new");
        assert!(!root
            .join(format!(".limitwise-install-{}", std::process::id()))
            .exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn systemd_unit_has_security_and_restart_settings() {
        let unit = systemd_unit(Path::new("/tmp/Limit Wise/limitwise"));
        assert!(unit.contains("NoNewPrivileges=true"));
        assert!(unit.contains("Restart=on-failure"));
        assert!(unit.contains("\"/tmp/Limit Wise/limitwise\" daemon"));
    }

    #[test]
    fn plist_escapes_paths() {
        let paths = Paths {
            data_dir: PathBuf::from("/tmp/a&b"),
            database: PathBuf::from("/tmp/a&b/db"),
            logs_dir: PathBuf::from("/tmp/a&b/logs"),
            installed_binary: PathBuf::from("/tmp/a&b/bin/limitwise"),
        };
        assert!(launch_agent_plist(&paths).contains("a&amp;b"));
    }
}
