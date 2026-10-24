# Waits for the Windows stage of the Phase 7 final verification, removes
# its target directory to make room, then runs the mutation list of code
# built on Windows only (phase7w) on the repository at the frozen commit.
# Usage: pwsh -File chain-windows.ps1 -Commit <sha>
param([Parameter(Mandatory = $true)][string]$Commit)
$V = $PSScriptRoot
$log = Join-Path $V 'chain-windows.log'
$repo = '<winhome>\VSCodeProjects\Monolith'
$summary = Join-Path $V 'out-windows\summary.txt'
"waiting for the Windows stage $(Get-Date -Format s)" | Out-File $log -Append
while (-not ((Test-Path $summary) -and (Select-String -Path $summary -Pattern '^done' -Quiet))) {
    Start-Sleep -Seconds 60
}
"Windows stage ended $(Get-Date -Format s)" | Out-File $log -Append
Remove-Item (Join-Path $V 'win-target') -Recurse -Force -ErrorAction SilentlyContinue
$head = git -C $repo rev-parse HEAD
if ($head -ne $Commit) {
    "the repository is at $head, not ${Commit}; phase7w not run" | Out-File $log -Append
    exit 1
}
Set-Location $repo
$env:PYTHONUNBUFFERED = '1'
python mutation\run.py phase7w 1 *> (Join-Path $V 'out-windows\mutation-phase7w.log')
"phase7w exit $LASTEXITCODE $(Get-Date -Format s)" | Out-File $log -Append
Copy-Item (Join-Path $repo "mutation\results-phase7w-$($Commit.Substring(0,7)).json") (Join-Path $V 'out-windows') -ErrorAction SilentlyContinue
