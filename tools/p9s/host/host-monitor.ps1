#Requires -Version 7.0
# P9-S host resource monitor (live successor of host-sampler.ps1 and
# gate.ps1). Every 5 s on the monotonic clock it reads the raw Windows
# performance classes (class and property names are not localized; no
# elevation), feeds the policy of hostmon-lib.ps1 and atomically replaces
# the state file that every gated step reads (p9s-action.ps1,
# assert-host-safe.ps1). It writes only -OutDir and the state file
# with its lock, changes nothing on the host, installs no service or
# scheduled task, and ends at -DurationSeconds, when -StopFile appears or on
# Ctrl+C; a clean end publishes status STOPPED. At STOP_REQUIRED or
# ABORT_REQUIRED it alerts (console, events.log, a message box with -Popup)
# and never acts on a VM: shutdown stays with the operator.
# It makes no VirtualBox call of any kind: no COM object, no VBoxManage,
# no VirtualBox lock, no quarantine check. A hung or missing VirtualBox
# cannot delay a sample. The VM states belong to the coordinator, which
# reads them on demand (p9s-vmstate.ps1); the monitor only counts the
# VirtualBox VM processes of the whole host and their working set, from
# the Windows process list.
#   pwsh -NoProfile -File host-monitor.ps1 -OutDir <new folder> [-PagingStop Required|NotRequired] [-Popup]
# The external disk (drive letter, model, a folder on it) comes from the
# local configuration (-ConfigPath, P9S_HOST_CONFIG or p9s-local.json next
# to this script; see p9s-local.example.json) unless given as parameters.
# Without it the external disk cannot be confirmed and every sample is
# UNKNOWN (fail closed).
# Exit codes: 0 ended normally, 1 error, 2 refused arguments, 3 another
# monitor holds the state file.
param(
    [Parameter(Mandatory)][string]$OutDir,
    [string]$StatePath,
    [string]$StopFile,
    [double]$DurationSeconds = 14400,
    [ValidateSet('Monitor', 'Calibrate')][string]$Mode = 'Monitor',
    [ValidateSet('Required', 'NotRequired')][string]$PagingStop = 'Required',
    [string]$ThresholdsPath,
    [string]$ConfigPath,
    [string]$ExternalDrive,
    [string]$ExpectedExternalModel,
    [string]$ExternalProbePath,
    [int]$ProcessSnapshotSeconds = 60,
    [switch]$Popup,
    [string]$TestFixture,
    [int]$TestIntervalMs = 0
)
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'hostmon-lib.ps1')
. (Join-Path $PSScriptRoot 'p9s-config.ps1')
$localCfg = Read-P9SConfig -Path (Get-P9SConfigPath -ConfigPath $ConfigPath -ToolDir $PSScriptRoot)
if ($localCfg.external_disk) {
    if (-not $ExternalDrive) { $ExternalDrive = $localCfg.external_disk.drive_letter }
    if (-not $ExpectedExternalModel) { $ExpectedExternalModel = $localCfg.external_disk.model }
    if (-not $ExternalProbePath) { $ExternalProbePath = $localCfg.external_disk.probe_path }
}
$inv = [Globalization.CultureInfo]::InvariantCulture
if (-not $StatePath) { $StatePath = Get-DefaultStatePath }
if (-not $StopFile) { $StopFile = Join-Path $OutDir 'STOP' }
if (-not $ThresholdsPath) { $ThresholdsPath = Join-Path $PSScriptRoot 'paging-thresholds.json' }
if ($TestIntervalMs -gt 0 -and -not $TestFixture) { 'refused: -TestIntervalMs is for fixture mode only'; exit 2 }
if (Test-Path -LiteralPath (Join-Path $OutDir 'samples.csv')) { "refused: $OutDir already holds a run"; exit 2 }
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
New-Item -ItemType Directory -Force -Path (Split-Path -Parent $StatePath) | Out-Null
$lockPath = "$StatePath.lock"
try { $lock = [IO.FileStream]::new($lockPath, [IO.FileMode]::OpenOrCreate, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None) }
catch { "refused: another monitor holds $lockPath"; exit 3 }

function F($v, [int]$d = 1) { if (-not (Test-Finite $v)) { '' } else { ([double]$v).ToString("F$d", $inv) } }
function Add-Line([string]$Path, [string]$Text) { [IO.File]::AppendAllText($Path, $Text + "`r`n", [Text.Encoding]::ASCII) }
function Now-Utc { [DateTime]::UtcNow.ToString('yyyy-MM-ddTHH:mm:ss.fffZ') }
function Now-Mono { [Diagnostics.Stopwatch]::GetTimestamp() / [double][Diagnostics.Stopwatch]::Frequency }

# Disk numbers of the system drive, the external drive and every page file,
# from Windows metadata (no assumption that the page file is on C:).
function Resolve-HostLayout {
    $errors = [System.Collections.Generic.List[string]]::new()
    $sys = $null; $ext = $null; $model = $null; $bus = $null; $pf = [System.Collections.Generic.List[int]]::new(); $pfNames = @()
    try { $sys = [int](Get-Partition -DriveLetter $env:SystemDrive.TrimEnd(':') -ErrorAction Stop).DiskNumber } catch { $errors.Add("system disk: $($_.Exception.Message)") }
    if (-not $ExternalDrive -or -not $ExpectedExternalModel) { $errors.Add('external disk: not configured') }
    else {
        try {
            $ext = [int](Get-Partition -DriveLetter $ExternalDrive -ErrorAction Stop).DiskNumber
            $dk = Get-Disk -Number $ext -ErrorAction Stop; $model = [string]$dk.FriendlyName; $bus = [string]$dk.BusType
        } catch { $errors.Add("external disk: $($_.Exception.Message)") }
    }
    try {
        foreach ($u in @(Get-CimInstance Win32_PageFileUsage -ErrorAction Stop)) {
            $pfNames += [string]$u.Name
            $n = [int](Get-Partition -DriveLetter ([string]$u.Name).Substring(0, 1) -ErrorAction Stop).DiskNumber
            if (-not $pf.Contains($n)) { $pf.Add($n) }
        }
    } catch { $errors.Add("page file: $($_.Exception.Message)"); $pf.Clear() }
    [pscustomobject]@{
        system_disk = $sys; external_disk = $ext; external_model = $model; external_bus = $bus
        external_identity_ok = ($null -ne $model -and [bool]$ExpectedExternalModel -and $model -eq $ExpectedExternalModel)
        pagefile_disks = $(if ($pf.Count) { @($pf) } else { $null }); pagefile_names = $pfNames; errors = @($errors)
    }
}
$storageProviders = 'disk|Ntfs|partmgr|volmgr|storahci|stornvme|USBSTOR|UASPStor|USBHUB|Storport|StorPort|volsnap|Kernel-PnP|Kernel-Power|WHEA'
# Image names of VirtualBox VM processes (any VM of any project on this
# host): counted from the Windows process list, never asked of VirtualBox.
$vmProcessNames = 'VirtualBoxVM', 'VBoxHeadless'
function Read-HostRaw($Layout, $EventState) {
    $r = [ordered]@{ ok = $true; error = $null; mono = (Now-Mono); utc = (Now-Utc); mem = $null; cpu = $null; pf_usage = @(); pf_perf = @(); disks = @()
        external_present = $null; external_identity_ok = $Layout.external_identity_ok; eventlog_ok = $null; storage_events_new = 0; storage_event_ids = @(); procs = $null }
    try {
        $r.mem = Get-CimInstance Win32_PerfRawData_PerfOS_Memory -ErrorAction Stop
        $r.cpu = Get-CimInstance Win32_PerfRawData_PerfOS_Processor -Filter "Name='_Total'" -ErrorAction Stop
        $r.pf_perf = @(Get-CimInstance Win32_PerfRawData_PerfOS_PagingFile -ErrorAction Stop)
        $r.pf_usage = @(Get-CimInstance Win32_PageFileUsage -ErrorAction Stop)
        $r.disks = @(Get-CimInstance Win32_PerfRawData_PerfDisk_PhysicalDisk -ErrorAction Stop)
    } catch { $r.ok = $false; $r.error = $_.Exception.Message }
    $r.external_present = if ($ExternalProbePath) { Test-Path -LiteralPath $ExternalProbePath } else { $null }
    $evs = $null
    try { $evs = @(Get-WinEvent -FilterHashtable @{ LogName = 'System'; Level = 1, 2, 3; StartTime = $EventState.since } -ErrorAction Stop) }
    catch { if ($_.FullyQualifiedErrorId -like 'NoMatchingEventsFound*') { $evs = @() } else { $r.eventlog_ok = $false } }
    if ($null -ne $evs) {
        $r.eventlog_ok = $true
        $ids = @()
        foreach ($e in $evs) { if ($e.ProviderName -match $storageProviders -and $EventState.seen.Add([long]$e.RecordId)) { $ids += "$($e.ProviderName)/$($e.Id)/L$($e.Level)" } }
        $r.storage_events_new = $ids.Count; $r.storage_event_ids = $ids
        $EventState.since = (Get-Date).AddSeconds(-30)
    }
    $vp = @(Get-Process -Name $vmProcessNames -ErrorAction SilentlyContinue)
    $w = Get-Process vmmemWSL -ErrorAction SilentlyContinue | Select-Object -First 1
    $r.procs = [pscustomobject]@{
        vbox_vm_procs = $vp.Count; vbox_vm_ws_mb = $(if ($vp.Count) { [double](($vp | Measure-Object -Property WorkingSet64 -Sum).Sum) / 1MB } else { 0.0 })
        wsl_ws_mb = $(if ($w) { $w.WorkingSet64 / 1MB } else { $null })
    }
    [pscustomobject]$r
}
# Process names only (no command line, path, window title or user).
function Get-ProcSnapshot {
    $h = @{}
    foreach ($p in Get-CimInstance Win32_PerfRawData_PerfProc_Process -ErrorAction Stop) {
        if ($p.Name -in '_Total', 'Idle') { continue }
        $h[[int]$p.IDProcess] = [pscustomobject]@{ name = ([string]$p.Name -replace '#\d+$', ''); faults = [double]$p.PageFaultsPersec; io = [double]$p.IOReadBytesPersec; t = [double]$p.Timestamp_PerfTime; f = [double]$p.Frequency_PerfTime; ws = [double]$p.WorkingSet }
    }
    $h
}
function Write-ProcRows($Prev, $Cur, [string]$Path) {
    $rows = foreach ($k in $Cur.Keys) {
        $a = $Prev[$k]; $b = $Cur[$k]
        if (-not $a -or $a.name -ne $b.name -or $b.t -le $a.t) { continue }
        $dt = ($b.t - $a.t) / $b.f
        $df = $b.faults - $a.faults; if ($df -lt 0) { $df += 4294967296.0 }
        $di = $b.io - $a.io
        [pscustomobject]@{ name = $b.name; faults = $df / $dt; io = $(if ($di -ge 0) { $di / $dt / 1MB } else { $null }); ws = $b.ws / 1MB }
    }
    $u = Now-Utc
    $i = 0; foreach ($x in @($rows | Sort-Object faults -Descending | Select-Object -First 5)) { $i++; Add-Line $Path ('{0},page_faults_per_s,{1},{2},{3},{4}' -f $u, $i, $x.name, (F $x.faults 0), (F $x.ws 0)) }
    $i = 0; foreach ($x in @($rows | Where-Object { $null -ne $_.io } | Sort-Object io -Descending | Select-Object -First 5)) { $i++; Add-Line $Path ('{0},io_read_mb_per_s,{1},{2},{3},{4}' -f $u, $i, $x.name, (F $x.io 2), (F $x.ws 0)) }
}

$csvPath = Join-Path $OutDir 'samples.csv'; $eventsPath = Join-Path $OutDir 'events.log'; $procPath = Join-Path $OutDir 'processes.csv'
$freq = [Diagnostics.Stopwatch]::Frequency
$startUtc = [DateTime]::UtcNow
$runId = 'hm-' + $startUtc.ToString('yyyyMMddTHHmmssfffZ') + '-' + $PID
$th = Get-PagingThresholds -Path $ThresholdsPath
$cfg = New-HostMonConfig -PagingStop $PagingStop -Mode $Mode -Thresholds $th
$state = New-MonitorState -RunId $runId -Config $cfg
$me = Get-Process -Id $PID
$info = [ordered]@{
    pid = $PID; process_start_utc = $me.StartTime.ToUniversalTime().ToString('o'); process_start_ticks = $me.StartTime.ToUniversalTime().Ticks
    mode = $Mode; paging_stop = $PagingStop; thresholds_sha256 = $th.sha256; thresholds_approved = $th.approved; test = [bool]$TestFixture
    kit = [ordered]@{ lib_sha256 = (Get-FileSha256 (Join-Path $PSScriptRoot 'hostmon-lib.ps1')); monitor_sha256 = (Get-FileSha256 $PSCommandPath) }
}
$interval = if ($TestIntervalMs -gt 0) { $TestIntervalMs / 1000.0 } else { $cfg.interval_s }
$layout = $null
if (-not $TestFixture) { $layout = Resolve-HostLayout }
[ordered]@{
    run_id = $runId; start_utc = $startUtc.ToString('o'); pid = $PID; pwsh = [string]$PSVersionTable.PSVersion
    os_build = [string][Environment]::OSVersion.Version; mode = $Mode; paging_stop = $PagingStop
    interval_s = $interval; duration_s = $DurationSeconds; state_path = $StatePath; stop_file = $StopFile
    limits = Get-LimitsDoc $cfg
    thresholds = [ordered]@{ path = $ThresholdsPath; sha256 = $th.sha256; approved = $th.approved; valid = $th.valid; errors = @($th.errors); source = $th.source }
    layout = $layout; external_probe = $ExternalProbePath; expected_external_model = $ExpectedExternalModel
    config = [ordered]@{ path = $localCfg.path; present = $localCfg.present; ok = $localCfg.ok; errors = @($localCfg.errors) }
    samples_schema = 'p9s-hostmon-samples/2'; virtualbox = 'none: the monitor makes no VirtualBox call and takes no VirtualBox lock; the coordinator observes VM states'
    process_snapshot_s = $ProcessSnapshotSeconds; popup = [bool]$Popup; test_fixture = $TestFixture; monitor = $info
} | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $OutDir 'run.json') -Encoding utf8
# samples.csv layout p9s-hostmon-samples/2: version 1 (toolkit 83c9ef8 and
# earlier) had the VM states d12_state, u24_state, d12_ws_mb, u24_ws_mb in
# place of vbox_vm_procs, vbox_vm_ws_mb
Add-Line $csvPath 'utc,mono_s,seq,status,reasons,commit_pct,committed_mb,commit_limit_mb,gate_s,high95_s,gap_s,avail_mb,modified_mb,pages_per_s,pages_in_per_s,pages_out_per_s,page_reads_per_s,page_writes_per_s,page_faults_per_s,transition_faults_per_s,cache_faults_per_s,demand_zero_faults_per_s,pagefile_used_mb,pagefile_alloc_mb,pagefile_pct,sys_read_ms,sys_write_ms,sys_read_mbps,sys_write_mbps,sys_queue,sys_avg_queue,ext_read_ms,ext_write_ms,ext_read_mbps,ext_write_mbps,ext_queue,ext_avg_queue,cpu_pct,paging,p30_covered,p120_covered,ext_present,storage_events,vbox_vm_procs,vbox_vm_ws_mb,wsl_ws_mb,rate_issues,sample_ms'
if (-not $TestFixture) { Add-Line $procPath 'utc,metric,rank,name,rate,ws_mb' }
function Publish($doc) {
    $json = ConvertTo-StateJson -Doc $doc -Monitor $info -WrittenQpc ([Diagnostics.Stopwatch]::GetTimestamp()) -QpcFrequency $freq -WrittenUtc (Now-Utc)
    try { Write-StateAtomic -Path $StatePath -Text $json; return $true } catch { Add-Line $eventsPath "$(Now-Utc) STATE_WRITE_FAILED $($_.Exception.Message)"; return $false }
}
function Send-Alert([string]$Status, [string]$Why) {
    $text = "P9-S host monitor: $Status ($Why). No further test steps; orderly shutdown by the operator."
    Add-Line $eventsPath "$(Now-Utc) ALERT $Status $Why"
    Write-Host "*** $text ***" -ForegroundColor Red
    try { [Console]::Beep(1000, 400) } catch {}
    if ($Popup) {
        try { Start-Process -FilePath (Join-Path $env:WINDIR 'System32\msg.exe') -ArgumentList @('*', '/TIME:0', $text) -WindowStyle Hidden | Out-Null }
        catch { Add-Line $eventsPath "$(Now-Utc) ALERT_POPUP_FAILED $($_.Exception.Message)" }
    }
}

$null = Publish (New-StatusOnlyDoc -State $state -Status 'INITIALIZING' -Reasons @('MONITOR_STARTING') -Mono (Now-Mono) -Utc (Now-Utc))
Add-Line $eventsPath "$(Now-Utc) START run $runId pid $PID mode $Mode paging-stop $PagingStop interval $interval s thresholds approved $($th.approved) state $StatePath"
if ($layout) { Add-Line $eventsPath "$(Now-Utc) LAYOUT system disk $($layout.system_disk); external disk $($layout.external_disk) '$($layout.external_model)' $($layout.external_bus) identity ok $($layout.external_identity_ok); page file disks $($layout.pagefile_disks -join ' ') ($($layout.pagefile_names -join ' ')); errors $($layout.errors.Count)" }
$fixtureQ = $null; $fxLast = $null
if ($TestFixture) { $fixtureQ = [System.Collections.Generic.Queue[object]]::new([object[]]@(ConvertFrom-FixtureSamples $TestFixture)) }
$eventState = [pscustomobject]@{ since = $startUtc.ToLocalTime(); seen = [System.Collections.Generic.HashSet[long]]::new() }
$prevRaw = $null; $seq = 0; $lastStatus = 'INITIALIZING'; $counts = @{}; $writeFails = 0; $late = 0
$procPrev = $null; $procDue = 0.0
$intervalTicks = [long]($interval * $freq)
$loopStart = [Diagnostics.Stopwatch]::GetTimestamp(); $k = 0
$endReason = 'duration reached'; $exitCode = 0
try {
    while ($true) {
        if (Test-Path -LiteralPath $StopFile) { $endReason = 'stop file'; break }
        if ((([Diagnostics.Stopwatch]::GetTimestamp() - $loopStart) / $freq) -ge $DurationSeconds) { $endReason = 'duration reached'; break }
        $t0 = [Diagnostics.Stopwatch]::GetTimestamp()
        if ($TestFixture) {
            if ($fixtureQ.Count) { $s = $fixtureQ.Dequeue() } else { $s = $fxLast.PSObject.Copy(); $s.mono = [double]$fxLast.mono + 5.0 }
            $fxLast = $s
        } else {
            $raw = Read-HostRaw $layout $eventState
            $s = ConvertTo-DerivedSample -Prev $prevRaw -Cur $raw -Layout $layout
            $prevRaw = $raw
        }
        $seq++; $s.seq = $seq
        $doc = Step-Monitor -State $state -Sample $s
        if (-not (Publish $doc)) { $writeFails++ }
        $ms = ([Diagnostics.Stopwatch]::GetTimestamp() - $t0) * 1000.0 / $freq
        $counts[$doc.status] = 1 + [int]$counts[$doc.status]
        $m = $doc.memory; $sd = $doc.disks.system; $xd = $doc.disks.external; $w = $doc.paging.windows
        $px = Get-Prop $s 'procs'
        Add-Line $csvPath (@(
            $s.utc, (F $s.mono 3), $seq, $doc.status, ($doc.reasons -join ';'),
            (F $doc.commit.pct 2), (F ($(if (Test-Finite $doc.commit.committed_bytes) { $doc.commit.committed_bytes / 1MB } else { $null })) 0), (F ($(if (Test-Finite $doc.commit.limit_bytes) { $doc.commit.limit_bytes / 1MB } else { $null })) 0),
            (F $doc.gate.qualifying_seconds 3), (F $doc.commit.seconds_at_or_above_95 3), (F $doc.sampling.last_gap_s 3),
            (F $m.avail_mb 0), (F $m.modified_list_mb 0), (F $m.pages 1), (F $m.pages_in 1), (F $m.pages_out 1), (F $m.page_reads 1), (F $m.page_writes 1),
            (F $m.page_faults 0), (F $m.transition_faults 0), (F $m.cache_faults 0), (F $m.demand_zero_faults 0),
            (F $m.pagefile_used_mb 0), (F $m.pagefile_alloc_mb 0), (F $m.pagefile_pct 3),
            (F $sd.read_ms 3), (F $sd.write_ms 3), (F $sd.read_mbps 3), (F $sd.write_mbps 3), $sd.queue, (F $sd.avg_queue 3),
            (F $xd.read_ms 3), (F $xd.write_ms 3), (F $xd.read_mbps 3), (F $xd.write_mbps 3), $xd.queue, (F $xd.avg_queue 3),
            (F $doc.cpu_pct 1), $doc.paging.assessment, $w.'30'.covered, $w.'120'.covered,
            $doc.external_disk.present, $doc.storage_events.count_since_start,
            (Get-Prop $px 'vbox_vm_procs'), (F (Get-Prop $px 'vbox_vm_ws_mb') 0), (F (Get-Prop $px 'wsl_ws_mb') 0),
            ($m.rate_issues -join ';'), (F $ms 0)
        ) -join ',')
        if ($doc.status -ne $lastStatus) {
            Add-Line $eventsPath "$(Now-Utc) STATUS $lastStatus -> $($doc.status) at mono $(F $s.mono 3) seq $seq; $($doc.reasons -join ' ')"
            if ($doc.status -in 'STOP_REQUIRED', 'ABORT_REQUIRED') { Send-Alert $doc.status ($doc.reasons -join ', ') }
            elseif ($doc.status -in 'WARNING', 'UNKNOWN') { Write-Host "P9-S host monitor: $($doc.status) ($($doc.reasons -join ', '))" -ForegroundColor Yellow }
            $lastStatus = $doc.status
        }
        if (-not $TestFixture -and $ProcessSnapshotSeconds -gt 0 -and $s.mono -ge $procDue) {
            try { $snap = Get-ProcSnapshot; if ($procPrev) { Write-ProcRows $procPrev $snap $procPath }; $procPrev = $snap } catch { Add-Line $eventsPath "$(Now-Utc) PROCESS_SNAPSHOT_FAILED $($_.Exception.Message)" }
            $procDue = $s.mono + $ProcessSnapshotSeconds
        }
        $k++
        $due = $loopStart + $k * $intervalTicks
        $now = [Diagnostics.Stopwatch]::GetTimestamp()
        if ($now -ge $due) {
            $late++
            Add-Line $eventsPath "$(Now-Utc) LATE sample $seq took $(F $ms 0) ms; next slot skipped"
            $k = [long][math]::Floor(($now - $loopStart) / $intervalTicks) + 1; $due = $loopStart + $k * $intervalTicks
        }
        while ($true) {
            $left = $due - [Diagnostics.Stopwatch]::GetTimestamp()
            if ($left -le 0 -or (Test-Path -LiteralPath $StopFile)) { break }
            Start-Sleep -Milliseconds ([int][math]::Max(1, [math]::Min(500, [math]::Ceiling($left * 1000.0 / $freq))))
        }
    }
} catch {
    $endReason = "error: $($_.Exception.Message) (line $($_.InvocationInfo.ScriptLineNumber))"; $exitCode = 1
} finally {
    $null = Publish (New-StatusOnlyDoc -State $state -Status 'STOPPED' -Reasons @('MONITOR_STOPPED') -Mono (Now-Mono) -Utc (Now-Utc))
    Add-Line $eventsPath "$(Now-Utc) END $endReason; samples $seq, invalid $($state.invalid_samples), gaps over limit $($state.gaps_over_limit), max gap $(F $state.max_gap_seen 3) s, late $late, state write failures $writeFails"
    [ordered]@{
        run_id = $runId; end_utc = (Now-Utc); end_reason = $endReason; samples = $seq; invalid_samples = $state.invalid_samples
        gaps_over_limit = $state.gaps_over_limit; max_gap_s = $state.max_gap_seen; late_samples = $late; state_write_failures = $writeFails
        status_counts = $counts; last_status = $lastStatus; latched = $state.latched; storage_events = $state.storage_events
    } | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath (Join-Path $OutDir 'run-end.json') -Encoding utf8
    $lock.Dispose()
    try { [IO.File]::Delete($lockPath) } catch {}
}
exit $exitCode
