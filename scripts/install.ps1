# kq standalone installer (Windows).
#
#   powershell -ExecutionPolicy ByPass -c "irm https://github.com/holowan-biolabs/kq/releases/latest/download/install.ps1 | iex"
#
# Optional env:
#   KQ_VERSION      release tag, e.g. v0.1.0 (default: latest)
#   KQ_INSTALL_DIR  install directory (default: $env:USERPROFILE\.local\bin)
#   KQ_REPO         GitHub repo (default: holowan-biolabs/kq)

$ErrorActionPreference = "Stop"

$Repo = if ($env:KQ_REPO) { $env:KQ_REPO } else { "holowan-biolabs/kq" }
$Version = if ($env:KQ_VERSION) { $env:KQ_VERSION } else { "latest" }
$BinDir = if ($env:KQ_INSTALL_DIR) { $env:KQ_INSTALL_DIR } else { Join-Path $env:USERPROFILE ".local\bin" }

function Get-AssetBase {
    if ($Version -eq "latest") {
        return "https://github.com/$Repo/releases/latest/download"
    }
    return "https://github.com/$Repo/releases/download/$Version"
}

function Get-Target {
    switch ($env:PROCESSOR_ARCHITECTURE) {
        "AMD64" { return "x86_64-pc-windows-msvc" }
        "ARM64" { return "aarch64-pc-windows-msvc" }
        default { return $null }
    }
}

function Test-Url([string]$Url) {
    try {
        $r = Invoke-WebRequest -Uri $Url -Method Head -UseBasicParsing
        return $r.StatusCode -lt 400
    } catch {
        return $false
    }
}

function Install-FromRelease {
    $target = Get-Target
    if (-not $target) { return $false }
    $base = Get-AssetBase
    $name = "kq-$target.exe"
    $url = "$base/$name"
    if (-not (Test-Url $url)) { return $false }

    New-Item -ItemType Directory -Force -Path $BinDir | Out-Null
    $dest = Join-Path $BinDir "kq.exe"
    Write-Host "downloading $name ($Version)"
    Invoke-WebRequest -Uri $url -OutFile $dest -UseBasicParsing
    return $true
}

function Install-FromCargo {
    if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
        return $false
    }
    Write-Host "no prebuilt binary for this platform; building with cargo"
    if ($Version -eq "latest") {
        cargo install --git "https://github.com/$Repo" --locked --force
    } else {
        cargo install --git "https://github.com/$Repo" --tag $Version --locked --force
    }
    return $LASTEXITCODE -eq 0
}

if (Install-FromRelease) {
    $kq = Join-Path $BinDir "kq.exe"
    Write-Host "installed $(& $kq --version) -> $kq"
} elseif (Install-FromCargo) {
    Write-Host "installed via cargo"
} else {
    Write-Error @"
kq-install: no prebuilt Windows binary yet, and cargo is not on PATH.
Install Rust from https://rustup.rs/ then re-run this script:

  irm https://win.rustup.rs -OutFile rustup-init.exe
  .\rustup-init.exe -y
"@
}
