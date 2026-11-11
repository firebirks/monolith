# Gate C3 after Windows restarted (2026-11-11), detached: the two unmodified
# checks on the restored copies (wsl/c3r-unmodified.sh); if they pass,
# FT29 and HR77 (wsl/targeted-c3r.sh). Logs to c3r.log.
$V = $PSScriptRoot
$log = Join-Path $V 'c3r.log'
$wslV = '<work>/verify8'
function Note($text) { "$(Get-Date -Format s) $text" | Out-File $log -Append -Encoding ascii }
Note "unmodified step started"
wsl.exe -d Debian -- bash "$wslV/wsl/c3r-unmodified.sh" "$wslV/../run98.txt" *>> $log
$code = $LASTEXITCODE
Note "unmodified step exit $code"
if ($code -ne 0) { Note "stopped before the faults"; Note "gate C3r ended"; exit }
Note "gate C3r faults started"
wsl.exe -d Debian -- bash "$wslV/wsl/targeted-c3r.sh" *>> $log
Note "gate C3r exit $LASTEXITCODE"
Note "gate C3r ended"
