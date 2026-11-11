# Replays the start of gate C up to three times (wsl/repro-gatec.sh), each
# cycle on a WSL started again: wsl --shutdown when no other distribution
# runs; when docker-desktop runs, only Debian is ended and its page cache
# dropped, so that Docker is never stopped. Stops at the first cycle that
# does not pass. Logs to diag-gatec.log.
$V = $PSScriptRoot
$log = Join-Path $V 'diag-gatec.log'
$wslV = '<work>/verify8'
$env:WSL_UTF8 = '1'
function Note($text) { "$(Get-Date -Format s) $text" | Out-File $log -Append -Encoding ascii }
function Running { @((wsl.exe -l --running -q 2>$null) -split "`r?`n" | ForEach-Object { $_.Trim() } | Where-Object { $_ }) }
Note "replay of the start of gate C started: up to 3 cycles"
for ($c = 1; $c -le 3; $c++) {
    $before = Running
    if ($before -contains 'docker-desktop') {
        $mode = 'wsl --terminate Debian, page cache dropped (docker-desktop runs)'
        wsl.exe --terminate Debian | Out-Null
    } else {
        $mode = 'wsl --shutdown'
        wsl.exe --shutdown | Out-Null
    }
    $waited = 0
    while (((Running) -contains 'Debian') -and $waited -lt 120) { Start-Sleep 2; $waited += 2 }
    if ((Running) -contains 'Debian') { Note "cycle ${c}: Debian still runs after ${waited}s: stop"; break }
    Note "cycle ${c}: $mode (running before: $($before -join ', ')); Debian stopped after ${waited}s"
    if ($mode -like '*terminate*') { wsl.exe -d Debian -u root -- sh "$wslV/../dropcache.sh" *>> $log }
    wsl.exe -d Debian -- bash "$wslV/wsl/repro-gatec.sh" "$wslV/../run96.txt" $c "$mode" *>> $log
    $code = $LASTEXITCODE
    Note "cycle $c exit $code"
    if ($code -ne 0) { Note "stopped at cycle $c"; break }
}
Note "replay ended"
