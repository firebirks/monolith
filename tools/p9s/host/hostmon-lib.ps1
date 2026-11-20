#Requires -Version 7.0
# P9-S host resource monitor: shared functions. Dot-sourced by
# host-monitor.ps1, assert-host-safe.ps1, the gated action scripts and the
# tests; loading it has no side effect. Four parts:
#   1. counter arithmetic: raw performance-class readings -> one sample
#   2. policy: samples on a monotonic clock -> safety status (pure)
#   3. state file: atomic write, shared read
#   4. reader: the checks every test step applies before it acts
# Approved limits (owner, 2026-11-19): 5 s sampling, gaps above 12 s break
# coverage, 300 continuous s below 90 % commit before a test, 95 % for 30
# continuous s = STOP, 97 % at any valid sample = ABORT, paging windows 30
# and 120 s. Paging numbers are not approved: they live in
# paging-thresholds.json and only become a stop rule when that file says
# approved with every value set.

# ---------------------------------------------------------------- helpers
function Test-Finite($v) {
    if ($null -eq $v -or $v -is [bool] -or $v -is [string]) { return $false }
    try { $d = [double]$v } catch { return $false }
    -not ([double]::IsNaN($d) -or [double]::IsInfinity($d))
}
function Get-Prop($o, [string]$Name) {
    if ($null -eq $o) { return $null }
    if ($o -is [System.Collections.IDictionary]) { if ($o.Contains($Name)) { return $o[$Name] } else { return $null } }
    $p = $o.PSObject.Properties[$Name]
    if ($p) { $p.Value } else { $null }
}
function Get-DiskStat($Sample, $n) {
    if ($null -eq $n) { return $null }
    $d = Get-Prop $Sample 'disks'
    if ($null -eq $d) { return $null }
    if ($d -is [System.Collections.IDictionary]) {
        if ($d.Contains([int]$n)) { return $d[[int]$n] }
        if ($d.Contains([string]$n)) { return $d[[string]$n] }
        return $null
    }
    Get-Prop $d ([string]$n)
}
function Join-Or($a, $b) {
    if (($a -is [bool] -and $a) -or ($b -is [bool] -and $b)) { return $true }
    if ($null -eq $a -or $null -eq $b) { return $null }
    $false
}
function Join-And($a, $b) {
    if (($a -is [bool] -and -not $a) -or ($b -is [bool] -and -not $b)) { return $false }
    if (($a -is [bool] -and $a) -and ($b -is [bool] -and $b)) { return $true }
    $null
}
function Get-DefaultStatePath { Join-Path ([IO.Path]::GetTempPath()) 'p9s-hostmon\state.json' }
function Get-FileSha256([string]$Path) { (Get-FileHash -Algorithm SHA256 -LiteralPath $Path).Hash.ToLowerInvariant() }

# ------------------------------------------------- 1. counter arithmetic
# Rate of a cumulative counter between two raw readings, using the
# counter's own timestamps. 32-bit counters may wrap once; a decrease that
# implies more than -MaxPerSecond is a reset or discontinuity (no value).
function Get-CounterRate {
    param($Prev, $Cur, [Parameter(Mandatory)][string]$Name, [ValidateSet(32, 64)][int]$Bits = 32,
          [double]$MaxPerSecond = 1e7, [string]$TimeProp = 'Timestamp_PerfTime', [string]$FreqProp = 'Frequency_PerfTime')
    if ($null -eq $Prev -or $null -eq $Cur) { return [pscustomobject]@{ value = $null; issue = 'no-previous' } }
    $a = Get-Prop $Prev $Name; $b = Get-Prop $Cur $Name
    if ($null -eq $a -or $null -eq $b) { return [pscustomobject]@{ value = $null; issue = 'absent' } }
    $t0 = Get-Prop $Prev $TimeProp; $t1 = Get-Prop $Cur $TimeProp
    $f = if ($FreqProp) { Get-Prop $Cur $FreqProp } else { 1 }
    if ($null -eq $t0 -or $null -eq $t1 -or $null -eq $f -or [double]$f -le 0 -or [double]$t1 -le [double]$t0) { return [pscustomobject]@{ value = $null; issue = 'time' } }
    $dt = ([double]$t1 - [double]$t0) / [double]$f
    $d = [double]$b - [double]$a
    if ($d -lt 0) { if ($Bits -eq 32) { $d += 4294967296.0 } else { return [pscustomobject]@{ value = $null; issue = 'discontinuity' } } }
    $rate = $d / $dt
    if ($rate -gt $MaxPerSecond) { return [pscustomobject]@{ value = $null; issue = 'discontinuity' } }
    [pscustomobject]@{ value = $rate; issue = $null }
}
# PERF_AVERAGE_TIMER (32-bit ticks of Frequency_PerfTime over a 32-bit
# operation count): average milliseconds per operation; null when no
# operation happened in the interval.
function Get-AvgTimerMs($Prev, $Cur, [string]$Name) {
    if ($null -eq $Prev -or $null -eq $Cur) { return [pscustomobject]@{ value = $null; issue = 'no-previous' } }
    $n0 = Get-Prop $Prev $Name; $n1 = Get-Prop $Cur $Name
    $b0 = Get-Prop $Prev "${Name}_Base"; $b1 = Get-Prop $Cur "${Name}_Base"
    $f = Get-Prop $Cur 'Frequency_PerfTime'
    if ($null -eq $n0 -or $null -eq $n1 -or $null -eq $b0 -or $null -eq $b1 -or $null -eq $f -or [double]$f -le 0) { return [pscustomobject]@{ value = $null; issue = 'absent' } }
    $dn = [double]$n1 - [double]$n0; if ($dn -lt 0) { $dn += 4294967296.0 }
    $db = [double]$b1 - [double]$b0; if ($db -lt 0) { $db += 4294967296.0 }
    if ($db -eq 0) { return [pscustomobject]@{ value = $null; issue = $null } }
    if ($db -gt 1e8) { return [pscustomobject]@{ value = $null; issue = 'discontinuity' } }
    $ms = 1000.0 * ($dn / [double]$f) / $db
    if ($ms -gt 600000) { return [pscustomobject]@{ value = $null; issue = 'discontinuity' } }
    [pscustomobject]@{ value = $ms; issue = $null }
}
# One sample from two raw readings (see Read-HostRaw in host-monitor.ps1).
# $Layout: system_disk, external_disk, pagefile_disks (disk numbers).
function ConvertTo-DerivedSample {
    param($Prev, [Parameter(Mandatory)]$Cur, [Parameter(Mandatory)]$Layout)
    $issues = [System.Collections.Generic.List[string]]::new()
    $ok = [bool]$Cur.ok
    $err = $Cur.error
    $m1 = if ($ok) { $Cur.mem } else { $null }
    $m0 = if ($Prev -and $Prev.ok) { $Prev.mem } else { $null }
    if ($ok -and $m0 -and $m1) {
        $t0 = Get-Prop $m0 'Timestamp_PerfTime'; $t1 = Get-Prop $m1 'Timestamp_PerfTime'
        if ($null -ne $t0 -and $null -ne $t1 -and [double]$t1 -le [double]$t0) { $ok = $false; $err = 'memory counter timestamp did not advance' }
    }
    $rate = {
        param([string]$name, [double]$max)
        if (-not $ok -or $null -eq $m0) { return $null }
        $r = Get-CounterRate -Prev $m0 -Cur $m1 -Name $name -Bits 32 -MaxPerSecond $max
        if ($r.issue) { $issues.Add("${name}:$($r.issue)") }
        $r.value
    }
    $gauge = { param([string]$name) if ($ok) { Get-Prop $m1 $name } else { $null } }
    $disks = @{}
    if ($ok) {
        $prevDisks = @{}
        if ($Prev -and $Prev.ok) { foreach ($x in @($Prev.disks)) { if ($x) { $prevDisks[[string]$x.Name] = $x } } }
        foreach ($x in @($Cur.disks)) {
            if (-not $x -or [string]$x.Name -notmatch '^(\d+)( |$)') { continue }
            $n = [int]$Matches[1]
            $p = $prevDisks[[string]$x.Name]
            $rd = Get-AvgTimerMs $p $x 'AvgDisksecPerRead'; $wr = Get-AvgTimerMs $p $x 'AvgDisksecPerWrite'
            $rb = Get-CounterRate -Prev $p -Cur $x -Name 'DiskReadBytesPersec' -Bits 64 -MaxPerSecond 1e11
            $wb = Get-CounterRate -Prev $p -Cur $x -Name 'DiskWriteBytesPersec' -Bits 64 -MaxPerSecond 1e11
            $aq = Get-CounterRate -Prev $p -Cur $x -Name 'AvgDiskQueueLength' -Bits 64 -MaxPerSecond 1e6 -TimeProp 'Timestamp_Sys100NS' -FreqProp ''
            foreach ($q in @(@('read_ms', $rd), @('write_ms', $wr), @('read_bytes', $rb), @('write_bytes', $wb), @('avg_queue', $aq))) {
                if ($q[1].issue -and $q[1].issue -ne 'no-previous') { $issues.Add("disk$n.$($q[0]):$($q[1].issue)") }
            }
            $disks[$n] = [pscustomobject]@{
                read_ms = $rd.value; write_ms = $wr.value
                read_mbps = $(if ($null -ne $rb.value) { $rb.value / 1MB } else { $null })
                write_mbps = $(if ($null -ne $wb.value) { $wb.value / 1MB } else { $null })
                queue = Get-Prop $x 'CurrentDiskQueueLength'; avg_queue = $aq.value
            }
        }
    }
    $pfUsed = $null; $pfAlloc = $null; $pfPct = $null
    if ($ok) {
        $u = @($Cur.pf_usage | Where-Object { $_ })
        if ($u.Count) { $pfUsed = [double](($u | Measure-Object -Property CurrentUsage -Sum).Sum); $pfAlloc = [double](($u | Measure-Object -Property AllocatedBaseSize -Sum).Sum) }
        $tot = @($Cur.pf_perf | Where-Object { $_ -and $_.Name -eq '_Total' }) | Select-Object -First 1
        if ($tot -and (Test-Finite $tot.PercentUsage_Base) -and [double]$tot.PercentUsage_Base -gt 0) { $pfPct = 100.0 * [double]$tot.PercentUsage / [double]$tot.PercentUsage_Base }
    }
    $cpu = $null
    if ($ok -and $Prev -and $Prev.ok -and $Prev.cpu -and $Cur.cpu) {
        $dT = [double]$Cur.cpu.Timestamp_Sys100NS - [double]$Prev.cpu.Timestamp_Sys100NS
        if ($dT -gt 0) { $cpu = 100.0 * (1.0 - ([double]$Cur.cpu.PercentProcessorTime - [double]$Prev.cpu.PercentProcessorTime) / $dT) }
    }
    $mod = & $gauge 'ModifiedPageListBytes'
    [pscustomobject]@{
        seq = 0; mono = [double]$Cur.mono; utc = $Cur.utc; read_ok = $ok; read_error = $err
        committed_bytes = & $gauge 'CommittedBytes'; commit_limit_bytes = & $gauge 'CommitLimit'
        avail_mb = & $gauge 'AvailableMBytes'; modified_list_mb = $(if ($null -ne $mod) { [double]$mod / 1MB } else { $null })
        pages = & $rate 'PagesPersec' 1e7; pages_in = & $rate 'PagesInputPersec' 1e7; pages_out = & $rate 'PagesOutputPersec' 1e7
        page_reads = & $rate 'PageReadsPersec' 1e7; page_writes = & $rate 'PageWritesPersec' 1e7
        page_faults = & $rate 'PageFaultsPersec' 1e8; transition_faults = & $rate 'TransitionFaultsPersec' 1e8
        cache_faults = & $rate 'CacheFaultsPersec' 1e8; demand_zero_faults = & $rate 'DemandZeroFaultsPersec' 1e8
        rate_issues = @($issues)
        pagefile_used_mb = $pfUsed; pagefile_alloc_mb = $pfAlloc; pagefile_pct = $pfPct
        pagefile_disks = $Layout.pagefile_disks; system_disk = $Layout.system_disk; external_disk = $Layout.external_disk
        disks = $disks
        external_present = $Cur.external_present; external_identity_ok = $Cur.external_identity_ok
        eventlog_ok = $Cur.eventlog_ok; storage_events_new = [int]$Cur.storage_events_new; storage_event_ids = @($Cur.storage_event_ids)
        cpu_pct = $cpu; procs = $Cur.procs
    }
}
# Fixture samples (tests): JSON array of samples as built by the tests.
function ConvertFrom-FixtureSamples([string]$Path) {
    foreach ($x in (Get-Content -LiteralPath $Path -Raw | ConvertFrom-Json)) {
        $h = [ordered]@{}
        foreach ($p in $x.PSObject.Properties) { $h[$p.Name] = $p.Value }
        $dk = @{}
        if ($x.disks) { foreach ($p in $x.disks.PSObject.Properties) { $dk[[int]$p.Name] = $p.Value } }
        $h.disks = $dk
        [pscustomobject]$h
    }
}

# ------------------------------------------------------------- 2. policy
function New-HostMonConfig {
    param([ValidateSet('Required', 'NotRequired')][string]$PagingStop = 'Required',
          [ValidateSet('Monitor', 'Calibrate')][string]$Mode = 'Monitor',
          $Thresholds)
    [pscustomobject]@{
        interval_s = 5.0; max_gap_s = 12.0; pretest_seconds = 300.0; pretest_below_pct = 90.0
        sustained_pct = 95.0; sustained_seconds = 30.0; critical_pct = 97.0
        paging_windows_s = @(30.0, 120.0); max_state_age_s = 12.0
        paging_stop = $PagingStop; mode = $Mode; thresholds = $Thresholds
    }
}
function Get-PagingThresholdKeys { 'page_out_per_s', 'avail_mb_low', 'avail_decline_mb', 'pagefile_growth_mb', 'pagefile_disk_read_ms', 'page_in_per_s_with_low_memory' }
function ConvertTo-PagingThresholds {
    param([AllowEmptyString()][AllowNull()][string]$Json, [string]$Sha256)
    $errors = [System.Collections.Generic.List[string]]::new()
    $o = $null
    if ([string]::IsNullOrWhiteSpace($Json)) { $errors.Add('THRESHOLDS_MISSING') }
    else { try { $o = $Json | ConvertFrom-Json -ErrorAction Stop } catch { $errors.Add('THRESHOLDS_INVALID_JSON') } }
    if ($o -and $o.schema -ne 'p9s-hostmon-paging/1') { $errors.Add('THRESHOLDS_SCHEMA_UNSUPPORTED') }
    $sets = [ordered]@{}
    foreach ($set in 'suspect', 'confirm') {
        $h = [ordered]@{}
        foreach ($k in Get-PagingThresholdKeys) {
            $v = if ($o -and $o.$set) { Get-Prop $o.$set $k } else { $null }
            if ($null -ne $v) {
                if ((Test-Finite $v) -and [double]$v -ge 0) { $v = [double]$v } else { $errors.Add("THRESHOLD_INVALID:$set.$k"); $v = $null }
            }
            $h[$k] = $v
        }
        $sets[$set] = [pscustomobject]$h
    }
    $approved = [bool]($o -and $o.approved -eq $true)
    if ($approved) {
        $missing = @(foreach ($set in 'suspect', 'confirm') { foreach ($k in Get-PagingThresholdKeys) { if ($null -eq $sets[$set].$k) { "$set.$k" } } })
        if ($missing.Count -or [string]::IsNullOrWhiteSpace([string]$o.approved_by) -or [string]::IsNullOrWhiteSpace([string]$o.approved_utc)) { $errors.Add('APPROVAL_INCOMPLETE') }
    }
    $valid = $errors.Count -eq 0
    [pscustomobject]@{
        approved = ($approved -and $valid); valid = $valid; errors = @($errors)
        source = $(if ($o) { [string]$o.source } else { $null }); approved_by = $(if ($o) { $o.approved_by } else { $null }); approved_utc = $(if ($o) { $o.approved_utc } else { $null })
        suspect = $sets.suspect; confirm = $sets.confirm; sha256 = $Sha256
    }
}
function Get-PagingThresholds([string]$Path) {
    if (-not (Test-Path -LiteralPath $Path)) { return ConvertTo-PagingThresholds -Json '' }
    ConvertTo-PagingThresholds -Json (Get-Content -LiteralPath $Path -Raw) -Sha256 (Get-FileSha256 $Path)
}
function New-MonitorState {
    param([Parameter(Mandatory)][string]$RunId, [Parameter(Mandatory)]$Config)
    [pscustomobject]@{
        run_id = $RunId; config = $Config
        samples = 0; invalid_samples = 0; gaps_over_limit = 0; max_gap_seen = 0.0
        prev_mono = $null; last_valid_mono = $null
        gate_since = $null; gate_resets = 0; last_reset_reason = 'MONITOR_START'
        high_since = $null
        latched = $null
        history = [System.Collections.Generic.List[object]]::new()
        paging_cover_since = $null
        storage_events = 0; storage_ids = [System.Collections.Generic.List[string]]::new()
    }
}
function Get-LimitsDoc($c) {
    [ordered]@{
        interval_s = $c.interval_s; max_gap_s = $c.max_gap_s; pretest_seconds = $c.pretest_seconds; pretest_below_pct = $c.pretest_below_pct
        sustained_pct = $c.sustained_pct; sustained_seconds = $c.sustained_seconds; critical_pct = $c.critical_pct
        paging_windows_s = @($c.paging_windows_s); max_state_age_s = $c.max_state_age_s
    }
}
function Get-PagingPolicy($c) {
    $t = $c.thresholds
    $approved = [bool]($t -and $t.approved -and $t.valid)
    [pscustomobject]@{
        approved = $approved
        calibration = $(if ($approved) { 'APPROVED' } else { 'UNCALIBRATED' })
        stop_rule = $(if ($approved) { 'enforced' } else { 'not_enforced' })
        errors = $(if ($t) { @($t.errors) } else { @('THRESHOLDS_MISSING') })
    }
}
function Get-WindowStats($State, [double]$Now, [double]$W) {
    $cs = $State.paging_cover_since
    $covered = ($null -ne $cs) -and (($Now - $cs) -ge ($W - 1e-9))
    $st = [ordered]@{
        window_s = $W; covered = $covered; samples = 0; complete = $false
        page_in_mean = $null; page_out_mean = $null; page_writes_mean = $null
        avail_min_mb = $null; avail_max_mb = $null; avail_last_mb = $null; avail_decline_mb = $null
        pagefile_growth_mb = $null; pagefile_disk_read_ms_mean = $null; pagefile_disk_write_ms_mean = $null
        external_read_ms_mean = $null; external_write_ms_mean = $null
    }
    if (-not $covered) { return $st }
    $rate = @($State.history | Where-Object { $_.valid -and $_.mono -gt ($Now - $W + 1e-9) })
    $level = @($State.history | Where-Object { $_.mono -ge ($Now - $W - 1e-9) })
    $st.samples = $rate.Count
    if ($rate.Count -eq 0) { return $st }
    $wsum = 0.0; $sumIn = 0.0; $sumOut = 0.0; $sumWr = 0.0; $sumPr = 0.0; $sumPw = 0.0; $sumEr = 0.0; $sumEw = 0.0
    foreach ($e in $rate) {
        $w = [double]$e.dt; $wsum += $w
        $sumIn += $w * $e.pages_in; $sumOut += $w * $e.pages_out
        if (Test-Finite $e.page_writes) { $sumWr += $w * $e.page_writes }
        if (Test-Finite $e.pf_read_ms) { $sumPr += $w * $e.pf_read_ms }
        if (Test-Finite $e.pf_write_ms) { $sumPw += $w * $e.pf_write_ms }
        if (Test-Finite $e.ext_read_ms) { $sumEr += $w * $e.ext_read_ms }
        if (Test-Finite $e.ext_write_ms) { $sumEw += $w * $e.ext_write_ms }
    }
    $st.page_in_mean = $sumIn / $wsum; $st.page_out_mean = $sumOut / $wsum; $st.page_writes_mean = $sumWr / $wsum
    $complete = $true
    foreach ($e in $level) { if (-not (Test-Finite $e.avail) -or -not (Test-Finite $e.pf_used) -or -not $e.pf_disk_present) { $complete = $false } }
    $st.complete = $complete
    $av = @($level | Where-Object { Test-Finite $_.avail } | ForEach-Object { [double]$_.avail })
    if ($av.Count) {
        $st.avail_min_mb = ($av | Measure-Object -Minimum).Minimum; $st.avail_max_mb = ($av | Measure-Object -Maximum).Maximum
        $st.avail_last_mb = $av[-1]; $st.avail_decline_mb = $st.avail_max_mb - $st.avail_last_mb
    }
    $pf = @($level | Where-Object { Test-Finite $_.pf_used } | ForEach-Object { [double]$_.pf_used })
    if ($pf.Count) { $st.pagefile_growth_mb = $pf[-1] - ($pf | Measure-Object -Minimum).Minimum }
    if (@($rate | Where-Object { -not $_.pf_disk_present }).Count -eq 0) { $st.pagefile_disk_read_ms_mean = $sumPr / $wsum; $st.pagefile_disk_write_ms_mean = $sumPw / $wsum }
    $st.external_read_ms_mean = $sumEr / $wsum; $st.external_write_ms_mean = $sumEw / $wsum
    $st
}
# R1 page-out with falling or low available memory; R2 page-file growth with
# a slow page-file disk; R3 page-in with low available memory. Page-in alone
# never fires a rule. Returns $true, $false, $null (cannot evaluate) or
# 'disabled' (a threshold is not set).
function Test-PagingRule([string]$Rule, $T, $St) {
    switch ($Rule) {
        'R1' {
            if ($null -eq $T.page_out_per_s -or ($null -eq $T.avail_mb_low -and $null -eq $T.avail_decline_mb)) { return 'disabled' }
            if ($null -eq $St.page_out_mean) { return $null }
            if ($St.page_out_mean -lt $T.page_out_per_s) { return $false }
            $a = if ($null -eq $T.avail_mb_low) { $false } elseif ($null -eq $St.avail_min_mb) { $null } else { $St.avail_min_mb -le $T.avail_mb_low }
            $b = if ($null -eq $T.avail_decline_mb) { $false } elseif ($null -eq $St.avail_decline_mb) { $null } else { $St.avail_decline_mb -ge $T.avail_decline_mb }
            return (Join-Or $a $b)
        }
        'R2' {
            if ($null -eq $T.pagefile_growth_mb -or $null -eq $T.pagefile_disk_read_ms) { return 'disabled' }
            $a = if ($null -eq $St.pagefile_growth_mb) { $null } else { $St.pagefile_growth_mb -ge $T.pagefile_growth_mb }
            $b = if ($null -eq $St.pagefile_disk_read_ms_mean) { $null } else { $St.pagefile_disk_read_ms_mean -ge $T.pagefile_disk_read_ms }
            return (Join-And $a $b)
        }
        'R3' {
            if ($null -eq $T.avail_mb_low -or $null -eq $T.page_in_per_s_with_low_memory) { return 'disabled' }
            $a = if ($null -eq $St.avail_min_mb) { $null } else { $St.avail_min_mb -le $T.avail_mb_low }
            $b = if ($null -eq $St.page_in_mean) { $null } else { $St.page_in_mean -ge $T.page_in_per_s_with_low_memory }
            return (Join-And $a $b)
        }
    }
    'disabled'
}
function Get-PagingAssessment($State, [double]$Now) {
    $c = $State.config
    $pol = Get-PagingPolicy $c
    $wins = [ordered]@{}
    foreach ($w in $c.paging_windows_s) { $wins[[string][int]$w] = Get-WindowStats $State $Now $w }
    $short = $wins['30']; $long = $wins['120']
    $t = $c.thresholds
    $suspect = [System.Collections.Generic.List[string]]::new()
    $confirm = [System.Collections.Generic.List[string]]::new()
    foreach ($r in 'R1', 'R2', 'R3') {
        if ($t -and $t.valid) {
            foreach ($st in $short, $long) {
                if ($st.covered) { $x = Test-PagingRule $r $t.suspect $st; if ($x -is [bool] -and $x -and -not $suspect.Contains($r)) { $suspect.Add($r) } }
            }
        }
        if ($pol.approved -and $short.covered -and $long.covered) {
            $a = Test-PagingRule $r $t.confirm $long; $b = Test-PagingRule $r $t.confirm $short
            if ($a -is [bool] -and $a -and $b -is [bool] -and $b) { $confirm.Add($r) }
        }
    }
    $complete = $short.covered -and $long.covered -and $short.complete -and $long.complete
    $assessment = if ($confirm.Count) { 'PRESSURE_CONFIRMED' } elseif ($suspect.Count) { 'PRESSURE_SUSPECTED' } elseif (-not $complete) { 'INSUFFICIENT_DATA' } elseif ($pol.approved) { 'NO_PRESSURE_INDICATED' } else { 'UNCALIBRATED' }
    [ordered]@{
        assessment = $assessment; calibration = $pol.calibration; stop_rule = $pol.stop_rule
        stop_required_by_policy = ($c.paging_stop -eq 'Required'); inputs_complete = $complete
        rules_fired = @($(if ($confirm.Count) { $confirm } else { $suspect })); thresholds_errors = @($pol.errors)
        windows = $wins
    }
}
function Set-Latch($State, [string]$Status, [string[]]$Reasons, [double]$Mono, $Utc) {
    $rank = @{ STOP_REQUIRED = 1; ABORT_REQUIRED = 2 }
    $cur = if ($State.latched) { $rank[$State.latched.status] } else { 0 }
    if ($rank[$Status] -gt $cur) { $State.latched = [ordered]@{ status = $Status; since_mono_s = $Mono; since_utc = $Utc; reasons = @($Reasons) } }
}
# Feeds one sample to the policy and returns the status document (without
# the writer's fields: schema, monitor identity, write time).
function Step-Monitor {
    param([Parameter(Mandatory)]$State, [Parameter(Mandatory)]$Sample)
    $c = $State.config
    $now = [double]$Sample.mono
    $unknown = [System.Collections.Generic.List[string]]::new()
    $warn = [System.Collections.Generic.List[string]]::new()
    $stop = [System.Collections.Generic.List[string]]::new()
    $abort = [System.Collections.Generic.List[string]]::new()
    $info = [System.Collections.Generic.List[string]]::new()
    $State.samples++

    # continuity of sampling (monotonic clock)
    $prevMono = $State.prev_mono
    $gap = $null; $gapBreak = $false; $clockBad = $false
    if ($null -ne $prevMono) {
        $gap = $now - $prevMono
        if ($gap -le 0) { $clockBad = $true; $unknown.Add('CLOCK_NOT_MONOTONIC') }
        elseif ($gap -gt $c.max_gap_s) { $gapBreak = $true; $unknown.Add('SAMPLE_GAP'); $State.gaps_over_limit++ }
        if ($gap -gt $State.max_gap_seen) { $State.max_gap_seen = $gap }
    }
    if (-not $clockBad) { $State.prev_mono = $now }

    # the reading itself
    $readOk = [bool]$Sample.read_ok
    if (-not $readOk) { $unknown.Add('SAMPLE_READ_FAILED') }
    $pct = $null; $commitValid = $false
    $cb = $Sample.committed_bytes; $lb = $Sample.commit_limit_bytes
    if ($readOk) {
        if (-not (Test-Finite $lb) -or [double]$lb -le 0) { $unknown.Add('COMMIT_LIMIT_INVALID') }
        elseif (-not (Test-Finite $cb) -or [double]$cb -lt 0) { $unknown.Add('COMMIT_INVALID') }
        else { $pct = 100.0 * [double]$cb / [double]$lb; $commitValid = $true }
    }
    $ev = Get-Prop $Sample 'eventlog_ok'
    if ($null -eq $ev) { $unknown.Add('EVENTLOG_UNCHECKED') } elseif (-not $ev) { $unknown.Add('EVENTLOG_UNREADABLE') }
    $xp = Get-Prop $Sample 'external_present'
    if ($null -eq $xp) { $unknown.Add('EXTERNAL_DISK_UNCHECKED') }
    elseif (-not $xp) { $stop.Add('EXTERNAL_DISK_MISSING') }
    elseif ((Get-Prop $Sample 'external_identity_ok') -ne $true) { $unknown.Add('EXTERNAL_DISK_UNEXPECTED') }
    $newEv = [int](Get-Prop $Sample 'storage_events_new')
    if ($newEv -gt 0) {
        $State.storage_events += $newEv
        foreach ($i in @(Get-Prop $Sample 'storage_event_ids')) { if ($i) { $State.storage_ids.Add([string]$i) } }
        $stop.Add('STORAGE_EVENT')
    }
    $problems = @($unknown | Where-Object { $_ -ne 'SAMPLE_GAP' })
    $readingValid = $commitValid -and $problems.Count -eq 0
    if ($readingValid) { $State.last_valid_mono = $now } else { $State.invalid_samples++ }

    # pre-test gate: continuous valid samples below 90 %
    $resetWhy = $null
    if (-not $readingValid) { $resetWhy = $(if ($problems.Count) { $problems[0] } else { 'INVALID_SAMPLE' }) }
    elseif ($pct -ge $c.pretest_below_pct) { $resetWhy = 'COMMIT_AT_OR_ABOVE_90' }
    if ($resetWhy) {
        if ($null -ne $State.gate_since) { $State.gate_resets++ }
        $State.gate_since = $null; $State.last_reset_reason = $resetWhy
    } elseif ($gapBreak) {
        if ($null -ne $State.gate_since) { $State.gate_resets++ }
        $State.gate_since = $now; $State.last_reset_reason = 'SAMPLE_GAP'
    } elseif ($null -eq $State.gate_since) { $State.gate_since = $now }
    $gateSeconds = if ($null -ne $State.gate_since) { $now - $State.gate_since } else { 0.0 }
    $qualified = $gateSeconds -ge $c.pretest_seconds

    # commit thresholds
    if ($readingValid -and $pct -ge $c.sustained_pct) {
        if ($null -eq $State.high_since -or $gapBreak) { $State.high_since = $now }
    } else { $State.high_since = $null }
    $highSeconds = if ($null -ne $State.high_since) { $now - $State.high_since } else { 0.0 }
    if ($readingValid) {
        if ($pct -ge $c.critical_pct) { $abort.Add('COMMIT_CRITICAL_97') }
        if ($highSeconds -ge $c.sustained_seconds) { $stop.Add('COMMIT_SUSTAINED_95') }
        if ($pct -ge $c.sustained_pct) { $warn.Add('COMMIT_AT_OR_ABOVE_95') }
        if ($pct -ge $c.pretest_below_pct) { $warn.Add('COMMIT_AT_OR_ABOVE_90') }
    }

    # paging history and assessment
    $pageValid = $readOk -and -not $clockBad -and -not $gapBreak -and ($null -ne $prevMono) -and (Test-Finite $Sample.pages_in) -and (Test-Finite $Sample.pages_out)
    if ($pageValid) { if ($null -eq $State.paging_cover_since) { $State.paging_cover_since = $prevMono } }
    else { $State.paging_cover_since = $null }
    $pfd = @(Get-Prop $Sample 'pagefile_disks' | Where-Object { $null -ne $_ })
    $pfPresent = $pfd.Count -gt 0
    $pfRead = $null; $pfWrite = $null
    foreach ($n in $pfd) {
        $ds = Get-DiskStat $Sample $n
        if ($null -eq $ds) { $pfPresent = $false; continue }
        if (Test-Finite $ds.read_ms) { $pfRead = [math]::Max($(if ($null -eq $pfRead) { 0.0 } else { [double]$pfRead }), [double]$ds.read_ms) }
        if (Test-Finite $ds.write_ms) { $pfWrite = [math]::Max($(if ($null -eq $pfWrite) { 0.0 } else { [double]$pfWrite }), [double]$ds.write_ms) }
    }
    if ($pfPresent) { if ($null -eq $pfRead) { $pfRead = 0.0 }; if ($null -eq $pfWrite) { $pfWrite = 0.0 } }
    $ext = Get-DiskStat $Sample (Get-Prop $Sample 'external_disk')
    $State.history.Add([pscustomobject]@{
        mono = $now; dt = $gap; valid = $pageValid
        pages_in = $Sample.pages_in; pages_out = $Sample.pages_out; page_writes = $Sample.page_writes
        avail = $(if ($readOk) { $Sample.avail_mb } else { $null }); pf_used = $(if ($readOk) { $Sample.pagefile_used_mb } else { $null })
        pf_disk_present = $pfPresent; pf_read_ms = $pfRead; pf_write_ms = $pfWrite
        ext_read_ms = $(if ($ext) { $ext.read_ms } else { $null }); ext_write_ms = $(if ($ext) { $ext.write_ms } else { $null })
    })
    while ($State.history.Count -and $State.history[0].mono -lt ($now - 130)) { $State.history.RemoveAt(0) }
    $paging = Get-PagingAssessment $State $now
    switch ($paging.assessment) {
        'PRESSURE_CONFIRMED' { $stop.Add('PAGING_PRESSURE_CONFIRMED') }
        'PRESSURE_SUSPECTED' { $warn.Add('PAGING_PRESSURE_SUSPECTED') }
        'INSUFFICIENT_DATA' { $info.Add('PAGING_INSUFFICIENT_DATA') }
    }

    # status
    if ($abort.Count) { Set-Latch $State 'ABORT_REQUIRED' (@($abort) + @($stop)) $now $Sample.utc }
    elseif ($stop.Count) { Set-Latch $State 'STOP_REQUIRED' @($stop) $now $Sample.utc }
    $required = $c.paging_stop -eq 'Required'
    $status = if ($State.latched) { $State.latched.status }
        elseif ($unknown.Count) { 'UNKNOWN' }
        elseif ($warn.Count) { 'WARNING' }
        elseif (-not $qualified) { 'QUALIFYING' }
        elseif ($required -and $paging.assessment -eq 'INSUFFICIENT_DATA') { 'UNKNOWN' }
        elseif ($required -and $paging.stop_rule -ne 'enforced') { $info.Add('PAGING_STOP_UNCALIBRATED'); 'UNCALIBRATED' }
        elseif ($c.mode -eq 'Calibrate') { 'CALIBRATING' }
        else { 'READY' }
    $reasons = [System.Collections.Generic.List[string]]::new()
    $latchedReasons = if ($State.latched) { @($State.latched.reasons) } else { @() }
    foreach ($r in @($abort) + @($stop) + @($unknown) + @($warn) + @($info) + $latchedReasons) { if ($r -and -not $reasons.Contains($r)) { $reasons.Add($r) } }
    $sys = Get-DiskStat $Sample (Get-Prop $Sample 'system_disk')
    [ordered]@{
        run_id = $State.run_id; seq = $Sample.seq; sample_utc = $Sample.utc; sample_mono_s = $now
        status = $status; reasons = @($reasons)
        latched = $State.latched
        last_valid_sample_age_s = $(if ($null -ne $State.last_valid_mono) { $now - $State.last_valid_mono } else { $null })
        commit = [ordered]@{ valid = $commitValid; pct = $pct; committed_bytes = $cb; limit_bytes = $lb; seconds_at_or_above_95 = $highSeconds }
        limits = Get-LimitsDoc $c
        gate = [ordered]@{ qualifying_seconds = $gateSeconds; required_seconds = $c.pretest_seconds; qualified = $qualified; since_mono_s = $State.gate_since; resets = $State.gate_resets; last_reset_reason = $State.last_reset_reason }
        sampling = [ordered]@{ interval_s = $c.interval_s; max_gap_s = $c.max_gap_s; last_gap_s = $gap; max_gap_seen_s = $State.max_gap_seen; samples = $State.samples; invalid_samples = $State.invalid_samples; gaps_over_limit = $State.gaps_over_limit }
        paging = $paging
        memory = [ordered]@{
            avail_mb = $Sample.avail_mb; modified_list_mb = $Sample.modified_list_mb
            pages = $Sample.pages; pages_in = $Sample.pages_in; pages_out = $Sample.pages_out; page_reads = $Sample.page_reads; page_writes = $Sample.page_writes
            page_faults = $Sample.page_faults; transition_faults = $Sample.transition_faults; cache_faults = $Sample.cache_faults; demand_zero_faults = $Sample.demand_zero_faults
            pagefile_used_mb = $Sample.pagefile_used_mb; pagefile_alloc_mb = $Sample.pagefile_alloc_mb; pagefile_pct = $Sample.pagefile_pct
            rate_issues = @($Sample.rate_issues)
        }
        disks = [ordered]@{ pagefile_disks = $(if ($pfd.Count) { $pfd } else { $null }); system_disk = $Sample.system_disk; external_disk = $Sample.external_disk; system = $sys; external = $ext }
        external_disk = [ordered]@{ present = $xp; identity_ok = (Get-Prop $Sample 'external_identity_ok') }
        storage_events = [ordered]@{ count_since_start = $State.storage_events; ids = @($State.storage_ids | Select-Object -Last 10) }
        cpu_pct = $Sample.cpu_pct
    }
}
# Status document for moments without a sample (start, clean stop).
function New-StatusOnlyDoc {
    param([Parameter(Mandatory)]$State, [Parameter(Mandatory)][string]$Status, [string[]]$Reasons, $Mono, $Utc)
    $c = $State.config
    $pol = Get-PagingPolicy $c
    [ordered]@{
        run_id = $State.run_id; seq = $null; sample_utc = $Utc; sample_mono_s = $Mono
        status = $Status; reasons = @($Reasons)
        latched = $State.latched
        last_valid_sample_age_s = $null
        commit = [ordered]@{ valid = $false; pct = $null; committed_bytes = $null; limit_bytes = $null; seconds_at_or_above_95 = 0.0 }
        limits = Get-LimitsDoc $c
        gate = [ordered]@{ qualifying_seconds = 0.0; required_seconds = $c.pretest_seconds; qualified = $false; since_mono_s = $null; resets = $State.gate_resets; last_reset_reason = 'MONITOR_NOT_SAMPLING' }
        sampling = [ordered]@{ interval_s = $c.interval_s; max_gap_s = $c.max_gap_s; last_gap_s = $null; max_gap_seen_s = $State.max_gap_seen; samples = $State.samples; invalid_samples = $State.invalid_samples; gaps_over_limit = $State.gaps_over_limit }
        paging = [ordered]@{ assessment = 'INSUFFICIENT_DATA'; calibration = $pol.calibration; stop_rule = $pol.stop_rule; stop_required_by_policy = ($c.paging_stop -eq 'Required'); inputs_complete = $false; rules_fired = @(); thresholds_errors = @($pol.errors); windows = [ordered]@{} }
        memory = $null; disks = $null; external_disk = $null
        storage_events = [ordered]@{ count_since_start = $State.storage_events; ids = @($State.storage_ids | Select-Object -Last 10) }
        cpu_pct = $null
    }
}

# --------------------------------------------------------- 3. state file
function ConvertTo-StateJson {
    param([Parameter(Mandatory)]$Doc, [Parameter(Mandatory)]$Monitor, [Parameter(Mandatory)][long]$WrittenQpc, [Parameter(Mandatory)][long]$QpcFrequency, [string]$WrittenUtc)
    $o = [ordered]@{ schema = 'p9s-hostmon-state/1'; written_utc = $WrittenUtc; written_qpc = $WrittenQpc; qpc_frequency = $QpcFrequency; monitor = $Monitor }
    foreach ($k in $Doc.Keys) { $o[$k] = $Doc[$k] }
    $o | ConvertTo-Json -Depth 12
}
# Write to a new temporary file in the same folder, flush it to disk, then
# rename it over the state file (MoveFileEx with replace: a reader sees the
# old or the new document, never a part). Retries while a reader holds the
# file; on failure the temporary file is removed and the error raised.
function Write-StateAtomic {
    param([Parameter(Mandatory)][string]$Path, [Parameter(Mandatory)][string]$Text, [switch]$TestFailBeforeReplace)
    $full = [IO.Path]::GetFullPath($Path)
    $tmp = [IO.Path]::Combine([IO.Path]::GetDirectoryName($full), '.' + [IO.Path]::GetFileName($full) + '.' + [guid]::NewGuid().ToString('N') + '.tmp')
    $bytes = [Text.UTF8Encoding]::new($false).GetBytes($Text)
    $fs = [IO.FileStream]::new($tmp, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None)
    try { $fs.Write($bytes, 0, $bytes.Length); $fs.Flush($true) } finally { $fs.Dispose() }
    try {
        if ($TestFailBeforeReplace) { throw 'injected failure before the replace' }
        $last = $null
        for ($i = 0; $i -lt 25; $i++) {
            try { [IO.File]::Move($tmp, $full, $true); return } catch { $last = $_.Exception.Message; Start-Sleep -Milliseconds 10 }
        }
        throw "state file not replaced after 25 attempts: $last"
    } catch {
        try { [IO.File]::Delete($tmp) } catch {}
        throw
    }
}
# Shared read (the writer may replace the file meanwhile).
function Read-StateFile {
    param([Parameter(Mandatory)][string]$Path)
    for ($i = 0; $i -lt 5; $i++) {
        if (-not [IO.File]::Exists($Path)) { return [pscustomobject]@{ ok = $false; text = $null; error = 'missing' } }
        try {
            $fs = [IO.FileStream]::new($Path, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::ReadWrite -bor [IO.FileShare]::Delete)
            try { $t = [IO.StreamReader]::new($fs, [Text.UTF8Encoding]::new($false)).ReadToEnd() } finally { $fs.Dispose() }
            return [pscustomobject]@{ ok = $true; text = $t; error = $null }
        } catch { Start-Sleep -Milliseconds 20 }
    }
    [pscustomobject]@{ ok = $false; text = $null; error = 'busy' }
}

# --------------------------------------------------------------- 4. reader
function Test-MonitorProcess($procId, $startTicks) {
    try { $p = Get-Process -Id ([int]$procId) -ErrorAction Stop } catch { return $false }
    try { return [math]::Abs($p.StartTime.ToUniversalTime().Ticks - [long]$startTicks) -lt 10000000 } catch { return $false }
}
# Everything a test step needs before it acts. ok only when every check
# passes; each failed check adds its reason code.
function Test-SafetyState {
    param([AllowNull()][AllowEmptyString()][string]$Text, [Parameter(Mandatory)][long]$NowQpc, [Parameter(Mandatory)][long]$QpcFrequency,
          [string]$ExpectedRunId, [ValidateSet('Required', 'NotRequired')][string]$PagingStop = 'Required',
          [switch]$AllowTestMonitor, [double]$MaxAgeSeconds = 12, [scriptblock]$IsMonitorAlive)
    $reasons = [System.Collections.Generic.List[string]]::new()
    $status = $null; $runId = $null; $age = $null
    $done = { [pscustomobject]@{ ok = ($reasons.Count -eq 0); status = $status; reasons = @($reasons); age_s = $age; run_id = $runId } }
    if ([string]::IsNullOrWhiteSpace($Text)) { $reasons.Add('STATE_EMPTY'); return (& $done) }
    try { $o = $Text | ConvertFrom-Json -ErrorAction Stop } catch { $reasons.Add('STATE_INVALID_JSON'); return (& $done) }
    if ($o -isnot [pscustomobject]) { $reasons.Add('STATE_INVALID_JSON'); return (& $done) }
    if ($o.schema -ne 'p9s-hostmon-state/1') { $reasons.Add('SCHEMA_UNSUPPORTED'); return (& $done) }
    foreach ($f in 'run_id', 'status', 'written_qpc', 'qpc_frequency', 'monitor', 'commit', 'gate', 'sampling', 'paging') {
        if (-not ($o.PSObject.Properties.Name -contains $f) -or $null -eq $o.$f) { $reasons.Add("FIELD_MISSING:$f") }
    }
    if ($o.monitor -and ($null -eq $o.monitor.pid -or $null -eq $o.monitor.process_start_ticks)) { $reasons.Add('FIELD_MISSING:monitor') }
    if ($reasons.Count) { return (& $done) }
    $status = [string]$o.status; $runId = [string]$o.run_id
    if (-not (Test-Finite $o.written_qpc) -or [long]$o.qpc_frequency -ne $QpcFrequency) { $reasons.Add('CLOCK_MISMATCH') }
    else {
        $age = ($NowQpc - [long]$o.written_qpc) / [double]$QpcFrequency
        if ($age -lt 0) { $reasons.Add('STATE_FROM_FUTURE') } elseif ($age -gt $MaxAgeSeconds) { $reasons.Add('STATE_STALE') }
    }
    if ($ExpectedRunId -and $runId -ne $ExpectedRunId) { $reasons.Add('RUN_ID_MISMATCH') }
    if ($o.monitor.test -eq $true -and -not $AllowTestMonitor) { $reasons.Add('TEST_MONITOR') }
    $alive = if ($IsMonitorAlive) { & $IsMonitorAlive $o.monitor.pid $o.monitor.process_start_ticks } else { Test-MonitorProcess $o.monitor.pid $o.monitor.process_start_ticks }
    if (-not $alive) { $reasons.Add('MONITOR_NOT_RUNNING') }
    if ($status -ne 'READY') { $reasons.Add("STATUS_$status") }
    if ($null -ne $o.latched) { $reasons.Add('LATCHED') }
    if ($o.gate.qualified -ne $true -or -not (Test-Finite $o.gate.qualifying_seconds) -or [double]$o.gate.qualifying_seconds -lt 300) { $reasons.Add('GATE_INCOMPLETE') }
    if ($o.commit.valid -ne $true -or -not (Test-Finite $o.commit.pct) -or [double]$o.commit.pct -ge 90) { $reasons.Add('COMMIT_INVALID') }
    if ((Test-Finite $o.sampling.last_gap_s) -and [double]$o.sampling.last_gap_s -gt 12) { $reasons.Add('SAMPLING_GAP') }
    if ($PagingStop -eq 'Required') {
        if ($o.paging.stop_rule -ne 'enforced') { $reasons.Add('PAGING_STOP_NOT_ENFORCED') }
        elseif ($o.paging.assessment -ne 'NO_PRESSURE_INDICATED') { $reasons.Add("PAGING_$($o.paging.assessment)") }
    }
    & $done
}
function Invoke-HostGate {
    param([string]$StatePath, [string]$RunId, [ValidateSet('Required', 'NotRequired')][string]$PagingStop = 'Required',
          [string]$Step = 'unnamed step', [switch]$AllowTestMonitor)
    if (-not $StatePath) { $StatePath = Get-DefaultStatePath }
    $r = Read-StateFile -Path $StatePath
    if (-not $r.ok) {
        return [pscustomobject]@{ ok = $false; status = $null; reasons = @($(if ($r.error -eq 'missing') { 'STATE_MISSING' } else { 'STATE_UNREADABLE' })); age_s = $null; run_id = $null; step = $Step; path = $StatePath }
    }
    $t = Test-SafetyState -Text $r.text -NowQpc ([Diagnostics.Stopwatch]::GetTimestamp()) -QpcFrequency ([Diagnostics.Stopwatch]::Frequency) -ExpectedRunId $RunId -PagingStop $PagingStop -AllowTestMonitor:$AllowTestMonitor
    $t | Add-Member -NotePropertyName step -NotePropertyValue $Step
    $t | Add-Member -NotePropertyName path -NotePropertyValue $StatePath
    $t
}
function Format-HostGate($g) {
    '{0} ({1}): status {2}, run {3}, age {4}{5}' -f $(if ($g.ok) { 'HOST GATE OK' } else { 'HOST GATE REFUSED' }), $g.step, $(if ($g.status) { $g.status } else { '-' }), $(if ($g.run_id) { $g.run_id } else { '-' }),
        $(if ($null -ne $g.age_s) { ([double]$g.age_s).ToString('F1', [Globalization.CultureInfo]::InvariantCulture) + ' s' } else { '-' }), $(if ($g.reasons.Count) { '; ' + ($g.reasons -join ', ') } else { '' })
}
# Throws unless the gate passes; used by every gated action script.
function Assert-HostGate {
    param([string]$StatePath, [string]$RunId, [ValidateSet('Required', 'NotRequired')][string]$PagingStop = 'Required', [string]$Step = 'unnamed step')
    $g = Invoke-HostGate -StatePath $StatePath -RunId $RunId -PagingStop $PagingStop -Step $Step
    $line = Format-HostGate $g
    if (-not $g.ok) { throw $line }
    $line
}
