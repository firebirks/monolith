# Gate C: the 96 Phase 1-7 faults of the corrected selection in WSL
# (wsl/targeted-c.sh) on the commit given. Logs to targeted-c.log.
# Usage: pwsh -File targeted-c.ps1 -Commit <sha>
param([Parameter(Mandatory = $true)][string]$Commit)
$V = $PSScriptRoot
$log = Join-Path $V 'targeted-c.log'
$wslV = '<work>/verify8'
function Note($text) { "$(Get-Date -Format s) $text" | Out-File $log -Append -Encoding ascii }
Note "gate C of $Commit started"
wsl.exe -d Debian -- bash "$wslV/wsl/targeted-c.sh" $Commit "$wslV/../run96.txt" 2 *>> $log
Note "gate C ended"
