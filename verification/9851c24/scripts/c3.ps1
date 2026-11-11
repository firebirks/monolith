# Gate C on c90d5e5 (98 faults), detached: the two unmodified checks with
# the new runner (wsl/c3-unmodified.sh); if they pass, the 98
# (wsl/targeted-c3.sh). Logs to c3.log.
$V = $PSScriptRoot
$log = Join-Path $V 'c3.log'
$wslV = '<work>/verify8'
function Note($text) { "$(Get-Date -Format s) $text" | Out-File $log -Append -Encoding ascii }
Note "unmodified step started"
wsl.exe -d Debian -- bash "$wslV/wsl/c3-unmodified.sh" "$wslV/../run98.txt" *>> $log
$code = $LASTEXITCODE
Note "unmodified step exit $code"
if ($code -ne 0) { Note "stopped before the faults"; Note "gate C3 ended"; exit }
Note "gate C3 faults started"
wsl.exe -d Debian -- bash "$wslV/wsl/targeted-c3.sh" "$wslV/../run98.txt" *>> $log
Note "gate C3 exit $LASTEXITCODE"
Note "gate C3 ended"
