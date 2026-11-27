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
        EmergencyLockWaitMs = 1500; CalibrationHostChecks = { @() }; StateGate = { [pscustomobject]@{ status = 'READY'; installation_id = 'inst-test-installed'; reasons = @() } }
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

# ---- session.close without a session record -------------------------------------------
# An emergency under a monitor without a halt writes no session record; its
# stop request ends after the same checks as with one.
function StopWith($ctx, [string[]]$Acpi = @(), [string[]]$Unresolved = @()) {
    $q = New-P9SStopRequestDoc $ctx
    foreach ($v in $Acpi) { $q.acpi[$v] = '2026-11-25T10:00:00.0000000Z' }
    foreach ($v in $Unresolved) { $q.unresolved[$v] = '2026-11-25T10:00:00.0000000Z' }
    Save-P9SStopRequest $ctx $q
}
function CloseNote($ctx) { $l = @((Audit $ctx) -split "`n" | Where-Object { $_ } | ForEach-Object { $_ | ConvertFrom-Json } | Where-Object { $_.action -eq 'session.close' }); if ($l.Count) { [string]$l[-1].note } else { '' } }
Case 'K44' 'session.close without a session record removes the stop request of an emergency whose VM is off, after a dry run that changes nothing; a session then opens' {
    $c = New-MockCtx @($READY_A) @{ 'P9S-D12' = 'running' }
    $em = Act $c @{ Action = 'emergency.acpi-shutdown'; Vm = 'P9S-D12' }
    if ($em.outcome -ne 'DONE' -or $c.Calls.Count -ne 1) { throw "setup: emergency $($em.outcome) $($em.reasons -join ',')" }
    Expect (-not (Test-Path -LiteralPath $c.SessionPath) -and (Test-Path -LiteralPath $c.StopRequestPath)) 'setup: the emergency wrote a session record, or no stop request'
    $q0 = Bytes $c.StopRequestPath
    $dr = Act $c @{ Action = 'session.close'; DryRun = $true }
    Expect ($dr.outcome -eq 'DRYRUN' -and (Bytes $c.StopRequestPath) -eq $q0 -and -not (Test-Path -LiteralPath $c.SessionPath) -and ($dr.lines -join ' ') -match 'dry run: would remove the stop request without a session record') "dry run: $($dr.outcome) $($dr.lines -join ' ')"
    $r = Act $c @{ Action = 'session.close' }
    Expect ($r.outcome -eq 'DONE' -and -not (Test-Path -LiteralPath $c.StopRequestPath) -and -not (Test-Path -LiteralPath $c.SessionPath)) "close: $($r.outcome) $($r.reasons -join ','); stop request $(Test-Path -LiteralPath $c.StopRequestPath)"
    Expect (($r.lines -join ' ') -match 'removed the stop request without a session record \(ACPI requests to P9S-D12; unresolved none\)') "lines: $($r.lines -join ' ')"
    Expect ((CloseNote $c) -match '^removed the stop request without a session record \(ACPI requests to P9S-D12') "audit note: $(CloseNote $c)"
    Expect (HoldFree $c) 'the close kept the emergency hold'
    $o = Open $c
    Expect ($o.outcome -eq 'DONE') "open: $($o.outcome) $($o.reasons -join ',')"
    Expect ($c.Calls.Count -eq 1) "dispatches: $($c.Calls.Count)"
}
Case 'K45' 'session.close without a session record is refused and changes nothing while a VM its stop request names runs, the request sent or only recorded; the request stays spent; once the VM is off the close removes it' {
    # sent: the guest ignores the request and keeps running
    $c = New-MockCtx @($READY_A) @{ 'P9S-D12' = 'running' }
    $cl = $c.Calls
    $c.Dispatch['vm.acpi'] = { param($vm) $cl.Add([pscustomobject]@{ kind = 'vm.acpi'; vm = $vm; arg = $null }) }.GetNewClosure()
    $em = Act $c @{ Action = 'emergency.acpi-shutdown'; Vm = 'P9S-D12'; Seconds = 2 }
    if ($em.outcome -ne 'TIMEOUT' -or $c.Calls.Count -ne 1) { throw "setup: emergency $($em.outcome) $($em.reasons -join ',')" }
    # recorded before a crash, never sent
    $c2 = New-MockCtx @($READY_A) @{ 'P9S-D12' = 'running' }
    StopWith $c2 -Acpi 'P9S-D12'
    foreach ($x in $c, $c2) {
        $b0 = (Bytes $x.StopRequestPath) + '|' + (Bytes $x.SessionPath)
        foreach ($dry in $true, $false) {
            $r = Act $x @{ Action = 'session.close'; DryRun = $dry }
            Expect ($r.outcome -eq 'REFUSED' -and (Has $r 'EMERGENCY_UNRESOLVED') -and ($r.lines -join ' ') -match 'P9S-D12 did not power off') "close (dry run $dry) while the VM runs: $($r.outcome) $($r.reasons -join ',')"
        }
        Expect (((Bytes $x.StopRequestPath) + '|' + (Bytes $x.SessionPath)) -eq $b0) 'a refused close changed the stop request or wrote a session record'
        $n = $x.Calls.Count
        $e2 = Act $x @{ Action = 'emergency.acpi-shutdown'; Vm = 'P9S-D12' }
        Expect ($e2.outcome -eq 'REFUSED' -and (Has $e2 'ACPI_ALREADY_REQUESTED') -and $x.Calls.Count -eq $n) "second emergency: $($e2.outcome) $($e2.reasons -join ',')"
        $o = Open $x
        Expect ($o.outcome -eq 'REFUSED' -and (Has $o 'STOP_REQUEST_PENDING')) "open while the VM runs: $($o.outcome) $($o.reasons -join ',')"
        $x.VmTable['P9S-D12'] = 'off'
        $r2 = Act $x @{ Action = 'session.close' }
        Expect ($r2.outcome -eq 'DONE' -and -not (Test-Path -LiteralPath $x.StopRequestPath)) "close once the VM is off: $($r2.outcome) $($r2.reasons -join ',')"
        $o2 = Open $x
        Expect ($o2.outcome -eq 'DONE') "open after the close: $($o2.outcome) $($o2.reasons -join ',')"
    }
    Expect ($c.Calls.Count -eq 1 -and $c2.Calls.Count -eq 0) "dispatches: $($c.Calls.Count), $($c2.Calls.Count)"
}
Case 'K46' 'session.close without a session record is refused and changes nothing while the state of a VM its stop request names is unknown, and takes aborted for off only after an ACPI request' {
    $answers = @(
        @('unreadable', { param($vm) 'unreadable' }), @('quarantined', { param($vm) 'quarantined' }), @('busy', { param($vm) 'busy' }),
        @('unknown', { param($vm) 'unknown' }), @('empty', { param($vm) '' }), @('none', { param($vm) $null }),
        @('a failed read', { param($vm) throw 'VirtualBox did not answer' }), @('two answers', { param($vm) @('off', 'off') }))
    foreach ($kind in 'acpi', 'unresolved') {
        $c = New-MockCtx @($READY_A)
        if ($kind -eq 'acpi') { StopWith $c -Acpi 'P9S-D12' } else { StopWith $c -Acpi 'P9S-D12' -Unresolved 'P9S-D12' }
        $b0 = (Bytes $c.StopRequestPath) + '|' + (Bytes $c.SessionPath)
        $all = @($answers) + @(, @('aborted', { param($vm) 'aborted' }))
        foreach ($a in $all) {
            if ($kind -eq 'acpi' -and $a[0] -eq 'aborted') { continue }
            $c.ReadVmState = $a[1]
            foreach ($dry in $true, $false) {
                $r = Act $c @{ Action = 'session.close'; DryRun = $dry }
                Expect ($r.outcome -eq 'REFUSED' -and (Has $r 'EMERGENCY_UNRESOLVED')) "$kind, the reader answers $($a[0]) (dry run $dry): $($r.outcome) $($r.reasons -join ',')"
            }
        }
        Expect (((Bytes $c.StopRequestPath) + '|' + (Bytes $c.SessionPath)) -eq $b0) "$kind`: a refused close changed the stop request or wrote a session record"
        $c.ReadVmState = $(if ($kind -eq 'acpi') { { param($vm) 'aborted' } } else { { param($vm) 'off' } })
        $r2 = Act $c @{ Action = 'session.close' }
        Expect ($r2.outcome -eq 'DONE' -and -not (Test-Path -LiteralPath $c.StopRequestPath)) "$kind, the VM seen $(if ($kind -eq 'acpi') { 'aborted' } else { 'off' }): $($r2.outcome) $($r2.reasons -join ',')"
        Expect ($c.Calls.Count -eq 0) "$kind`: dispatches $($c.Calls.Count)"
    }
}
Case 'K47' 'session.close without a session record changes nothing while an emergency holds the emergency hold, dry run included, in every monitor state; it removes the stop request once the hold is free' {
    $c = New-MockCtx @($READY_A)
    StopWith $c -Acpi 'P9S-U24'
    $b0 = (Bytes $c.SessionPath) + '|' + (Bytes $c.StopRequestPath)
    $h = HoldOpen $c
    try {
        foreach ($mon in $READY_A, $STOP_A, $ABORT_A) {
            $c.Prov.queue.Clear(); $c.Prov.queue.Enqueue($mon)
            foreach ($dry in $true, $false) {
                $r = Act $c @{ Action = 'session.close'; DryRun = $dry }
                Expect ($r.outcome -eq 'REFUSED' -and (Has $r 'EMERGENCY_IN_PROGRESS')) "close (dry run $dry): $($r.outcome) $($r.reasons -join ',')"
            }
        }
    } finally { $h.Dispose() }
    Expect (((Bytes $c.SessionPath) + '|' + (Bytes $c.StopRequestPath)) -eq $b0) 'a refused close changed the stop request or wrote a session record'
    $r2 = Act $c @{ Action = 'session.close' }
    Expect ($r2.outcome -eq 'DONE' -and -not (Test-Path -LiteralPath $c.StopRequestPath) -and -not (Test-Path -LiteralPath $c.SessionPath)) "close once the hold is free: $($r2.outcome) $($r2.reasons -join ',')"
    Expect (HoldFree $c) 'the close kept the emergency hold'
    Expect ($c.Calls.Count -eq 0) "dispatches: $($c.Calls.Count)"
}
Case 'K48' 'session.close without a session record: with no stop request it writes nothing; an unreadable one is refused and kept as it is' {
    $c = New-MockCtx @($READY_A)
    $r = Act $c @{ Action = 'session.close' }
    Expect ($r.outcome -eq 'DONE' -and ($r.lines -join ' ') -ceq 'no session to close' -and -not (Test-Path -LiteralPath $c.SessionPath) -and -not (Test-Path -LiteralPath $c.StopRequestPath)) "no stop request: $($r.outcome) $($r.lines -join ' ')"
    foreach ($txt in '{broken', '', '[]', '{"acpi":[]}', '{"unresolved":"P9S-D12"}') {
        $x = New-MockCtx @($READY_A)
        [IO.File]::WriteAllText($x.StopRequestPath, $txt)
        $b0 = Bytes $x.StopRequestPath
        $rx = Act $x @{ Action = 'session.close' }
        Expect ($rx.outcome -eq 'REFUSED' -and (Has $rx 'STOP_REQUEST_UNREADABLE') -and (Bytes $x.StopRequestPath) -eq $b0 -and -not (Test-Path -LiteralPath $x.SessionPath)) "stop request '$txt': $($rx.outcome) $($rx.reasons -join ',')"
    }
}
Case 'K49' 'after session.close without a session record, a monitor STOP still needs a new run and an ABORT also its acknowledgement with the VMs off' {
    foreach ($halt in 'STOP', 'ABORT') {
        $c = New-MockCtx @($READY_A) @{ 'P9S-D12' = 'running' }
        $em = Act $c @{ Action = 'emergency.acpi-shutdown'; Vm = 'P9S-D12' }
        if ($em.outcome -ne 'DONE' -or (Test-Path -LiteralPath $c.SessionPath)) { throw "setup: emergency $($em.outcome)" }
        $c.Prov.queue.Clear(); $c.Prov.queue.Enqueue($(if ($halt -eq 'STOP') { $STOP_A } else { $ABORT_A }))
        $r = Act $c @{ Action = 'session.close' }
        Expect ($r.outcome -eq 'DONE' -and -not (Test-Path -LiteralPath $c.StopRequestPath)) "$halt`: close $($r.outcome) $($r.reasons -join ',')"
        $o1 = Open $c
        Expect ($o1.outcome -eq 'REFUSED' -and (Sess $c).halt.status -eq "$($halt)_REQUIRED") "$halt`: open under the halt $($o1.outcome) $($o1.reasons -join ',')"
        $c.Prov.queue.Clear(); $c.Prov.queue.Enqueue($READY_A)
        $o2 = Open $c
        Expect ($o2.outcome -eq 'REFUSED' -and (Has $o2 'STOP_REQUIRES_NEW_MONITOR_RUN')) "$halt`: open on the same run $($o2.outcome) $($o2.reasons -join ',')"
        $c.Prov.queue.Clear(); $c.Prov.queue.Enqueue($READY_B)
        $o3 = Open $c
        if ($halt -eq 'STOP') { Expect ($o3.outcome -eq 'DONE') "STOP: open on a new run $($o3.outcome) $($o3.reasons -join ',')" }
        else {
            Expect ($o3.outcome -eq 'REFUSED' -and (Has $o3 'ABORT_NOT_ACKNOWLEDGED')) "ABORT: open on a new run without the acknowledgement $($o3.outcome) $($o3.reasons -join ',')"
            $o4 = Act $c @{ Action = 'session.open'; Kind = 'Test'; AcknowledgeAbort = [string](Sess $c).halt.abort_id }
            Expect ($o4.outcome -eq 'DONE') "ABORT: open with the acknowledgement $($o4.outcome) $($o4.reasons -join ',')"
        }
    }
}

# ---- records present but inaccessible -------------------------------------------------
# Deny entries for this account on synthetic records only, removed by each
# case: 'inaccessible' denies a record its data and attributes and its
# folder the listing (an existence test then answers false), 'data' denies
# only its data. A record that cannot be read is never an absent one.
$aclSid = [Security.Principal.WindowsIdentity]::GetCurrent().User
function Add-AclDeny([string]$Path, [string]$Rights, [switch]$Dir) {
    $i = if ($Dir) { [IO.DirectoryInfo]::new($Path) } else { [IO.FileInfo]::new($Path) }
    $rule = [Security.AccessControl.FileSystemAccessRule]::new($aclSid, [Security.AccessControl.FileSystemRights]$Rights, 'None', 'None', 'Deny')
    $s = [IO.FileSystemAclExtensions]::GetAccessControl($i, [Security.AccessControl.AccessControlSections]::Access); $s.AddAccessRule($rule); [IO.FileSystemAclExtensions]::SetAccessControl($i, $s)
    [pscustomobject]@{ path = $Path; dir = [bool]$Dir; rule = $rule }
}
function Lock-Record([string]$Path, [string]$How = 'inaccessible') {
    $l = [System.Collections.Generic.List[object]]::new()
    if ($How -ceq 'data') { $l.Add((Add-AclDeny $Path 'ReadData')) }
    else { $l.Add((Add-AclDeny $Path 'ReadData, ReadAttributes')); $l.Add((Add-AclDeny ([IO.Path]::GetDirectoryName($Path)) 'ReadData' -Dir)) }
    , $l
}
function Unlock-Record($l) {
    for ($i = $l.Count - 1; $i -ge 0; $i--) {
        $h = $l[$i]; $x = if ($h.dir) { [IO.DirectoryInfo]::new($h.path) } else { [IO.FileInfo]::new($h.path) }
        $s = [IO.FileSystemAclExtensions]::GetAccessControl($x, [Security.AccessControl.AccessControlSections]::Access); $null = $s.RemoveAccessRule($h.rule); [IO.FileSystemAclExtensions]::SetAccessControl($x, $s)
    }
}
# an action's result, or the exception it raised
function Try-Act($ctx, [hashtable]$r) {
    try { $x = Act $ctx $r; [pscustomobject]@{ outcome = [string]$x.outcome; reasons = @($x.reasons); dispatched = [int]$x.dispatched } }
    catch { [pscustomobject]@{ outcome = "EXCEPTION $($_.Exception.Message)"; reasons = @(); dispatched = 0 } }
}
function Why($r) { "$($r.outcome) [$(@($r.reasons) -join ',')]" }
# runs $Body with the caller's ErrorActionPreference set to $Pref; returns
# its last output and the errors it wrote
function With-Preference([string]$Pref, [scriptblock]$Body) {
    $all = @(& { $ErrorActionPreference = $Pref; & $Body } 2>&1)
    [pscustomobject]@{ out = @($all | Where-Object { $_ -isnot [Management.Automation.ErrorRecord] })[-1]; errors = @($all | Where-Object { $_ -is [Management.Automation.ErrorRecord] }) }
}
Case 'K50' 'a state file is missing only when the system reports it not found: a record that is inaccessible, held, not UTF-8 or a folder is an error, never missing; an existence test is no proof of absence' {
    $d = Join-Path $OutDir 'k50'; New-Item -ItemType Directory -Path $d | Out-Null
    $p = Join-Path $d 'record.json'
    function Check([string]$Label, [string]$Path, [string]$Want, [bool]$Absent) {
        $r = Read-StateFile -Path $Path
        $got = if ($r.ok) { 'ok' } else { [string]$r.error }
        Expect ($got -ceq $Want) "${Label}: read $got, expected $Want"
        $a = Test-StateFileAbsent $Path
        Expect ($a -eq $Absent) "${Label}: absent $a, expected $Absent"
        $r
    }
    $null = Check 'absent' $p 'missing' $true
    $null = Check 'absent with its folder' (Join-Path $d 'no-folder\record.json') 'missing' $true
    [IO.File]::WriteAllText($p, '{"a":1}')
    $r = Check 'readable' $p 'ok' $false; Expect ($r.text -ceq '{"a":1}') "readable: text $($r.text)"
    [IO.File]::WriteAllText($p, '')
    $r = Check 'empty' $p 'ok' $false; Expect ($r.text -ceq '') "empty: text '$($r.text)'"
    [IO.File]::WriteAllBytes($p, [byte[]](0x7B, 0xC3, 0x28, 0x7D))
    $null = Check 'not UTF-8' $p 'invalid' $false
    [IO.File]::WriteAllText($p, '{"a":2}')
    $l = Lock-Record $p 'data'
    try { $null = Check 'data denied' $p 'denied' $false } finally { Unlock-Record $l }
    $l = Lock-Record $p
    try {
        Expect (-not [IO.File]::Exists($p)) 'inaccessible: [IO.File]::Exists answered true (the case does not test what it names)'
        $null = Check 'inaccessible' $p 'denied' $false
    } finally { Unlock-Record $l }
    $h = [IO.FileStream]::new($p, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::None)
    try { $null = Check 'held without sharing' $p 'busy' $false } finally { $h.Dispose() }
    $q = Join-Path $d 'folder.json'; New-Item -ItemType Directory -Path $q | Out-Null
    $null = Check 'a folder' $q 'denied' $false
    # replaced, then deleted, while a reader holds it open (as Read-StateFile
    # opens it): the reader reads the document it opened, whole. A replace
    # either waits for the reader and fails after its tries (NTFS without
    # POSIX rename: the old document stays, no temporary file is left) or
    # takes effect for the next read; a deleted file is never read again.
    $v1 = '{"v":1}'; $v2 = '{"v":2,"pad":"xxxxxxxxxxxxxxxx"}'
    Write-StateAtomic -Path $p -Text $v1
    $h = [IO.FileStream]::new($p, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::ReadWrite -bor [IO.FileShare]::Delete)
    try {
        $threw = $false; try { Write-StateAtomic -Path $p -Text $v2 } catch { $threw = $true }
        $r = Read-StateFile -Path $p
        Expect ($r.ok -and $r.text -ceq $(if ($threw) { $v1 } else { $v2 })) "replaced while read (replace failed $threw): next read $($r.ok) $($r.error) $($r.text)"
        $old = [IO.StreamReader]::new($h, [Text.UTF8Encoding]::new($false)).ReadToEnd()
        Expect ($old -ceq $v1) "replaced while read: the open reader read '$old'"
        Expect (@(Get-ChildItem -LiteralPath $d -Force -Filter '*.tmp').Count -eq 0) 'replaced while read: a temporary file left'
    } finally { $h.Dispose() }
    Write-StateAtomic -Path $p -Text $v2
    $r = Read-StateFile -Path $p
    Expect ($r.ok -and $r.text -ceq $v2) "replaced after the read: $($r.ok) $($r.error) $($r.text)"
    $h = [IO.FileStream]::new($p, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::ReadWrite -bor [IO.FileShare]::Delete)
    try {
        Remove-Item -LiteralPath $p
        $r = Read-StateFile -Path $p
        Expect (-not $r.ok) "deleted while read: read ok again ($($r.text))"
        $old = [IO.StreamReader]::new($h, [Text.UTF8Encoding]::new($false)).ReadToEnd()
        Expect ($old -ceq $v2) "deleted while read: the open reader read '$old'"
    } finally { $h.Dispose() }
    $null = Check 'deleted' $p 'missing' $true
}
Case 'K51' 'a stop request that records an ACPI request and cannot be read (its data, or its data and attributes in a folder that cannot be listed): the emergency sends and writes nothing, session.close and session.open are refused; readable again, the request stays spent' {
    foreach ($how in 'data', 'inaccessible') {
        $c = New-MockCtx @($READY_A) @{ 'P9S-D12' = 'running' }
        StopWith $c -Acpi 'P9S-D12'
        $b0 = Bytes $c.StopRequestPath
        $l = Lock-Record $c.StopRequestPath $how
        try {
            $em = Try-Act $c @{ Action = 'emergency.acpi-shutdown'; Vm = 'P9S-D12'; Seconds = 2 }
            $cl = Try-Act $c @{ Action = 'session.close' }
            $op = Try-Act $c @{ Action = 'session.open'; Kind = 'Test' }
        } finally { Unlock-Record $l }
        Expect ($em.outcome -eq 'REFUSED' -and (Has $em 'STOP_REQUEST_UNREADABLE') -and (Has $em 'SHUTDOWN_HISTORY_UNCERTAIN') -and $em.dispatched -eq 0) "${how}: emergency $(Why $em)"
        Expect ($cl.outcome -eq 'REFUSED' -and (Has $cl 'STOP_REQUEST_UNREADABLE')) "${how}: close $(Why $cl)"
        Expect ($op.outcome -eq 'REFUSED' -and (Has $op 'STOP_REQUEST_PENDING')) "${how}: open $(Why $op)"
        Expect ($c.Calls.Count -eq 0 -and (Bytes $c.StopRequestPath) -eq $b0 -and -not (Test-Path -LiteralPath $c.SessionPath)) "${how}: calls $($c.Calls.Count); stop request changed $((Bytes $c.StopRequestPath) -ne $b0); session record $(Test-Path -LiteralPath $c.SessionPath)"
        Expect (@(Get-ChildItem -LiteralPath $c.Dir -Force -Filter '*.tmp').Count -eq 0 -and (HoldFree $c) -and (LockFree $c)) "${how}: a temporary file, the hold or the lock left"
        $e2 = Try-Act $c @{ Action = 'emergency.acpi-shutdown'; Vm = 'P9S-D12'; Seconds = 2 }
        Expect ($e2.outcome -eq 'REFUSED' -and (Has $e2 'ACPI_ALREADY_REQUESTED') -and $c.Calls.Count -eq 0) "${how}, readable again: emergency $(Why $e2); calls $($c.Calls.Count)"
        $c.VmTable['P9S-D12'] = 'off'
        $c2 = Try-Act $c @{ Action = 'session.close' }; $o2 = Try-Act $c @{ Action = 'session.open'; Kind = 'Test' }
        Expect ($c2.outcome -eq 'DONE' -and -not (Test-Path -LiteralPath $c.StopRequestPath) -and $o2.outcome -eq 'DONE') "${how}, the VM off: close $(Why $c2), open $(Why $o2)"
    }
}
Case 'K52' 'a stop request that cannot be read halts every ordinary action whatever ErrorActionPreference the caller set: a test action, the checkpoint of a running one and session.open are refused, nothing is dispatched, no error is written' {
    foreach ($pref in 'Stop', 'Continue', 'SilentlyContinue') {
        $c = New-MockCtx @($READY_A) @{ 'P9S-D12' = 'running' }
        $o = Open $c; if ($o.outcome -ne 'DONE') { throw "setup: open $($o.outcome)" }
        $s = (Read-P9SSession $c).session; $s.expected_vm['P9S-D12'] = 'running'; Save-P9SSession $c $s
        $n = New-MockCtx @($READY_A)
        foreach ($x in $c, $n) { StopWith $x -Acpi 'P9S-U24' }
        $l1 = Lock-Record $c.StopRequestPath; $l2 = Lock-Record $n.StopRequestPath
        try {
            $w = With-Preference $pref {
                $k = Try-Act $c @{ Action = 'guest.key'; Vm = 'P9S-D12'; Key = 'Enter' }
                $ck = try { Test-P9SCheckpoint $c $s (New-P9SResult 'wait' $null) } catch { [pscustomobject]@{ ok = "EXCEPTION $($_.Exception.Message)"; reasons = @() } }
                $op = Try-Act $n @{ Action = 'session.open'; Kind = 'Test' }
                [pscustomobject]@{ k = $k; ck = $ck; op = $op }
            }
        } finally { Unlock-Record $l2; Unlock-Record $l1 }
        Expect ($w.out.k.outcome -eq 'REFUSED' -and (Has $w.out.k 'EMERGENCY_STOP_REQUESTED') -and $c.Calls.Count -eq 0) "${pref}: test action $(Why $w.out.k); calls $($c.Calls.Count)"
        Expect ($w.out.ck.ok -eq $false -and (Has $w.out.ck 'EMERGENCY_STOP_REQUESTED')) "${pref}: checkpoint ok $($w.out.ck.ok) [$(@($w.out.ck.reasons) -join ',')]"
        Expect ($w.out.op.outcome -eq 'REFUSED' -and (Has $w.out.op 'STOP_REQUEST_PENDING') -and -not (Test-Path -LiteralPath $n.SessionPath)) "${pref}: open $(Why $w.out.op)"
        Expect ($w.errors.Count -eq 0) "${pref}: $($w.errors.Count) errors written: $(@($w.errors | ForEach-Object { $_.Exception.Message }) -join ' / ')"
    }
}
Case 'K53' 'a session record that cannot be read is never "no session": a session halted by ABORT stays halted for session.open, a test action and session.close, and nothing replaces its record; readable again, the ABORT still needs its acknowledgement' {
    foreach ($how in 'data', 'inaccessible') {
        $c = New-MockCtx @($READY_A)
        $o = Open $c; if ($o.outcome -ne 'DONE') { throw "setup: open $($o.outcome)" }
        $c.Prov.queue.Clear(); $c.Prov.queue.Enqueue($ABORT_A)
        $null = Act $c @{ Action = 'guest.key'; Vm = 'P9S-D12'; Key = 'Enter' }
        $h = (Sess $c).halt
        if ([string]$h.status -ne 'ABORT_REQUIRED' -or -not $h.abort_id) { throw "setup: halt $($h.status)" }
        $c.Prov.queue.Clear(); $c.Prov.queue.Enqueue($READY_B)
        $b0 = Bytes $c.SessionPath
        $l = Lock-Record $c.SessionPath $how
        try {
            $op = Try-Act $c @{ Action = 'session.open'; Kind = 'Test' }
            $k = Try-Act $c @{ Action = 'guest.key'; Vm = 'P9S-D12'; Key = 'Enter' }
            $cl = Try-Act $c @{ Action = 'session.close' }
        } finally { Unlock-Record $l }
        Expect ($op.outcome -eq 'REFUSED' -and (Has $op 'SESSION_UNREADABLE')) "${how}: open $(Why $op)"
        Expect ($k.outcome -eq 'REFUSED' -and (Has $k 'SESSION_UNREADABLE') -and $c.Calls.Count -eq 0) "${how}: test action $(Why $k); calls $($c.Calls.Count)"
        Expect ($cl.outcome -eq 'REFUSED' -and (Has $cl 'SESSION_UNREADABLE')) "${how}: close $(Why $cl)"
        Expect ((Bytes $c.SessionPath) -eq $b0) "${how}: the session record was replaced ($((Sess $c).session_id) $((Sess $c).lifecycle))"
        $o2 = Try-Act $c @{ Action = 'session.open'; Kind = 'Test' }
        Expect ($o2.outcome -eq 'REFUSED' -and (Has $o2 'ABORT_NOT_ACKNOWLEDGED')) "${how}, readable again: open $(Why $o2)"
        $o3 = Try-Act $c @{ Action = 'session.open'; Kind = 'Test'; AcknowledgeAbort = [string]$h.abort_id }
        Expect ($o3.outcome -eq 'DONE') "${how}, readable again: open with the acknowledgement $(Why $o3)"
    }
}
Case 'K54' 'a VirtualBox quarantine or operation record that cannot be read blocks as an unreadable one does, whatever ErrorActionPreference the caller set, and a second quarantine never replaces the first' {
    foreach ($pref in 'Stop', 'Continue', 'SilentlyContinue') {
        $c = New-MockCtx @($READY_A)
        $vb = New-P9SVBoxContext -Dir (Join-Path $c.Dir 'vbox') -VmNames @('P9S-D12', 'P9S-U24') -MutexName "Local\P9S-test-k54-$PID-$pref" -ExePath (Join-Path $c.Dir 'no-vboxmanage.exe')
        New-Item -ItemType Directory -Path $vb.Dir | Out-Null
        $c | Add-Member -NotePropertyName VBox -NotePropertyValue $vb
        $g0 = Get-P9SVBoxGate $c
        Expect ($g0.ok) "${pref}: nothing recorded, the gate refuses: $($g0.reasons -join ',')"
        $lp = Get-P9SVBoxPath $vb 'latch'; $rp = Get-P9SVBoxPath $vb 'record'
        [IO.File]::WriteAllText($lp, ([ordered]@{ schema = 'p9s-vbox-quarantine/1'; reason = 'VBOX_TIMEOUT'; operation_id = 'op-first'; utc = 'x' } | ConvertTo-Json))
        $b0 = Bytes $lp
        $l = Lock-Record $lp
        try {
            $w = With-Preference $pref {
                $g = try { Get-P9SVBoxGate $c } catch { [pscustomobject]@{ ok = "EXCEPTION $($_.Exception.Message)"; reasons = @() } }
                $op = Try-Act $c @{ Action = 'session.open'; Kind = 'Test' }
                $set = try { Set-P9SVBoxLatch $vb 'VBOX_TEST_SECOND' $null 'second' } catch { "EXCEPTION $($_.Exception.Message)" }
                [pscustomobject]@{ g = $g; op = $op; set = $set }
            }
        } finally { Unlock-Record $l }
        Expect ($w.out.g.ok -eq $false -and (Has $w.out.g 'VBOX_API_QUARANTINED') -and (Has $w.out.g 'VBOX_LATCH_UNREADABLE')) "${pref}: quarantine, gate ok $($w.out.g.ok) [$(@($w.out.g.reasons) -join ',')]"
        Expect ($w.out.op.outcome -eq 'REFUSED' -and (Has $w.out.op 'VBOX_API_QUARANTINED') -and -not (Test-Path -LiteralPath $c.SessionPath)) "${pref}: quarantine, open $(Why $w.out.op)"
        Expect ($w.out.set -eq $true -and (Bytes $lp) -eq $b0) "${pref}: a second quarantine returned '$($w.out.set)', the first replaced $((Bytes $lp) -ne $b0)"
        Expect ($w.errors.Count -eq 0) "${pref}: quarantine, $($w.errors.Count) errors written: $(@($w.errors | ForEach-Object { $_.Exception.Message }) -join ' / ')"
        Remove-Item -LiteralPath $lp
        [IO.File]::WriteAllText($rp, ([ordered]@{ schema = 'p9s-vbox-operation/1'; operation_id = 'op-2'; operation = 'vm.acpi'; vm = 'P9S-D12'; owner_pid = $PID; owner_start_ticks = 1; utc = 'x' } | ConvertTo-Json))
        $l = Lock-Record $rp
        try { $w = With-Preference $pref { try { Get-P9SVBoxGate $c } catch { [pscustomobject]@{ ok = "EXCEPTION $($_.Exception.Message)"; reasons = @() } } } } finally { Unlock-Record $l }
        Expect ($w.out.ok -eq $false -and (Has $w.out 'VBOX_OPERATION_UNFINISHED') -and $w.errors.Count -eq 0) "${pref}: operation record, gate ok $($w.out.ok) [$(@($w.out.reasons) -join ',')], $($w.errors.Count) errors written"
    }
}
Case 'K55' 'records that are really absent keep the protocol of a new installation: a session opens, the first emergency records its request and sends it once, a close with the VM off removes it, a session opens again' {
    $c = New-MockCtx @($READY_A) @{ 'P9S-D12' = 'running' }
    foreach ($p in $c.SessionPath, $c.StopRequestPath) { $r = Read-StateFile -Path $p; Expect (-not $r.ok -and $r.error -ceq 'missing') "$([IO.Path]::GetFileName($p)): $($r.error)" }
    $o = Open $c
    Expect ($o.outcome -eq 'DONE') "open: $(Why $o)"
    $em = Act $c @{ Action = 'emergency.acpi-shutdown'; Vm = 'P9S-D12' }
    $q = Read-P9SStopRequestState $c
    Expect ($em.outcome -eq 'DONE' -and $em.dispatched -eq 1 -and $c.Calls.Count -eq 1 -and $q.ok -and $q.doc.acpi.Contains('P9S-D12')) "emergency: $(Why $em); calls $($c.Calls.Count); recorded $($q.ok -and $q.doc.acpi.Contains('P9S-D12'))"
    $cl = Act $c @{ Action = 'session.close' }
    Expect ($cl.outcome -eq 'DONE' -and -not (Test-Path -LiteralPath $c.StopRequestPath)) "close: $(Why $cl)"
    $o2 = Open $c
    Expect ($o2.outcome -eq 'DONE' -and $c.Calls.Count -eq 1) "open again: $(Why $o2); calls $($c.Calls.Count)"
    $n = New-MockCtx @($READY_A); Remove-Item -LiteralPath $n.Dir -Recurse
    $q2 = Read-P9SStopRequestState $n; $s2 = Read-P9SSession $n
    Expect (-not $q2.present -and $s2.ok -and -not $s2.session) "a state folder not created yet: stop request present $($q2.present), session record ok $($s2.ok)"
}

# ---- the persistent state gate (coordinator-lib.ps1, R14/X5) --------------------
# A scripted gate: the statuses in turn, the last one kept; "READY@<id>" is
# READY for another installation. Every answer is logged.
function Set-Gate($ctx, [string[]]$Statuses) {
    $q = [System.Collections.Generic.Queue[string]]::new(); foreach ($s in $Statuses) { $q.Enqueue($s) }
    $log = [System.Collections.Generic.List[string]]::new()
    $ctx | Add-Member -NotePropertyName GateLog -NotePropertyValue $log -Force
    $ctx.StateGate = {
        $s = if ($q.Count -gt 1) { $q.Dequeue() } else { $q.Peek() }; $log.Add($s)
        $st, $id = $s -split '@', 2
        [pscustomobject]@{ status = $st; installation_id = $(if ($st -ceq 'READY') { if ($id) { $id } else { 'inst-test-installed' } } else { $null }); reasons = @("TEST_$st") }
    }.GetNewClosure()
}
# VM state reads counted (the quiet context must make none)
function Watch-VmReads($ctx) {
    $reads = [System.Collections.Generic.List[string]]::new(); $orig = $ctx.ReadVmState
    $ctx.ReadVmState = { param($vm) $reads.Add($vm); & $orig $vm }.GetNewClosure()
    $ctx | Add-Member -NotePropertyName VmReads -NotePropertyValue $reads -Force
}
function Files($ctx) { @(Get-ChildItem -LiteralPath $ctx.Dir -Force | ForEach-Object Name | Sort-Object) -join ',' }
# the statuses of p9s-state.ps1 but READY and SANDBOX, and the one of a context without the classifier
$notReady = 'LOCATION_UNSAFE', 'ANCHOR_UNREADABLE', 'ANCHOR_MISSING', 'SETUP_LEFTOVER', 'INSTALLATION_LOST', 'MIGRATION_SOURCES_AMBIGUOUS', 'MIGRATION_REQUIRED', 'NOT_INITIALIZED',
    'INTERRUPTED', 'ACCOUNT_MISMATCH', 'STATE_MISMATCH', 'STATE_MISSING', 'STATE_INCOMPLETE', 'STATE_UNCERTAIN', 'STATE_DIVERGED', 'STATE_CLASSIFIER_MISSING'
$plan56 = Join-Path $OutDir 'plan-k56.json'
[ordered]@{ schema = 'p9s-plan/1'; name = 'one-key'; steps = @(@{ Action = 'guest.key'; Vm = 'P9S-D12'; Key = 'Enter' }) } | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $plan56
$budget56 = @(
    @{ Action = 'session.open'; Kind = 'Test' }, @{ Action = 'session.close' }, @{ Action = 'vm.start'; Vm = 'P9S-D12' }, @{ Action = 'guest.key'; Vm = 'P9S-D12'; Key = 'Enter' },
    @{ Action = 'guest.type'; Vm = 'P9S-D12'; Text = 'abc' }, @{ Action = 'guest.unlock'; Vm = 'P9S-D12' }, @{ Action = 'scenario.begin'; Name = 'x' }, @{ Action = 'phase.advance'; Name = 'x' },
    @{ Action = 'wait'; Seconds = 5 }, @{ Action = 'sequence.run'; Plan = $plan56 }, @{ Action = 'emergency.acpi-shutdown'; Vm = 'P9S-D12' }, @{ Action = 'emergency.wait-off'; Vm = 'P9S-D12'; Seconds = 4 },
    @{ Action = 'vbox.recover'; Operation = 'vbox-20261127T120000000Z-1-0000abcd' }, @{ Action = 'vm.start'; Vm = 'P9S-D12'; DryRun = $true }, @{ Action = 'emergency.acpi-shutdown'; Vm = 'P9S-D12'; DryRun = $true })
Case 'K56' 'persistent state not READY: every action that can spend budget or reach a VM is refused with the status, dry runs included, before anything is read or written; the emergency says to shut down outside the toolkit' {
    foreach ($s in $notReady) {
        foreach ($rq in $budget56) {
            $c = New-MockCtx @($READY_A) @{ 'P9S-D12' = 'running' }; Set-Gate $c @($s); Watch-VmReads $c
            $r = Act $c $rq
            $tag = "$s $($rq.Action)$(if ($rq.DryRun) { ' -DryRun' })"
            Expect ($r.outcome -ceq 'REFUSED' -and (Has $r "STATE_NOT_READY:$s") -and (Has $r "TEST_$s")) "${tag}: $(Why $r)"
            Expect ($c.Calls.Count -eq 0 -and $c.VmReads.Count -eq 0 -and $c.Prov.reads -eq 0) "${tag}: calls $($c.Calls.Count), VM reads $($c.VmReads.Count), monitor reads $($c.Prov.reads)"
            Expect ((Files $c) -ceq '') "${tag}: files written: $(Files $c)"
            if ($rq.Action -ceq 'emergency.acpi-shutdown' -and -not $rq.DryRun) { Expect (($r.lines -join ' ') -match 'Nothing was raised, written or sent' -and ($r.lines -join ' ') -match 'from the guest or from VirtualBox') "${tag}: $($r.lines -join ' | ')" }
        }
    }
    foreach ($how in 'missing', 'throws', 'empty') {
        $c = New-MockCtx @($READY_A) @{ 'P9S-D12' = 'running' }; Watch-VmReads $c
        switch ($how) { 'missing' { $c.PSObject.Properties.Remove('StateGate') } 'throws' { $c.StateGate = { throw 'classifier failed' } } 'empty' { $c.StateGate = { $null } } }
        $want = if ($how -ceq 'missing') { 'STATE_NOT_READY:STATE_GATE_MISSING' } else { 'STATE_NOT_READY:STATE_UNVERIFIABLE' }
        foreach ($rq in @(@{ Action = 'session.open'; Kind = 'Test' }, @{ Action = 'emergency.acpi-shutdown'; Vm = 'P9S-D12' })) {
            $r = Act $c $rq
            Expect ($r.outcome -ceq 'REFUSED' -and (Has $r $want) -and $c.Calls.Count -eq 0 -and (Files $c) -ceq '') "gate ${how}, $($rq.Action): $(Why $r); files $(Files $c)"
        }
    }
}
Case 'K57' 'persistent state not READY: the observe actions still run, write nothing (no audit line, no observation), read no VM state and no VM configuration, and say so' {
    $obs = @(@{ Action = 'session.status' }, @{ Action = 'audit.oob'; MonitorRun = (Join-Path $OutDir 'k57-run') }, @{ Action = 'vbox.status' }, @{ Action = 'calib.dry-run'; Scenario = 'nominal' },
        @{ Action = 'emergency.status' }, @{ Action = 'audit.vmconfig'; Vm = 'P9S-D12'; Reference = (Join-Path $OutDir 'k57-reference.txt') }, @{ Action = 'calib.preflight' })
    Set-Content -LiteralPath (Join-Path $OutDir 'k57-reference.txt') -Value 'memory=4096'; New-Item -ItemType Directory -Path (Join-Path $OutDir 'k57-run') | Out-Null
    foreach ($s in @('STATE_UNCERTAIN', 'MIGRATION_REQUIRED', 'NOT_INITIALIZED', 'ANCHOR_MISSING')) {
        foreach ($rq in $obs) {
            $c = New-MockCtx @($READY_A) @{ 'P9S-D12' = 'running' }; Set-Gate $c @($s); Watch-VmReads $c
            $c.ReadVmConfig = { param($vm) $script:k57cfg++; @() }; $script:k57cfg = 0
            $r = Act $c $rq
            Expect ($r.outcome -cne 'REFUSED' -or -not (@($r.reasons) | Where-Object { $_ -clike 'STATE_NOT_READY*' })) "$s $($rq.Action): refused for the state: $(Why $r)"
            Expect ($c.VmReads.Count -eq 0 -and $script:k57cfg -eq 0 -and $c.Calls.Count -eq 0) "$s $($rq.Action): VM reads $($c.VmReads.Count), configuration reads $script:k57cfg, calls $($c.Calls.Count)"
            Expect ((Files $c) -ceq '') "$s $($rq.Action): files written: $(Files $c)"
            Expect (($r.lines -join ' ') -match 'nothing is written and VirtualBox is not called') "$s $($rq.Action): $($r.lines -join ' | ')"
        }
    }
    $c = New-MockCtx @($READY_A); Watch-VmReads $c
    $r = Act $c @{ Action = 'emergency.status' }
    Expect ($c.VmReads.Count -eq 2 -and (Files $c) -match 'coordinator-audit') "READY: the observe action reads and records as before: reads $($c.VmReads.Count), files $(Files $c)"
}
Case 'K58' 'SANDBOX (-StatePath elsewhere): what reaches a VM is refused, a dry run is shown, the rest runs in the sandbox; the dispatch recheck refuses a SANDBOX gate whatever the caller decided' {
    foreach ($rq in @(@{ Action = 'vm.start'; Vm = 'P9S-D12' }, @{ Action = 'guest.key'; Vm = 'P9S-D12'; Key = 'Enter' }, @{ Action = 'guest.type'; Vm = 'P9S-D12'; Text = 'abc' }, @{ Action = 'guest.unlock'; Vm = 'P9S-D12' },
                      @{ Action = 'sequence.run'; Plan = $plan56 }, @{ Action = 'emergency.acpi-shutdown'; Vm = 'P9S-D12' })) {
        $c = New-MockCtx @($READY_A) @{ 'P9S-D12' = 'running' }; Set-Gate $c @('SANDBOX'); $null = Open $c
        $r = Act $c $rq
        Expect ($r.outcome -ceq 'REFUSED' -and (Has $r 'STATE_SANDBOX') -and $c.Calls.Count -eq 0) "$($rq.Action): $(Why $r); calls $($c.Calls.Count)"
        Expect (-not (Test-Path -LiteralPath $c.StopRequestPath)) "$($rq.Action): a stop request was raised"
    }
    $c = New-MockCtx @($READY_A) @{ 'P9S-D12' = 'off' }; Set-Gate $c @('SANDBOX')
    $o = Open $c; Expect ($o.outcome -ceq 'DONE') "session.open in a sandbox: $(Why $o)"
    $d = Act $c @{ Action = 'vm.start'; Vm = 'P9S-D12'; DryRun = $true }; Expect ($d.outcome -ceq 'DRYRUN' -and $c.Calls.Count -eq 0) "vm.start -DryRun: $(Why $d)"
    $e = Act $c @{ Action = 'emergency.acpi-shutdown'; Vm = 'P9S-D12'; DryRun = $true }; Expect ($e.outcome -ceq 'DRYRUN' -and $c.Calls.Count -eq 0) "emergency -DryRun: $(Why $e)"
    $res = New-P9SResult 'vm.start' 'P9S-D12'
    $msg = ''; try { Invoke-P9SDispatch $c $res 'vm.start' @('P9S-D12') } catch { $msg = $_.Exception.Message }
    Expect ($msg -match 'not dispatched: STATE_NOT_READY:SANDBOX' -and $c.Calls.Count -eq 0 -and (Has $res 'STATE_NOT_READY:SANDBOX')) "direct dispatch in a sandbox: '$msg', calls $($c.Calls.Count)"
}
Case 'K59' 'READY when the action starts, another status under the lock: refused before the session, the monitor or a VM is read, nothing written' {
    foreach ($s in 'STATE_UNCERTAIN', 'ANCHOR_MISSING', 'STATE_DIVERGED', 'READY@inst-other0000') {
        $c = New-MockCtx @($READY_A) @{ 'P9S-D12' = 'running' }; $null = Open $c
        $sess = Get-Content -Raw -LiteralPath $c.SessionPath; $aud = Audit $c
        Set-Gate $c @('READY', $s); Watch-VmReads $c; $c.Prov.reads = 0
        $r = Act $c @{ Action = 'guest.key'; Vm = 'P9S-D12'; Key = 'Enter' }
        $want = if ($s -like 'READY@*') { 'STATE_INSTALLATION_CHANGED' } else { "STATE_NOT_READY:$s" }
        Expect ($r.outcome -ceq 'REFUSED' -and (Has $r $want) -and $c.Calls.Count -eq 0) "${s}: $(Why $r)"
        Expect ($c.GateLog.Count -eq 2 -and $c.VmReads.Count -eq 0 -and $c.Prov.reads -eq 0) "${s}: gate asked $($c.GateLog.Count) times, VM reads $($c.VmReads.Count), monitor reads $($c.Prov.reads)"
        Expect ((Get-Content -Raw -LiteralPath $c.SessionPath) -ceq $sess -and (Audit $c) -ceq $aud) "${s}: the session or the audit changed"
    }
}
Case 'K60' 'READY at the start and under the lock, another status at the last check or at the dispatch itself: nothing is dispatched' {
    # the gate is asked at the start, under the lock, at the last checkpoint
    # and by the dispatch: a change at the checkpoint refuses, a change at
    # the dispatch fails it unsent
    foreach ($at in 3, 4) {
        $c = New-MockCtx @($READY_A) @{ 'P9S-D12' = 'off' }; $null = Open $c
        $q = @(for ($i = 1; $i -lt $at; $i++) { 'READY' }) + @('STATE_UNCERTAIN'); Set-Gate $c $q
        $r = Act $c @{ Action = 'vm.start'; Vm = 'P9S-D12' }
        Expect ($c.Calls.Count -eq 0 -and $r.dispatched -eq 0 -and (Has $r 'STATE_NOT_READY:STATE_UNCERTAIN')) "change at gate call ${at}: $(Why $r); calls $($c.Calls.Count); gate log $($c.GateLog -join ',')"
        if ($at -eq 3) { Expect ($r.outcome -ceq 'REFUSED' -and (Has $r 'STATE_CHANGED_BEFORE_DISPATCH')) "checkpoint: $(Why $r)" }
        else { Expect ($r.outcome -ceq 'FAILED' -and (Has $r 'DISPATCH_FAILED') -and -not (Sess $c).inflight) "dispatch: $(Why $r); in flight $((Sess $c).inflight)" }
        Expect ((Sess $c).expected_vm.'P9S-D12' -ceq 'off') "change at gate call ${at}: the session expects $((Sess $c).expected_vm.'P9S-D12')"
    }
    $c = New-MockCtx @($READY_A) @{ 'P9S-D12' = 'running' }; $null = Open $c
    Set-Gate $c @('READY', 'READY', 'READY', 'READY@inst-other0000')
    $r = Act $c @{ Action = 'guest.unlock'; Vm = 'P9S-D12' }
    Expect ($c.Calls.Count -eq 0 -and (Has $r 'STATE_INSTALLATION_CHANGED')) "another installation at the dispatch: $(Why $r)"
}
Case 'K61' 'a long action and a sequence recheck the persistent state at every checkpoint: typing stops after the chunk already sent, a sequence runs no further step' {
    $c = New-MockCtx @($READY_A) @{ 'P9S-D12' = 'running' }; $null = Open $c
    Set-Gate $c @('READY', 'READY', 'READY', 'READY', 'STATE_UNCERTAIN')
    $r = Act $c @{ Action = 'guest.type'; Vm = 'P9S-D12'; Text = 'abcdefghijklmnopqrstuvwx' }
    Expect ($r.outcome -ceq 'STOPPED_AT_CHECKPOINT' -and $c.Calls.Count -eq 1 -and (Has $r 'STATE_NOT_READY:STATE_UNCERTAIN')) "type: $(Why $r); chunks $($c.Calls.Count)"
    $c = New-MockCtx @($READY_A) @{ 'P9S-D12' = 'running' }; $null = Open $c
    $plan = Join-Path $c.Dir 'plan.json'
    [ordered]@{ schema = 'p9s-plan/1'; name = 'three-keys'; steps = @(@{ Action = 'guest.key'; Vm = 'P9S-D12'; Key = 'Tab' }, @{ Action = 'guest.key'; Vm = 'P9S-D12'; Key = 'Tab' }, @{ Action = 'guest.key'; Vm = 'P9S-D12'; Key = 'Enter' }) } | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $plan
    Set-Gate $c @('READY', 'READY', 'READY', 'READY', 'ANCHOR_MISSING')
    $r = Act $c @{ Action = 'sequence.run'; Plan = $plan }
    Expect ($c.Calls.Count -eq 1 -and $r.outcome -ceq 'STOPPED_AT_CHECKPOINT' -and (Has $r 'STATE_NOT_READY:ANCHOR_MISSING')) "sequence: $(Why $r); dispatched $($c.Calls.Count)"
}
Case 'K62' 'emergency: READY at the start, another status once the lock is held: nothing recorded, nothing sent, the stop request raised before the wait stays; READY throughout, it sends once' {
    foreach ($s in 'STATE_UNCERTAIN', 'READY@inst-other0000') {
        $c = New-MockCtx @($READY_A) @{ 'P9S-D12' = 'running' }; $null = Open $c
        Set-Gate $c @('READY', $s)
        $r = Act $c @{ Action = 'emergency.acpi-shutdown'; Vm = 'P9S-D12' }
        Expect ($r.outcome -ceq 'REFUSED' -and (Has $r 'EMERGENCY_UNRESOLVED') -and $c.Calls.Count -eq 0 -and $r.dispatched -eq 0) "${s}: $(Why $r); calls $($c.Calls.Count)"
        Expect (($r.lines -join ' ') -match 'NOTHING WAS SENT and nothing was recorded') "${s}: $($r.lines -join ' | ')"
        $q = Read-P9SStopRequestState $c
        Expect ($q.ok -and -not $q.doc.acpi.Contains('P9S-D12') -and (Sess $c).lifecycle -ceq 'ACTIVE') "${s}: stop request $($q.ok) acpi $(@($q.doc.acpi.Keys) -join ','); session $((Sess $c).lifecycle)"
    }
    $c = New-MockCtx @($READY_A) @{ 'P9S-D12' = 'running' }; $null = Open $c
    $r = Act $c @{ Action = 'emergency.acpi-shutdown'; Vm = 'P9S-D12' }
    Expect ($r.outcome -ceq 'DONE' -and $c.Calls.Count -eq 1) "READY throughout: $(Why $r)"
    $c = New-MockCtx @($READY_A) @{ 'P9S-D12' = 'running' }; $null = Open $c
    Set-Gate $c @('READY', 'READY', 'STATE_UNCERTAIN')
    $r = Act $c @{ Action = 'emergency.acpi-shutdown'; Vm = 'P9S-D12' }
    $q = Read-P9SStopRequestState $c
    Expect ($r.outcome -ceq 'FAILED' -and (Has $r 'EMERGENCY_UNRESOLVED') -and $c.Calls.Count -eq 0 -and $q.ok -and $q.doc.acpi.Contains('P9S-D12')) "a change at the dispatch: $(Why $r); calls $($c.Calls.Count); the request stays spent: $($q.ok -and $q.doc.acpi.Contains('P9S-D12'))"
}
Case 'K63' 'every VirtualBox write of the coordinator and of the ACPI validation goes through the state recheck' {
    foreach ($f in 'coordinator-lib.ps1', 'p9s-acpival.ps1') {
        $src = [IO.File]::ReadAllText((Join-Path $kit $f))
        $direct = @([regex]::Matches($src, '(?m)^.*Dispatch\[.*$') | ForEach-Object { $_.Value.Trim() })
        $allowed = @($direct | Where-Object { $_ -ceq '& $Ctx.Dispatch[$Name] @Arguments' })
        Expect ($direct.Count -eq $allowed.Count -and ($f -cne 'coordinator-lib.ps1' -or $allowed.Count -eq 1)) "${f}: $($direct.Count) direct dispatcher calls: $($direct -join ' / ')"
    }
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
