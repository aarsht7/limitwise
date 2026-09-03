use std::env;
use std::fs;
use std::path::{Path, PathBuf};

pub const FIVE_HOUR_RESERVE_PERCENT: f64 = 10.0;
pub const MISSED_GRACE_SECONDS: i64 = 300;
pub const DEFAULT_POLL_SECONDS: u64 = 15;

#[derive(Clone, Debug)]
pub struct Paths {
    pub data_dir: PathBuf,
    pub database: PathBuf,
    pub logs_dir: PathBuf,
    pub installed_binary: PathBuf,
}

impl Paths {
    pub fn discover() -> Result<Self, String> {
        let home = home_dir()?;
        let data_dir = if cfg!(target_os = "macos") {
            home.join("Library")
                .join("Application Support")
                .join("LimitWise")
        } else if let Ok(value) = env::var("XDG_DATA_HOME") {
            PathBuf::from(value).join("limitwise")
        } else {
            home.join(".local").join("share").join("limitwise")
        };
        Ok(Self {
            database: data_dir.join("limitwise.sqlite3"),
            logs_dir: data_dir.join("logs"),
            installed_binary: data_dir.join("bin").join("limitwise"),
            data_dir,
        })
    }

    pub fn ensure(&self) -> Result<(), String> {
        fs::create_dir_all(&self.logs_dir).map_err(|e| e.to_string())?;
        fs::create_dir_all(
            self.installed_binary
                .parent()
                .ok_or_else(|| "invalid installed binary path".to_string())?,
        )
        .map_err(|e| e.to_string())?;
        set_private_dir(&self.data_dir)?;
        set_private_dir(&self.logs_dir)?;
        Ok(())
    }
}

pub fn home_dir() -> Result<PathBuf, String> {
    env::var_os("LIMITWISE_HOME")
        .or_else(|| env::var_os("HOME"))
        .or_else(|| env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .ok_or_else(|| "LIMITWISE_HOME, HOME, or USERPROFILE is required".to_string())
}

pub fn poll_seconds() -> u64 {
    env::var("LIMITWISE_POLL_SECONDS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(DEFAULT_POLL_SECONDS)
}

pub fn codex_binary() -> PathBuf {
    env::var_os("LIMITWISE_CODEX_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("codex"))
}

pub fn system_timezone() -> String {
    if let Ok(value) = env::var("TZ") {
        if !value.trim().is_empty() {
            return value;
        }
    }
    if let Ok(target) = fs::read_link("/etc/localtime") {
        let rendered = target.to_string_lossy();
        if let Some((_, zone)) = rendered.split_once("zoneinfo/") {
            return zone.to_string();
        }
    }
    "UTC".to_string()
}

#[cfg(unix)]
pub fn set_private_file(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).map_err(|e| e.to_string())
}

#[cfg(not(unix))]
pub fn set_private_file(_path: &Path) -> Result<(), String> {
    Ok(())
}

#[cfg(unix)]
pub fn set_private_dir(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())
}

#[cfg(not(unix))]
pub fn set_private_dir(_path: &Path) -> Result<(), String> {
    Ok(())
}
