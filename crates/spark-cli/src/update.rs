//! `spark update [vX.Y.Z]`: replaces this spark.exe with a release from GitHub (runs install.ps1).

const INSTALLER: &str = "https://raw.githubusercontent.com/AcelateOrg/Spark/main/install.ps1";

pub fn run(args: &[String]) -> i32 {
    let version = match args {
        [] => None,
        [v] if v.starts_with('v') => Some(v.clone()),
        [v] => Some(format!("v{v}")),
        _ => {
            eprintln!("usage: spark update [version]   e.g. spark update, spark update v0.2.0");
            return 1;
        }
    };
    let Ok(exe) = std::env::current_exe() else {
        eprintln!("spark update: cannot find spark.exe");
        return 1;
    };
    let exe = exe.canonicalize().ok().and_then(|p| p.to_str().map(|s| std::path::PathBuf::from(s.trim_start_matches(r"\\?\")))).unwrap_or(exe);
    let Some(bin) = exe.parent() else { return 1 };
    if exe.components().any(|c| c.as_os_str() == "target") {
        eprintln!("spark update: this spark was built from source ({}).", exe.display());
        eprintln!("Update it with:  git pull && cargo build --release");
        return 1;
    }
    if !cfg!(windows) {
        eprintln!("spark update: prebuilt releases are Windows-only for now; build from source:");
        eprintln!("  git clone https://github.com/AcelateOrg/Spark.git && cd Spark && cargo build --release");
        return 1;
    }
    println!("spark update: current version {}", engine::VERSION);
    let mut cmd = std::process::Command::new("powershell");
    cmd.args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", &format!("irm {INSTALLER} | iex")])
        .env("SPARK_BIN", bin)
        // The folder is already on PATH (or the user runs spark by full path): leave PATH alone.
        .env("SPARK_NO_PATH", "1");
    if let Some(v) = version {
        cmd.env("SPARK_VERSION", v);
    }
    match cmd.status() {
        Ok(s) if s.success() => 0,
        Ok(_) => {
            eprintln!("spark update: the installer failed (see above)");
            1
        }
        Err(e) => {
            eprintln!("spark update: cannot start powershell: {e}");
            1
        }
    }
}
