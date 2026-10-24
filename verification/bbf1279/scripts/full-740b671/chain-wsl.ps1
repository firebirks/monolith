# Waits for the container stage of the Phase 7 final verification, keeps
# its log, then runs the WSL stage and the mutation lists in WSL, one after
# the other: they share the VM with the container.
# Usage: pwsh -File chain-wsl.ps1 -Commit <sha>
param([Parameter(Mandatory = $true)][string]$Commit)
$V = $PSScriptRoot
$log = Join-Path $V 'chain-wsl.log'
$wslV = '<work>'
"waiting for the container $(Get-Date -Format s)" | Out-File $log -Append
docker wait monolith-final7 *>> $log
docker logs monolith-final7 *> (Join-Path $V 'container\container.log')
"container ended $(Get-Date -Format s)" | Out-File $log -Append
wsl.exe -d Debian -- bash -c "tr -d '\r' < $wslV/wsl/final-wsl.sh > ~/final-wsl.sh && tr -d '\r' < $wslV/wsl/final-mutation.sh > ~/final-mutation.sh && chmod +x ~/final-wsl.sh ~/final-mutation.sh" *>> $log
wsl.exe -d Debian -- bash -lc "~/final-wsl.sh $Commit" *>> $log
"wsl stage ended $(Get-Date -Format s)" | Out-File $log -Append
wsl.exe -d Debian -- bash -lc "~/final-mutation.sh $Commit 2" *>> $log
"mutation ended $(Get-Date -Format s)" | Out-File $log -Append
