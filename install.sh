#!/bin/sh
# Spark CLI installer for Linux and macOS.
#
#   curl -fsSL https://raw.githubusercontent.com/AcelateOrg/Spark/main/install.sh | sh
#
# Downloads spark from the latest GitHub release into ~/.local/bin (Windows: use install.ps1).
# Run it again (or `spark update`) to update. Options (environment variables):
#   SPARK_VERSION = v0.1.0          install this release instead of the latest
#   SPARK_BIN     = /usr/local/bin  install into this folder
#   SPARK_NO_PATH = 1               do not print PATH hints
#   SPARK_ARCHIVE = file.tar.gz     install from a local spark-<os>-<arch>.tar.gz (testing)
#   SPARK_REPO    = owner/repo      download from a fork (default AcelateOrg/Spark)
#
# Prebuilt: Linux x64, macOS arm64 (Apple Silicon). Anything else: build from source
#   git clone https://github.com/AcelateOrg/Spark.git && cd Spark && cargo build --release -p spark-cli
set -eu

repo="${SPARK_REPO:-AcelateOrg/Spark}"
bin="${SPARK_BIN:-$HOME/.local/bin}"

fail() {
    echo "spark install: $*" >&2
    exit 1
}

case "$(uname -s)" in
    Linux) os=linux ;;
    Darwin) os=macos ;;
    *) fail "unsupported OS '$(uname -s)' (Windows: irm https://raw.githubusercontent.com/$repo/main/install.ps1 | iex)" ;;
esac
case "$(uname -m)" in
    x86_64 | amd64) arch=x64 ;;
    arm64 | aarch64) arch=arm64 ;;
    *) arch="$(uname -m)" ;;
esac
asset="spark-$os-$arch.tar.gz"
case "$asset" in
    spark-linux-x64.tar.gz | spark-macos-arm64.tar.gz) ;;
    *) fail "no prebuilt $asset (prebuilt: Linux x64, macOS arm64). Build from source:
  git clone https://github.com/$repo.git && cd Spark && cargo build --release -p spark-cli" ;;
esac

download() { # url file
    if command -v curl >/dev/null 2>&1; then
        curl -fSL --progress-bar -o "$2" "$1"
    elif command -v wget >/dev/null 2>&1; then
        wget -q -O "$2" "$1"
    else
        fail "needs curl or wget"
    fi
}

tmp="$(mktemp -d 2>/dev/null || mktemp -d -t spark-install)"
trap 'rm -rf "$tmp"' EXIT INT TERM

if [ -n "${SPARK_ARCHIVE:-}" ]; then
    cp "$SPARK_ARCHIVE" "$tmp/spark.tar.gz"
    tag=local
else
    if [ -n "${SPARK_VERSION:-}" ]; then
        tag="$SPARK_VERSION"
        url="https://github.com/$repo/releases/download/$tag/$asset"
    else
        tag=latest
        url="https://github.com/$repo/releases/latest/download/$asset"
    fi
    echo "Downloading Spark $tag ($asset) ..."
    download "$url" "$tmp/spark.tar.gz" || fail "download failed: $url (does release $tag have $asset?)"
fi

mkdir -p "$tmp/x" "$bin"
tar -xzf "$tmp/spark.tar.gz" -C "$tmp/x" || fail "$asset is not a valid .tar.gz"
new="$(find "$tmp/x" -type f -name spark | head -n 1)"
[ -n "$new" ] || fail "$asset does not contain the spark executable"
chmod +x "$new"
# macOS: files downloaded by a browser are quarantined; curl doesn't do that, but a local archive might.
if [ "$os" = macos ] && command -v xattr >/dev/null 2>&1; then
    xattr -d com.apple.quarantine "$new" 2>/dev/null || true
fi

# Replace atomically: a running spark (`spark update`) keeps its old inode.
cp "$new" "$bin/spark.new"
chmod 755 "$bin/spark.new"
mv -f "$bin/spark.new" "$bin/spark"

if [ -z "${SPARK_NO_PATH:-}" ]; then
    case ":$PATH:" in
        *":$bin:"*) ;;
        *)
            echo "Add $bin to your PATH, e.g.:"
            echo "  echo 'export PATH=\"$bin:\$PATH\"' >> ~/.profile   (zsh: ~/.zshrc)"
            ;;
    esac
fi

"$bin/spark" --version
echo "Spark $tag installed to $bin/spark"
echo "Next:  spark new mygame   then   spark mygame"
