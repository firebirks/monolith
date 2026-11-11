# Gate C on c896861 (wsl/targeted-c2.sh), detached. Logs to targeted-c2.log.
$V = $PSScriptRoot
$log = Join-Path $V 'targeted-c2.log'
$wslV = '<work>/verify8'
function Note($text) { "$(Get-Date -Format s) $text" | Out-File $log -Append -Encoding ascii }
Note "gate C2 started"
wsl.exe -d Debian -- bash "$wslV/wsl/targeted-c2.sh" "$wslV/../run96.txt" *>> $log
Note "gate C2 exit $LASTEXITCODE"
Note "gate C2 ended"
