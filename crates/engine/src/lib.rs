//! Stockfish over UCI: process management, a worker pool with priorities and
//! cancellation, analysis caching, and Elo-tier search budgets.
//!
//! Every score leaving this crate is from **White's** point of view.

pub mod limits;
pub mod pool;
pub mod process;
pub mod types;
pub mod uci;

pub use limits::EloTier;
pub use pool::{AnalysisHandle, AnalysisStore, EngineConfig, EnginePool};
pub use types::{Analysis, AnalysisRequest, EngineError, Limit, Priority, PvLine, Terminal};

/// Locate a Stockfish binary: `STOCKFISH_PATH`, then `engines/` under the
/// given root (as laid out by `cargo xtask fetch-stockfish`), then `PATH`.
pub fn find_stockfish(root: &std::path::Path) -> Option<std::path::PathBuf> {
    if let Ok(p) = std::env::var("STOCKFISH_PATH") {
        let p = std::path::PathBuf::from(p);
        if p.exists() {
            return Some(p);
        }
    }
    let names: &[&str] = if cfg!(windows) {
        &["stockfish-windows-x86-64-universal.exe", "stockfish.exe"]
    } else if cfg!(target_arch = "aarch64") {
        &["stockfish-linux-arm64-universal", "stockfish-macos-universal", "stockfish"]
    } else {
        &["stockfish-linux-x86-64-universal", "stockfish-macos-universal", "stockfish"]
    };
    for dir in [root.join("engines").join("stockfish"), root.join("engines")] {
        for n in names {
            let cand = dir.join(n);
            if cand.is_file() {
                return Some(cand);
            }
        }
    }
    let name = if cfg!(windows) { "stockfish.exe" } else { "stockfish" };
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths).map(|d| d.join(name)).find(|p| p.exists())
    })
}
