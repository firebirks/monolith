# Runs the Windows stage of the Phase 8 final verification, removes its
# target directory to make room, then runs the mutation lists of code built
# on Windows only (phase7w, phase8w) on the repository at the frozen commit.
# Usage: pwsh -File chain-windows.ps1 -Commit <sha>
param([Parameter(Mandatory = $true)][string]$Commit)
$V = $PSScriptRoot
$log = Join-Path $V 'chain-windows.log'
$repo = '<repo>'
$full = git -C $repo rev-parse $Commit
"Windows stage started $(Get-Date -Format s)" | Out-File $log -Append
pwsh -NoProfile -File (Join-Path $V 'windows-stage.ps1') -Commit $full *>> $log
"Windows stage ended $(Get-Date -Format s)" | Out-File $log -Append
git -C $repo worktree remove --force (Join-Path $V 'win-src') *>> $log
Remove-Item (Join-Path $V 'win-target') -Recurse -Force -ErrorAction SilentlyContinue
$head = git -C $repo rev-parse HEAD
if ($head -ne $full) {
    "the repository is at $head, not ${full}; phase7w and phase8w not run" | Out-File $log -Append
    exit 1
}
Set-Location $repo
$env:PYTHONUNBUFFERED = '1'
$out = Join-Path $V 'out-windows'
foreach ($list in 'phase7w', 'phase8w') {
    python mutation\run.py $list 1 *> (Join-Path $out "mutation-$list.log")
    "$list exit $LASTEXITCODE $(Get-Date -Format s)" | Out-File $log -Append
    Copy-Item (Join-Path $repo "mutation\results-$list-$($full.Substring(0,7)).json") $out -ErrorAction SilentlyContinue
}
"done $(Get-Date -Format s)" | Out-File $log -Append
