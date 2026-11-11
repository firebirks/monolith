# Waits for the container stage of the Phase 8 final verification, keeps
# its log, then runs the WSL stage and the mutation baseline in WSL, one
# after the other: they share the VM with the container, and a heavy stage
# beside the mutation runs could make a fault time out.
# Usage: pwsh -File chain-wsl.ps1 -Commit <sha>
param([Parameter(Mandatory = $true)][string]$Commit)
$V = $PSScriptRoot
$log = Join-Path $V 'chain-wsl.log'
$wslV = '<work>/verify8'
$repo = '<repo>'
$full = git -C $repo rev-parse $Commit
"waiting for the container $(Get-Date -Format s)" | Out-File $log -Append
docker wait monolith-final8 *>> $log
docker logs monolith-final8 *> (Join-Path $V 'container\container.log')
"container ended $(Get-Date -Format s)" | Out-File $log -Append
wsl.exe -d Debian -- bash "$wslV/wsl/final-wsl.sh" $full *>> $log
"wsl stage ended $(Get-Date -Format s)" | Out-File $log -Append
wsl.exe -d Debian -- bash "$wslV/wsl/final-mutation.sh" $full 2 *>> $log
"mutation ended $(Get-Date -Format s)" | Out-File $log -Append
