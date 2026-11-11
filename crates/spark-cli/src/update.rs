//! `spark update [vX.Y.Z]`: replaces this spark executable with a release from GitHub.
//! Windows runs `install.ps1` (PowerShell), Linux / macOS run `install.sh` (curl or wget + tar).

const INSTALL_PS1: &str = "https://raw.githubusercontent.com/AcelateOrg/Spark/main/install.ps1";
const INSTALL_SH: &str = "https://raw.githubusercontent.com/AcelateOrg/Spark/main/install.sh";
const FROM_SOURCE: &str = "  git clone https://github.com/AcelateOrg/Spark.git && cd Spark && cargo build --release -p spark-cli";

/// Parses `[version]` -> `Some("v0.2.0")`, `None` = latest. `Err` = usage error.
fn parse_version(args: &[String]) -> Result<Option<String>, ()> {
    match args {
        [] => Ok(None),
        [v] if v.starts_with('-') => Err(()),
        [v] if v.starts_with('v') => Ok(Some(v.clone())),
        [v] => Ok(Some(format!("v{v}"))),
        _ => Err(()),
    }
}

/// Release asset for this OS / CPU (must match .github/workflows/release.yml and the installers).
pub fn asset_name() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => Some("spark-windows-x64.zip"),
        ("linux", "x86_64") => Some("spark-linux-x64.tar.gz"),
        ("macos", "aarch64") => Some("spark-macos-arm64.tar.gz"),
        _ => None,
    }
}

pub fn run(args: &[String]) -> i32 {
    let Ok(version) = parse_version(args) else {
        eprintln!("usage: spark update [version]   e.g. spark update, spark update v0.2.0");
        return 1;
    };
    let Ok(exe) = std::env::current_exe() else {
        eprintln!("spark update: cannot find the spark executable");
        return 1;
    };
    let exe = exe.canonicalize().ok().and_then(|p| p.to_str().map(|s| std::path::PathBuf::from(s.trim_start_matches(r"\\?\")))).unwrap_or(exe);
    let Some(bin) = exe.parent() else { return 1 };
    if exe.components().any(|c| c.as_os_str() == "target") {
        eprintln!("spark update: this spark was built from source ({}).", exe.display());
        eprintln!("Update it with:  git pull && cargo build --release -p spark-cli");
        return 1;
    }
    if asset_name().is_none() {
        eprintln!(
            "spark update: no prebuilt release for {} {} (prebuilt: Windows x64, Linux x64, macOS arm64); build from source:",
            std::env::consts::OS,
            std::env::consts::ARCH
        );
        eprintln!("{FROM_SOURCE}");
        return 1;
    }
    println!("spark update: current version {}", engine::VERSION);
    let mut cmd = if cfg!(windows) {
        let mut c = std::process::Command::new("powershell");
        c.args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", &format!("irm {INSTALL_PS1} | iex")]);
        c
    } else {
        // curl or wget, whichever exists; the script itself also needs one of them.
        let script = format!(
            "set -e; if command -v curl >/dev/null 2>&1; then curl -fsSL {INSTALL_SH} | sh; \
             elif command -v wget >/dev/null 2>&1; then wget -qO- {INSTALL_SH} | sh; \
             else echo 'spark update: needs curl or wget' >&2; exit 1; fi"
        );
        let mut c = std::process::Command::new("sh");
        c.args(["-c", &script]);
        c
    };
    // The folder is already on PATH (or the user runs spark by full path): leave PATH alone.
    cmd.env("SPARK_BIN", bin).env("SPARK_NO_PATH", "1");
    if let Some(v) = version {
        cmd.env("SPARK_VERSION", v);
    }
    let shell = if cfg!(windows) { "powershell" } else { "sh" };
    match cmd.status() {
        Ok(s) if s.success() => 0,
        Ok(_) => {
            eprintln!("spark update: the installer failed (see above). Or build from source:\n{FROM_SOURCE}");
            1
        }
        Err(e) => {
            eprintln!("spark update: cannot start {shell}: {e}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions() {
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert_eq!(parse_version(&s(&[])), Ok(None));
        assert_eq!(parse_version(&s(&["0.2.0"])), Ok(Some("v0.2.0".into())));
        assert_eq!(parse_version(&s(&["v0.2.0"])), Ok(Some("v0.2.0".into())));
        assert!(parse_version(&s(&["--help"])).is_err());
        assert!(parse_version(&s(&["a", "b"])).is_err());
    }

    #[test]
    fn asset_names_match_the_release_workflow() {
        let workflow = include_str!("../../../.github/workflows/release.yml");
        for a in ["spark-windows-x64.zip", "spark-linux-x64.tar.gz", "spark-macos-arm64.tar.gz"] {
            assert!(workflow.contains(a), "release.yml does not build {a}");
        }
        let sh = include_str!("../../../install.sh");
        assert!(sh.contains("spark-linux-x64.tar.gz") || sh.contains("spark-$os-$arch.tar.gz"));
    }
}
