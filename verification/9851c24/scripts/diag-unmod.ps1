# Runs wsl/repro-unmodified.sh detached, as targeted-c.ps1 runs gate C:
# up to six runs of the unmodified check of tree1. Logs to diag-unmod.log.
$V = $PSScriptRoot
$log = Join-Path $V 'diag-unmod.log'
$wslV = '<work>/verify8'
function Note($text) { "$(Get-Date -Format s) $text" | Out-File $log -Append -Encoding ascii }
Note "repro of the unmodified check started"
wsl.exe -d Debian -- bash "$wslV/wsl/repro-unmodified.sh" "$wslV/../run96.txt" 6 *>> $log
Note "repro ended"
