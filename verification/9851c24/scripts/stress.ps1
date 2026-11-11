# Runs the stress of transfer_restarts under CPU contention
# (wsl/stress-restarts.sh) on the running WSL, detached from the session.
# Logs to stress.log.
$V = $PSScriptRoot
$log = Join-Path $V 'stress.log'
$wslV = '<work>/verify8'
function Note($text) { "$(Get-Date -Format s) $text" | Out-File $log -Append -Encoding ascii }
Note "stress started"
wsl.exe -d Debian -- bash "$wslV/wsl/stress-restarts.sh" *>> $log
Note "stress exit $LASTEXITCODE"
Note "stress ended"
