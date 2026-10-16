# Windows build: produces a single, self-contained slipcase.exe (static CRT, no installer needed).
#   .\build.ps1
$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot
$env:CARGO_RESOLVER_INCOMPATIBLE_RUST_VERSIONS = "fallback"
rustup target add x86_64-pc-windows-msvc
cargo build --release --target x86_64-pc-windows-msvc
New-Item -ItemType Directory -Force -Path dist | Out-Null
Copy-Item "target\x86_64-pc-windows-msvc\release\slipcase.exe" "dist\slipcase-windows-x86_64.exe" -Force
Write-Host "-> dist\slipcase-windows-x86_64.exe"
