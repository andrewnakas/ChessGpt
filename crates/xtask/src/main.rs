//! Project tasks: `cargo xtask <command>`.
//!
//!   setup            check Node, install web deps, fetch Stockfish, generate types
//!   fetch-stockfish  download the pinned Stockfish release into engines/
//!   gen-types        write web/src/lib/api/types.ts (--check fails if stale)
//!   dev              run the API server and the Vite dev server together
//!   build            production build: web bundle embedded in one release binary

use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus};

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().parent().unwrap().to_path_buf()
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("setup") => setup(),
        Some("fetch-stockfish") => fetch_stockfish(),
        Some("gen-types") => gen_types(args.iter().any(|a| a == "--check")),
        Some("dev") => dev(),
        Some("build") => build(),
        _ => {
            eprintln!("usage: cargo xtask <setup|fetch-stockfish|gen-types [--check]|dev|build>");
            std::process::exit(2);
        }
    }
}

/// `npm` is a .cmd shim on Windows and cannot be spawned directly.
fn npm(args: &[&str]) -> Command {
    let mut c = if cfg!(windows) {
        let mut c = Command::new("cmd");
        c.arg("/C").arg("npm");
        c
    } else {
        Command::new("npm")
    };
    c.args(args).current_dir(root().join("web"));
    c
}

fn run(mut c: Command, what: &str) -> Result<()> {
    let st: ExitStatus = c.status().with_context(|| format!("failed to start {what}"))?;
    if !st.success() {
        bail!("{what} failed with {st}");
    }
    Ok(())
}

fn setup() -> Result<()> {
    let out = Command::new(if cfg!(windows) { "node.exe" } else { "node" }).arg("--version").output();
    match out {
        Ok(o) if o.status.success() => {
            let v = String::from_utf8_lossy(&o.stdout).trim().to_string();
            let major: u32 = v.trim_start_matches('v').split('.').next().unwrap_or("0").parse().unwrap_or(0);
            if major < 24 {
                bail!("Node {v} found; Node 24 LTS or newer is required");
            }
            println!("node {v}");
        }
        _ => bail!(
            "Node.js not found. Install Node 24 LTS:\n  Windows: winget install OpenJS.NodeJS.LTS\n  macOS:   brew install node@24\n  Linux:   https://nodejs.org/en/download"
        ),
    }
    gen_types(false)?;
    run(npm(&["install"]), "npm install")?;
    fetch_stockfish()?;
    println!("setup complete. Next: cargo xtask dev");
    Ok(())
}

struct Asset {
    file: String,
    sha256: String,
    binary: String,
    tag: String,
}

fn pinned_asset() -> Result<Asset> {
    let lock = std::fs::read_to_string(root().join("crates/xtask/stockfish.lock"))?;
    let platform = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => "windows-x86_64",
        ("linux", "x86_64") => "linux-x86_64",
        ("linux", "aarch64") => "linux-aarch64",
        (os, arch) => bail!(
            "no pinned Stockfish build for {os}/{arch}; install Stockfish yourself and set STOCKFISH_PATH"
        ),
    };
    let mut tag = String::new();
    for line in lock.lines().filter(|l| !l.starts_with('#')) {
        let cols: Vec<&str> = line.split_whitespace().collect();
        match cols.as_slice() {
            ["tag", t] => tag = t.to_string(),
            [p, file, sha, bin] if *p == platform => {
                return Ok(Asset { file: file.to_string(), sha256: sha.to_string(), binary: bin.to_string(), tag });
            }
            _ => {}
        }
    }
    bail!("stockfish.lock has no entry for {platform}")
}

fn fetch_stockfish() -> Result<()> {
    let a = pinned_asset()?;
    let engines = root().join("engines");
    let bin = engines.join(&a.binary);
    if bin.is_file() {
        println!("stockfish present: {}", bin.display());
        return Ok(());
    }
    std::fs::create_dir_all(&engines)?;
    let archive = engines.join(&a.file);
    let url = format!(
        "https://github.com/official-stockfish/Stockfish/releases/download/{}/{}",
        a.tag, a.file
    );
    println!("downloading {url}");
    let mut c = Command::new("curl");
    c.args(["-fSL", "--retry", "3", "-o"]).arg(&archive).arg(&url);
    run(c, "curl")?;
    let digest = hex(&Sha256::digest(std::fs::read(&archive)?));
    if digest != a.sha256 {
        std::fs::remove_file(&archive).ok();
        bail!("checksum mismatch for {}: got {digest}, expected {}", a.file, a.sha256);
    }
    // bsdtar ships with Windows 10+ and extracts both .zip and .tar.gz.
    let mut c = Command::new("tar");
    c.arg("-xf").arg(&archive).arg("-C").arg(&engines);
    run(c, "tar")?;
    std::fs::remove_file(&archive).ok();
    if !bin.is_file() {
        bail!("archive did not contain {}", a.binary);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755))?;
    }
    println!("stockfish installed: {}", bin.display());
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn gen_types(check: bool) -> Result<()> {
    let path = root().join("web/src/lib/api/types.ts");
    let ts = api_types::typescript();
    let current = std::fs::read_to_string(&path).unwrap_or_default();
    if check {
        if current.replace("\r\n", "\n") != ts {
            bail!("{} is stale; run `cargo xtask gen-types`", path.display());
        }
        println!("types up to date");
        return Ok(());
    }
    std::fs::create_dir_all(path.parent().unwrap())?;
    if current != ts {
        std::fs::write(&path, ts)?;
        println!("wrote {}", path.display());
    }
    Ok(())
}

struct KillOnDrop(Child);
impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
    }
}

fn dev() -> Result<()> {
    gen_types(false)?;
    let mut server = Command::new("cargo");
    server
        .args(["run", "-p", "server", "--"])
        .env("CHESSGPT_DEV", "1")
        .current_dir(root());
    let server = KillOnDrop(server.spawn().context("starting server")?);
    let web = KillOnDrop(npm(&["run", "dev"]).spawn().context("starting vite")?);
    println!("API on http://localhost:8080, UI on http://localhost:5173 (Ctrl-C stops both)");
    let mut procs = [server, web];
    loop {
        for p in procs.iter_mut() {
            if let Some(st) = p.0.try_wait()? {
                println!("a dev process exited with {st}; stopping");
                return Ok(());
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(300));
    }
}

fn build() -> Result<()> {
    gen_types(false)?;
    run(npm(&["run", "build"]), "web build")?;
    let mut c = Command::new("cargo");
    c.args(["build", "--release", "-p", "server"]).current_dir(root());
    run(c, "cargo build")?;
    let exe = if cfg!(windows) { "chessgpt.exe" } else { "chessgpt" };
    println!("built {}", root().join("target/release").join(exe).display());
    Ok(())
}
