# The Phase 8 final verification of one frozen commit, one stage after the
# other, so that no heavy stage runs beside another (a mutation run under
# load can count a timeout as caught; the private network test has timing
# windows): the container stage, the Windows stage, the mutation lists of
# code built on Windows only, the WSL stage, then the mutation baseline of
# Phases 1 to 8 in WSL. Each stage logs to its own directory; this script
# logs to master.log.
# Usage: pwsh -File master.ps1 -Commit <sha>
param([Parameter(Mandatory = $true)][string]$Commit)
$V = $PSScriptRoot
$log = Join-Path $V 'master.log'
$repo = '<repo>'
$full = git -C $repo rev-parse $Commit
$wslV = '<work>/verify8'
function Note($text) { "$(Get-Date -Format s) $text" | Out-File $log -Append -Encoding ascii }
Note "frozen candidate $full"

# 1. The container: the toolchain matrix, the private network with three
# restart cycles and the peer of phase-7-complete, the scenarios.
pwsh -NoProfile -File (Join-Path $V 'container\start-container.ps1') -Commit $full -Cycles 3 *>> $log
docker wait monolith-final8 *>> $log
docker logs monolith-final8 *> (Join-Path $V 'container\container.log')
Note "container stage ended"

# 2. Windows, on a worktree of the commit.
pwsh -NoProfile -File (Join-Path $V 'windows-stage.ps1') -Commit $full *>> $log
git -C $repo worktree remove --force (Join-Path $repo 'target\final8-src') *>> $log
Remove-Item (Join-Path $repo 'target\final8-win') -Recurse -Force -ErrorAction SilentlyContinue
Note "Windows stage ended"

# 3. The mutation lists of code built on Windows only, on the repository at
# the commit.
$head = git -C $repo rev-parse HEAD
if ($head -ne $full) {
    Note "the repository is at $head, not ${full}; phase7w and phase8w not run"
} else {
    Push-Location $repo
    $env:PYTHONUNBUFFERED = '1'
    $out = Join-Path $V 'out-windows'
    foreach ($list in 'phase7w', 'phase8w') {
        python mutation\run.py $list 1 *> (Join-Path $out "mutation-$list.log")
        Note "$list exit $LASTEXITCODE"
        Copy-Item (Join-Path $repo "mutation\results-$list-$($full.Substring(0,7)).json") $out -ErrorAction SilentlyContinue
    }
    Pop-Location
}

# 4. WSL: deny, audit, identity and trailers, the documentation, the fuzz
# targets, the client on WSLg, the Linux package.
wsl.exe -d Debian -- bash "$wslV/wsl/final-wsl.sh" $full *>> $log
Note "WSL stage ended"

# 5. The mutation baseline of Phases 1 to 8.
wsl.exe -d Debian -- bash "$wslV/wsl/final-mutation.sh" $full 2 *>> $log
Note "mutation baseline ended"
Note "done"
