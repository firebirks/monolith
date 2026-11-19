#Requires -Version 7.0
# Synthetic, deterministic tests of the P9-S host resource monitor
# (hostmon-lib.ps1, host-monitor.ps1, assert-host-safe.ps1 and the gated
# action scripts). No memory pressure, no disk load, no VM: the policy is
# fed made-up samples on a controlled clock. Two cases run the monitor
# itself in fixture mode for a few seconds (crash and restart); the gated
# action scripts run with -DryRun only, so even a gate defect could not
# type into or start a VM.
#   pwsh -NoProfile -File tests\hostmon-tests.ps1 -OutDir <new folder>
param([Parameter(Mandatory)][string]$OutDir)
$ErrorActionPreference = 'Stop'
$kit = Split-Path $PSScriptRoot -Parent
$lib = Join-Path $kit 'hostmon-lib.ps1'
. $lib
if (Test-Path -LiteralPath $OutDir) { throw "$OutDir exists" }
New-Item -ItemType Directory -Path $OutDir | Out-Null
$inv = [Globalization.CultureInfo]::InvariantCulture
$results = [System.Collections.Generic.List[object]]::new()
$script:fails = $null

function Case([string]$id, [string]$name, [scriptblock]$body) {
    $script:fails = [System.Collections.Generic.List[string]]::new()
    $sw = [Diagnostics.Stopwatch]::StartNew()
    try { & $body } catch { $script:fails.Add("exception: $($_.Exception.Message) (line $($_.InvocationInfo.ScriptLineNumber))") }
    $ok = $script:fails.Count -eq 0
    $results.Add([pscustomobject]@{ id = $id; name = $name; result = $(if ($ok) { 'PASS' } else { 'FAIL' }); ms = $sw.ElapsedMilliseconds; detail = ($script:fails -join ' | ') })
    '{0} {1} {2}{3}' -f $(if ($ok) { 'PASS' } else { 'FAIL' }), $id, $name, $(if ($ok) { '' } else { ' :: ' + ($script:fails -join ' | ') })
}
function Expect([bool]$cond, [string]$what) { if (-not $cond) { $script:fails.Add($what) } }
function Has($doc, [string]$code) { @($doc.reasons) -contains $code }

# ---- fixtures -------------------------------------------------------------
$GiB = [math]::Pow(2, 30)
$Limit = 100 * $GiB            # commit percent = committed GiB exactly
function S([double]$t, $commit = 50, [hashtable]$o = @{}) {
    $s = [ordered]@{
        seq = 0; mono = $t; utc = '2026-11-19T00:00:00.000Z'; read_ok = $true; read_error = $null
        committed_bytes = $(if ($null -eq $commit) { $null } else { [double]$commit * $GiB })
        commit_limit_bytes = $Limit
        avail_mb = 16000.0; modified_list_mb = 200.0
        pages = 100.0; pages_in = 100.0; pages_out = 0.0; page_reads = 90.0; page_writes = 0.0
        page_faults = 1000.0; transition_faults = 500.0; cache_faults = 100.0; demand_zero_faults = 300.0
        rate_issues = @()
        pagefile_used_mb = 100.0; pagefile_alloc_mb = 5888.0; pagefile_pct = 1.7
        pagefile_disks = @(0); system_disk = 0; external_disk = 2
        disks = @{
            0 = [pscustomobject]@{ read_ms = 0.5; write_ms = 0.3; read_mbps = 5.0; write_mbps = 1.0; queue = 0; avg_queue = 0.01 }
            2 = [pscustomobject]@{ read_ms = 2.0; write_ms = 1.0; read_mbps = 0.1; write_mbps = 0.1; queue = 0; avg_queue = 0.01 }
        }
        external_present = $true; external_identity_ok = $true
        eventlog_ok = $true; storage_events_new = 0; storage_event_ids = @()
        cpu_pct = 10.0; vm = @{}
    }
    foreach ($k in $o.Keys) { $s[$k] = $o[$k] }
    [pscustomobject]$s
}
function Times([double]$from, [double]$to, [double]$step = 5) { for ($t = $from; $t -le $to + 1e-9; $t += $step) { $t } }
function Run($state, $samples) { $docs = [System.Collections.Generic.List[object]]::new(); foreach ($x in $samples) { $docs.Add((Step-Monitor -State $state -Sample $x)) }; , $docs }
function Steady($state, [double]$from, [double]$to, [hashtable]$o = @{}, $commit = 50) { Run $state @(Times $from $to | ForEach-Object { S $_ $commit $o }) }

$thrValues = [ordered]@{
    suspect = [ordered]@{ page_out_per_s = 500; avail_mb_low = 4096; avail_decline_mb = 4096; pagefile_growth_mb = 128; pagefile_disk_read_ms = 20; page_in_per_s_with_low_memory = 2000 }
    confirm = [ordered]@{ page_out_per_s = 2000; avail_mb_low = 2048; avail_decline_mb = 6144; pagefile_growth_mb = 300; pagefile_disk_read_ms = 50; page_in_per_s_with_low_memory = 5000 }
}
function Thr([bool]$approved) {
    $o = [ordered]@{ schema = 'p9s-hostmon-paging/1'; approved = $approved; approved_by = $(if ($approved) { 'test fixture only' } else { $null }); approved_utc = $(if ($approved) { '2026-11-19T00:00:00Z' } else { $null }); source = 'test fixture'; suspect = $thrValues.suspect; confirm = $thrValues.confirm }
    ConvertTo-PagingThresholds -Json ($o | ConvertTo-Json -Depth 5)
}
$fxApproved = Thr $true
$fxCandidate = Thr $false
$realThr = Get-PagingThresholds -Path (Join-Path $kit 'paging-thresholds.json')
function Cfg([string]$paging = 'NotRequired', $th = $fxApproved, [string]$mode = 'Monitor') { New-HostMonConfig -PagingStop $paging -Thresholds $th -Mode $mode }
function NewState($cfg, [string]$run = 'run-test-1') { New-MonitorState -RunId $run -Config $cfg }

$F = 10000000L
$monitorInfo = [ordered]@{ pid = 4242; process_start_utc = '2026-11-19T00:00:00.0000000Z'; process_start_ticks = 1L; mode = 'Monitor'; paging_stop = 'NotRequired'; thresholds_sha256 = 'x'; test = $false; kit = 'test' }
function Json($doc, [long]$writtenQpc = 1000L * $F, $info = $monitorInfo) { ConvertTo-StateJson -Doc $doc -Monitor $info -WrittenQpc $writtenQpc -QpcFrequency $F -WrittenUtc '2026-11-19T00:00:00.000Z' }
$alive = { param($procId, $ticks) $true }
$dead = { param($procId, $ticks) $false }
function Check([string]$json, [long]$nowQpc = 1004L * $F, [hashtable]$p = @{}) {
    $a = @{ Text = $json; NowQpc = $nowQpc; QpcFrequency = $F; IsMonitorAlive = $alive; PagingStop = 'NotRequired' }
    foreach ($k in $p.Keys) { $a[$k] = $p[$k] }
    Test-SafetyState @a
}
function ReadyDoc() { $st = NewState (Cfg); $d = Steady $st 0 300; $d[$d.Count - 1] }

"== P9-S host monitor synthetic tests, $((Get-Date).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ')), pwsh $($PSVersionTable.PSVersion)"

# ---- pre-test gate --------------------------------------------------------
Case 'G1' 'exactly 300 qualifying seconds: READY' {
    $st = NewState (Cfg); $d = Steady $st 0 300
    Expect ($d[59].status -eq 'QUALIFYING') "t=295 status $($d[59].status)"
    Expect ($d[60].status -eq 'READY') "t=300 status $($d[60].status)"
    Expect ($d[60].gate.qualifying_seconds -eq 300) "seconds $($d[60].gate.qualifying_seconds)"
    Expect ($d[60].gate.qualified) 'qualified false'
}
Case 'G2' '299 seconds: NOT READY' {
    $st = NewState (Cfg); $d = Run $st @((Times 0 295 | ForEach-Object { S $_ }) + (S 299))
    $l = $d[$d.Count - 1]
    Expect ($l.status -eq 'QUALIFYING') "status $($l.status)"; Expect (-not $l.gate.qualified) 'qualified'
    Expect ($l.gate.qualifying_seconds -eq 299) "seconds $($l.gate.qualifying_seconds)"
}
Case 'G3' '289 seconds: NOT READY' {
    $st = NewState (Cfg); $d = Run $st @((Times 0 285 | ForEach-Object { S $_ }) + (S 289))
    $l = $d[$d.Count - 1]
    Expect ($l.status -eq 'QUALIFYING') "status $($l.status)"; Expect (-not $l.gate.qualified) 'qualified'
}
Case 'G4' 'one reading at 90 %: interval reset' {
    $st = NewState (Cfg)
    $d = Run $st @(Times 0 300 | ForEach-Object { if ($_ -eq 150) { S $_ 90 } else { S $_ } })
    Expect ($d[30].status -eq 'WARNING' -and (Has $d[30] 'COMMIT_AT_OR_ABOVE_90')) "t=150 $($d[30].status) $($d[30].reasons -join ',')"
    Expect ($d[60].status -eq 'QUALIFYING') "t=300 $($d[60].status)"
    Expect ($d[60].gate.qualifying_seconds -eq 145) "t=300 seconds $($d[60].gate.qualifying_seconds)"
    $e = Steady $st 305 455
    Expect ($e[$e.Count - 2].status -eq 'QUALIFYING' -and $e[$e.Count - 1].status -eq 'READY') "t=450/455 $($e[$e.Count - 2].status)/$($e[$e.Count - 1].status)"
}
Case 'G5' 'one invalid sample: interval reset' {
    $st = NewState (Cfg)
    $d = Run $st @(Times 0 300 | ForEach-Object { if ($_ -eq 150) { S $_ $null } else { S $_ } })
    Expect ($d[30].status -eq 'UNKNOWN' -and (Has $d[30] 'COMMIT_INVALID')) "t=150 $($d[30].status) $($d[30].reasons -join ',')"
    Expect ($d[60].gate.qualifying_seconds -eq 145) "t=300 seconds $($d[60].gate.qualifying_seconds)"
}
Case 'G5b' 'failed counter read: UNKNOWN and reset' {
    $st = NewState (Cfg)
    $d = Run $st @(Times 0 300 | ForEach-Object { if ($_ -eq 150) { S $_ 50 @{ read_ok = $false; read_error = 'CIM failure' } } else { S $_ } })
    Expect ($d[30].status -eq 'UNKNOWN' -and (Has $d[30] 'SAMPLE_READ_FAILED')) "t=150 $($d[30].status) $($d[30].reasons -join ',')"
    Expect ($d[60].gate.qualifying_seconds -eq 145) "t=300 seconds $($d[60].gate.qualifying_seconds)"
}
Case 'G6' 'gap above 12 s: UNKNOWN and reset' {
    $st = NewState (Cfg)
    $d = Run $st @((Times 0 145 | ForEach-Object { S $_ }) + (Times 158 300 | ForEach-Object { S $_ }))
    $g = $d[30]
    Expect ($g.status -eq 'UNKNOWN' -and (Has $g 'SAMPLE_GAP')) "t=158 $($g.status) $($g.reasons -join ',')"
    Expect ($g.sampling.last_gap_s -eq 13) "gap $($g.sampling.last_gap_s)"
    Expect ($d[31].status -eq 'QUALIFYING') "t=163 $($d[31].status)"
    Expect ($d[$d.Count - 1].gate.qualifying_seconds -eq 140) "t=298 seconds $($d[$d.Count - 1].gate.qualifying_seconds)"
}
Case 'G6b' 'gap of exactly 12 s is still continuous coverage' {
    $st = NewState (Cfg)
    $d = Run $st @((Times 0 140 | ForEach-Object { S $_ }) + (Times 152 312 | ForEach-Object { S $_ }))
    Expect ($d[29].status -eq 'QUALIFYING') "t=152 $($d[29].status)"
    Expect ($d[$d.Count - 1].status -eq 'READY' -and $d[$d.Count - 1].gate.qualifying_seconds -eq 312) "t=312 $($d[$d.Count - 1].status) $($d[$d.Count - 1].gate.qualifying_seconds)"
}
Case 'G6c' 'clock not increasing: UNKNOWN' {
    $st = NewState (Cfg); $d = Run $st @((S 0), (S 5), (S 5))
    Expect ($d[2].status -eq 'UNKNOWN' -and (Has $d[2] 'CLOCK_NOT_MONOTONIC')) "$($d[2].status) $($d[2].reasons -join ',')"
}
Case 'G7' 'monitor restart: interval reset, old run refused' {
    $st = NewState (Cfg) 'run-A'; $d = Steady $st 0 300
    Expect ($d[60].status -eq 'READY') "run A t=300 $($d[60].status)"
    $st2 = NewState (Cfg) 'run-B'; $e = Steady $st2 305 310
    Expect ($e[0].status -eq 'QUALIFYING' -and $e[0].gate.qualifying_seconds -eq 0) "run B first $($e[0].status) $($e[0].gate.qualifying_seconds)"
    $r = Check (Json $d[60]) -p @{ ExpectedRunId = 'run-B' }
    Expect (-not $r.ok -and ($r.reasons -contains 'RUN_ID_MISMATCH')) "reader $($r.ok) $($r.reasons -join ',')"
}
Case 'G8' 'stale READY state rejected' {
    $j = Json (ReadyDoc)
    $ok = Check $j (1004L * $F); $stale = Check $j (1013L * $F)
    Expect ($ok.ok) "fresh READY refused: $($ok.reasons -join ',')"
    Expect (-not $stale.ok -and ($stale.reasons -contains 'STATE_STALE')) "stale accepted or wrong reason: $($stale.reasons -join ',')"
}

# ---- commit ---------------------------------------------------------------
Case 'C1' 'commit below 90 %: no commit warning' {
    $st = NewState (Cfg); $d = Steady $st 0 300 @{} 89.9
    $w = @($d | Where-Object { $_.status -ne 'QUALIFYING' -and $_.status -ne 'READY' })
    Expect ($w.Count -eq 0) "non-ready states: $(($w | ForEach-Object status) -join ',')"
    Expect ($d[60].status -eq 'READY') "t=300 $($d[60].status)"
}
Case 'C2' 'commit 90 to below 95 %: WARNING' {
    $st = NewState (Cfg); $d = Run $st @((S 0), (S 5 92), (S 10 94.99))
    Expect ($d[1].status -eq 'WARNING' -and (Has $d[1] 'COMMIT_AT_OR_ABOVE_90')) "92: $($d[1].status)"
    Expect ($d[2].status -eq 'WARNING' -and -not (Has $d[2] 'COMMIT_AT_OR_ABOVE_95')) "94.99: $($d[2].status) $($d[2].reasons -join ',')"
}
Case 'C3' '95 % for less than 30 s: WARNING, no stop' {
    $st = NewState (Cfg); $d = Steady $st 0 25 @{} 95
    $l = $d[$d.Count - 1]
    Expect ($l.status -eq 'WARNING' -and (Has $l 'COMMIT_AT_OR_ABOVE_95')) "$($l.status) $($l.reasons -join ',')"
    Expect ($l.commit.seconds_at_or_above_95 -eq 25) "seconds $($l.commit.seconds_at_or_above_95)"
    Expect (-not (@($d | Where-Object status -eq 'STOP_REQUIRED').Count)) 'a STOP appeared'
}
Case 'C4' '95 % for 30 continuous s: STOP_REQUIRED, latched' {
    $st = NewState (Cfg); $d = Steady $st 0 30 @{} 95
    $l = $d[$d.Count - 1]
    Expect ($d[5].status -eq 'WARNING') "t=25 $($d[5].status)"
    Expect ($l.status -eq 'STOP_REQUIRED' -and (Has $l 'COMMIT_SUSTAINED_95')) "t=30 $($l.status) $($l.reasons -join ',')"
    $e = Steady $st 35 400
    Expect ($e[$e.Count - 1].status -eq 'STOP_REQUIRED') "after recovery $($e[$e.Count - 1].status)"
}
Case 'C5' 'one sample at 97 %: ABORT_REQUIRED, latched' {
    $st = NewState (Cfg); $d = Run $st @((S 0), (S 5 97), (S 10 50))
    Expect ($d[1].status -eq 'ABORT_REQUIRED' -and (Has $d[1] 'COMMIT_CRITICAL_97')) "97: $($d[1].status)"
    Expect ($d[2].status -eq 'ABORT_REQUIRED') "after: $($d[2].status)"
    $st2 = NewState (Cfg); $e = Run $st2 @((S 0 96.99))
    Expect ($e[0].status -eq 'WARNING') "96.99: $($e[0].status)"
    $st3 = NewState (Cfg); $f = Run $st3 @((S 0 97))
    Expect ($f[0].status -eq 'ABORT_REQUIRED') "first sample 97: $($f[0].status)"
}
Case 'C6' 'missing sample near 95 %: no fabricated interval' {
    $st = NewState (Cfg)
    $d = Run $st @((Times 0 10 | ForEach-Object { S $_ 95 }) + (Times 23 38 | ForEach-Object { S $_ 95 }))
    $l = $d[$d.Count - 1]
    Expect ($l.status -ne 'STOP_REQUIRED') "gap case $($l.status)"
    Expect ($l.commit.seconds_at_or_above_95 -eq 15) "gap case seconds $($l.commit.seconds_at_or_above_95)"
    $st2 = NewState (Cfg)
    $e = Run $st2 @(Times 0 45 | ForEach-Object { if ($_ -eq 15) { S $_ $null } else { S $_ 95 } })
    Expect ($e[8].status -ne 'STOP_REQUIRED') "invalid case t=40 $($e[8].status)"
    Expect ($e[9].status -eq 'WARNING') "invalid case t=45 $($e[9].status)"
    $f = Run $st2 @((S 50 95))
    Expect ($f[0].status -eq 'STOP_REQUIRED') "invalid case t=50 $($f[0].status)"
}
Case 'C7' 'commit limit zero or missing: UNKNOWN' {
    $st = NewState (Cfg); $d = Run $st @((S 0 50 @{ commit_limit_bytes = 0 }), (S 5 50 @{ commit_limit_bytes = $null }), (S 10 50 @{ commit_limit_bytes = -5 }))
    foreach ($i in 0..2) { Expect ($d[$i].status -eq 'UNKNOWN' -and (Has $d[$i] 'COMMIT_LIMIT_INVALID')) "case $i $($d[$i].status) $($d[$i].reasons -join ',')" }
    Expect ($null -eq $d[0].commit.pct) 'a percentage was computed'
}

# ---- paging ---------------------------------------------------------------
Case 'P1' 'isolated page-in spike, ample memory: no pressure' {
    $st = NewState (Cfg)
    $d = Run $st @(Times 0 300 | ForEach-Object { if ($_ -eq 200) { S $_ 50 @{ pages_in = 30000.0; pages = 30000.0 } } else { S $_ } })
    $bad = @($d | Where-Object { $_.paging.assessment -in 'PRESSURE_SUSPECTED', 'PRESSURE_CONFIRMED' })
    Expect ($bad.Count -eq 0) 'pressure reported'
    Expect ($d[60].paging.assessment -eq 'NO_PRESSURE_INDICATED' -and $d[60].status -eq 'READY') "t=300 $($d[60].paging.assessment) $($d[60].status)"
    $st2 = NewState (Cfg 'NotRequired' $fxCandidate)
    $e = Run $st2 @(Times 0 300 | ForEach-Object { if ($_ -eq 200) { S $_ 50 @{ pages_in = 30000.0 } } else { S $_ } })
    Expect ($e[60].paging.assessment -eq 'UNCALIBRATED') "uncalibrated: $($e[60].paging.assessment)"
}
Case 'P2' 'prolonged page-in without page-file pressure: no pressure' {
    $st = NewState (Cfg)
    $d = Steady $st 0 300 @{ pages_in = 8000.0; pages = 8000.0; page_reads = 7900.0; avail_mb = 9000.0 }
    Expect (-not @($d | Where-Object { $_.paging.assessment -like 'PRESSURE_*' }).Count) 'pressure reported'
    Expect ($d[60].status -eq 'READY') "t=300 $($d[60].status)"
}
Case 'P3' 'page-out with falling available memory' {
    $mk = { param($t) $a = [math]::Max(1500.0, 6000.0 - 40.0 * $t); S $t 50 @{ pages_out = 3000.0; page_writes = 60.0; avail_mb = $a } }
    $st = NewState (Cfg)
    $d = Run $st @(Times 0 300 | ForEach-Object { & $mk $_ })
    $first = @($d | Where-Object { $_.status -eq 'STOP_REQUIRED' } | Select-Object -First 1)
    Expect ($first.Count -eq 1 -and (Has $first[0] 'PAGING_PRESSURE_CONFIRMED')) "approved: no confirmed stop"
    $pre = @($d | Where-Object { $_.paging.assessment -eq 'PRESSURE_SUSPECTED' })
    Expect ($pre.Count -ge 1) 'no suspected phase before confirmation'
    $st2 = NewState (Cfg 'NotRequired' $fxCandidate)
    $e = Run $st2 @(Times 0 300 | ForEach-Object { & $mk $_ })
    Expect (-not @($e | Where-Object { $_.status -in 'STOP_REQUIRED', 'ABORT_REQUIRED' }).Count) 'unapproved values caused a stop'
    Expect ($e[60].status -eq 'WARNING' -and $e[60].paging.assessment -eq 'PRESSURE_SUSPECTED') "unapproved t=300 $($e[60].status) $($e[60].paging.assessment)"
}
Case 'P4' 'page-file growth with slow page-file disk' {
    $mk = { param($t) $disk0 = [pscustomobject]@{ read_ms = 80.0; write_ms = 40.0; read_mbps = 50.0; write_mbps = 30.0; queue = 4; avg_queue = 3.5 }
        $dk = @{ 0 = $disk0; 2 = [pscustomobject]@{ read_ms = 2.0; write_ms = 1.0; read_mbps = 0.1; write_mbps = 0.1; queue = 0; avg_queue = 0.0 } }
        S $t 50 @{ pagefile_used_mb = 100.0 + 20.0 * $t; disks = $dk } }
    $st = NewState (Cfg); $d = Run $st @(Times 0 300 | ForEach-Object { & $mk $_ })
    $stop = @($d | Where-Object { $_.status -eq 'STOP_REQUIRED' } | Select-Object -First 1)
    Expect ($stop.Count -eq 1 -and (Has $stop[0] 'PAGING_PRESSURE_CONFIRMED')) 'approved: no confirmed stop'
    Expect (@($d | Where-Object { $_.paging.assessment -eq 'PRESSURE_SUSPECTED' }).Count -ge 1) 'no suspected phase'
    $hi = @($d | Where-Object { $_.status -eq 'STOP_REQUIRED' })[0]
    Expect (@($hi.paging.rules_fired) -contains 'R2') "rules $($hi.paging.rules_fired -join ',')"
}
Case 'P5' 'missing system-drive counter: INSUFFICIENT_DATA' {
    $dk = @{ 2 = [pscustomobject]@{ read_ms = 2.0; write_ms = 1.0; read_mbps = 0.1; write_mbps = 0.1; queue = 0; avg_queue = 0.0 } }
    $st = NewState (Cfg 'Required' $fxApproved); $d = Steady $st 0 300 @{ disks = $dk }
    Expect ($d[60].paging.assessment -eq 'INSUFFICIENT_DATA') "assessment $($d[60].paging.assessment)"
    Expect ($d[60].status -eq 'UNKNOWN' -and (Has $d[60] 'PAGING_INSUFFICIENT_DATA')) "required: $($d[60].status)"
    $st2 = NewState (Cfg 'NotRequired' $fxApproved); $e = Steady $st2 0 300 @{ disks = $dk }
    Expect ($e[60].status -eq 'READY' -and (Has $e[60] 'PAGING_INSUFFICIENT_DATA')) "not required: $($e[60].status) $($e[60].reasons -join ',')"
}
Case 'P6' 'missing page-file location: INSUFFICIENT_DATA' {
    $st = NewState (Cfg 'Required' $fxApproved); $d = Steady $st 0 300 @{ pagefile_disks = $null }
    Expect ($d[60].paging.assessment -eq 'INSUFFICIENT_DATA') "assessment $($d[60].paging.assessment)"
    Expect ($d[60].status -ne 'READY') "status $($d[60].status)"
}
Case 'P7' 'counter wrap kept, reset or discontinuity rejected' {
    $mem0 = [pscustomobject]@{ Timestamp_PerfTime = 100L * $F; Frequency_PerfTime = $F; PagesInputPersec = 4294967000L; PagesOutputPersec = 1000L; CommittedBytes = 10GB; CommitLimit = 40GB; AvailableMBytes = 9000 }
    $mem1 = [pscustomobject]@{ Timestamp_PerfTime = 105L * $F; Frequency_PerfTime = $F; PagesInputPersec = 500L; PagesOutputPersec = 500L; CommittedBytes = 10GB; CommitLimit = 40GB; AvailableMBytes = 9000 }
    $w = Get-CounterRate -Prev $mem0 -Cur $mem1 -Name 'PagesInputPersec' -Bits 32 -MaxPerSecond 1e7
    Expect ($null -ne $w.value -and [math]::Abs($w.value - 159.2) -lt 1e-6) "wrap value $($w.value) issue $($w.issue)"
    $r = Get-CounterRate -Prev $mem0 -Cur $mem1 -Name 'PagesOutputPersec' -Bits 32 -MaxPerSecond 1e7
    Expect ($null -eq $r.value -and $r.issue -eq 'discontinuity') "reset value $($r.value) issue $($r.issue)"
    $mem2 = [pscustomobject]@{ Timestamp_PerfTime = 100L * $F; Frequency_PerfTime = $F; PagesInputPersec = 600L }
    $t = Get-CounterRate -Prev $mem0 -Cur $mem2 -Name 'PagesInputPersec' -Bits 32 -MaxPerSecond 1e7
    Expect ($null -eq $t.value -and $t.issue -eq 'time') "time value $($t.value) issue $($t.issue)"
    $st = NewState (Cfg 'Required' $fxApproved)
    $d = Run $st @(Times 0 300 | ForEach-Object { if ($_ -eq 250) { S $_ 50 @{ pages_in = $null; pages_out = $null; rate_issues = @('PagesInputPersec:discontinuity') } } else { S $_ } })
    Expect ($d[50].paging.assessment -eq 'INSUFFICIENT_DATA') "t=250 $($d[50].paging.assessment)"
    Expect ($d[60].paging.assessment -eq 'INSUFFICIENT_DATA' -and -not $d[60].paging.windows.'120'.covered) "t=300 $($d[60].paging.assessment)"
    Expect ($d[60].gate.qualifying_seconds -eq 300) "commit gate disturbed: $($d[60].gate.qualifying_seconds)"
}
Case 'P8' 'uncalibrated numerical limits: gate not ready when the stop is required' {
    Expect (-not $realThr.approved) 'kit thresholds claim approval'
    $st = NewState (Cfg 'Required' $realThr); $d = Steady $st 0 300
    Expect ($d[60].paging.assessment -eq 'UNCALIBRATED' -and $d[60].paging.stop_rule -eq 'not_enforced') "$($d[60].paging.assessment) $($d[60].paging.stop_rule)"
    Expect ($d[60].status -eq 'UNCALIBRATED' -and $d[60].gate.qualified) "status $($d[60].status)"
    $r = Check (Json $d[60]) -p @{ PagingStop = 'Required' }
    Expect (-not $r.ok -and ($r.reasons -contains 'STATUS_UNCALIBRATED')) "reader $($r.reasons -join ',')"
    $st2 = NewState (Cfg 'NotRequired' $realThr); $e = Steady $st2 0 300
    $r2 = Check (Json $e[60]) -p @{ PagingStop = 'Required' }
    Expect (-not $r2.ok -and ($r2.reasons -contains 'PAGING_STOP_NOT_ENFORCED')) "reader on a not-required monitor $($r2.reasons -join ',')"
}
Case 'P9' 'absent or renamed counter property: null, no exception' {
    $a = [pscustomobject]@{ Timestamp_PerfTime = 100L * $F; Frequency_PerfTime = $F }
    $b = [pscustomobject]@{ Timestamp_PerfTime = 105L * $F; Frequency_PerfTime = $F }
    $r = Get-CounterRate -Prev $a -Cur $b -Name 'PagesOutputPersec' -Bits 32 -MaxPerSecond 1e7
    Expect ($null -eq $r.value -and $r.issue -eq 'absent') "value $($r.value) issue $($r.issue)"
}
Case 'P10' 'windows of 30 and 120 s reported, 120 s not covered early' {
    $st = NewState (Cfg); $d = Steady $st 0 130
    Expect ($d[6].paging.windows.'30'.covered -and -not $d[6].paging.windows.'120'.covered) "t=30 $($d[6].paging.windows.'30'.covered)/$($d[6].paging.windows.'120'.covered)"
    Expect ($d[24].paging.windows.'120'.covered) "t=120 not covered"
    Expect ($d[5].paging.assessment -eq 'INSUFFICIENT_DATA') "t=25 $($d[5].paging.assessment)"
}
Case 'P11' 'derived sample from raw counters (rates, latency, page file, disks)' {
    $m0 = [pscustomobject]@{ Timestamp_PerfTime = 100L * $F; Frequency_PerfTime = $F; CommittedBytes = 20GB; CommitLimit = 40GB; AvailableMBytes = 9000; ModifiedPageListBytes = 100MB
        PagesPersec = 1000L; PagesInputPersec = 1000L; PagesOutputPersec = 0L; PageReadsPersec = 500L; PageWritesPersec = 0L; PageFaultsPersec = 0L; TransitionFaultsPersec = 0L; CacheFaultsPersec = 0L; DemandZeroFaultsPersec = 0L }
    $m1 = [pscustomobject]@{ Timestamp_PerfTime = 105L * $F; Frequency_PerfTime = $F; CommittedBytes = 20GB; CommitLimit = 40GB; AvailableMBytes = 8000; ModifiedPageListBytes = 200MB
        PagesPersec = 6000L; PagesInputPersec = 5000L; PagesOutputPersec = 1000L; PageReadsPersec = 1500L; PageWritesPersec = 50L; PageFaultsPersec = 50000L; TransitionFaultsPersec = 1000L; CacheFaultsPersec = 500L; DemandZeroFaultsPersec = 2000L }
    function Dk($n, $t, $rn, $rb, $wn, $wb, $readBytes, $writeBytes) { [pscustomobject]@{ Name = $n; Timestamp_PerfTime = $t * $F; Frequency_PerfTime = $F; Timestamp_Sys100NS = $t * $F
        AvgDisksecPerRead = $rn; AvgDisksecPerRead_Base = $rb; AvgDisksecPerWrite = $wn; AvgDisksecPerWrite_Base = $wb; DiskReadBytesPersec = $readBytes; DiskWriteBytesPersec = $writeBytes; CurrentDiskQueueLength = 1; AvgDiskQueueLength = $t * $F / 2 } }
    $raw0 = [pscustomobject]@{ ok = $true; error = $null; mono = 100.0; utc = 'a'; mem = $m0; cpu = $null
        pf_usage = @([pscustomobject]@{ Name = 'C:\pagefile.sys'; CurrentUsage = 100; AllocatedBaseSize = 5888 }); pf_perf = @([pscustomobject]@{ Name = '_Total'; PercentUsage = 17; PercentUsage_Base = 1000 })
        disks = @((Dk '0 C:' 100 0 0 0 0 0 0), (Dk '2 D:' 100 0 0 0 0 0 0)); external_present = $true; external_identity_ok = $true; eventlog_ok = $true; storage_events_new = 0; storage_event_ids = @(); vm = @{} }
    $raw1 = [pscustomobject]@{ ok = $true; error = $null; mono = 105.0; utc = 'b'; mem = $m1; cpu = $null
        pf_usage = @([pscustomobject]@{ Name = 'C:\pagefile.sys'; CurrentUsage = 150; AllocatedBaseSize = 5888 }); pf_perf = @([pscustomobject]@{ Name = '_Total'; PercentUsage = 25; PercentUsage_Base = 1000 })
        disks = @((Dk '0 C:' 105 (100 * 0.010 * $F) 100 (10 * 0.002 * $F) 10 (50MB) (5MB)), (Dk '2 D:' 105 0 0 0 0 0 0)); external_present = $true; external_identity_ok = $true; eventlog_ok = $true; storage_events_new = 0; storage_event_ids = @(); vm = @{} }
    $layout = [pscustomobject]@{ system_disk = 0; external_disk = 2; pagefile_disks = @(0) }
    $s = ConvertTo-DerivedSample -Prev $raw0 -Cur $raw1 -Layout $layout
    Expect ($s.pages_in -eq 800 -and $s.pages_out -eq 200 -and $s.page_reads -eq 200 -and $s.page_writes -eq 10) "rates in $($s.pages_in) out $($s.pages_out) reads $($s.page_reads) writes $($s.page_writes)"
    $d0 = $s.disks[0]
    Expect ([math]::Abs($d0.read_ms - 10) -lt 1e-9 -and [math]::Abs($d0.write_ms - 2) -lt 1e-9) "latency read $($d0.read_ms) write $($d0.write_ms)"
    Expect ([math]::Abs($d0.read_mbps - 10) -lt 1e-9 -and [math]::Abs($d0.avg_queue - 0.5) -lt 1e-9) "read MB/s $($d0.read_mbps) avg queue $($d0.avg_queue)"
    Expect ($null -eq $s.disks[2].read_ms) "no-I/O latency should be null, got $($s.disks[2].read_ms)"
    Expect ($s.pagefile_used_mb -eq 150 -and [math]::Abs($s.pagefile_pct - 2.5) -lt 1e-9) "page file $($s.pagefile_used_mb) $($s.pagefile_pct)"
    Expect ($s.committed_bytes -eq 20GB -and $s.commit_limit_bytes -eq 40GB -and $s.avail_mb -eq 8000) 'gauges'
    $first = ConvertTo-DerivedSample -Prev $null -Cur $raw0 -Layout $layout
    Expect ($null -eq $first.pages_in -and $first.committed_bytes -eq 20GB) 'first sample: rates must be null, gauges present'
}

# ---- escalation and modes (added after the first green run; each checked
# against a mutant, see synthetic-tests\targeted-mutants.txt) --------------
Case 'D1' 'external disk missing: STOP_REQUIRED, latched' {
    $st = NewState (Cfg); $d = Run $st @((S 0), (S 5 50 @{ external_present = $false }), (S 10))
    Expect ($d[1].status -eq 'STOP_REQUIRED' -and (Has $d[1] 'EXTERNAL_DISK_MISSING')) "missing: $($d[1].status) $($d[1].reasons -join ',')"
    Expect ($d[2].status -eq 'STOP_REQUIRED') "after: $($d[2].status)"
}
Case 'D2' 'storage or USB event: STOP_REQUIRED, latched' {
    $st = NewState (Cfg); $d = Run $st @((S 0), (S 5 50 @{ storage_events_new = 1; storage_event_ids = @('disk/153/L3') }), (S 10))
    Expect ($d[1].status -eq 'STOP_REQUIRED' -and (Has $d[1] 'STORAGE_EVENT')) "event: $($d[1].status) $($d[1].reasons -join ',')"
    Expect ($d[2].status -eq 'STOP_REQUIRED' -and $d[2].storage_events.count_since_start -eq 1) "after: $($d[2].status) $($d[2].storage_events.count_since_start)"
}
Case 'D3' 'event log unreadable or unchecked, wrong external disk: UNKNOWN' {
    $st = NewState (Cfg); $d = Run $st @((S 0 50 @{ eventlog_ok = $false }), (S 5 50 @{ eventlog_ok = $null }), (S 10 50 @{ external_identity_ok = $false }), (S 15 50 @{ external_present = $null }))
    Expect ($d[0].status -eq 'UNKNOWN' -and (Has $d[0] 'EVENTLOG_UNREADABLE')) "unreadable: $($d[0].status)"
    Expect ($d[1].status -eq 'UNKNOWN' -and (Has $d[1] 'EVENTLOG_UNCHECKED')) "unchecked: $($d[1].status)"
    Expect ($d[2].status -eq 'UNKNOWN' -and (Has $d[2] 'EXTERNAL_DISK_UNEXPECTED')) "identity: $($d[2].status)"
    Expect ($d[3].status -eq 'UNKNOWN' -and (Has $d[3] 'EXTERNAL_DISK_UNCHECKED')) "presence unchecked: $($d[3].status)"
    Expect ($d[3].gate.qualifying_seconds -eq 0) "gate kept across unknown samples: $($d[3].gate.qualifying_seconds)"
}
Case 'C8' 'STOP latched, then 97 %: upgraded to ABORT_REQUIRED' {
    $st = NewState (Cfg); $d = Steady $st 0 30 @{} 95
    $e = Run $st @((S 35 97))
    Expect ($d[6].status -eq 'STOP_REQUIRED' -and $e[0].status -eq 'ABORT_REQUIRED') "$($d[6].status) -> $($e[0].status)"
    Expect ($e[0].latched.status -eq 'ABORT_REQUIRED') "latch $($e[0].latched.status)"
}
Case 'G9' 'Calibrate mode: CALIBRATING, never READY' {
    $st = NewState (Cfg 'NotRequired' $fxApproved 'Calibrate'); $d = Steady $st 0 310
    Expect (-not @($d | Where-Object status -eq 'READY').Count) 'READY in calibrate mode'
    Expect ($d[$d.Count - 1].status -eq 'CALIBRATING' -and $d[$d.Count - 1].gate.qualified) "last: $($d[$d.Count - 1].status)"
    $st2 = NewState (Cfg 'NotRequired' $fxApproved 'Calibrate'); $e = Run $st2 @((S 0), (S 5 97))
    Expect ($e[1].status -eq 'ABORT_REQUIRED') "abort still applies: $($e[1].status)"
}

# ---- state file and reader -----------------------------------------------
$sdir = Join-Path $OutDir 'state'
New-Item -ItemType Directory -Path $sdir | Out-Null
Case 'S1a' 'atomic replacement: failure before replace keeps the old file' {
    $p = Join-Path $sdir 'atomic-a.json'
    $j1 = Json (ReadyDoc)
    Write-StateAtomic -Path $p -Text $j1
    $threw = $false
    try { Write-StateAtomic -Path $p -Text '{"partial":' -TestFailBeforeReplace } catch { $threw = $true }
    Expect $threw 'injected failure not raised'
    Expect ((Get-Content -LiteralPath $p -Raw) -eq $j1) 'destination changed'
    Expect (@(Get-ChildItem -LiteralPath $sdir -Filter '*.tmp' -Force).Count -eq 0) 'temporary file left behind'
}
Case 'S1b' 'atomic replacement under concurrent reads: no partial JSON' {
    $p = Join-Path $sdir 'atomic-b.json'
    Write-StateAtomic -Path $p -Text '{"n":0,"pad":""}'
    $job = Start-ThreadJob -ArgumentList $lib, $p -ScriptBlock {
        param($lib, $p)
        . $lib
        $fail = 0
        for ($i = 1; $i -le 400; $i++) {
            $pad = if ($i % 2) { 'x' * 20000 } else { '' }
            try { Write-StateAtomic -Path $p -Text ('{"n":' + $i + ',"pad":"' + $pad + '"}') } catch { $fail++ }
        }
        $fail
    }
    $okReads = 0; $busy = 0; $bad = 0
    while ($job.State -eq 'Running' -or $job.State -eq 'NotStarted') {
        $r = Read-StateFile -Path $p
        if (-not $r.ok) { $busy++; continue }
        try { $null = $r.text | ConvertFrom-Json -ErrorAction Stop; $okReads++ } catch { $bad++ }
    }
    $writeFails = Receive-Job $job -Wait; Remove-Job $job
    "      reads ok $okReads, busy $busy, invalid $bad; writer failures $writeFails"
    Expect ($bad -eq 0) "$bad partial or invalid reads"
    Expect ($okReads -gt 0) 'no successful read'
    Expect (@(Get-ChildItem -LiteralPath $sdir -Filter '*.tmp' -Force).Count -eq 0) 'temporary files left behind'
}
Case 'S2' 'invalid JSON rejected' {
    $r = Check '{"schema": "p9s-hostmon-state/1", "status": "READY"'
    Expect (-not $r.ok -and ($r.reasons -contains 'STATE_INVALID_JSON')) "$($r.reasons -join ',')"
}
Case 'S3' 'stale status rejected (age above 12 s, or written in the future)' {
    $j = Json (ReadyDoc)
    $a = Check $j (1012L * $F); $b = Check $j (1012L * $F + 1); $c = Check $j (999L * $F)
    Expect ($a.ok) "age 12.0 s refused: $($a.reasons -join ',')"
    Expect (-not $b.ok -and ($b.reasons -contains 'STATE_STALE')) "age 12.0000001 s: $($b.reasons -join ',')"
    Expect (-not $c.ok -and ($c.reasons -contains 'STATE_FROM_FUTURE')) "future: $($c.reasons -join ',')"
}
Case 'S4' 'unsupported schema rejected' {
    $j = (Json (ReadyDoc)) -replace 'p9s-hostmon-state/1', 'p9s-hostmon-state/2'
    $r = Check $j
    Expect (-not $r.ok -and ($r.reasons -contains 'SCHEMA_UNSUPPORTED')) "$($r.reasons -join ',')"
}
Case 'S5' 'reader rejects every status but READY' {
    $doc = ReadyDoc
    foreach ($s in 'INITIALIZING', 'CALIBRATING', 'QUALIFYING', 'WARNING', 'STOP_REQUIRED', 'ABORT_REQUIRED', 'UNKNOWN', 'STALE', 'UNCALIBRATED', 'STOPPED', 'SOMETHING_ELSE') {
        $doc.status = $s
        $r = Check (Json $doc)
        Expect (-not $r.ok -and ($r.reasons -contains "STATUS_$s")) "$s accepted or wrong reason: $($r.reasons -join ',')"
    }
}
Case 'S6' 'missing file, dead monitor, test monitor, inconsistent READY: rejected' {
    $m = Read-StateFile -Path (Join-Path $sdir 'nothing-here.json')
    Expect (-not $m.ok -and $m.error -eq 'missing') "missing: $($m.error)"
    $g = Invoke-HostGate -StatePath (Join-Path $sdir 'nothing-here.json') -Step 'test'
    Expect (-not $g.ok -and ($g.reasons -contains 'STATE_MISSING')) "gate on missing: $($g.reasons -join ',')"
    $j = Json (ReadyDoc)
    $r = Check $j -p @{ IsMonitorAlive = $dead }
    Expect (-not $r.ok -and ($r.reasons -contains 'MONITOR_NOT_RUNNING')) "dead: $($r.reasons -join ',')"
    $ti = [ordered]@{}; foreach ($k in $monitorInfo.Keys) { $ti[$k] = $monitorInfo[$k] }; $ti.test = $true
    $t = Check (Json (ReadyDoc) (1000L * $F) $ti)
    Expect (-not $t.ok -and ($t.reasons -contains 'TEST_MONITOR')) "test monitor: $($t.reasons -join ',')"
    $d = ReadyDoc; $d.gate.qualifying_seconds = 299.0
    $x = Check (Json $d)
    Expect (-not $x.ok -and ($x.reasons -contains 'GATE_INCOMPLETE')) "gate: $($x.reasons -join ',')"
    $d = ReadyDoc; $d.commit.pct = $null
    $y = Check (Json $d)
    Expect (-not $y.ok -and ($y.reasons -contains 'COMMIT_INVALID')) "commit: $($y.reasons -join ',')"
    $z = Check ((Json (ReadyDoc)) -replace '"run_id"', '"run_x"')
    Expect (-not $z.ok -and ($z.reasons -contains 'FIELD_MISSING:run_id')) "field: $($z.reasons -join ',')"
}

# ---- the monitor process itself (fixture mode, a few seconds) -------------
$mon = Join-Path $kit 'host-monitor.ps1'
$fixture = Join-Path $OutDir 'fixture-ready.json'
@(Times 0 300 | ForEach-Object { $x = S $_; $x.disks = @{ '0' = $x.disks[0]; '2' = $x.disks[2] }; $x }) | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $fixture -Encoding utf8
$mstate = Join-Path $sdir 'monitor-state.json'
function Start-Mon([string]$name) {
    $o = Join-Path $OutDir $name
    Start-Process -FilePath (Get-Process -Id $PID).Path -PassThru -WindowStyle Hidden -ArgumentList @('-NoProfile', '-File', $mon, '-OutDir', $o, '-StatePath', $mstate, '-TestFixture', $fixture, '-TestIntervalMs', '50', '-PagingStop', 'NotRequired', '-DurationSeconds', '120')
}
function Wait-Gate([scriptblock]$until, [int]$seconds = 20) {
    $sw = [Diagnostics.Stopwatch]::StartNew()
    while ($sw.Elapsed.TotalSeconds -lt $seconds) { $g = Invoke-HostGate -StatePath $mstate -Step 'test' -PagingStop NotRequired -AllowTestMonitor; if (& $until $g) { return $g }; Start-Sleep -Milliseconds 100 }
    return $null
}
$script:runA = $null
Case 'S7' 'monitor crash: READY left behind is rejected' {
    $p = Start-Mon 'mon-A'
    $g = Wait-Gate { param($g) $g.ok }
    Expect ($null -ne $g) 'monitor never reached READY'
    if ($g) { $script:runA = $g.run_id }
    Stop-Process -Id $p.Id -Force; $p.WaitForExit(10000) | Out-Null
    $after = Invoke-HostGate -StatePath $mstate -Step 'test' -PagingStop NotRequired -AllowTestMonitor
    Expect ($after.status -eq 'READY') "file status after crash: $($after.status)"
    Expect (-not $after.ok -and ($after.reasons -contains 'MONITOR_NOT_RUNNING')) "after crash: $($after.reasons -join ',')"
}
Case 'S8' 'clean restart: fresh gate, old run refused, single instance, clean stop' {
    $p = Start-Mon 'mon-B'
    $g0 = Wait-Gate { param($g) $g.run_id -and $g.run_id -ne $script:runA }
    Expect ($null -ne $g0 -and -not $g0.ok) "new run accepted before qualifying: $($g0.status)"
    $old = Invoke-HostGate -StatePath $mstate -Step 'test' -PagingStop NotRequired -AllowTestMonitor -RunId $script:runA
    Expect (-not $old.ok -and ($old.reasons -contains 'RUN_ID_MISMATCH')) "old run id: $($old.reasons -join ',')"
    $g1 = Wait-Gate { param($g) $g.ok }
    Expect ($null -ne $g1) 'restarted monitor never reached READY'
    $second = Start-Mon 'mon-C'; $second.WaitForExit(20000) | Out-Null
    Expect ($second.ExitCode -eq 3) "second instance exit code $($second.ExitCode)"
    New-Item -ItemType File -Path (Join-Path $OutDir 'mon-B\STOP') | Out-Null
    Expect ($p.WaitForExit(15000)) 'monitor did not stop'
    $fin = Invoke-HostGate -StatePath $mstate -Step 'test' -PagingStop NotRequired -AllowTestMonitor
    Expect ($fin.status -eq 'STOPPED' -and -not $fin.ok) "final: $($fin.status) $($fin.ok)"
    Expect (-not (Test-Path -LiteralPath "$mstate.lock")) 'lock file left'
    Expect ((Test-Path -LiteralPath (Join-Path $OutDir 'mon-B\samples.csv')) -and (Test-Path -LiteralPath (Join-Path $OutDir 'mon-B\run-end.json'))) 'logs not flushed'
}

Case 'S9' 'monitor alerts once when a stop latches, and keeps refusing' {
    $fx2 = Join-Path $OutDir 'fixture-abort.json'
    @(Times 0 120 | ForEach-Object { $x = S $_ $(if ($_ -eq 60) { 97 } else { 50 }); $x.disks = @{ '0' = $x.disks[0]; '2' = $x.disks[2] }; $x }) | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $fx2 -Encoding utf8
    $o = Join-Path $OutDir 'mon-D'
    $sp2 = Join-Path $sdir 'monitor-state-abort.json'
    $p = Start-Process -FilePath (Get-Process -Id $PID).Path -PassThru -WindowStyle Hidden -ArgumentList @('-NoProfile', '-File', $mon, '-OutDir', $o, '-StatePath', $sp2, '-TestFixture', $fx2, '-TestIntervalMs', '50', '-PagingStop', 'NotRequired', '-DurationSeconds', '60')
    $sw = [Diagnostics.Stopwatch]::StartNew(); $seen = $null
    while ($sw.Elapsed.TotalSeconds -lt 20) { $g = Invoke-HostGate -StatePath $sp2 -Step 'test' -PagingStop NotRequired -AllowTestMonitor; if ($g.status -eq 'ABORT_REQUIRED') { $seen = $g; break }; Start-Sleep -Milliseconds 100 }
    Expect ($null -ne $seen -and -not $seen.ok) 'ABORT_REQUIRED never published'
    Start-Sleep -Milliseconds 1500
    $later = Invoke-HostGate -StatePath $sp2 -Step 'test' -PagingStop NotRequired -AllowTestMonitor
    Expect ($later.status -eq 'ABORT_REQUIRED' -and -not $later.ok) "after the fixture returned to 50 %: $($later.status)"
    New-Item -ItemType File -Path (Join-Path $o 'STOP') | Out-Null
    Expect ($p.WaitForExit(15000)) 'monitor did not stop'
    $alerts = @(Get-Content -LiteralPath (Join-Path $o 'events.log') | Where-Object { $_ -match ' ALERT ' })
    Expect ($alerts.Count -eq 1 -and $alerts[0] -match 'ABORT_REQUIRED') "alert lines: $($alerts.Count)"
}

# ---- gated action scripts (dry run only) ----------------------------------
Case 'E1' 'com-type refuses without a READY monitor; typed shutdown retired' {
    $ct = Join-Path $kit 'com-type.ps1'
    $msg = ''; try { & $ct -Vm P9S-D12 -Text 'abc' -StatePath (Join-Path $sdir 'nothing-here.json') -DryRun } catch { $msg = $_.Exception.Message }
    Expect ($msg -like '*HOST GATE REFUSED*STATE_MISSING*') "message: $msg"
    $msg = ''; try { & $ct -Vm P9S-D12 -Text 'abc' -StatePath $mstate -DryRun } catch { $msg = $_.Exception.Message }
    Expect ($msg -like '*HOST GATE REFUSED*') "stopped monitor: $msg"
    # stage C: shutting a guest down by typing into it is retired (ACPI instead)
    $msg = ''; try { & $ct -Vm P9S-D12 -Text 'rm all' -Shutdown -DryRun -StatePath (Join-Path $sdir 'nothing-here.json') } catch { $msg = $_.Exception.Message }
    Expect ($msg -like '*retired*emergency.acpi-shutdown*') "shutdown text: $msg"
    $msg = ''; try { & $ct -Vm P9S-D12 -Text 'systemctl poweroff' -Shutdown -DryRun -StatePath (Join-Path $sdir 'nothing-here.json') } catch { $msg = $_.Exception.Message }
    Expect ($msg -like '*retired*emergency.acpi-shutdown*') "typed shutdown: $msg"
}
Case 'E2' 'vm-action refuses without a READY monitor' {
    $va = Join-Path $kit 'vm-action.ps1'
    $msg = ''; try { & $va -Vm P9S-U24 -Action Start -StatePath $mstate -DryRun } catch { $msg = $_.Exception.Message }
    Expect ($msg -like '*HOST GATE REFUSED*') "start: $msg"
    $msg = ''; try { & $va -Vm P9S-U24 -Action Keys -Scancodes '1d 10 90 9d' -StatePath (Join-Path $sdir 'nothing-here.json') -DryRun } catch { $msg = $_.Exception.Message }
    Expect ($msg -like '*HOST GATE REFUSED*STATE_MISSING*') "keys: $msg"
    $msg = ''; try { & $va -Vm P9S-U24 -Action Keys -Scancodes 'zz' -StatePath $mstate -DryRun } catch { $msg = $_.Exception.Message }
    Expect ($msg -like '*HOST GATE REFUSED*' -or $msg -like '*scancode*') "bad scancodes: $msg"
}

# ---- syntax ---------------------------------------------------------------
Case 'X1' 'every kit script parses (pwsh 7)' {
    foreach ($f in Get-ChildItem -LiteralPath $kit -Recurse -Filter *.ps1) {
        $tok = $null; $err = $null
        $null = [System.Management.Automation.Language.Parser]::ParseFile($f.FullName, [ref]$tok, [ref]$err)
        Expect ($err.Count -eq 0) "$($f.Name): $($err | ForEach-Object Message)"
    }
}

$pass = @($results | Where-Object result -eq 'PASS').Count
$fail = @($results | Where-Object result -eq 'FAIL').Count
"== $pass passed, $fail failed, 0 skipped"
$results | ConvertTo-Json -Depth 3 | Set-Content -LiteralPath (Join-Path $OutDir 'results.json') -Encoding utf8
$results | ForEach-Object { '{0} {1} {2} ({3} ms){4}' -f $_.result, $_.id, $_.name, $_.ms, $(if ($_.detail) { ' :: ' + $_.detail } else { '' }) } | Set-Content -LiteralPath (Join-Path $OutDir 'results.txt') -Encoding utf8
if ($fail) { exit 1 } else { exit 0 }
