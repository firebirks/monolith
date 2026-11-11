# Gate C on c896861, first step (wsl/c2-unmodified.sh): the two unmodified
# checks with the new runner, detached. Logs to c2.log.
$V = $PSScriptRoot
$log = Join-Path $V 'c2.log'
$wslV = '<work>/verify8'
function Note($text) { "$(Get-Date -Format s) $text" | Out-File $log -Append -Encoding ascii }
Note "unmodified step started"
wsl.exe -d Debian -- bash "$wslV/wsl/c2-unmodified.sh" "$wslV/../run96.txt" *>> $log
Note "unmodified step exit $LASTEXITCODE"
Note "unmodified step ended"
