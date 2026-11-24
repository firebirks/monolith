#Requires -Version 7.0
# Synthetic, deterministic tests of the P9-S action coordinator
# (coordinator-lib.ps1, p9s-action.ps1, the com-type.ps1 / vm-action.ps1
# shims). No VM is started and no guest input is sent: in-process cases use
# a mock context (scripted monitor states, mocked VM states, a dispatcher
# that only records calls); the command-line cases use -DryRun, the
# simulated calibration, read-only VBoxManage queries, or a `wait` action
# that touches no VM. Monitor states are produced by the real policy of
# hostmon-lib.ps1 from synthetic samples.
#   pwsh -NoProfile -File tests\coordinator-tests.ps1 -OutDir <new folder>
param([Parameter(Mandatory)][string]$OutDir)
$ErrorActionPreference = 'Stop'
$kit = Split-Path $PSScriptRoot -Parent
. (Join-Path $kit 'hostmon-lib.ps1')
. (Join-Path $kit 'coordinator-lib.ps1')
if (Test-Path -LiteralPath $OutDir) { throw "$OutDir exists" }
New-Item -ItemType Directory -Path $OutDir | Out-Null
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
# A case that cannot run on this machine is recorded as NOT_RUN with the
# reason, never left out; the suite then exits 3 instead of 0.
function NotRun([string]$id, [string]$name, [string]$why) {
    $results.Add([pscustomobject]@{ id = $id; name = $name; result = 'NOT_RUN'; ms = 0; detail = $why })
    "NOT_RUN $id $name :: $why"
}

# ---- monitor states from the real policy ----------------------------------
$GiB = [math]::Pow(2, 30)
function S([double]$t, $commit = 50, [hashtable]$o = @{}) {
    $s = [ordered]@{
        seq = 0; mono = $t; utc = '2026-11-19T00:00:00.000Z'; read_ok = $true; read_error = $null
        committed_bytes = $(if ($null -eq $commit) { $null } else { [double]$commit * $GiB }); commit_limit_bytes = 100 * $GiB
        avail_mb = 16000.0; modified_list_mb = 200.0; pages = 100.0; pages_in = 100.0; pages_out = 0.0; page_reads = 90.0; page_writes = 0.0
        page_faults = 1000.0; transition_faults = 500.0; cache_faults = 100.0; demand_zero_faults = 300.0; rate_issues = @()
        pagefile_used_mb = 100.0; pagefile_alloc_mb = 5888.0; pagefile_pct = 1.7; pagefile_disks = @(0); system_disk = 0; external_disk = 2
        disks = @{ 0 = [pscustomobject]@{ read_ms = 0.5; write_ms = 0.3; read_mbps = 5.0; write_mbps = 1.0; queue = 0; avg_queue = 0.01 }
                   2 = [pscustomobject]@{ read_ms = 2.0; write_ms = 1.0; read_mbps = 0.1; write_mbps = 0.1; queue = 0; avg_queue = 0.01 } }
        external_present = $true; external_identity_ok = $true; eventlog_ok = $true; storage_events_new = 0; storage_event_ids = @(); cpu_pct = 10.0; vm = @{}
    }
    foreach ($k in $o.Keys) { $s[$k] = $o[$k] }
    [pscustomobject]$s
}
function Feed($st, [double]$from, [double]$to, $commit = 50) { $d = $null; for ($t = $from; $t -le $to + 1e-9; $t += 5) { $d = Step-Monitor -State $st -Sample (S $t $commit) }; $d }
$thr = [ordered]@{
    suspect = [ordered]@{ page_out_per_s = 500; avail_mb_low = 4096; avail_decline_mb = 4096; pagefile_growth_mb = 128; pagefile_disk_read_ms = 20; page_in_per_s_with_low_memory = 2000 }
    confirm = [ordered]@{ page_out_per_s = 2000; avail_mb_low = 2048; avail_decline_mb = 6144; pagefile_growth_mb = 300; pagefile_disk_read_ms = 50; page_in_per_s_with_low_memory = 5000 }
}
$fxApproved = ConvertTo-PagingThresholds -Json ([ordered]@{ schema = 'p9s-hostmon-paging/1'; approved = $true; approved_by = 'test fixture only'; approved_utc = '2026-11-19T00:00:00Z'; source = 'test'; suspect = $thr.suspect; confirm = $thr.confirm } | ConvertTo-Json -Depth 5)
$kitThr = Get-PagingThresholds -Path (Join-Path $kit 'paging-thresholds.json')
$F = 10000000L
$NOW = 1000L * $F
function Info($cfgMode = 'Monitor', $procId = 4242, $ticks = 1L) { [ordered]@{ pid = $procId; process_start_utc = 'x'; process_start_ticks = $ticks; mode = $cfgMode; paging_stop = 'Required'; thresholds_sha256 = 'x'; thresholds_approved = $true; test = $false; kit = 'test' } }
function J($doc, [double]$age = 1, $mode = 'Monitor') { ConvertTo-StateJson -Doc $doc -Monitor (Info $mode) -WrittenQpc ([long]($NOW - $age * $F)) -QpcFrequency $F -WrittenUtc 'x' }
function NewMon([string]$run, $th = $fxApproved, [string]$mode = 'Monitor') { New-MonitorState -RunId $run -Config (New-HostMonConfig -PagingStop Required -Mode $mode -Thresholds $th) }
$mA = NewMon 'run-A'; $READY_A = J (Feed $mA 0 300)
$STOP_A = J (Feed $mA 305 335 95)
$mA2 = NewMon 'run-A'; $null = Feed $mA2 0 300; $ABORT_A = J (Feed $mA2 305 305 97)
$mA3 = NewMon 'run-A'; $null = Feed $mA3 0 300; $WARN_A = J (Feed $mA3 305 305 92)
$mA4 = NewMon 'run-A'; $null = Feed $mA4 0 300; $UNKNOWN_A = J (Step-Monitor -State $mA4 -Sample (S 305 $null))
$READY_B = J (Feed (NewMon 'run-B') 0 300)
$UNCAL_A = J (Feed (NewMon 'run-A' $kitThr) 0 300)
$CALIB = J (Feed (NewMon 'run-C' $kitThr 'Calibrate') 0 300) 1 'Calibrate'
$STALE_A = J (Feed (NewMon 'run-A') 0 300) 13
$STOPPED_A = J (New-StatusOnlyDoc -State (NewMon 'run-A') -Status 'STOPPED' -Reasons @('MONITOR_STOPPED'))

# ---- mock context --------------------------------------------------------------
$script:ctxN = 0
function New-MockCtx([object[]]$States, [hashtable]$Vms = @{}, [bool]$Alive = $true) {
    $script:ctxN++
    $dir = Join-Path $OutDir ('ctx-{0:D2}' -f $script:ctxN); New-Item -ItemType Directory -Path $dir | Out-Null
    $calls = [System.Collections.Generic.List[object]]::new()
    $q = [System.Collections.Generic.Queue[object]]::new(); foreach ($x in $States) { $q.Enqueue($x) }
    $prov = @{ queue = $q; after = $null; before = $null; afterText = $null; reads = 0 }
    $vmTable = @{ 'P9S-D12' = 'off'; 'P9S-U24' = 'off' }; foreach ($k in $Vms.Keys) { $vmTable[$k] = $Vms[$k] }
    $readState = {
        param($p)
        $prov.reads++
        if ($null -ne $prov.after) { $x = if ($calls.Count -ge $prov.after) { $prov.afterText } else { $prov.before } }
        elseif ($prov.queue.Count -gt 1) { $x = $prov.queue.Dequeue() } else { $x = $prov.queue.Peek() }
        if ($null -eq $x) { [pscustomobject]@{ ok = $false; text = $null; error = 'missing' } } else { [pscustomobject]@{ ok = $true; text = $x; error = $null } }
    }.GetNewClosure()
    $aliveFn = if ($Alive) { { param($a, $b) $true } } else { { param($a, $b) $false } }
    $rec = { param($kind, $list) { param($vm, $arg) $list.Add([pscustomobject]@{ kind = $kind; vm = $vm; arg = $arg }) }.GetNewClosure() }
    # ACPI mock: records the request; the guest then powers off
    $acpi = { param($list, $tbl) { param($vm) $list.Add([pscustomobject]@{ kind = 'vm.acpi'; vm = $vm; arg = $null }); $tbl[$vm] = 'off' }.GetNewClosure() }
    [pscustomobject]@{
        StatePath = Join-Path $dir 'state.json'; SessionPath = Join-Path $dir 'coordinator-session.json'; LockPath = Join-Path $dir 'coordinator.lock'
        StopRequestPath = Join-Path $dir 'coordinator-stop.request'; AuditPath = Join-Path $dir 'coordinator-audit.jsonl'; ThresholdsPath = Join-Path $kit 'paging-thresholds.json'
        ReadState = $readState; NowQpc = { $NOW }.GetNewClosure(); QpcFrequency = $F; IsMonitorAlive = $aliveFn
        ReadVmState = { param($vm) $vmTable[$vm] }.GetNewClosure(); ReadVmConfig = { param($vm) @() }
        Dispatch = @{ 'vm.start' = (& $rec 'vm.start' $calls); 'guest.text' = (& $rec 'guest.text' $calls); 'guest.key' = (& $rec 'guest.key' $calls); 'guest.unlock' = (& $rec 'guest.unlock' $calls); 'vm.acpi' = (& $acpi $calls $vmTable) }
        Sleep = { param($ms) }; Pid = $PID; ProcessStartTicks = (Get-Process -Id $PID).StartTime.ToUniversalTime().Ticks
        EmergencyLockWaitMs = 1500; CalibrationHostChecks = { @() }
        Calls = $calls; Prov = $prov; VmTable = $vmTable; Dir = $dir
    }
}
function Act($ctx, [hashtable]$r) { Invoke-P9SAction -Ctx $ctx -Request $r }
function Open($ctx) { Act $ctx @{ Action = 'session.open'; Kind = 'Test' } }
function Has($res, [string]$code) { @($res.reasons) -contains $code }
function Audit($ctx) { if (Test-Path -LiteralPath $ctx.AuditPath) { Get-Content -LiteralPath $ctx.AuditPath -Raw } else { '' } }
function Sess($ctx) { Get-Content -LiteralPath $ctx.SessionPath -Raw | ConvertFrom-Json }

"== P9-S coordinator synthetic tests, $((Get-Date).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ')), pwsh $($PSVersionTable.PSVersion)"

# ---- 1-11: state enforcement ------------------------------------------------
Case 'K01' 'READY allows an approved normal action (exact dispatch)' {
    $c = New-MockCtx @($READY_A) @{ 'P9S-D12' = 'running' }
    $o = Open $c; Expect ($o.outcome -eq 'DONE') "open: $($o.outcome) $($o.reasons -join ',')"
    $r = Act $c @{ Action = 'guest.key'; Vm = 'P9S-D12'; Key = 'Enter' }
    Expect ($r.outcome -eq 'DISPATCHED' -and $c.Calls.Count -eq 1) "key: $($r.outcome) calls $($c.Calls.Count) $($r.reasons -join ',')"
    Expect ((@($c.Calls[0].arg) -join ' ') -eq '28 156' -and $c.Calls[0].vm -eq 'P9S-D12') "codes $(@($c.Calls[0].arg) -join ' ')"
}
Case 'K02' 'UNCALIBRATED rejects acceptance-test actions' {
    $c = New-MockCtx @($UNCAL_A)
    $o = Open $c; Expect ($o.outcome -eq 'REFUSED' -and (Has $o 'STATUS_UNCALIBRATED')) "open: $($o.outcome) $($o.reasons -join ',')"
    $r = Act $c @{ Action = 'scenario.begin'; Name = 't-clean-15' }
    Expect ($r.outcome -eq 'REFUSED' -and (Has $r 'STATUS_UNCALIBRATED') -and $c.Calls.Count -eq 0) "begin: $($r.outcome) $($r.reasons -join ',')"
}
Case 'K03' 'UNKNOWN rejects an action' {
    $c = New-MockCtx @($READY_A, $READY_A, $UNKNOWN_A) @{ 'P9S-D12' = 'running' }; $null = Open $c
    $r = Act $c @{ Action = 'guest.type'; Vm = 'P9S-D12'; Text = 'abc' }
    Expect ($r.outcome -eq 'REFUSED' -and (Has $r 'STATUS_UNKNOWN') -and $c.Calls.Count -eq 0) "$($r.outcome) $($r.reasons -join ',')"
}
Case 'K04' 'STALE rejects an action' {
    $c = New-MockCtx @($READY_A, $READY_A, $STALE_A) @{ 'P9S-D12' = 'running' }; $null = Open $c
    $r = Act $c @{ Action = 'guest.key'; Vm = 'P9S-D12'; Key = 'Tab' }
    Expect ($r.outcome -eq 'REFUSED' -and (Has $r 'STATE_STALE') -and $c.Calls.Count -eq 0) "$($r.outcome) $($r.reasons -join ',')"
}
Case 'K05' 'STOP_REQUIRED rejects new test actions and halts the session' {
    $c = New-MockCtx @($READY_A, $READY_A, $STOP_A) @{ 'P9S-D12' = 'running' }; $null = Open $c
    $r = Act $c @{ Action = 'guest.key'; Vm = 'P9S-D12'; Key = 'Enter' }
    Expect ($r.outcome -eq 'REFUSED' -and (Has $r 'STATUS_STOP_REQUIRED') -and $c.Calls.Count -eq 0) "$($r.outcome) $($r.reasons -join ',')"
    $s = Sess $c; Expect ($s.lifecycle -eq 'HALTED_STOP' -and $s.halt.status -eq 'STOP_REQUIRED' -and @($s.halt.reasons) -contains 'COMMIT_SUSTAINED_95') "session $($s.lifecycle) $($s.halt.reasons -join ',')"
    $c.Prov.queue.Clear(); $c.Prov.queue.Enqueue($READY_A)
    $r2 = Act $c @{ Action = 'guest.key'; Vm = 'P9S-D12'; Key = 'Enter' }
    Expect ($r2.outcome -eq 'REFUSED' -and (Has $r2 'SESSION_HALTED_STOP')) "after STOP, READY again: $($r2.outcome) $($r2.reasons -join ',')"
}
Case 'K06' 'ABORT_REQUIRED rejects new test actions and records the first reason' {
    $c = New-MockCtx @($READY_A, $READY_A, $ABORT_A) @{ 'P9S-D12' = 'running' }; $null = Open $c
    $r = Act $c @{ Action = 'vm.start'; Vm = 'P9S-U24' }
    Expect ($r.outcome -eq 'REFUSED' -and (Has $r 'STATUS_ABORT_REQUIRED') -and $c.Calls.Count -eq 0) "$($r.outcome) $($r.reasons -join ',')"
    $s = Sess $c; Expect ($s.lifecycle -eq 'HALTED_ABORT' -and @($s.halt.reasons) -contains 'COMMIT_CRITICAL_97' -and $s.halt.abort_id -match '^abort-[0-9a-f]{12}$') "session $($s.lifecycle) $($s.halt.abort_id)"
    Expect ((Audit $c) -match '"outcome":"HALTED"') 'no HALTED audit record'
}
Case 'K07' 'missing state rejects an action' {
    $c = New-MockCtx @($null)
    $r = Open $c; Expect ($r.outcome -eq 'REFUSED' -and (Has $r 'STATE_MISSING')) "$($r.outcome) $($r.reasons -join ',')"
}
Case 'K08' 'invalid JSON rejects an action' {
    $c = New-MockCtx @('{"schema": "p9s-hostmon-state/1", "status": "READY"')
    $r = Open $c; Expect ($r.outcome -eq 'REFUSED' -and (Has $r 'STATE_INVALID_JSON')) "$($r.outcome) $($r.reasons -join ',')"
}
Case 'K09' 'unexpected monitor instance rejects an action' {
    $c = New-MockCtx @($READY_A) @{} $false
    $r = Open $c; Expect ($r.outcome -eq 'REFUSED' -and (Has $r 'MONITOR_NOT_RUNNING')) "$($r.outcome) $($r.reasons -join ',')"
}
Case 'K10' 'monitor restart invalidates the qualification of the session' {
    $c = New-MockCtx @($READY_A, $READY_A, $READY_B) @{ 'P9S-D12' = 'running' }; $null = Open $c
    $r = Act $c @{ Action = 'guest.key'; Vm = 'P9S-D12'; Key = 'Enter' }
    Expect ($r.outcome -eq 'REFUSED' -and (Has $r 'MONITOR_RUN_CHANGED') -and $c.Calls.Count -eq 0) "$($r.outcome) $($r.reasons -join ',')"
}
Case 'K11' 'commit WARNING blocks new high-risk actions' {
    $c = New-MockCtx @($READY_A, $READY_A, $WARN_A); $null = Open $c
    $r = Act $c @{ Action = 'vm.start'; Vm = 'P9S-D12' }
    Expect ($r.outcome -eq 'REFUSED' -and (Has $r 'STATUS_WARNING') -and $c.Calls.Count -eq 0) "$($r.outcome) $($r.reasons -join ',')"
    $s = Sess $c; Expect ($s.lifecycle -eq 'ACTIVE') "a WARNING must block, not halt: $($s.lifecycle)"
}

# ---- 12-18: emergency path, allowlists, injection ----------------------------
Case 'K12' 'STOP keeps the emergency shutdown available' {
    $c = New-MockCtx @($STOP_A) @{ 'P9S-D12' = 'running' }
    $r = Act $c @{ Action = 'emergency.acpi-shutdown'; Vm = 'P9S-D12' }
    Expect ($r.outcome -eq 'DONE' -and $c.Calls.Count -eq 1 -and $c.Calls[0].kind -eq 'vm.acpi') "$($r.outcome) $($r.reasons -join ',')"
    Expect (Test-Path -LiteralPath $c.StopRequestPath) 'no stop request written'
}
Case 'K13' 'ABORT keeps the emergency shutdown available, also with no monitor at all' {
    $c = New-MockCtx @($ABORT_A) @{ 'P9S-U24' = 'running' }
    $r = Act $c @{ Action = 'emergency.acpi-shutdown'; Vm = 'P9S-U24' }
    Expect ($r.outcome -eq 'DONE' -and $c.Calls.Count -eq 1) "abort: $($r.outcome) $($r.reasons -join ',')"
    $c2 = New-MockCtx @($null) @{ 'P9S-U24' = 'running'; 'P9S-D12' = 'running' }
    $r2 = Act $c2 @{ Action = 'emergency.acpi-shutdown'; Vm = 'P9S-U24' }
    Expect ($r2.outcome -eq 'DONE' -and $c2.Calls.Count -eq 1) "missing state: $($r2.outcome)"
    $w = Act $c2 @{ Action = 'emergency.wait-off'; Vm = 'P9S-D12'; Seconds = 4 }
    Expect ($w.outcome -eq 'TIMEOUT') "wait-off while running: $($w.outcome)"
    $c2.VmTable['P9S-D12'] = 'off'
    $w2 = Act $c2 @{ Action = 'emergency.wait-off'; Vm = 'P9S-D12'; Seconds = 4 }
    Expect ($w2.outcome -eq 'DONE') "wait-off when off: $($w2.outcome)"
}
Case 'K14' 'emergency commands cannot start a VM' {
    $c = New-MockCtx @($STOP_A)
    foreach ($a in 'emergency.vm-start', 'emergency.start', 'emergency.guest-poweron') {
        $r = Act $c @{ Action = $a; Vm = 'P9S-D12' }; Expect ($r.outcome -eq 'REJECTED' -and (Has $r 'UNKNOWN_ACTION')) "$a $($r.outcome)"
    }
    $r = Act $c @{ Action = 'vm.start'; Vm = 'P9S-D12' }
    Expect ($r.outcome -eq 'REFUSED' -and $c.Calls.Count -eq 0) "vm.start under STOP: $($r.outcome)"
}
Case 'K15' 'emergency commands cannot send messages or other text' {
    $c = New-MockCtx @($STOP_A) @{ 'P9S-D12' = 'running' }
    $r = Act $c @{ Action = 'emergency.acpi-shutdown'; Vm = 'P9S-D12'; Text = 'hello peer' }
    Expect ($r.outcome -eq 'REJECTED' -and (Has $r 'PARAMETER_NOT_ALLOWED:Text') -and $c.Calls.Count -eq 0) "$($r.outcome) $($r.reasons -join ',')"
    $r2 = Act $c @{ Action = 'emergency.acpi-shutdown'; Vm = 'P9S-D12' }
    Expect ($r2.outcome -eq 'DONE' -and $null -eq $c.Calls[0].arg) "the shutdown dispatch must carry no text: $($c.Calls[0].arg)"
}
Case 'K16' 'unrecognized action rejected' {
    $c = New-MockCtx @($READY_A)
    foreach ($a in 'vm.poweroff', 'vm.reset', 'snapshot.restore', 'shell', 'VBoxManage', '', 'guest.Type') {
        $r = Act $c @{ Action = $a; Vm = 'P9S-D12' }; Expect ($r.outcome -eq 'REJECTED' -and (Has $r 'UNKNOWN_ACTION')) "'$a' $($r.outcome)"
    }
    Expect ($c.Calls.Count -eq 0 -and $c.Prov.reads -eq 0) "rejections read state or dispatched: reads $($c.Prov.reads) calls $($c.Calls.Count)"
}
Case 'K17' 'unrecognized VM target rejected' {
    $c = New-MockCtx @($READY_A)
    foreach ($v in 'P9S-X', 'p9s-d12', 'P9S-D12 ', 'Windows11', '', 'P9S-D12,P9S-U24') {
        $r = Act $c @{ Action = 'vm.start'; Vm = $v }; Expect ($r.outcome -eq 'REJECTED' -and ((Has $r 'VM_NOT_ALLOWED') -or (Has $r 'PARAMETER_MISSING:Vm'))) "'$v' $($r.outcome) $($r.reasons -join ',')"
    }
    Expect ($c.Calls.Count -eq 0) 'dispatched'
}
Case 'K18' 'command injection rejected; dispatch uses argument arrays' {
    $c = New-MockCtx @($READY_A) @{ 'P9S-D12' = 'running' }; $null = Open $c
    $bad = @(
        @{ Action = 'vm.start; calc'; Vm = 'P9S-D12' }, @{ Action = 'vm.start'; Vm = 'P9S-D12"; calc' },
        @{ Action = 'guest.type'; Vm = 'P9S-D12'; Text = 'ls; rm -rf /' }, @{ Action = 'guest.type'; Vm = 'P9S-D12'; Text = 'a|b' },
        @{ Action = 'guest.type'; Vm = 'P9S-D12'; Text = '$(calc)' }, @{ Action = 'guest.type'; Vm = 'P9S-D12'; Text = ('a' * 121) },
        @{ Action = 'guest.key'; Vm = 'P9S-D12'; Key = '1c 9c' }, @{ Action = 'scenario.begin'; Name = '..\x' },
        @{ Action = 'guest.key'; Vm = 'P9S-D12'; Key = 'Enter'; Extra = 'x' }
    )
    foreach ($b in $bad) { $r = Act $c $b; Expect ($r.outcome -eq 'REJECTED') "accepted: $(($b.GetEnumerator() | ForEach-Object { "$($_.Key)=$($_.Value)" }) -join ' ')" }
    Expect ($c.Calls.Count -eq 0) "dispatched $($c.Calls.Count)"
    $sa = Get-VBoxStartArgs 'P9S-D12'
    Expect ($sa -is [array] -and ($sa -join '|') -eq 'startvm|P9S-D12|--type|gui') "start args $($sa -join '|')"
    Expect ((Get-P9SScancodesForText 'a-b') -is [array]) 'text is not turned into scan-code arrays'
}

# ---- 19-21: checkpoints, queue, time of check ----------------------------------
Case 'K19' 'long-running actions recheck safety at checkpoints' {
    $c = New-MockCtx @($READY_A) @{ 'P9S-D12' = 'running' }; $null = Open $c
    $c.Prov.after = 1; $c.Prov.before = $READY_A; $c.Prov.afterText = $STOP_A
    $r = Act $c @{ Action = 'guest.type'; Vm = 'P9S-D12'; Text = 'abcdefghijklmnopqrstuvwx' }
    Expect ($r.outcome -eq 'STOPPED_AT_CHECKPOINT' -and $c.Calls.Count -eq 1) "type: $($r.outcome) chunks $($c.Calls.Count)"
    Expect ((Sess $c).lifecycle -eq 'HALTED_STOP') 'not halted'
    $c2 = New-MockCtx @($READY_A); $null = Open $c2
    $c2.Prov.after = $null; $c2.Prov.reads = 0; $c2.Prov.queue.Clear(); foreach ($x in @($READY_A, $READY_A, $READY_A, $READY_A, $WARN_A)) { $c2.Prov.queue.Enqueue($x) }
    $w = Act $c2 @{ Action = 'wait'; Seconds = 60 }
    Expect ($w.outcome -eq 'STOPPED_AT_CHECKPOINT' -and (Has $w 'STATUS_WARNING')) "wait: $($w.outcome) $($w.reasons -join ',')"
    Expect ($c2.Prov.reads -le 6) "wait kept reading after the stop: $($c2.Prov.reads)"
}
Case 'K20' 'queued actions do not run after STOP' {
    $c = New-MockCtx @($READY_A) @{ 'P9S-D12' = 'running' }; $null = Open $c
    $plan = Join-Path $c.Dir 'plan.json'
    [ordered]@{ schema = 'p9s-plan/1'; name = 'three-keys'; steps = @(@{ Action = 'guest.key'; Vm = 'P9S-D12'; Key = 'Tab' }, @{ Action = 'guest.key'; Vm = 'P9S-D12'; Key = 'Tab' }, @{ Action = 'guest.key'; Vm = 'P9S-D12'; Key = 'Enter' }) } | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $plan
    $c.Prov.after = 1; $c.Prov.before = $READY_A; $c.Prov.afterText = $STOP_A
    $r = Act $c @{ Action = 'sequence.run'; Plan = $plan }
    Expect ($r.outcome -eq 'STOPPED_AT_CHECKPOINT' -and $c.Calls.Count -eq 1) "$($r.outcome) dispatched $($c.Calls.Count)"
    $cancelled = @((Audit $c) -split "`n" | Where-Object { $_ -match '"outcome":"CANCELLED"' })
    Expect ($cancelled.Count -eq 2) "cancelled records $($cancelled.Count)"
}
Case 'K21' 'state change between validation and dispatch: nothing dispatched' {
    $c = New-MockCtx @($READY_A) @{ 'P9S-D12' = 'running' }; $null = Open $c
    $c.Prov.queue.Clear(); $c.Prov.queue.Enqueue($READY_A); $c.Prov.queue.Enqueue($ABORT_A)
    $r = Act $c @{ Action = 'guest.key'; Vm = 'P9S-D12'; Key = 'Enter' }
    Expect ($r.outcome -eq 'REFUSED' -and (Has $r 'STATE_CHANGED_BEFORE_DISPATCH') -and $c.Calls.Count -eq 0) "$($r.outcome) $($r.reasons -join ',') calls $($c.Calls.Count)"
    Expect ((Sess $c).lifecycle -eq 'HALTED_ABORT') 'abort seen at the last check not recorded'
}

# ---- 22: out-of-band changes -----------------------------------------------------
Case 'K22' 'out-of-band VM changes are reported' {
    $c = New-MockCtx @($READY_A); $null = Open $c
    $c.VmTable['P9S-U24'] = 'running'
    $r = Act $c @{ Action = 'vm.start'; Vm = 'P9S-D12' }
    Expect ($r.outcome -eq 'REFUSED' -and (Has $r 'OUT_OF_BAND_VM_STATE:P9S-U24') -and $c.Calls.Count -eq 0) "live: $($r.outcome) $($r.reasons -join ',')"
    $samples = @(
        [pscustomobject]@{ utc = '2026-11-19T10:00:00.000Z'; d12_state = 'off'; u24_state = 'off' },
        [pscustomobject]@{ utc = '2026-11-19T10:00:05.000Z'; d12_state = 'running'; u24_state = 'off' },
        [pscustomobject]@{ utc = '2026-11-19T10:00:10.000Z'; d12_state = 'running'; u24_state = 'running' },
        [pscustomobject]@{ utc = '2026-11-19T10:05:00.000Z'; d12_state = 'aborted'; u24_state = 'running' })
    $audit = @([pscustomobject]@{ utc = '2026-11-19T10:00:03.000Z'; action = 'vm.start'; vm = 'P9S-D12'; outcome = 'DISPATCHED' })
    $ev = @(Find-OutOfBandVmChanges -Samples $samples -Audit $audit)
    $un = @($ev | Where-Object { -not $_.explained })
    Expect ($ev.Count -eq 3 -and $un.Count -eq 2) "events $($ev.Count) unexplained $($un.Count)"
    Expect (@($un | Where-Object { $_.vm -eq 'P9S-U24' -and $_.to -eq 'running' }).Count -eq 1 -and @($un | Where-Object { $_.to -eq 'aborted' }).Count -eq 1) 'wrong events'
    $ref = @('name="P9S-D12"', 'memory=4096', 'nictrace1="off"', 'VMState="poweroff"', 'VMStateChangeTime="2026-11-19T12:41:00.000000000"')
    $cur = @('name="P9S-D12"', 'memory=8192', 'nictrace1="on"', 'VMState="running"', 'VMStateChangeTime="2026-11-19T13:00:00.000000000"')
    $d = @(Compare-VmConfig -Reference $ref -Current $cur)
    Expect ($d.Count -eq 2 -and @($d | Where-Object capture).Count -eq 1) "config diff $(($d | ForEach-Object key) -join ',')"
}

# ---- 23-25: calibration --------------------------------------------------------
Case 'K23' 'calibration capability cannot launch acceptance scenarios' {
    $c = New-MockCtx @($CALIB) @{ 'P9S-D12' = 'running' }
    $o = Act $c @{ Action = 'session.open'; Kind = 'Calibration' }
    Expect ($o.outcome -eq 'REFUSED' -and (Has $o 'CALIBRATION_LIVE_DISABLED')) "live calibration session: $($o.outcome) $($o.reasons -join ',')"
    $o2 = Act $c @{ Action = 'session.open'; Kind = 'Calibration'; DryRun = $true }
    Expect ($o2.outcome -eq 'DRYRUN' -and -not (Test-Path -LiteralPath $c.SessionPath)) "dry-run calibration session: $($o2.outcome)"
    foreach ($a in @(@{ Action = 'scenario.begin'; Name = 'x' }, @{ Action = 'guest.type'; Vm = 'P9S-D12'; Text = 'hello' }, @{ Action = 'session.open'; Kind = 'Test' })) {
        $r = Act $c $a; Expect ($r.outcome -eq 'REFUSED' -and ((Has $r 'STATUS_UNCALIBRATED') -or (Has $r 'STATUS_CALIBRATING'))) "$($a.Action) under calibration: $($r.outcome) $($r.reasons -join ',')"
    }
    $steps = @(Get-P9SCalibrationStepKinds)
    Expect (-not (@($steps | Where-Object { $_ -match 'type|message|scenario|transfer|interrupt|unlock' }).Count)) "calibration step kinds: $($steps -join ',')"
    Expect ($c.Calls.Count -eq 0) 'dispatched'
}
Case 'K24' 'normal capability cannot be obtained from UNCALIBRATED or CALIBRATING' {
    $calNotReq = J (Feed (New-MonitorState -RunId 'run-D' -Config (New-HostMonConfig -PagingStop NotRequired -Mode Calibrate -Thresholds $fxApproved)) 0 300) 1 'Calibrate'
    Expect (($calNotReq | ConvertFrom-Json).status -eq 'CALIBRATING') 'fixture is not CALIBRATING'
    foreach ($s in $UNCAL_A, $CALIB, $calNotReq) {
        $c = New-MockCtx @($s); $r = Open $c
        Expect ($r.outcome -eq 'REFUSED' -and -not (Test-Path -LiteralPath $c.SessionPath)) "$($r.outcome) $($r.reasons -join ',')"
    }
}
Case 'K25' 'calibration dry run: whole workflow simulated, no real dispatch' {
    $n = Invoke-P9SCalibrationDryRun -Scenario nominal -Thresholds $kitThr
    Expect ($n.verdict -eq 'COMPLETED' -and $n.real_dispatches -eq 0) "nominal: $($n.verdict) $($n.halted_at)"
    Expect (@($n.steps | Where-Object { $_.step -eq 'qualify-300s' -and $_.outcome -eq 'PASS' }).Count -eq 1) 'no 300-s qualification after Tor'
    Expect (@($n.pending).Count -ge 1) 'pending definitions not reported'
    $w = Invoke-P9SCalibrationDryRun -Scenario warning-during-start -Thresholds $kitThr
    Expect ($w.verdict -eq 'HALTED' -and $w.halted_at -like 'settle P9S-U24*' -and @($w.reasons) -contains 'STATUS_WARNING') "warning: $($w.verdict) at $($w.halted_at) $($w.reasons -join ',')"
    Expect (@($w.steps | Where-Object { $_.outcome -eq 'CANCELLED' }).Count -ge 3 -and @($w.steps | Where-Object { $_.step -like 'acpi-shutdown*' -and $_.outcome -like 'EMERGENCY*' }).Count -eq 2) 'cancel / shutdown path'
    $a = Invoke-P9SCalibrationDryRun -Scenario abort-during-observe -Thresholds $kitThr
    Expect ($a.verdict -eq 'HALTED' -and $a.halted_at -eq 'observe' -and (@($a.reasons) -match 'ABORT').Count -ge 1) "abort: $($a.verdict) $($a.halted_at) $($a.reasons -join ',')"
    $m = Invoke-P9SCalibrationDryRun -Scenario monitor-mode -Thresholds $kitThr
    Expect ($m.verdict -eq 'HALTED' -and $m.halted_at -eq 'preflight' -and @($m.reasons) -contains 'CALIBRATION_REQUIRES_CALIBRATE_MODE') "monitor mode: $($m.verdict) $($m.reasons -join ',')"
    $s = Invoke-P9SCalibrationDryRun -Scenario stale-monitor -Thresholds $kitThr
    Expect ($s.verdict -eq 'HALTED' -and @($s.reasons) -contains 'STATE_STALE') "stale: $($s.verdict) $($s.reasons -join ',')"
}

# ---- 26-28: partial commands, privacy, crash -------------------------------------
Case 'K26' 'failed checks execute nothing, not even part of a plan' {
    $c = New-MockCtx @($READY_A) @{ 'P9S-D12' = 'running' }; $null = Open $c
    $plan = Join-Path $c.Dir 'plan-bad.json'
    [ordered]@{ schema = 'p9s-plan/1'; name = 'bad'; steps = @(@{ Action = 'guest.key'; Vm = 'P9S-D12'; Key = 'Tab' }, @{ Action = 'guest.key'; Vm = 'P9S-D12'; Key = 'Tab' }, @{ Action = 'guest.type'; Vm = 'P9S-D12'; Text = 'a;b' }) } | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $plan
    $r = Act $c @{ Action = 'sequence.run'; Plan = $plan }
    Expect ($r.outcome -eq 'REJECTED' -and $c.Calls.Count -eq 0) "plan with a bad third step: $($r.outcome) dispatched $($c.Calls.Count)"
    $plan2 = Join-Path $c.Dir 'plan-nested.json'
    [ordered]@{ schema = 'p9s-plan/1'; name = 'nested'; steps = @(@{ Action = 'emergency.acpi-shutdown'; Vm = 'P9S-D12' }) } | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $plan2
    $r2 = Act $c @{ Action = 'sequence.run'; Plan = $plan2 }
    Expect ($r2.outcome -eq 'REJECTED' -and $c.Calls.Count -eq 0) "plan with a non-test step: $($r2.outcome)"
    $c3 = New-MockCtx @($STOP_A) @{ 'P9S-D12' = 'running' }
    foreach ($a in @(@{ Action = 'vm.start'; Vm = 'P9S-U24' }, @{ Action = 'guest.type'; Vm = 'P9S-D12'; Text = 'abc' }, @{ Action = 'guest.unlock'; Vm = 'P9S-D12' })) { $null = Act $c3 $a }
    Expect ($c3.Calls.Count -eq 0) "dispatched under STOP: $($c3.Calls.Count)"
}
Case 'K27' 'logs hold no message content and no secret' {
    $c = New-MockCtx @($READY_A) @{ 'P9S-D12' = 'running' }; $null = Open $c
    $null = Act $c @{ Action = 'guest.type'; Vm = 'P9S-D12'; Text = 'SECRETMESSAGE123 xyz' }
    $null = Act $c @{ Action = 'guest.unlock'; Vm = 'P9S-D12' }
    $null = Act $c @{ Action = 'scenario.begin'; Name = 'Bad Name With Words' }
    $log = Audit $c
    Expect ($log -notmatch 'SECRETMESSAGE' -and $log -notmatch 'xyz' -and $log -notmatch 'With Words') 'content in the audit log'
    Expect ($log -match '"text_length":20') 'text length not recorded'
    Expect ($c.Calls.Count -eq 4 -and $c.Calls[3].kind -eq 'guest.unlock' -and $null -eq $c.Calls[3].arg) "unlock dispatch must carry no secret (calls $($c.Calls.Count))"
    Expect (-not (@(Get-ChildItem -LiteralPath $c.Dir -Recurse -File | Where-Object { (Get-Content -Raw -LiteralPath $_.FullName) -match 'SECRETMESSAGE' }).Count)) 'content in a coordinator file'
}
Case 'K28' 'coordinator crash: unresolved in-flight action is never taken as safe' {
    $c = New-MockCtx @($READY_A) @{ 'P9S-D12' = 'running' }; $null = Open $c
    $s = Get-Content -LiteralPath $c.SessionPath -Raw | ConvertFrom-Json -AsHashtable
    $s.inflight = @{ action = 'guest.type'; vm = 'P9S-D12'; pid = 999999; start_ticks = 1; utc = 'x' }
    Write-StateAtomic -Path $c.SessionPath -Text ($s | ConvertTo-Json -Depth 6)
    $r = Act $c @{ Action = 'guest.key'; Vm = 'P9S-D12'; Key = 'Enter' }
    Expect ($r.outcome -eq 'REFUSED' -and (Has $r 'INFLIGHT_UNRESOLVED') -and $c.Calls.Count -eq 0) "$($r.outcome) $($r.reasons -join ',')"
    $e = Act $c @{ Action = 'emergency.acpi-shutdown'; Vm = 'P9S-D12' }
    Expect ($e.outcome -eq 'DONE') "emergency with in-flight record: $($e.outcome)"
    $cl = Act $c @{ Action = 'session.close' }
    Expect ($cl.outcome -eq 'DONE' -and $null -eq (Sess $c).inflight) 'close did not resolve the in-flight record'
}

# ---- reliability extras ---------------------------------------------------------------
Case 'K29' 'a second coordinator is refused at once (no wait, no deadlock)' {
    $c = New-MockCtx @($READY_A); $null = Open $c
    $hold = [IO.FileStream]::new($c.LockPath, [IO.FileMode]::OpenOrCreate, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
    try {
        $sw = [Diagnostics.Stopwatch]::StartNew(); $r = Act $c @{ Action = 'scenario.begin'; Name = 'x' }
        Expect ($r.outcome -eq 'REFUSED' -and (Has $r 'COORDINATOR_BUSY') -and $sw.ElapsedMilliseconds -lt 1000) "$($r.outcome) $($sw.ElapsedMilliseconds) ms"
    } finally { $hold.Dispose() }
}
Case 'K30' 'emergency shutdown vs a busy coordinator: bounded wait, never interleaved' {
    $c = New-MockCtx @($READY_A) @{ 'P9S-D12' = 'running' }
    $job = Start-ThreadJob -ArgumentList $c.LockPath, $c.StopRequestPath -ScriptBlock {
        param($lockPath, $req)
        $h = [IO.FileStream]::new($lockPath, [IO.FileMode]::OpenOrCreate, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
        $sw = [Diagnostics.Stopwatch]::StartNew()
        while (-not (Test-Path -LiteralPath $req) -and $sw.ElapsedMilliseconds -lt 10000) { Start-Sleep -Milliseconds 50 }
        Start-Sleep -Milliseconds 300; $h.Dispose()
    }
    Start-Sleep -Milliseconds 500
    $r = Act $c @{ Action = 'emergency.acpi-shutdown'; Vm = 'P9S-D12' }
    Receive-Job $job -Wait | Out-Null; Remove-Job $job
    Expect ($r.outcome -eq 'DONE' -and $c.Calls.Count -eq 1) "holder that yields: $($r.outcome) $($r.reasons -join ',')"
    $c2 = New-MockCtx @($READY_A) @{ 'P9S-D12' = 'running' }
    $hold = [IO.FileStream]::new($c2.LockPath, [IO.FileMode]::OpenOrCreate, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
    try { $r2 = Act $c2 @{ Action = 'emergency.acpi-shutdown'; Vm = 'P9S-D12' } } finally { $hold.Dispose() }
    Expect ($r2.outcome -eq 'REFUSED' -and (Has $r2 'EMERGENCY_LOCK_TIMEOUT') -and $c2.Calls.Count -eq 0) "holder that never yields: $($r2.outcome)"
}
Case 'K31' 'ABORT lifecycle cannot be dismissed by a normal action' {
    $c = New-MockCtx @($READY_A, $READY_A, $ABORT_A) @{ 'P9S-D12' = 'running' }; $null = Open $c
    $null = Act $c @{ Action = 'guest.key'; Vm = 'P9S-D12'; Key = 'Enter' }
    $id = (Sess $c).halt.abort_id
    $null = Act $c @{ Action = 'session.close' }
    $c.Prov.queue.Clear(); $c.Prov.queue.Enqueue($READY_B)
    $o1 = Open $c; Expect ($o1.outcome -eq 'REFUSED' -and (Has $o1 'ABORT_NOT_ACKNOWLEDGED')) "no ack: $($o1.outcome) $($o1.reasons -join ',')"
    $o2 = Act $c @{ Action = 'session.open'; Kind = 'Test'; AcknowledgeAbort = 'abort-000000000000' }
    Expect ($o2.outcome -eq 'REFUSED' -and (Has $o2 'ABORT_NOT_ACKNOWLEDGED')) "wrong ack: $($o2.outcome)"
    $o3 = Act $c @{ Action = 'session.open'; Kind = 'Test'; AcknowledgeAbort = $id }
    Expect ($o3.outcome -eq 'REFUSED' -and (Has $o3 'VMS_NOT_OFF')) "ack with a VM running: $($o3.outcome) $($o3.reasons -join ',')"
    $c.VmTable['P9S-D12'] = 'off'
    $o4 = Act $c @{ Action = 'session.open'; Kind = 'Test'; AcknowledgeAbort = $id }
    Expect ($o4.outcome -eq 'DONE') "ack, VMs off, fresh monitor run: $($o4.outcome) $($o4.reasons -join ',')"
}
Case 'K32' 'after STOP a new session needs a new monitor run' {
    $c = New-MockCtx @($READY_A, $READY_A, $STOP_A) @{ 'P9S-D12' = 'running' }; $null = Open $c
    $null = Act $c @{ Action = 'guest.key'; Vm = 'P9S-D12'; Key = 'Enter' }
    $null = Act $c @{ Action = 'session.close' }
    $c.Prov.queue.Clear(); $c.Prov.queue.Enqueue($READY_A)
    $o = Open $c; Expect ($o.outcome -eq 'REFUSED' -and (Has $o 'STOP_REQUIRES_NEW_MONITOR_RUN')) "same run: $($o.outcome) $($o.reasons -join ',')"
    $c.Prov.queue.Clear(); $c.Prov.queue.Enqueue($READY_B)
    $o2 = Open $c; Expect ($o2.outcome -eq 'DONE') "new run: $($o2.outcome) $($o2.reasons -join ',')"
}
Case 'K33' 'emergency stop request halts running automation and blocks new sessions' {
    $c = New-MockCtx @($READY_A) @{ 'P9S-D12' = 'running' }; $null = Open $c
    Write-StateAtomic -Path $c.StopRequestPath -Text '{"reason":"test"}'
    $r = Act $c @{ Action = 'guest.key'; Vm = 'P9S-D12'; Key = 'Enter' }
    Expect ($r.outcome -eq 'REFUSED' -and (Has $r 'EMERGENCY_STOP_REQUESTED') -and $c.Calls.Count -eq 0) "$($r.outcome) $($r.reasons -join ',')"
    $null = Act $c @{ Action = 'session.close' }
    Expect (-not (Test-Path -LiteralPath $c.StopRequestPath)) 'close did not clear the stop request'
}
Case 'K34' 'calibration preflight lists the bootstrap conditions, read only' {
    $c = New-MockCtx @($CALIB)
    $r = Act $c @{ Action = 'calib.preflight' }
    Expect ($r.outcome -eq 'DONE' -and $c.Calls.Count -eq 0) "$($r.outcome)"
    $txt = $r.lines -join "`n"
    Expect ($txt -match 'monitor state' -and $txt -match 'authorization' -and $txt -match 'LIVE CALIBRATION: NOT AVAILABLE') "lines: $txt"
    $c2 = New-MockCtx @($READY_A)
    $r2 = Act $c2 @{ Action = 'calib.preflight' }
    Expect (($r2.lines -join ' ') -match 'CALIBRATION_REQUIRES_CALIBRATE_MODE') 'monitor mode not refused'
}

# ---- the emergency hold ----------------------------------------------------------------
# A handle opened here stands for another coordinator process: shared as
# an emergency holds the hold, exclusive as session.close holds it.
function HoldOpen($ctx, [switch]$Exclusive) {
    if ($Exclusive) { [IO.FileStream]::new((Get-P9SEmergencyHoldPath $ctx), [IO.FileMode]::OpenOrCreate, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None) }
    else { [IO.FileStream]::new((Get-P9SEmergencyHoldPath $ctx), [IO.FileMode]::OpenOrCreate, [IO.FileAccess]::Read, [IO.FileShare]::Read) }
}
function HoldFree($ctx) { try { (HoldOpen $ctx -Exclusive).Dispose(); $true } catch { $false } }
function LockFree($ctx) { try { [IO.FileStream]::new($ctx.LockPath, [IO.FileMode]::OpenOrCreate, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None).Dispose(); $true } catch { $false } }
function Bytes([string]$p) { if (Test-Path -LiteralPath $p) { [Convert]::ToBase64String([IO.File]::ReadAllBytes($p)) } else { '<absent>' } }
Case 'K39' 'session.close changes nothing while an emergency holds the emergency hold, dry run included, in every monitor state; it closes once the hold is free' {
    $c = New-MockCtx @($READY_A) @{ 'P9S-D12' = 'running' }
    $o = Open $c; if ($o.outcome -ne 'DONE') { throw "setup: open $($o.outcome)" }
    if (-not (Add-P9SStopRequestIfAbsent $c)) { throw 'setup: no stop request' }
    $before = (Bytes $c.SessionPath) + '|' + (Bytes $c.StopRequestPath)
    $h = HoldOpen $c
    try {
        foreach ($mon in $READY_A, $STOP_A, $ABORT_A) {
            $c.Prov.queue.Clear(); $c.Prov.queue.Enqueue($mon)
            foreach ($dry in $true, $false) {
                $r = Act $c @{ Action = 'session.close'; DryRun = $dry }
                Expect ($r.outcome -eq 'REFUSED' -and (Has $r 'EMERGENCY_IN_PROGRESS') -and ($r.lines -join ' ') -match 'coordinator-emergency\.hold') "close (dry run $dry): $($r.outcome) $($r.reasons -join ',')"
            }
        }
    } finally { $h.Dispose() }
    Expect (((Bytes $c.SessionPath) + '|' + (Bytes $c.StopRequestPath)) -eq $before) 'a refused close changed the session record or the stop request'
    Expect ($c.Calls.Count -eq 0) "dispatches: $($c.Calls.Count)"
    $r2 = Act $c @{ Action = 'session.close' }
    Expect ($r2.outcome -eq 'DONE' -and -not (Test-Path -LiteralPath $c.StopRequestPath) -and (Sess $c).lifecycle -eq 'CLOSED') "close once the hold is free: $($r2.outcome) $($r2.reasons -join ',')"
    Expect (HoldFree $c) 'the close kept the emergency hold'
}
Case 'K40' 'an emergency that cannot take the emergency hold in time raises nothing, sends nothing and says so, under READY, STOP and ABORT' {
    foreach ($mon in $READY_A, $STOP_A, $ABORT_A) {
        $c = New-MockCtx @($READY_A) @{ 'P9S-D12' = 'running' }
        $o = Open $c; if ($o.outcome -ne 'DONE') { throw "setup: open $($o.outcome)" }
        $c.Prov.queue.Clear(); $c.Prov.queue.Enqueue($mon); $c.EmergencyLockWaitMs = 600
        $s0 = Bytes $c.SessionPath
        $x = HoldOpen $c -Exclusive
        try { $sw = [Diagnostics.Stopwatch]::StartNew(); $r = Act $c @{ Action = 'emergency.acpi-shutdown'; Vm = 'P9S-D12' }; $ms = $sw.ElapsedMilliseconds }
        finally { $x.Dispose() }
        $l = $r.lines -join ' '
        Expect ($r.outcome -eq 'REFUSED' -and (Has $r 'EMERGENCY_LOCK_TIMEOUT') -and $c.Calls.Count -eq 0) "while a close holds the hold: $($r.outcome) $($r.reasons -join ',') calls $($c.Calls.Count)"
        Expect (-not (Test-Path -LiteralPath $c.StopRequestPath)) 'a stop request was raised without the hold'
        Expect ($l -match 'coordinator-emergency\.hold' -and $l -match 'nothing raised' -and $l -match 'is not in place') "lines: $l"
        Expect ((Bytes $c.SessionPath) -eq $s0) 'the session record changed'
        Expect ($ms -ge 500 -and $ms -lt 2500 -and (LockFree $c)) "waited $ms ms for a deadline of 600 ms; lock free $(LockFree $c)"
        $r2 = Act $c @{ Action = 'emergency.acpi-shutdown'; Vm = 'P9S-D12' }
        Expect ($r2.outcome -eq 'DONE' -and $r2.dispatched -eq 1 -and $c.Calls.Count -eq 1) "once the hold is free: $($r2.outcome) $($r2.reasons -join ',')"
        Expect (HoldFree $c) 'the emergency kept the hold'
    }
    # a stop request raised by another emergency stays, and the message says so
    $c = New-MockCtx @($READY_A) @{ 'P9S-D12' = 'running' }; $null = Open $c
    $null = Add-P9SStopRequestIfAbsent $c; $q0 = Bytes $c.StopRequestPath; $c.EmergencyLockWaitMs = 600
    $x = HoldOpen $c -Exclusive
    try { $r = Act $c @{ Action = 'emergency.acpi-shutdown'; Vm = 'P9S-D12' } } finally { $x.Dispose() }
    Expect ($r.outcome -eq 'REFUSED' -and (Has $r 'EMERGENCY_LOCK_TIMEOUT') -and ($r.lines -join ' ') -match 'stays in place until a session\.close removes it' -and (Bytes $c.StopRequestPath) -eq $q0 -and $c.Calls.Count -eq 0) "with a stop request in place: $($r.outcome) $($r.lines -join ' ')"
}
Case 'K41' 'an emergency holds the emergency hold from before its stop request until its decision under the lock, and releases it on every path' {
    $c = New-MockCtx @($READY_A) @{ 'P9S-D12' = 'running' }; $null = Open $c
    $seen = @{ hold = $null; stop = $null }; $hp = Get-P9SEmergencyHoldPath $c; $sp = $c.StopRequestPath; $calls = $c.Calls; $tbl = $c.VmTable
    $c.Dispatch['vm.acpi'] = {
        param($vm)
        try { [IO.FileStream]::new($hp, [IO.FileMode]::OpenOrCreate, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None).Dispose(); $seen.hold = $false } catch { $seen.hold = $true }
        $seen.stop = (Test-Path -LiteralPath $sp)
        $calls.Add([pscustomobject]@{ kind = 'vm.acpi'; vm = $vm; arg = $null }); $tbl[$vm] = 'off'
    }.GetNewClosure()
    $r = Act $c @{ Action = 'emergency.acpi-shutdown'; Vm = 'P9S-D12' }
    Expect ($r.outcome -eq 'DONE' -and $r.dispatched -eq 1 -and $c.Calls.Count -eq 1) "sent: $($r.outcome) $($r.reasons -join ',')"
    Expect ($seen.hold -eq $true -and $seen.stop -eq $true) "at the dispatch, under the lock: hold held $($seen.hold), stop request $($seen.stop)"
    Expect ((HoldFree $c) -and (LockFree $c)) 'held after the request was sent'
    # refused for an unknown history (the stop request unreadable)
    $u = New-MockCtx @($READY_A) @{ 'P9S-D12' = 'running' }; $null = Open $u
    Write-StateAtomic -Path $u.StopRequestPath -Text '{broken'
    $ru = Act $u @{ Action = 'emergency.acpi-shutdown'; Vm = 'P9S-D12' }
    Expect ((Has $ru 'SHUTDOWN_HISTORY_UNCERTAIN') -and $u.Calls.Count -eq 0 -and (HoldFree $u) -and (LockFree $u)) "unknown history: $($ru.outcome) $($ru.reasons -join ',')"
    # the VM already off: nothing to send
    $off = New-MockCtx @($READY_A); $null = Open $off
    $rf = Act $off @{ Action = 'emergency.acpi-shutdown'; Vm = 'P9S-D12' }
    Expect ($rf.outcome -eq 'DONE' -and $off.Calls.Count -eq 0 -and (HoldFree $off) -and (LockFree $off)) "VM off: $($rf.outcome)"
    # the request fails
    $e = New-MockCtx @($READY_A) @{ 'P9S-D12' = 'running' }; $null = Open $e
    $e.Dispatch['vm.acpi'] = { param($vm) throw 'the request failed' }
    $re = Act $e @{ Action = 'emergency.acpi-shutdown'; Vm = 'P9S-D12' }
    Expect ($re.outcome -eq 'FAILED' -and (Has $re 'ACPI_REQUEST_FAILED') -and (HoldFree $e) -and (LockFree $e)) "failed request: $($re.outcome) $($re.reasons -join ',')"
    # an exception under the lock leaves the action
    $x = New-MockCtx @($READY_A) @{ 'P9S-D12' = 'running' }; $null = Open $x
    $x.ReadState = { param($p) throw 'the monitor state reader failed' }
    $thrown = $false; try { $null = Act $x @{ Action = 'emergency.acpi-shutdown'; Vm = 'P9S-D12' } } catch { $thrown = $true }
    Expect ($thrown -and $x.Calls.Count -eq 0 -and (HoldFree $x) -and (LockFree $x)) "exception: thrown $thrown, hold free $(HoldFree $x), lock free $(LockFree $x)"
}
Case 'K42' 'an emergency that times out on the coordinator lock raised its stop request before it waited, says so truly and releases the hold' {
    $c = New-MockCtx @($READY_A) @{ 'P9S-D12' = 'running' }; $null = Open $c
    $c.EmergencyLockWaitMs = 600
    $lk = [IO.FileStream]::new($c.LockPath, [IO.FileMode]::OpenOrCreate, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
    try { $r = Act $c @{ Action = 'emergency.acpi-shutdown'; Vm = 'P9S-D12' }; $free = HoldFree $c; $raised = Test-Path -LiteralPath $c.StopRequestPath } finally { $lk.Dispose() }
    Expect ($r.outcome -eq 'REFUSED' -and (Has $r 'EMERGENCY_LOCK_TIMEOUT') -and $c.Calls.Count -eq 0) "lock held: $($r.outcome) $($r.reasons -join ',')"
    Expect ($raised -and ($r.lines -join ' ') -match 'stays in place until a session\.close removes it') "the halt was not in place while the emergency waited for the lock: raised $raised; $($r.lines -join ' ')"
    Expect $free 'the emergency kept the hold after its lock timeout'
    $cl = Act $c @{ Action = 'session.close' }
    Expect ($cl.outcome -eq 'DONE' -and -not (Test-Path -LiteralPath $c.StopRequestPath)) "the empty request goes with the next close: $($cl.outcome)"
}
Case 'K43' 'one deadline covers the emergency hold and the coordinator lock' {
    $c = New-MockCtx @($READY_A) @{ 'P9S-D12' = 'running' }; $null = Open $c
    $c.EmergencyLockWaitMs = 1500
    $ready = [Threading.ManualResetEventSlim]::new($false)
    $job = Start-ThreadJob -ArgumentList (Get-P9SEmergencyHoldPath $c), $c.LockPath, $ready -ScriptBlock {
        param($holdPath, $lockPath, $ready)
        $x = [IO.FileStream]::new($holdPath, [IO.FileMode]::OpenOrCreate, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
        $l = [IO.FileStream]::new($lockPath, [IO.FileMode]::OpenOrCreate, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
        $ready.Set()
        Start-Sleep -Milliseconds 900; $x.Dispose()
        Start-Sleep -Milliseconds 2600; $l.Dispose()
    }
    if (-not $ready.Wait(10000)) { throw 'setup: the holder did not start' }
    $sw = [Diagnostics.Stopwatch]::StartNew(); $r = Act $c @{ Action = 'emergency.acpi-shutdown'; Vm = 'P9S-D12' }; $ms = $sw.ElapsedMilliseconds
    Receive-Job $job -Wait | Out-Null; Remove-Job $job
    Expect ($r.outcome -eq 'REFUSED' -and (Has $r 'EMERGENCY_LOCK_TIMEOUT') -and $c.Calls.Count -eq 0 -and (Test-Path -LiteralPath $c.StopRequestPath)) "hold for 0.9 s, then the lock: $($r.outcome) $($r.reasons -join ',')"
    Expect ($ms -ge 1400 -and $ms -lt 2100) "the hold and the lock took $ms ms for one deadline of 1500 ms"
}

# ---- command line, shims, real-process crash (no VM is touched) -----------------
$cli = Join-Path $kit 'p9s-action.ps1'
$pw = (Get-Process -Id $PID).Path
$vb = Join-Path $env:ProgramFiles 'Oracle\VirtualBox\VBoxManage.exe'
# VM states for the "nothing changed" checks, read only, through the
# toolkit's VirtualBox layer (host-wide lock, one query at a time, no
# retry); its records go to this run's folder. A failed read is
# 'unreadable', never a state.
$oracleVBox = New-P9SVBoxContext -Dir (Join-Path $OutDir 'vbox-oracle') -VmNames @('P9S-D12', 'P9S-U24')
function VmStateLine([string]$v) { Get-P9SVmStateReal $v $oracleVBox }
function VmStates {
    if (-not (Test-Path -LiteralPath $vb)) { return 'no VirtualBox' }
    @(foreach ($v in 'P9S-D12', 'P9S-U24') { "$v $(VmStateLine $v)" }) -join ' '
}
Case 'K35' 'command line: calibration dry run changes no VM; refusals exit 1' {
    $before = (VmStates) -join ' '
    $d = Join-Path $OutDir 'cli-1'; New-Item -ItemType Directory -Path $d | Out-Null
    $o = & $pw -NoProfile -File $cli -Action calib.dry-run -Scenario nominal -StatePath (Join-Path $d 'state.json') 2>&1
    $code = $LASTEXITCODE
    Expect ($code -eq 0 -and ($o -join ' ') -match 'SIMULATED' -and ($o -join ' ') -match 'COMPLETED') "exit $code"
    $o2 = & $pw -NoProfile -File $cli -Action vm.start -Vm P9S-D12 -DryRun -StatePath (Join-Path $d 'state.json') 2>&1
    Expect ($LASTEXITCODE -eq 1 -and ($o2 -join ' ') -match 'HOST GATE REFUSED' -and ($o2 -join ' ') -match 'STATE_MISSING') "vm.start without monitor: $LASTEXITCODE $($o2 -join ' ')"
    $o3 = & $pw -NoProfile -File $cli -Action 'vm.start' -Vm 'P9S-D12; calc' -DryRun -StatePath (Join-Path $d 'state.json') 2>&1
    Expect ($LASTEXITCODE -eq 1 -and ($o3 -join ' ') -match 'ACTION REJECTED') "injection: $LASTEXITCODE"
    $after = (VmStates) -join ' '
    Expect ($before -eq $after -and $after -notmatch 'running') "VM states $before -> $after"
    $aud = Get-Content -LiteralPath (Join-Path $d 'coordinator-audit.jsonl')
    Expect (@($aud | Where-Object { $_ -match '"dispatched":[1-9]' }).Count -eq 0) 'a dispatch was recorded'
}
Case 'K36' 'shims route through the coordinator' {
    $d = Join-Path $OutDir 'cli-2'; New-Item -ItemType Directory -Path $d | Out-Null
    $msg = ''; try { & (Join-Path $kit 'com-type.ps1') -Vm P9S-D12 -Text 'abc' -StatePath (Join-Path $d 'state.json') -DryRun } catch { $msg = $_.Exception.Message }
    Expect ($msg -match 'HOST GATE REFUSED \(guest.type P9S-D12\)') "com-type: $msg"
    $msg = ''; try { & (Join-Path $kit 'vm-action.ps1') -Vm P9S-U24 -Action Keys -Scancodes '1d 10 90 9d' -StatePath (Join-Path $d 'state.json') -DryRun } catch { $msg = $_.Exception.Message }
    Expect ($msg -match 'HOST GATE REFUSED \(guest.key P9S-U24\)') "vm-action keys: $msg"
    $msg = ''; try { & (Join-Path $kit 'vm-action.ps1') -Vm P9S-U24 -Action Keys -Scancodes '1d 2e ae 9e' -StatePath (Join-Path $d 'state.json') -DryRun } catch { $msg = $_.Exception.Message }
    Expect ($msg -match 'scancodes') "unknown scancodes: $msg"
    Expect (@(Get-Content -LiteralPath (Join-Path $d 'coordinator-audit.jsonl')).Count -ge 2) 'shim calls not audited by the coordinator'
}
# K37 starts the real coordinator, which reads the VM states with VBoxManage
# (read only); without VirtualBox it cannot run. K28 covers the same rule
# in process.
$k37 = 'real coordinator process killed mid-action: in-flight record blocks the next action'
if (-not (Test-Path -LiteralPath $vb)) { NotRun 'K37' $k37 'needs VirtualBox: the real coordinator reads the VM states with VBoxManage, not installed here' }
else {
Case 'K37' $k37 {
    $d = Join-Path $OutDir 'cli-3'; New-Item -ItemType Directory -Path $d | Out-Null
    $me = Get-Process -Id $PID
    $info = [ordered]@{ pid = $PID; process_start_utc = 'x'; process_start_ticks = $me.StartTime.ToUniversalTime().Ticks; mode = 'Monitor'; paging_stop = 'Required'; thresholds_sha256 = 'x'; thresholds_approved = $true; test = $false; kit = 'test' }
    $doc = Feed (NewMon 'run-K') 0 300
    $state = Join-Path $d 'state.json'
    Write-StateAtomic -Path $state -Text (ConvertTo-StateJson -Doc $doc -Monitor $info -WrittenQpc ([Diagnostics.Stopwatch]::GetTimestamp()) -QpcFrequency ([Diagnostics.Stopwatch]::Frequency) -WrittenUtc 'x')
    $vmNow = @{}
    foreach ($v in 'P9S-D12', 'P9S-U24') { $vmNow[$v] = $(if ((VmStateLine $v) -eq 'off') { 'off' } else { 'other' }) }
    $sess = [ordered]@{ schema = 'p9s-coordinator-session/1'; session_id = 'ses-k37'; kind = 'Test'; opened_utc = 'x'; monitor_run_id = 'run-K'; lifecycle = 'ACTIVE'; halt = $null; previous_halt = $null; expected_vm = $vmNow; inflight = $null; last_marker = $null }
    Write-StateAtomic -Path (Join-Path $d 'coordinator-session.json') -Text ($sess | ConvertTo-Json -Depth 5)
    $p = Start-Process -FilePath $pw -PassThru -WindowStyle Hidden -ArgumentList @('-NoProfile', '-File', $cli, '-Action', 'wait', '-Seconds', '60', '-StatePath', $state)
    $sw = [Diagnostics.Stopwatch]::StartNew(); $seen = $false
    while ($sw.Elapsed.TotalSeconds -lt 15) { $t = Get-Content -Raw -LiteralPath (Join-Path $d 'coordinator-session.json'); if ($t -match '"inflight":\s*\{') { $seen = $true; break }; Start-Sleep -Milliseconds 100 }
    Expect $seen 'the wait never recorded an in-flight action'
    Stop-Process -Id $p.Id -Force; $p.WaitForExit(10000) | Out-Null
    $o = & $pw -NoProfile -File $cli -Action scenario.begin -Name after-crash -StatePath $state 2>&1
    Expect ($LASTEXITCODE -eq 1 -and ($o -join ' ') -match 'INFLIGHT_UNRESOLVED') "after crash: $LASTEXITCODE $($o -join ' ')"
}
}
Case 'K38' 'every coordinator script parses (pwsh 7)' {
    foreach ($f in Get-ChildItem -LiteralPath $kit -Recurse -Filter *.ps1) {
        $tok = $null; $err = $null
        $null = [System.Management.Automation.Language.Parser]::ParseFile($f.FullName, [ref]$tok, [ref]$err)
        Expect ($err.Count -eq 0) "$($f.Name): $($err | ForEach-Object Message)"
    }
}

$pass = @($results | Where-Object result -eq 'PASS').Count
$fail = @($results | Where-Object result -eq 'FAIL').Count
$notRun = @($results | Where-Object result -eq 'NOT_RUN').Count
"== $pass passed, $fail failed, $notRun not run"
$results | ConvertTo-Json -Depth 3 | Set-Content -LiteralPath (Join-Path $OutDir 'results.json') -Encoding utf8
$results | ForEach-Object { '{0} {1} {2} ({3} ms){4}' -f $_.result, $_.id, $_.name, $_.ms, $(if ($_.detail) { ' :: ' + $_.detail } else { '' }) } | Set-Content -LiteralPath (Join-Path $OutDir 'results.txt') -Encoding utf8
if ($fail) { exit 1 } elseif ($notRun) { exit 3 } else { exit 0 }
