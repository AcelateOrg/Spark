# Spark CLI installer for Windows.
#
#   irm https://raw.githubusercontent.com/AcelateOrg/Spark/main/install.ps1 | iex
#
# Downloads spark.exe from the latest GitHub release into %LOCALAPPDATA%\Spark\bin and adds it to the user PATH.
# Run it again (or `spark update`) to update. Options (environment variables):
#   SPARK_VERSION = v0.1.0     install this release instead of the latest
#   SPARK_BIN     = C:\tools   install into this folder
#   SPARK_NO_PATH = 1          do not touch PATH
#   SPARK_ZIP     = file.zip   install from a local spark-windows-x64.zip (testing)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
$repo = 'AcelateOrg/Spark'
$assetName = 'spark-windows-x64.zip'

$bin = if ($env:SPARK_BIN) { $env:SPARK_BIN } else { Join-Path $env:LOCALAPPDATA 'Spark\bin' }
$tmp = Join-Path ([IO.Path]::GetTempPath()) ('spark-install-' + [guid]::NewGuid())
New-Item -ItemType Directory -Force -Path $tmp, $bin | Out-Null

try {
    $zip = Join-Path $tmp 'spark.zip'
    if ($env:SPARK_ZIP) {
        Copy-Item $env:SPARK_ZIP $zip
        $tag = 'local'
    } else {
        [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
        $api = if ($env:SPARK_VERSION) { "https://api.github.com/repos/$repo/releases/tags/$($env:SPARK_VERSION)" }
               else { "https://api.github.com/repos/$repo/releases/latest" }
        $release = Invoke-RestMethod -Uri $api -Headers @{ 'User-Agent' = 'spark-installer' }
        $asset = $release.assets | Where-Object { $_.name -eq $assetName } | Select-Object -First 1
        if (-not $asset) { throw "release $($release.tag_name) has no $assetName" }
        $tag = $release.tag_name
        Write-Host "Downloading Spark $tag ..."
        Invoke-WebRequest -Uri $asset.browser_download_url -OutFile $zip -UseBasicParsing
    }
    Expand-Archive -Path $zip -DestinationPath (Join-Path $tmp 'x') -Force
    $new = Join-Path $tmp 'x\spark.exe'
    if (-not (Test-Path $new)) { throw "$assetName does not contain spark.exe" }

    $exe = Join-Path $bin 'spark.exe'
    # Leftovers of earlier updates (deleted once they are no longer running).
    Get-ChildItem $bin -Filter 'spark.exe.old*' -ErrorAction SilentlyContinue | Remove-Item -Force -ErrorAction SilentlyContinue
    # A running spark.exe (`spark update`) can't be overwritten, but it can be renamed.
    if (Test-Path $exe) { Move-Item $exe "$exe.old-$([DateTime]::Now.Ticks)" -Force }
    Copy-Item $new $exe -Force
    # Not running (fresh install, installer started by hand): remove it right away.
    Get-ChildItem $bin -Filter 'spark.exe.old*' -ErrorAction SilentlyContinue | Remove-Item -Force -ErrorAction SilentlyContinue
} finally {
    Remove-Item $tmp -Recurse -Force -ErrorAction SilentlyContinue
}

if (-not $env:SPARK_NO_PATH) {
    $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    $parts = if ($userPath) { $userPath -split ';' | Where-Object { $_ } } else { @() }
    if ($parts -notcontains $bin) {
        [Environment]::SetEnvironmentVariable('Path', (($parts + $bin) -join ';'), 'User')
        Write-Host "Added $bin to your PATH. Open a new terminal to use 'spark'."
    }
    if (($env:Path -split ';') -notcontains $bin) { $env:Path = "$env:Path;$bin" }
}

& $exe --version
Write-Host "Spark $tag installed to $exe"
Write-Host "Next:  spark new mygame   then   spark mygame"
