param(
    [string]$Out = "",
    [string]$Target = ""
)
$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent $PSScriptRoot
if ([string]::IsNullOrWhiteSpace($Out)) { $Out = Join-Path $Root "dist" }
$Manifest = Join-Path $Root "Cargo.toml"
$Config = Join-Path $Root ".cargo/config.toml"
$BuildDir = $env:CARGO_TARGET_DIR
if ([string]::IsNullOrWhiteSpace($BuildDir)) { $BuildDir = Join-Path $Root "target" }
$BuildDir = [System.IO.Path]::GetFullPath($BuildDir)
$TargetArgs = @()
if (-not [string]::IsNullOrWhiteSpace($Target)) {
    $TargetArgs = @("--target", $Target)
    $BuildDir = Join-Path $BuildDir $Target
}
$ExeName = "screenphoto"
if ($Target.Contains("windows") -or ([string]::IsNullOrWhiteSpace($Target) -and $env:OS -eq "Windows_NT")) { $ExeName += ".exe" }
$Binary = Join-Path $BuildDir "release/$ExeName"

& cargo --config $Config build --manifest-path $Manifest --release --locked @TargetArgs --bin screenphoto
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
& cargo --config $Config run --manifest-path $Manifest --release --locked --features dev-tools --bin screenphoto-dev -- --root $Root package --binary $Binary @TargetArgs --out $Out
exit $LASTEXITCODE
