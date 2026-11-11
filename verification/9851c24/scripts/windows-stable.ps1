# Gate D of the Phase 8 convergence plan: the workspace tests on the
# current stable Rust (1.99.1) on Windows, with the screens' fixtures, as
# the Windows stage ran them on 1.95.0 and 1.85.1. A worktree of the commit
# and a target directory of its own.
# Usage: pwsh -File windows-stable.ps1 -Commit <sha>
param([Parameter(Mandatory = $true)][string]$Commit)
$ErrorActionPreference = 'Continue'
$here = $PSScriptRoot
$out = Join-Path $here 'out-stable'
$repo = '<repo>'
$src = Join-Path $repo 'target\final8-stable-src'
New-Item -ItemType Directory -Path $out -Force | Out-Null
$summary = Join-Path $out 'summary.txt'
git -C $repo worktree remove --force $src 2>$null
git -C $repo worktree add --detach $src $Commit | Out-Null
Set-Location $src
$env:CARGO_TARGET_DIR = Join-Path $repo 'target\final8-stable'
"commit $(git rev-parse HEAD) started $(Get-Date -Format s)" | Out-File $summary -Encoding ascii
"$(rustc +1.99.1 --version) / $(cargo +1.99.1 --version)" | Out-File $summary -Append -Encoding ascii
$started = Get-Date
cargo +1.99.1 test --workspace --locked --features monolith-ui/fixtures *> (Join-Path $out 'test-1.99.1.log')
$code = $LASTEXITCODE
"test-1.99.1 exit $code ($([int]((Get-Date) - $started).TotalSeconds) s)" | Out-File $summary -Append -Encoding ascii
"ended $(Get-Date -Format s)" | Out-File $summary -Append -Encoding ascii
