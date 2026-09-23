use std::net::SocketAddr;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct Config {
    pub bind: SocketAddr,
    pub data_dir: PathBuf,
    pub stockfish: Option<PathBuf>,
    pub engine_workers: Option<usize>,
    pub engine_threads: Option<u32>,
    pub engine_hash_mb: Option<u32>,
    /// "local" (single user, no login) or "hosted" (user accounts).
    pub mode: String,
    pub public_url: Option<String>,
    pub dev: bool,
    /// Coach model offered to browsers with WebGPU; None turns that off.
    pub device_model: Option<api_types::DeviceModel>,
}

/// Default in-browser coach model (WebLLM's prebuilt list, Apache-2.0).
pub const DEFAULT_DEVICE_MODEL: &str = "Qwen3.5-4B-q4f16_1-MLC";

/// `CHESSGPT_DEVICE_MODEL` (an MLC model id, or "off"), with
/// `CHESSGPT_DEVICE_MODEL_URL` / `_LIB` / `_MB` for a custom build.
fn device_model() -> Option<api_types::DeviceModel> {
    let id = env("CHESSGPT_DEVICE_MODEL").unwrap_or_else(|| DEFAULT_DEVICE_MODEL.into());
    if id.eq_ignore_ascii_case("off") {
        return None;
    }
    Some(api_types::DeviceModel {
        size_mb: env("CHESSGPT_DEVICE_MODEL_MB").and_then(|v| v.parse().ok()).unwrap_or(2600),
        url: env("CHESSGPT_DEVICE_MODEL_URL"),
        lib_url: env("CHESSGPT_DEVICE_MODEL_LIB"),
        id,
    })
}

fn env(k: &str) -> Option<String> {
    std::env::var(k).ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

pub fn default_data_dir() -> PathBuf {
    if cfg!(windows) {
        if let Some(d) = env("LOCALAPPDATA") {
            return Path::new(&d).join("chessgpt");
        }
    } else if let Some(d) = env("XDG_DATA_HOME") {
        return Path::new(&d).join("chessgpt");
    } else if let Some(h) = env("HOME") {
        return Path::new(&h).join(".local/share/chessgpt");
    }
    PathBuf::from("data")
}

/// Directories that may contain `engines/` (repo checkout or next to the binary).
pub fn search_roots() -> Vec<PathBuf> {
    let mut roots = vec![];
    if let Ok(cwd) = std::env::current_dir() {
        roots.push(cwd);
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        roots.push(dir.to_path_buf());
        // target/{debug,release}/chessgpt -> repo root
        if let Some(repo) = dir.parent().and_then(|p| p.parent()) {
            roots.push(repo.to_path_buf());
        }
    }
    roots.push(Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."));
    roots
}

impl Config {
    pub fn from_env() -> Config {
        let mode = env("CHESSGPT_MODE").unwrap_or_else(|| "local".into());
        let default_bind = if mode == "hosted" { "0.0.0.0:8080" } else { "127.0.0.1:8080" };
        Config {
            bind: env("CHESSGPT_BIND").unwrap_or_else(|| default_bind.into()).parse().expect("CHESSGPT_BIND must be host:port"),
            data_dir: env("CHESSGPT_DATA_DIR").map(PathBuf::from).unwrap_or_else(default_data_dir),
            stockfish: env("STOCKFISH_PATH").map(PathBuf::from),
            engine_workers: env("ENGINE_WORKERS").and_then(|v| v.parse().ok()),
            engine_threads: env("ENGINE_THREADS").and_then(|v| v.parse().ok()),
            engine_hash_mb: env("ENGINE_HASH_MB").and_then(|v| v.parse().ok()),
            public_url: env("CHESSGPT_PUBLIC_URL"),
            dev: env("CHESSGPT_DEV").is_some_and(|v| v != "0"),
            device_model: device_model(),
            mode,
        }
    }
}
