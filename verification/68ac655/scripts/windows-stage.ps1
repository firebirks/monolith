# The Windows part of the Phase 6 final verification, on a worktree of the
# frozen commit: formatting, clippy, the tests on 1.95.0 and 1.85.1 with the
# screens' fixtures, the Windows package, the workflows by script, the
# frame measurements, and the release client opened on this desktop: idle
# CPU and memory, and a picture of its window in each theme.
# Usage: pwsh -File windows-stage.ps1 -Commit <sha>
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
"commit $(git rev-parse HEAD)" | Out-File $summary -Encoding ascii
"$(rustc --version) / $(rustc +1.85.1 --version)" | Out-File $summary -Append -Encoding ascii

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
Step test-1.95.0 { cargo test --workspace --locked @f }
Step check-1.85.1 { cargo +1.85.1 check --workspace --all-targets --locked @f }
Step test-1.85.1 { cargo +1.85.1 test --workspace --locked @f }
Step package { pwsh -NoProfile -File packaging\windows\build.ps1 }
Copy-Item (Join-Path $src 'target\package\*.zip*') $out -ErrorAction SilentlyContinue
Step scenarios { cargo run --release --locked -p monolith-desktop --example scenarios -- (Join-Path $out 'scenarios') }
Step tor-smoke-memory { cargo run --release --locked -p monolith-desktop --example tor_smoke }
Step screens { cargo run --locked -p monolith-ui --features fixtures --example screens -- (Join-Path $out 'screens') }
Step measure { cargo run --release --locked -p monolith-ui --features fixtures --example measure }

# The packaged client, idle on the first-start screen.
$exe = Get-ChildItem (Join-Path $src 'target\package') -Recurse -Filter monolith-desktop.exe | Select-Object -First 1
$idle = Join-Path $out 'idle'
New-Item -ItemType Directory -Path $idle | Out-Null
$p = Start-Process -FilePath $exe.FullName -ArgumentList '--data-dir', (Join-Path $idle 'data'), '--config', (Join-Path $idle 'desktop.conf') -PassThru
Start-Sleep -Seconds 5
$p.Refresh()
$c1 = $p.TotalProcessorTime
Start-Sleep -Seconds 10
$p.Refresh()
$c2 = $p.TotalProcessorTime
"idle: working set $([math]::Round($p.WorkingSet64 / 1MB)) MB, private $([math]::Round($p.PrivateMemorySize64 / 1MB)) MB, cpu over 10 s $([math]::Round(($c2 - $c1).TotalMilliseconds)) ms, cpu since start $([math]::Round($c2.TotalMilliseconds)) ms" | Out-File $summary -Append -Encoding ascii
$p.CloseMainWindow() | Out-Null
if ($p.WaitForExit(15000)) { "idle client closed with code $($p.ExitCode)" | Out-File $summary -Append -Encoding ascii } else { "idle client did not close in 15 s" | Out-File $summary -Append -Encoding ascii }

# The development build that photographs its own window, in each theme.
Step gui-build { cargo build --release --locked -p monolith-desktop --features screenshot }
foreach ($theme in 'light', 'dark') {
    $dir = Join-Path $out "gui-$theme"
    New-Item -ItemType Directory -Path $dir | Out-Null
    "theme = $theme" | Set-Content -Path (Join-Path $dir 'desktop.conf') -Encoding ascii
    $env:MONOLITH_SCREENSHOT = Join-Path $out "gui-$theme.png"
    $g = Start-Process -FilePath (Join-Path $env:CARGO_TARGET_DIR 'release\monolith-desktop.exe') -ArgumentList '--data-dir', (Join-Path $dir 'data'), '--config', (Join-Path $dir 'desktop.conf') -PassThru
    if ($g.WaitForExit(60000)) { "gui-$theme exit $($g.ExitCode)" | Out-File $summary -Append -Encoding ascii } else { "gui-$theme did not close"; $g.Kill() }
    Remove-Item Env:MONOLITH_SCREENSHOT
}
"done" | Out-File $summary -Append -Encoding ascii
Set-Location $repo
