# Watches the host while the mutation run of the re-verification runs in
# WSL (Debian).
# Memory, every 20 s: below 4 GB available it drops the page cache of WSL;
# below 2 GB, if dropping it was not enough, it stops the run
# (~/mutpause.sh stop) and continues it once 4 GB have been available for
# a minute.
# Disk, every 5 min: below 50 GB (decimal) free on C: it removes the old
# build output in WSL (~/clean-old-targets.sh; the disk watchdog of the
# earlier session removes the Windows side); below 20 GB it stops the run
# until 25 GB are free again.
# Each event is one line of phase7-watchdog.log, and the readings are one
# line every 10 min. Stops when phase7-watchdog.stop exists, continuing a
# run it stopped.
$V = $PSScriptRoot
$log = Join-Path $V 'phase7-watchdog.log'
$stopFile = Join-Path $V 'phase7-watchdog.stop'

function Note($text) { "$(Get-Date -Format s) $text" | Out-File $log -Append -Encoding ascii }
function AvailGB { [math]::Round((Get-CimInstance Win32_OperatingSystem).FreePhysicalMemory / 1MB, 2) }
function DiskGB { [math]::Round((Get-PSDrive C).Free / 1e9, 1) }
function DebianUp { ((wsl.exe -l -q --running) -join ' ' -replace "`0", '') -match 'Debian' }
function InWsl([string[]]$cmd) { (& wsl.exe -d Debian @cmd 2>&1 | Out-String).Trim() }

$reasons = [System.Collections.Generic.HashSet[string]]::new()
function StopRun($why) {
    if ($reasons.Count -eq 0) { Note "stop the run ($why): $(InWsl @('-e', 'bash', '<home>/mutpause.sh', 'stop', $why))" }
    else { Note "also $why" }
    [void]$reasons.Add($why)
}
function ContinueRun($why) {
    [void]$reasons.Remove($why)
    if ($reasons.Count -eq 0) { Note "continue the run ($why over): $(InWsl @('-e', 'bash', '<home>/mutpause.sh', 'cont', $why))" }
    else { Note "$why over, still stopped for $($reasons -join ', ')" }
}

Note "start, available $(AvailGB) GB, C: free $(DiskGB) GB"
$good = 0
$drops = 0
$lastDropNote = [datetime]::MinValue
$lastDisk = [datetime]::MinValue
$lastReport = [datetime]::MinValue
while (-not (Test-Path $stopFile)) {
    $up = DebianUp
    $avail = AvailGB
    if ($up -and $avail -lt 4) {
        InWsl @('-u', 'root', '-e', 'bash', '<home>/dropcache.sh') | Out-Null
        $drops++
        Start-Sleep -Seconds 10
        $after = AvailGB
        if (((Get-Date) - $lastDropNote).TotalMinutes -ge 5) {
            Note "page cache dropped ($drops times in all), available $avail -> $after GB"
            $lastDropNote = Get-Date
        }
        if ($after -lt 2 -and -not $reasons.Contains('memory')) { StopRun 'memory' }
        $avail = $after
    }
    if ($reasons.Contains('memory')) {
        if ($avail -ge 4) { $good++ } else { $good = 0 }
        if ($good -ge 3) { $good = 0; ContinueRun 'memory' }
    }
    if (((Get-Date) - $lastDisk).TotalMinutes -ge 5) {
        $lastDisk = Get-Date
        $free = DiskGB
        if ($up -and $free -lt 50) { Note "C: free $free GB: $(InWsl @('-e', 'bash', '<home>/clean-old-targets.sh') -replace "`r?`n", '; ')" }
        if ($up -and $free -lt 20 -and -not $reasons.Contains('disk')) { StopRun 'disk' }
        if ($reasons.Contains('disk') -and $free -ge 25) { ContinueRun 'disk' }
    }
    if (((Get-Date) - $lastReport).TotalMinutes -ge 10) {
        $lastReport = Get-Date
        $wslAvail = if ($up) { [math]::Round([double]((InWsl @('-e', 'grep', 'MemAvailable', '/proc/meminfo')) -replace '\D', '') / 1MB, 1) } else { '-' }
        Note "available $avail GB, in WSL $wslAvail GB, C: free $(DiskGB) GB, Debian $(if ($up) { 'up' } else { 'down' }), stopped: $(if ($reasons.Count) { $reasons -join ', ' } else { 'no' })"
    }
    Start-Sleep -Seconds 20
}
if ($reasons.Count -gt 0) { Note "continue the run on exit: $(InWsl @('-e', 'bash', '<home>/mutpause.sh', 'cont', 'watchdog stopped'))" }
Note "stopped"
