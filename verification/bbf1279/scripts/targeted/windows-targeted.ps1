# The Windows part of the targeted re-verification: formatting, clippy
# (with and without the screens' fixtures) and the tests of monolith-core
# on 1.95.0 and 1.85.1, on a worktree of the candidate.
# Usage: pwsh -File windows-targeted.ps1 -Commit <sha>
param([Parameter(Mandatory = $true)][string]$Commit)
$ErrorActionPreference = 'Continue'
$here = $PSScriptRoot
$src = Join-Path $here 'win-src'
$out = Join-Path $here 'out-windows'
$repo = '<winhome>\VSCodeProjects\Monolith'
if (Test-Path $out) { Remove-Item $out -Recurse -Force }
New-Item -ItemType Directory -Path $out | Out-Null
git -C $repo worktree remove --force $src 2>$null
git -C $repo worktree add --detach $src $Commit | Out-Null
Set-Location $src
$env:CARGO_TARGET_DIR = Join-Path $here 'win-target'
$summary = Join-Path $out 'summary.txt'
"commit $(git rev-parse HEAD) started $(Get-Date -Format s)" | Out-File $summary -Encoding ascii
"$(rustc --version) / $(rustc +1.85.1 --version)" | Out-File $summary -Append -Encoding ascii
"status: $(git status --porcelain | Measure-Object -Line | Select-Object -ExpandProperty Lines) changed paths" | Out-File $summary -Append -Encoding ascii

function Step($name, [scriptblock]$block) {
    $started = Get-Date
    & $block *> (Join-Path $out "$name.log")
    $code = $LASTEXITCODE
    "$name exit $code ($([int]((Get-Date) - $started).TotalSeconds) s)" | Out-File $summary -Append -Encoding ascii
}

$f = '--features', 'monolith-ui/fixtures'
Step fmt { cargo fmt --all -- --check }
Step clippy { cargo clippy --workspace --all-targets --locked -- -D warnings }
Step clippy-fixtures { cargo clippy --workspace --all-targets --locked @f -- -D warnings }
Step test-core-1.95.0 { cargo test -p monolith-core --locked }
Step test-core-1.85.1 { cargo +1.85.1 test -p monolith-core --locked }
"status after: $(git status --porcelain | Measure-Object -Line | Select-Object -ExpandProperty Lines) changed paths" | Out-File $summary -Append -Encoding ascii
"done $(Get-Date -Format s)" | Out-File $summary -Append -Encoding ascii
