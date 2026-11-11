# Gate B: the 84 targeted Phase 1-7 faults in WSL (wsl/targeted-mutation.sh)
# on the commit given. Logs to targeted.log.
# Usage: pwsh -File targeted.ps1 -Commit <sha>
param([Parameter(Mandatory = $true)][string]$Commit)
$V = $PSScriptRoot
$log = Join-Path $V 'targeted.log'
$wslV = '<work>/verify8'
function Note($text) { "$(Get-Date -Format s) $text" | Out-File $log -Append -Encoding ascii }
Note "gate B of $Commit started"
wsl.exe -d Debian -- bash "$wslV/wsl/targeted-mutation.sh" $Commit 2 *>> $log
Note "gate B ended"
