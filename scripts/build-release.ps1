# Builds the release executable reproducibly and prints its SHA-256.
#
# Two builds of the same commit must produce the same bytes, otherwise the
# checksum in dist/ cannot verify a rebuild. `build.rs` already passes /Brepro,
# which stops the MSVC linker stamping the time and a random PDB GUID into the
# executable. What remains is absolute paths: panic messages from dependencies
# embed their source paths, so a build from C:\Users\<you>\.cargo\... would differ
# from one under another profile. The remaps below rewrite those prefixes to
# fixed locations, so the hash does not depend on whose cargo home compiled it.
#
# Scope: the hash is reproducible for builds of this checkout — including from an
# empty target/ — but not across checkout directories, because Cargo hashes the
# package's absolute path into crate metadata and that reorders symbols.
#
# Usage, from anywhere:
#   pwsh -File scripts\build-release.ps1        (PowerShell 7)
#   powershell -File scripts\build-release.ps1  (Windows PowerShell 5.1)
#
# Run this, then compare the printed hash with dist\*.sha256.

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

$cargoHome = if ($env:CARGO_HOME) { $env:CARGO_HOME } else { Join-Path $env:USERPROFILE '.cargo' }
$registry = Join-Path $cargoHome 'registry\src'

# Cargo splits RUSTFLAGS on whitespace, which breaks a path containing spaces.
# The encoded form takes one variable per argument, separated by 0x1f.
$sep = [char]0x1f
$env:RUSTFLAGS = $null
$env:CARGO_ENCODED_RUSTFLAGS = @(
    "--remap-path-prefix=$registry=/cargo/registry"
    "--remap-path-prefix=$root=/build"
) -join $sep

cargo build --release --locked --target x86_64-pc-windows-msvc
if ($LASTEXITCODE -ne 0) { throw "cargo build failed with exit code $LASTEXITCODE" }

$exe = Join-Path $root 'target\x86_64-pc-windows-msvc\release\discord-mcp.exe'
if (-not (Test-Path $exe)) { throw "expected artifact not found: $exe" }
(Get-FileHash $exe -Algorithm SHA256).Hash.ToLower()
