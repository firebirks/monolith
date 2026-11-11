# The check of the corrected test (fixcheck/fixcheck.sh), detached. Logs to fixcheck.log.
$V = $PSScriptRoot
$log = Join-Path $V 'fixcheck.log'
$wslV = '<work>/verify8'
function Note($text) { "$(Get-Date -Format s) $text" | Out-File $log -Append -Encoding ascii }
Note "fixcheck started"
wsl.exe -d Debian -- bash "$wslV/fixcheck/fixcheck.sh" *>> $log
Note "fixcheck exit $LASTEXITCODE"
Note "fixcheck ended"
