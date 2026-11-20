#Requires -Version 7.0
# Synthetic, deterministic tests of the VirtualBox invocation layer
# (p9s-vbox.ps1) and of its use by the coordinator. No VirtualBox command
# runs: "VBoxManage" is a mock script started with pwsh, which takes its
# behaviour from environment variables and from named events set by the
# test; worker processes contend for test-only mutex names; edge cases use
# fake process objects. A second mutex held by every mock while it runs
# detects any overlap. Mutex and event names are unique to this run, so
# parallel runs do not interfere, and no production name is ever acquired.
#   pwsh -NoProfile -File tests\vbox-tests.ps1 -OutDir <new folder>
param([Parameter(Mandatory)][string]$OutDir)
$ErrorActionPreference = 'Stop'
$kit = Split-Path $PSScriptRoot -Parent
. (Join-Path $kit 'hostmon-lib.ps1')
. (Join-Path $kit 'coordinator-lib.ps1')
if (Test-Path -LiteralPath $OutDir) { throw "$OutDir exists" }
New-Item -ItemType Directory -Path $OutDir | Out-Null
$OutDir = (Resolve-Path -LiteralPath $OutDir).Path
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

# ---- mock VBoxManage, workers, names ----------------------------------------------
$pw = (Get-Process -Id $PID).Path
$run = [guid]::NewGuid().ToString('N').Substring(0, 12)
$mock = Join-Path $OutDir 'mockvbox.ps1'
$worker = Join-Path $OutDir 'worker.ps1'
$userPath = 'C:' + '\Us' + 'ers\someone\VirtualBox VMs\P9S-D12\P9S-D12.vbox'
@'
$ErrorActionPreference = 'Stop'
$mode = $env:P9S_MOCK_MODE; $dir = $env:P9S_MOCK_DIR; $tag = $env:P9S_MOCK_TAG
$t0 = [Diagnostics.Stopwatch]::GetTimestamp()
$ov = $null; $overlap = $false
if ($env:P9S_MOCK_OVERLAP) {
    $ov = [Threading.Mutex]::new($false, $env:P9S_MOCK_OVERLAP)
    try { $overlap = -not $ov.WaitOne(0) } catch { $overlap = $false }
}
if ($env:P9S_MOCK_STARTED) { try { $null = [Threading.EventWaitHandle]::OpenExisting($env:P9S_MOCK_STARTED).Set() } catch {} }
if ($mode -eq 'block') { $e = [Threading.EventWaitHandle]::OpenExisting($env:P9S_MOCK_RELEASE); $null = $e.WaitOne(120000) }
if ($env:P9S_MOCK_HOLD_MS) { Start-Sleep -Milliseconds ([int]$env:P9S_MOCK_HOLD_MS) }
$t1 = [Diagnostics.Stopwatch]::GetTimestamp()
if ($ov -and -not $overlap) { $ov.ReleaseMutex() }
[ordered]@{ tag = $tag; pid = $PID; argc = $args.Count; first = [string]$args[0]; start = $t0; end = $t1; overlap = $overlap } | ConvertTo-Json -Compress | Set-Content -LiteralPath (Join-Path $dir "mock-$tag-$PID.json")
switch ($mode) {
    'fail' { [Console]::Error.WriteLine($env:P9S_MOCK_ERRTEXT); exit 1 }
    'empty' { exit 0 }
    'config' { foreach ($l in ($env:P9S_MOCK_CONFIG -split '\|')) { $l }; exit 0 }
    default { 'name="' + $args[1] + '"'; 'VMState="' + $(if ($env:P9S_MOCK_STATE) { $env:P9S_MOCK_STATE } else { 'poweroff' }) + '"'; exit 0 }
}
'@ | Set-Content -LiteralPath $mock -Encoding ascii
@'
param([Parameter(Mandatory)][string]$Spec)
$ErrorActionPreference = 'Stop'
$s = Get-Content -Raw -LiteralPath $Spec | ConvertFrom-Json -AsHashtable
. (Join-Path $s.kit 'hostmon-lib.ps1')
. (Join-Path $s.kit 'p9s-vbox.ps1')
if ($s.cwd) { Set-Location -LiteralPath $s.cwd }
foreach ($k in @($s.env.Keys)) { [Environment]::SetEnvironmentVariable($k, [string]$s.env[$k]) }
$at = [string]$s.crash_at
$onPhase = if ($at) { { param($p) if ($p -eq $at) { [Environment]::Exit(9) } }.GetNewClosure() } else { $null }
$vb = New-P9SVBoxContext -Dir $s.dir -VmNames @('P9S-D12', 'P9S-U24') -MutexName $s.mutex -ExePath $s.exe -PrefixArgs @($s.prefix) -OnPhase $onPhase `
    -FailLatchWrite:([bool]$s.fail_latch) -LockWaitMs ([int]$s.lock_ms) -ExecMs @{ 'vm.state' = [int]$s.exec_ms; 'vm.start' = [int]$s.exec_ms } -ConfirmMs ([int]$s.confirm_ms)
if ($s.ready) { $null = [Threading.EventWaitHandle]::OpenExisting($s.ready).Set() }
if ($s.go) { $null = [Threading.EventWaitHandle]::OpenExisting($s.go).WaitOne(60000) }
$t0 = [Diagnostics.Stopwatch]::GetTimestamp()
$r = switch ($s.mode) {
    'recover' { Invoke-P9SVBoxRecovery -VBox $vb -OperationId $s.recover_id }
    'name' { [pscustomobject]@{ name = (Get-P9SVBoxMutexName); cwd = (Get-Location).Path } }
    'inproc' { Invoke-P9SVBoxInProcess -VBox $vb -Operation 'guest.keys' -Vm 'P9S-D12' -Body { param($x) $x } -Arguments @('payload') }
    default { Invoke-P9SVBoxOperation -VBox $vb -Operation $s.op -Vm $s.vm }
}
$t1 = [Diagnostics.Stopwatch]::GetTimestamp()
[ordered]@{ result = $r; t0 = $t0; t1 = $t1; pid = $PID } | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $s.out
if ($s.hold_after) { $null = [Threading.EventWaitHandle]::OpenExisting($s.hold_after).WaitOne(120000) }
'@ | Set-Content -LiteralPath $worker -Encoding ascii

$script:events = [System.Collections.Generic.List[object]]::new()
$script:releases = [System.Collections.Generic.List[object]]::new()
$script:caseN = 0
function New-Names {
    $script:caseN++
    $p = "Global\P9S-TEST-$run-$($script:caseN)"
    $d = Join-Path $OutDir ('case-{0:D2}' -f $script:caseN); New-Item -ItemType Directory -Path $d | Out-Null
    [pscustomobject]@{ mutex = "$p-lock"; overlap = "$p-overlap"; prefix = $p; dir = $d; n = 0 }
}
function New-Event([string]$Name, [switch]$Release) {
    $e = [Threading.EventWaitHandle]::new($false, [Threading.EventResetMode]::ManualReset, $Name)
    $script:events.Add($e); if ($Release) { $script:releases.Add($e) }
    $e
}
function Wait-Event($e, [int]$ms = 60000) { if (-not $e.WaitOne($ms)) { throw 'an expected event did not arrive' } }
function Get-MockEnv($N, [string]$Tag, [hashtable]$More = @{}) {
    $h = [ordered]@{ P9S_MOCK_MODE = 'ok'; P9S_MOCK_DIR = $N.dir; P9S_MOCK_TAG = $Tag; P9S_MOCK_OVERLAP = $N.overlap; P9S_MOCK_STARTED = ''; P9S_MOCK_RELEASE = ''; P9S_MOCK_HOLD_MS = ''; P9S_MOCK_STATE = ''; P9S_MOCK_ERRTEXT = ''; P9S_MOCK_CONFIG = '' }
    foreach ($k in $More.Keys) { $h[$k] = $More[$k] }
    $h
}
function Set-MockEnv($N, [string]$Tag, [hashtable]$More = @{}) { $h = Get-MockEnv $N $Tag $More; foreach ($k in $h.Keys) { [Environment]::SetEnvironmentVariable($k, [string]$h[$k]) } }
function New-TestVBox($N, [int]$LockMs = 60000, [int]$ExecMs = 60000, [int]$ConfirmMs = 5000, [scriptblock]$Factory, [scriptblock]$OnPhase, [switch]$FailLatch, [string]$Exe = $pw) {
    New-P9SVBoxContext -Dir $N.dir -VmNames @('P9S-D12', 'P9S-U24') -MutexName $N.mutex -ExePath $Exe -PrefixArgs @('-NoProfile', '-NonInteractive', '-File', $mock) `
        -ProcessFactory $Factory -OnPhase $OnPhase -FailLatchWrite:$FailLatch -LockWaitMs $LockMs -ExecMs @{ 'vm.state' = $ExecMs; 'vm.config' = $ExecMs; 'vm.start' = $ExecMs; 'vm.acpi' = $ExecMs } -ConfirmMs $ConfirmMs
}
function Start-Worker($N, [hashtable]$Spec) {
    $N.n++
    $tag = "w$($N.n)"
    $s = [ordered]@{ kit = $kit; dir = $N.dir; mutex = $N.mutex; exe = $pw; prefix = @('-NoProfile', '-NonInteractive', '-File', $mock); lock_ms = 60000; exec_ms = 60000; confirm_ms = 5000
        mode = 'op'; op = 'vm.state'; vm = 'P9S-D12'; env = (Get-MockEnv $N $tag); out = (Join-Path $N.dir "worker-$tag.json") }
    foreach ($k in $Spec.Keys) { if ($k -eq 'mock') { foreach ($m in $Spec.mock.Keys) { $s.env[$m] = $Spec.mock[$m] } } else { $s[$k] = $Spec[$k] } }
    $sp = Join-Path $N.dir "spec-$tag.json"
    $s | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $sp
    $p = Start-Process -FilePath $pw -PassThru -WindowStyle Hidden -ArgumentList @('-NoProfile', '-NonInteractive', '-File', $worker, '-Spec', $sp)
    $null = $p.Handle
    [pscustomobject]@{ p = $p; out = $s.out; tag = $tag }
}
function Wait-Worker($W, [int]$ms = 120000) {
    if (-not $W.p.WaitForExit($ms)) { throw "worker $($W.tag) did not end within $ms ms" }
    $W.p.WaitForExit()
    if (Test-Path -LiteralPath $W.out) { (Get-Content -Raw -LiteralPath $W.out | ConvertFrom-Json).result } else { $null }
}
function Mocks($N, [string]$Tag = '*') { @(Get-ChildItem -LiteralPath $N.dir -Filter "mock-$Tag-*.json" -ErrorAction SilentlyContinue | ForEach-Object { Get-Content -Raw -LiteralPath $_.FullName | ConvertFrom-Json }) }
function Latch($N) { $q = Read-P9SVBoxJson (Join-Path $N.dir 'vbox-quarantine.json') 'p9s-vbox-quarantine/1'; if ($q.present) { $q.doc } else { $null } }
function Wait-Phase($N, [string]$Phase, [int]$ms = 60000) {
    $sw = [Diagnostics.Stopwatch]::StartNew()
    while ($sw.ElapsedMilliseconds -lt $ms) { $r = Read-P9SVBoxJson (Join-Path $N.dir 'vbox-operation.json') 'p9s-vbox-operation/1'; if ($r.ok -and $r.doc -and $r.doc.phase -eq $Phase) { return }; Start-Sleep -Milliseconds 50 }
    throw "phase $Phase not reached"
}
function Test-LockFree($N) { $h = Enter-P9SVBoxMutex $N.mutex 2000; if ($h.ok) { Exit-P9SVBoxMutex $h $true $N.mutex; $true } else { $false } }
function Files($N) { (Get-ChildItem -LiteralPath $N.dir -File | Where-Object { $_.Name -like 'vbox-*' } | ForEach-Object { Get-Content -Raw -LiteralPath $_.FullName }) -join "`n" }
# a fake child for the in-process edge cases: exits after $ExitAt ms of waiting
function New-FakeFactory([int]$ExitAt, [switch]$EndedThrows) {
    $sim = @{ waited = 0; exitAt = $ExitAt; throwEnded = [bool]$EndedThrows }
    $wait = { param($ms) $sim.waited += [int]$ms; $sim.exitAt -le $sim.waited }.GetNewClosure()
    $ended = { if ($sim.throwEnded) { throw 'access denied' }; $sim.exitAt -le $sim.waited }.GetNewClosure()
    $obj = [pscustomobject]@{ Id = 4000000; StartTicks = 1L; Wait = $wait; Ended = $ended; Code = { 0 }
        Output = { param($ms) [pscustomobject]@{ complete = $true; out = 'VMState="poweroff"'; err = '' } } }
    { param($psi) $obj }.GetNewClosure()
}

# ---- monitor states and a mock coordinator context ------------------------------
$GiB = [math]::Pow(2, 30)
function S([double]$t, $commit = 50) {
    [pscustomobject]@{
        seq = 0; mono = $t; utc = 'x'; read_ok = $true; read_error = $null; committed_bytes = [double]$commit * $GiB; commit_limit_bytes = 100 * $GiB
        avail_mb = 16000.0; modified_list_mb = 200.0; pages = 100.0; pages_in = 100.0; pages_out = 0.0; page_reads = 90.0; page_writes = 0.0
        page_faults = 1000.0; transition_faults = 500.0; cache_faults = 100.0; demand_zero_faults = 300.0; rate_issues = @()
        pagefile_used_mb = 100.0; pagefile_alloc_mb = 5888.0; pagefile_pct = 1.7; pagefile_disks = @(0); system_disk = 0; external_disk = 2
        disks = @{ 0 = [pscustomobject]@{ read_ms = 0.5; write_ms = 0.3; read_mbps = 5.0; write_mbps = 1.0; queue = 0; avg_queue = 0.01 }
                   2 = [pscustomobject]@{ read_ms = 2.0; write_ms = 1.0; read_mbps = 0.1; write_mbps = 0.1; queue = 0; avg_queue = 0.01 } }
        external_present = $true; external_identity_ok = $true; eventlog_ok = $true; storage_events_new = 0; storage_event_ids = @(); cpu_pct = 10.0; vm = @{}
    }
}
function Feed($st, [double]$from, [double]$to, $commit = 50) { $d = $null; for ($t = $from; $t -le $to + 1e-9; $t += 5) { $d = Step-Monitor -State $st -Sample (S $t $commit) }; $d }
$thr = [ordered]@{
    suspect = [ordered]@{ page_out_per_s = 500; avail_mb_low = 4096; avail_decline_mb = 4096; pagefile_growth_mb = 128; pagefile_disk_read_ms = 20; page_in_per_s_with_low_memory = 2000 }
    confirm = [ordered]@{ page_out_per_s = 2000; avail_mb_low = 2048; avail_decline_mb = 6144; pagefile_growth_mb = 300; pagefile_disk_read_ms = 50; page_in_per_s_with_low_memory = 5000 }
}
$fxApproved = ConvertTo-PagingThresholds -Json ([ordered]@{ schema = 'p9s-hostmon-paging/1'; approved = $true; approved_by = 'test fixture only'; approved_utc = '2026-11-19T00:00:00Z'; source = 'test'; suspect = $thr.suspect; confirm = $thr.confirm } | ConvertTo-Json -Depth 5)
$F = 10000000L; $NOW = 1000L * $F
function J($doc) { ConvertTo-StateJson -Doc $doc -Monitor ([ordered]@{ pid = 4242; process_start_utc = 'x'; process_start_ticks = 1L; mode = 'Monitor'; paging_stop = 'Required'; thresholds_sha256 = 'x'; thresholds_approved = $true; test = $false; kit = 'test' }) -WrittenQpc ([long]($NOW - $F)) -QpcFrequency $F -WrittenUtc 'x' }
function NewMon([string]$run) { New-MonitorState -RunId $run -Config (New-HostMonConfig -PagingStop Required -Mode Monitor -Thresholds $fxApproved) }
$mA = NewMon 'run-A'; $READY_A = J (Feed $mA 0 300); $STOP_A = J (Feed $mA 305 335 95)
$mA2 = NewMon 'run-A'; $null = Feed $mA2 0 300; $ABORT_A = J (Feed $mA2 305 305 97)
$script:ctxN = 0
function New-MockCtx([object[]]$States, [hashtable]$Vms = @{}, $VBox) {
    $script:ctxN++
    $dir = Join-Path $OutDir ('ctx-{0:D2}' -f $script:ctxN); New-Item -ItemType Directory -Path $dir | Out-Null
    $calls = [System.Collections.Generic.List[object]]::new()
    $q = [System.Collections.Generic.Queue[object]]::new(); foreach ($x in $States) { $q.Enqueue($x) }
    $prov = @{ queue = $q; reads = 0; vmreads = 0; onText = $null }
    $vmTable = @{ 'P9S-D12' = 'off'; 'P9S-U24' = 'off' }; foreach ($k in $Vms.Keys) { $vmTable[$k] = $Vms[$k] }
    $readState = { param($p) if ($prov.queue.Count -gt 1) { $x = $prov.queue.Dequeue() } else { $x = $prov.queue.Peek() }; [pscustomobject]@{ ok = $true; text = $x; error = $null } }.GetNewClosure()
    $rec = { param($kind, $list) { param($vm, $arg) $list.Add([pscustomobject]@{ kind = $kind; vm = $vm; arg = $arg }) }.GetNewClosure() }
    $text = { param($list, $p) { param($vm, $arg) $list.Add([pscustomobject]@{ kind = 'guest.text'; vm = $vm; arg = $arg }); if ($p.onText) { & $p.onText } }.GetNewClosure() }
    [pscustomobject]@{
        StatePath = Join-Path $dir 'state.json'; SessionPath = Join-Path $dir 'coordinator-session.json'; LockPath = Join-Path $dir 'coordinator.lock'
        StopRequestPath = Join-Path $dir 'coordinator-stop.request'; AuditPath = Join-Path $dir 'coordinator-audit.jsonl'; ThresholdsPath = Join-Path $kit 'paging-thresholds.json'
        ReadState = $readState; NowQpc = { $NOW }.GetNewClosure(); QpcFrequency = $F; IsMonitorAlive = { param($a, $b) $true }
        ReadVmState = { param($vm) $prov.vmreads++; $vmTable[$vm] }.GetNewClosure(); ReadVmConfig = { param($vm) @() }
        Dispatch = @{ 'vm.start' = (& $rec 'vm.start' $calls); 'guest.text' = (& $text $calls $prov); 'guest.key' = (& $rec 'guest.key' $calls); 'guest.unlock' = (& $rec 'guest.unlock' $calls); 'vm.acpi' = (& $rec 'vm.acpi' $calls) }
        Sleep = { param($ms) }; Pid = $PID; ProcessStartTicks = (Get-Process -Id $PID).StartTime.ToUniversalTime().Ticks
        EmergencyLockWaitMs = 1500; CalibrationHostChecks = { @() }; VBox = $VBox
        Calls = $calls; Prov = $prov; VmTable = $vmTable; Dir = $dir
    }
}
function Act($ctx, [hashtable]$r) { Invoke-P9SAction -Ctx $ctx -Request $r }
function Has($res, [string]$code) { @($res.reasons) -contains $code }
function Put-Latch($N, [string]$Reason = 'VBOX_COMMAND_TIMEOUT', $Rec = $null) {
    $vb = New-TestVBox $N
    $null = Set-P9SVBoxLatch $vb $Reason $Rec 'test fixture'
    (Latch $N).operation_id
}

"== P9-S VirtualBox layer synthetic tests, $((Get-Date).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ')), pwsh $($PSVersionTable.PSVersion), run $run"

# ---- serialization -------------------------------------------------------------------
Case 'V01' 'separate processes, different VMs: the second waits for the lock and runs only after the first' {
    $n = New-Names
    $rel = New-Event "$($n.prefix)-rel" -Release; $sa = New-Event "$($n.prefix)-sa"; $rb = New-Event "$($n.prefix)-rb"
    $a = Start-Worker $n @{ vm = 'P9S-D12'; mock = @{ P9S_MOCK_MODE = 'block'; P9S_MOCK_RELEASE = "$($n.prefix)-rel"; P9S_MOCK_STARTED = "$($n.prefix)-sa" } }
    Wait-Event $sa
    $b = Start-Worker $n @{ vm = 'P9S-U24'; ready = "$($n.prefix)-rb" }
    Wait-Event $rb
    Start-Sleep -Milliseconds 1500
    Expect ((Mocks $n $b.tag).Count -eq 0 -and -not $b.p.HasExited) 'the second command started while the first one ran'
    $null = $rel.Set()
    $ra = Wait-Worker $a; $rbx = Wait-Worker $b
    Expect ($ra.outcome -eq 'COMPLETED' -and $rbx.outcome -eq 'COMPLETED') "outcomes $($ra.outcome) $($rbx.outcome)"
    $ma = @(Mocks $n $a.tag)[0]; $mb = @(Mocks $n $b.tag)[0]
    Expect ($ma -and $mb -and [long]$mb.start -ge [long]$ma.end) 'the second command started before the first ended'
    Expect ($ma -and $mb -and -not $ma.overlap -and -not $mb.overlap) 'overlap detected'
    Expect ([int]$rbx.lock_wait_ms -ge 1000) "second process waited only $($rbx.lock_wait_ms) ms"
    Expect (-not (Latch $n)) 'quarantine after normal operations'
}
Case 'V02' 'six simultaneous read-only requests from six processes never overlap' {
    $n = New-Names
    $go = New-Event "$($n.prefix)-go"
    $ws = @(for ($i = 1; $i -le 6; $i++) { $r = New-Event "$($n.prefix)-ready$i"; [pscustomobject]@{ ev = $r; w = (Start-Worker $n @{ vm = $(if ($i % 2) { 'P9S-D12' } else { 'P9S-U24' }); ready = "$($n.prefix)-ready$i"; go = "$($n.prefix)-go"; mock = @{ P9S_MOCK_HOLD_MS = '300' } }) } })
    foreach ($x in $ws) { Wait-Event $x.ev }
    $null = $go.Set()
    $rs = @(foreach ($x in $ws) { Wait-Worker $x.w })
    Expect (@($rs | Where-Object { $_.outcome -eq 'COMPLETED' }).Count -eq 6) "completed: $(@($rs | ForEach-Object outcome) -join ',')"
    $ms = @(Mocks $n | Sort-Object { [long]$_.start })
    Expect ($ms.Count -eq 6) "mock commands: $($ms.Count)"
    Expect (@($ms | Where-Object { $_.overlap }).Count -eq 0) 'overlap detector fired'
    for ($i = 1; $i -lt $ms.Count; $i++) { Expect ([long]$ms[$i].start -ge [long]$ms[$i - 1].end) "commands $($i - 1) and $i overlap in time" }
}
Case 'V03' 'a lock wait that times out runs nothing and quarantines; later operations are refused' {
    $n = New-Names
    $rel = New-Event "$($n.prefix)-rel" -Release; $sa = New-Event "$($n.prefix)-sa"
    $a = Start-Worker $n @{ mock = @{ P9S_MOCK_MODE = 'block'; P9S_MOCK_RELEASE = "$($n.prefix)-rel"; P9S_MOCK_STARTED = "$($n.prefix)-sa" } }
    Wait-Event $sa
    Set-MockEnv $n 'late'
    $r = Invoke-P9SVBoxOperation -VBox (New-TestVBox $n -LockMs 800) -Operation 'vm.state' -Vm 'P9S-U24'
    Expect ($r.outcome -eq 'LOCK_TIMEOUT' -and $r.latched -and (Mocks $n 'late').Count -eq 0) "$($r.outcome) latched $($r.latched) mocks $((Mocks $n 'late').Count)"
    Expect ((Latch $n).reason -eq 'VBOX_LOCK_TIMEOUT') "latch $((Latch $n).reason)"
    $null = $rel.Set(); $ra = Wait-Worker $a
    Expect ($ra.outcome -eq 'COMPLETED') "holder $($ra.outcome)"
    $r2 = Invoke-P9SVBoxOperation -VBox (New-TestVBox $n) -Operation 'vm.state' -Vm 'P9S-D12'
    Expect ($r2.outcome -eq 'QUARANTINED' -and (Mocks $n 'late').Count -eq 0) "after: $($r2.outcome)"
}

# ---- timeouts ------------------------------------------------------------------------
Case 'V04' 'command timeout, child ends during the confirmation window: TIMED_OUT_EXITED, quarantined, never success' {
    $n = New-Names
    $rel = New-Event "$($n.prefix)-rel" -Release
    $a = Start-Worker $n @{ exec_ms = 1500; confirm_ms = 30000; mock = @{ P9S_MOCK_MODE = 'block'; P9S_MOCK_RELEASE = "$($n.prefix)-rel" } }
    Wait-Phase $n 'TIMED_OUT'
    $null = $rel.Set()
    $r = Wait-Worker $a
    Expect ($r.outcome -eq 'TIMED_OUT_EXITED' -and $r.reason -eq 'VBOX_COMMAND_TIMEOUT' -and $r.latched) "$($r.outcome) $($r.reason) latched $($r.latched)"
    Expect ([int]$r.elapsed_ms -ge 1500 -and $r.outcome -ne 'COMPLETED') "elapsed $($r.elapsed_ms)"
    Expect ((Latch $n).reason -eq 'VBOX_COMMAND_TIMEOUT') 'no quarantine'
}
Case 'V05' 'child still running after the timeout: TIMED_OUT_ALIVE with its id, lock released only after the quarantine; a fresh process is refused' {
    $n = New-Names
    $rel = New-Event "$($n.prefix)-rel" -Release
    $a = Start-Worker $n @{ exec_ms = 1500; confirm_ms = 1500; mock = @{ P9S_MOCK_MODE = 'block'; P9S_MOCK_RELEASE = "$($n.prefix)-rel" } }
    $r = Wait-Worker $a
    $l = Latch $n
    Expect ($r.outcome -eq 'TIMED_OUT_ALIVE' -and $r.reason -eq 'VBOX_CHILD_UNRESOLVED' -and $r.latched -and -not $r.retained) "$($r.outcome) $($r.reason) latched $($r.latched)"
    Expect ($l -and [int]$l.child_pid -eq [int]$r.child_pid -and (Test-P9SChildAlive $l.child_pid $l.child_start_ticks) -eq 'alive') "latch child $($l.child_pid) vs $($r.child_pid)"
    Expect (Test-LockFree $n) 'the lock stayed taken'
    $b = Start-Worker $n @{ vm = 'P9S-U24' }
    $rb = Wait-Worker $b
    Expect ($rb.outcome -eq 'QUARANTINED' -and (Mocks $n $b.tag).Count -eq 0) "fresh process: $($rb.outcome)"
    $null = $rel.Set()
    $sw = [Diagnostics.Stopwatch]::StartNew(); while ((Test-P9SChildAlive $l.child_pid $l.child_start_ticks) -eq 'alive' -and $sw.ElapsedMilliseconds -lt 30000) { Start-Sleep -Milliseconds 100 }
    Expect ((Latch $n) -and (Latch $n).operation_id -eq $l.operation_id) 'the quarantine went away by itself'
}
Case 'V06' 'the end of the child cannot be confirmed: TERMINATION_UNVERIFIED, quarantined' {
    $n = New-Names
    $r = Invoke-P9SVBoxOperation -VBox (New-TestVBox $n -Factory (New-FakeFactory -ExitAt 10 -EndedThrows)) -Operation 'vm.state' -Vm 'P9S-D12'
    Expect ($r.outcome -eq 'TERMINATION_UNVERIFIED' -and $r.latched -and (Latch $n).reason -eq 'VBOX_TERMINATION_UNVERIFIED') "$($r.outcome) $($r.reason)"
}
Case 'V07' 'boundary: an exit within the limit is COMPLETED, one after it is not' {
    $n = New-Names
    $r1 = Invoke-P9SVBoxOperation -VBox (New-TestVBox $n -ExecMs 1000 -ConfirmMs 1000 -Factory (New-FakeFactory -ExitAt 1000)) -Operation 'vm.state' -Vm 'P9S-D12'
    Expect ($r1.outcome -eq 'COMPLETED' -and -not $r1.latched) "at the limit: $($r1.outcome)"
    $n2 = New-Names
    $r2 = Invoke-P9SVBoxOperation -VBox (New-TestVBox $n2 -ExecMs 1000 -ConfirmMs 1000 -Factory (New-FakeFactory -ExitAt 1001)) -Operation 'vm.state' -Vm 'P9S-D12'
    Expect ($r2.outcome -eq 'TIMED_OUT_EXITED' -and $r2.latched) "just after: $($r2.outcome)"
    $n3 = New-Names
    $r3 = Invoke-P9SVBoxOperation -VBox (New-TestVBox $n3 -ExecMs 1000 -ConfirmMs 1000 -Factory (New-FakeFactory -ExitAt 2001)) -Operation 'vm.state' -Vm 'P9S-D12'
    Expect ($r3.outcome -eq 'TIMED_OUT_ALIVE' -and $r3.latched) "after the confirmation: $($r3.outcome)"
}
Case 'V08' 'non-zero exit: ERROR with a sanitized diagnostic, no quarantine, lock released' {
    $n = New-Names
    Set-MockEnv $n 'fail' @{ P9S_MOCK_MODE = 'fail'; P9S_MOCK_ERRTEXT = "VBoxManage: error: Could not find a registered machine at $userPath; VBOX_E_OBJECT_NOT_FOUND; token=ABC123SECRET" }
    $r = Invoke-P9SVBoxOperation -VBox (New-TestVBox $n) -Operation 'vm.state' -Vm 'P9S-D12'
    Expect ($r.outcome -eq 'ERROR' -and $r.exit_code -eq 1 -and $r.reason -eq 'VBOX_COMMAND_FAILED' -and -not $r.latched) "$($r.outcome) $($r.exit_code)"
    Expect ($r.diag -match 'VBOX_E_OBJECT_NOT_FOUND' -and $r.diag -notmatch 'someone|ABC123SECRET|registered|:\\') "diag: $($r.diag)"
    $all = Files $n
    Expect ($all -notmatch 'someone|ABC123SECRET|registered machine') 'error text in the records'
    Expect (Test-LockFree $n) 'lock not released'
    Expect (-not (Latch $n) -and -not (Test-Path -LiteralPath (Join-Path $n.dir 'vbox-operation.json'))) 'record or quarantine left'
}
Case 'V09' 'exit without a readable result gives unreadable, never off; normal completion releases the lock at once' {
    $n = New-Names
    $vb = New-TestVBox $n
    Set-MockEnv $n 'empty' @{ P9S_MOCK_MODE = 'empty' }
    $s = Get-P9SVmStateReal 'P9S-D12' $vb
    Expect ($s -eq 'unreadable' -and $script:P9SLastVBoxResult.outcome -eq 'COMPLETED') "state $s"
    Set-MockEnv $n 'ok' @{ P9S_MOCK_STATE = 'running' }
    $s2 = Get-P9SVmStateReal 'P9S-U24' $vb
    Expect ($s2 -eq 'running' -and [int]$script:P9SLastVBoxResult.lock_wait_ms -lt 1000) "state $s2 wait $($script:P9SLastVBoxResult.lock_wait_ms)"
    Set-MockEnv $n 'ok2'
    Expect ((Get-P9SVmStateReal 'P9S-D12' $vb) -eq 'off') 'poweroff not mapped to off'
}

# ---- crashes and recovery --------------------------------------------------------------
Case 'V10' 'owner ends before starting VBoxManage: taken over without quarantine, whether Windows reports the lock abandoned or not' {
    # with a handle held here, Windows keeps the mutex and reports it abandoned
    $n = New-Names
    $keep = [Threading.Mutex]::new($false, $n.mutex)
    $a = Start-Worker $n @{ crash_at = 'ACQUIRED' }
    $null = Wait-Worker $a
    Expect ($a.p.ExitCode -eq 9) "worker exit $($a.p.ExitCode)"
    Set-MockEnv $n 'next'
    $r = Invoke-P9SVBoxOperation -VBox (New-TestVBox $n) -Operation 'vm.state' -Vm 'P9S-D12'
    $keep.Dispose()
    Expect ($r.abandoned -and $r.outcome -eq 'COMPLETED' -and $r.note -eq 'VBOX_LOCK_ABANDONED_NO_CHILD' -and -not (Latch $n)) "held handle: $($r.outcome) abandoned $($r.abandoned) note $($r.note)"
    # without another handle the mutex disappears with its owner; the record is the evidence
    $n2 = New-Names
    $b = Start-Worker $n2 @{ crash_at = 'ACQUIRED' }
    $null = Wait-Worker $b
    Set-MockEnv $n2 'next'
    $r2 = Invoke-P9SVBoxOperation -VBox (New-TestVBox $n2) -Operation 'vm.state' -Vm 'P9S-D12'
    Expect (-not $r2.abandoned -and $r2.outcome -eq 'COMPLETED' -and $r2.note -eq 'VBOX_OWNER_ENDED_NO_CHILD' -and -not (Latch $n2)) "no handle: $($r2.outcome) abandoned $($r2.abandoned) note $($r2.note)"
}
Case 'V11' 'owner ends while VBoxManage runs: abandoned, the child outlives it, quarantined with its id, left alone' {
    $n = New-Names
    $rel = New-Event "$($n.prefix)-rel" -Release
    $a = Start-Worker $n @{ crash_at = 'RUNNING'; mock = @{ P9S_MOCK_MODE = 'block'; P9S_MOCK_RELEASE = "$($n.prefix)-rel" } }
    $null = Wait-Worker $a
    Expect ($a.p.ExitCode -eq 9) "worker exit $($a.p.ExitCode)"
    $r = Invoke-P9SVBoxOperation -VBox (New-TestVBox $n) -Operation 'vm.state' -Vm 'P9S-D12'
    $l = Latch $n
    Expect ($r.outcome -eq 'QUARANTINED' -and $r.reason -eq 'VBOX_CHILD_UNRESOLVED') "$($r.outcome) $($r.reason)"
    Expect ($l -and $l.child_pid -and (Test-P9SChildAlive $l.child_pid $l.child_start_ticks) -eq 'alive') 'child not recorded or not alive'
    $null = $rel.Set()
    $sw = [Diagnostics.Stopwatch]::StartNew(); while ((Test-P9SChildAlive $l.child_pid $l.child_start_ticks) -eq 'alive' -and $sw.ElapsedMilliseconds -lt 30000) { Start-Sleep -Milliseconds 100 }
    Expect ((Test-P9SChildAlive $l.child_pid $l.child_start_ticks) -eq 'gone' -and (Latch $n)) 'child did not end, or the quarantine went away'
}
Case 'V12' 'owner ends while starting VBoxManage: VBOX_API_STATE_UNKNOWN, and the coordinator does not open a session' {
    $n = New-Names
    $a = Start-Worker $n @{ crash_at = 'STARTING' }
    $null = Wait-Worker $a
    $vb = New-TestVBox $n
    $r = Invoke-P9SVBoxOperation -VBox $vb -Operation 'vm.state' -Vm 'P9S-D12'
    Expect ($r.outcome -eq 'QUARANTINED' -and $r.reason -eq 'VBOX_API_STATE_UNKNOWN') "$($r.outcome) $($r.reason)"
    $c = New-MockCtx @($READY_A) @{} $vb
    $o = Act $c @{ Action = 'session.open'; Kind = 'Test' }
    Expect ($o.outcome -eq 'REFUSED' -and (Has $o 'VBOX_API_QUARANTINED') -and (Has $o 'VBOX_API_STATE_UNKNOWN') -and $c.Prov.vmreads -eq 0) "open: $($o.outcome) $($o.reasons -join ',') reads $($c.Prov.vmreads)"
}
Case 'V13' 'incomplete or unreadable operation records quarantine' {
    $n = New-Names
    $a = Start-Worker $n @{ crash_at = 'ACQUIRED' }; $null = Wait-Worker $a
    $p = Join-Path $n.dir 'vbox-operation.json'
    $d = Get-Content -Raw -LiteralPath $p | ConvertFrom-Json -AsHashtable; $d.phase = 'RUNNING'; $d.child_pid = $null
    $d | ConvertTo-Json | Set-Content -LiteralPath $p
    $r = Invoke-P9SVBoxOperation -VBox (New-TestVBox $n) -Operation 'vm.state' -Vm 'P9S-D12'
    Expect ($r.outcome -eq 'QUARANTINED' -and $r.reason -eq 'VBOX_RECORD_INCOMPLETE') "incomplete: $($r.outcome) $($r.reason)"
    $n2 = New-Names
    $b = Start-Worker $n2 @{ crash_at = 'ACQUIRED' }; $null = Wait-Worker $b
    Set-Content -LiteralPath (Join-Path $n2.dir 'vbox-operation.json') -Value '{ not json'
    $r2 = Invoke-P9SVBoxOperation -VBox (New-TestVBox $n2) -Operation 'vm.state' -Vm 'P9S-D12'
    Expect ($r2.outcome -eq 'QUARANTINED' -and $r2.reason -eq 'VBOX_RECORD_UNREADABLE') "unreadable: $($r2.outcome) $($r2.reason)"
}
Case 'V14' 'a record of a process that still runs but no longer holds the lock is stale: quarantined' {
    $n = New-Names
    $ticks = (Get-Process -Id $PID).StartTime.ToUniversalTime().Ticks
    [ordered]@{ schema = 'p9s-vbox-operation/1'; operation_id = (New-P9SVBoxOperationId); operation = 'vm.state'; vm = 'P9S-D12'; phase = 'RUNNING'; owner_pid = $PID; owner_start_ticks = $ticks; child_pid = $PID; child_start_ticks = $ticks } |
        ConvertTo-Json | Set-Content -LiteralPath (Join-Path $n.dir 'vbox-operation.json')
    $r = Invoke-P9SVBoxOperation -VBox (New-TestVBox $n) -Operation 'vm.state' -Vm 'P9S-D12'
    Expect ($r.outcome -eq 'QUARANTINED' -and $r.reason -eq 'VBOX_RECORD_STALE' -and -not $r.abandoned) "$($r.outcome) $($r.reason)"
}
Case 'V15' 'two simultaneous recoveries: exactly one clears, their queries never overlap' {
    $n = New-Names
    $id = Put-Latch $n
    $go = New-Event "$($n.prefix)-go"; $r1 = New-Event "$($n.prefix)-r1"; $r2 = New-Event "$($n.prefix)-r2"
    $a = Start-Worker $n @{ mode = 'recover'; recover_id = $id; ready = "$($n.prefix)-r1"; go = "$($n.prefix)-go"; mock = @{ P9S_MOCK_HOLD_MS = '300' } }
    $b = Start-Worker $n @{ mode = 'recover'; recover_id = $id; ready = "$($n.prefix)-r2"; go = "$($n.prefix)-go"; mock = @{ P9S_MOCK_HOLD_MS = '300' } }
    Wait-Event $r1; Wait-Event $r2; $null = $go.Set()
    $x = @((Wait-Worker $a), (Wait-Worker $b))
    Expect (@($x | Where-Object { $_.outcome -eq 'CLEARED' }).Count -eq 1 -and @($x | Where-Object { $_.outcome -eq 'REFUSED' -and @($_.reasons) -contains 'VBOX_NOT_QUARANTINED' }).Count -eq 1) "outcomes: $(@($x | ForEach-Object { "$($_.outcome) $(@($_.reasons) -join '+')" }) -join ' / ')"
    Expect (@(Mocks $n | Where-Object { $_.overlap }).Count -eq 0 -and (Mocks $n).Count -eq 2) "queries $((Mocks $n).Count)"
    Expect (-not (Latch $n)) 'quarantine still in place'
}
Case 'V16' 'recovery refuses a wrong id or a running child, and keeps the quarantine when a query fails' {
    $n = New-Names
    $me = Get-Process -Id $PID
    $id = Put-Latch $n 'VBOX_CHILD_UNRESOLVED' ([ordered]@{ operation_id = (New-P9SVBoxOperationId); operation = 'vm.state'; vm = 'P9S-D12'; child_pid = $PID; child_start_ticks = $me.StartTime.ToUniversalTime().Ticks })
    $vb = New-TestVBox $n
    $w = Invoke-P9SVBoxRecovery -VBox $vb -OperationId (New-P9SVBoxOperationId)
    Expect ($w.outcome -eq 'REFUSED' -and @($w.reasons) -contains 'VBOX_RECOVERY_ID_MISMATCH') "wrong id: $($w.outcome) $($w.reasons -join ',')"
    $k = Invoke-P9SVBoxRecovery -VBox $vb -OperationId $id
    Expect ($k.outcome -eq 'REFUSED' -and @($k.reasons) -contains 'VBOX_CHILD_STILL_RUNNING' -and (Mocks $n).Count -eq 0) "running child: $($k.outcome) $($k.reasons -join ',')"
    $n2 = New-Names
    $id2 = Put-Latch $n2
    $vb2 = New-TestVBox $n2
    Set-MockEnv $n2 'probe-fail' @{ P9S_MOCK_MODE = 'fail'; P9S_MOCK_ERRTEXT = 'VBOX_E_INVALID_OBJECT_STATE' }
    $f = Invoke-P9SVBoxRecovery -VBox $vb2 -OperationId $id2
    Expect ($f.outcome -eq 'REFUSED' -and @($f.reasons) -contains 'VBOX_RECOVERY_PROBE_FAILED' -and (Latch $n2) -and (Mocks $n2).Count -eq 1) "probe failure: $($f.outcome) $($f.reasons -join ',') queries $((Mocks $n2).Count)"
    Set-MockEnv $n2 'probe-ok'
    $ok = Invoke-P9SVBoxRecovery -VBox $vb2 -OperationId $id2
    Expect ($ok.outcome -eq 'CLEARED' -and -not (Latch $n2) -and $ok.states['P9S-D12'] -eq 'off') "retry: $($ok.outcome)"
}
Case 'V17' 'interrupted while VBoxManage runs: quarantined before the lock is released' {
    $n = New-Names
    $rel = New-Event "$($n.prefix)-rel" -Release
    Set-MockEnv $n 'cancel' @{ P9S_MOCK_MODE = 'block'; P9S_MOCK_RELEASE = "$($n.prefix)-rel" }
    $r = Invoke-P9SVBoxOperation -VBox (New-TestVBox $n -OnPhase { param($p) if ($p -eq 'RUNNING') { throw 'interrupted' } }) -Operation 'vm.state' -Vm 'P9S-D12'
    Expect ($r.outcome -eq 'CANCELLED' -and $r.reason -eq 'VBOX_OPERATION_CANCELLED' -and $r.latched -and (Latch $n).reason -eq 'VBOX_OPERATION_CANCELLED') "$($r.outcome) $($r.reason)"
    Expect (Test-LockFree $n) 'lock not released after the quarantine'
    $null = $rel.Set()
}
Case 'V18' 'a quarantine that cannot be written keeps the lock: no other operation starts' {
    $n = New-Names
    $rel = New-Event "$($n.prefix)-rel" -Release; $hold = New-Event "$($n.prefix)-hold" -Release
    $a = Start-Worker $n @{ exec_ms = 1000; confirm_ms = 1000; fail_latch = $true; hold_after = "$($n.prefix)-hold"; mock = @{ P9S_MOCK_MODE = 'block'; P9S_MOCK_RELEASE = "$($n.prefix)-rel" } }
    $sw = [Diagnostics.Stopwatch]::StartNew(); while (-not (Test-Path -LiteralPath $a.out) -and $sw.ElapsedMilliseconds -lt 60000) { Start-Sleep -Milliseconds 100 }
    $ra = (Get-Content -Raw -LiteralPath $a.out | ConvertFrom-Json).result
    Expect ($ra.outcome -eq 'TIMED_OUT_ALIVE' -and -not $ra.latched -and $ra.retained) "holder: $($ra.outcome) latched $($ra.latched) retained $($ra.retained)"
    Set-MockEnv $n 'second'
    $r = Invoke-P9SVBoxOperation -VBox (New-TestVBox $n -LockMs 1500) -Operation 'vm.state' -Vm 'P9S-D12'
    Expect ($r.outcome -eq 'LOCK_TIMEOUT' -and (Mocks $n 'second').Count -eq 0) "second: $($r.outcome)"
    $null = $hold.Set(); $null = Wait-Worker $a; $null = $rel.Set()
}
Case 'V19' 'in-process call ended with its process: VBOX_OPERATION_INTERRUPTED' {
    $n = New-Names
    $a = Start-Worker $n @{ mode = 'inproc'; crash_at = 'IN_PROCESS' }
    $null = Wait-Worker $a
    $r = Invoke-P9SVBoxOperation -VBox (New-TestVBox $n) -Operation 'vm.state' -Vm 'P9S-D12'
    Expect ($r.outcome -eq 'QUARANTINED' -and $r.reason -eq 'VBOX_OPERATION_INTERRUPTED') "$($r.outcome) $($r.reason)"
}
Case 'V32' 'an unfinished operation blocks the coordinator; vbox.recover with its id quarantines or takes it over, never skips the checks' {
    $n = New-Names
    $a = Start-Worker $n @{ crash_at = 'STARTING' }; $null = Wait-Worker $a
    $vb = New-TestVBox $n
    $id = (Read-P9SVBoxJson (Join-Path $n.dir 'vbox-operation.json') 'p9s-vbox-operation/1').doc.operation_id
    $c = New-MockCtx @($READY_A) @{} $vb
    $o = Act $c @{ Action = 'session.open'; Kind = 'Test' }
    Expect ($o.outcome -eq 'REFUSED' -and (Has $o 'VBOX_OPERATION_UNFINISHED') -and $c.Prov.vmreads -eq 0) "open: $($o.outcome) $($o.reasons -join ',')"
    Set-MockEnv $n 'probe'
    $r1 = Act $c @{ Action = 'vbox.recover'; Operation = $id }
    Expect ($r1.outcome -eq 'REFUSED' -and (Has $r1 'VBOX_API_STATE_UNKNOWN') -and (Latch $n).operation_id -eq $id -and (Mocks $n).Count -eq 0) "first: $($r1.outcome) $($r1.reasons -join ',')"
    $r2 = Act $c @{ Action = 'vbox.recover'; Operation = $id }
    Expect ($r2.outcome -eq 'DONE' -and -not (Latch $n) -and (Mocks $n).Count -eq 2) "second: $($r2.outcome) $($r2.reasons -join ',')"
    $n2 = New-Names
    $b = Start-Worker $n2 @{ crash_at = 'ACQUIRED' }; $null = Wait-Worker $b
    $id2 = (Read-P9SVBoxJson (Join-Path $n2.dir 'vbox-operation.json') 'p9s-vbox-operation/1').doc.operation_id
    Set-MockEnv $n2 'probe2'
    $r3 = Invoke-P9SVBoxRecovery -VBox (New-TestVBox $n2) -OperationId $id2
    Expect ($r3.outcome -eq 'CLEARED' -and (Mocks $n2).Count -eq 2 -and -not (Test-Path -LiteralPath (Join-Path $n2.dir 'vbox-operation.json'))) "taken over: $($r3.outcome) $($r3.reasons -join ',')"
}

# ---- coordinator integration -------------------------------------------------------------
Case 'V20' 'quarantine refuses test actions and sessions; STOP and ABORT still refuse; nothing is read or dispatched' {
    $n = New-Names
    $vb = New-TestVBox $n
    $c = New-MockCtx @($READY_A) @{ 'P9S-D12' = 'running' } $vb
    $o = Act $c @{ Action = 'session.open'; Kind = 'Test' }
    Expect ($o.outcome -eq 'DONE') "open before: $($o.outcome) $($o.reasons -join ',')"
    $null = Put-Latch $n 'VBOX_API_STATE_UNKNOWN'
    $reads = $c.Prov.vmreads
    $k = Act $c @{ Action = 'guest.key'; Vm = 'P9S-D12'; Key = 'Enter' }
    Expect ($k.outcome -eq 'REFUSED' -and (Has $k 'VBOX_API_QUARANTINED') -and (Has $k 'VBOX_API_STATE_UNKNOWN') -and $c.Calls.Count -eq 0 -and $c.Prov.vmreads -eq $reads) "key: $($k.outcome) $($k.reasons -join ',')"
    $c2 = New-MockCtx @($READY_A, $READY_A, $STOP_A) @{ 'P9S-D12' = 'running' } $vb
    $s = Act $c2 @{ Action = 'scenario.begin'; Name = 'x' }
    Expect ($s.outcome -eq 'REFUSED' -and (Has $s 'VBOX_API_QUARANTINED') -and $c2.Calls.Count -eq 0) "stop: $($s.outcome) $($s.reasons -join ',')"
    $c3 = New-MockCtx @($ABORT_A) @{} $vb
    $s3 = Act $c3 @{ Action = 'session.open'; Kind = 'Test' }
    Expect ($s3.outcome -eq 'REFUSED' -and (Has $s3 'VBOX_API_QUARANTINED') -and $c3.Calls.Count -eq 0) "abort: $($s3.outcome) $($s3.reasons -join ',')"
}
Case 'V21' 'emergency under quarantine sends nothing, claims nothing, and records the emergency' {
    $n = New-Names
    $vb = New-TestVBox $n
    $c = New-MockCtx @($READY_A) @{ 'P9S-D12' = 'running' } $vb
    $null = Act $c @{ Action = 'session.open'; Kind = 'Test' }
    $null = Put-Latch $n 'VBOX_CHILD_UNRESOLVED'
    $reads = $c.Prov.vmreads
    $e = Act $c @{ Action = 'emergency.acpi-shutdown'; Vm = 'P9S-D12' }
    Expect ($e.outcome -eq 'FAILED' -and (Has $e 'VBOX_API_QUARANTINED') -and (Has $e 'EMERGENCY_UNRESOLVED') -and $e.dispatched -eq 0 -and $c.Calls.Count -eq 0) "acpi: $($e.outcome) $($e.reasons -join ',') calls $($c.Calls.Count)"
    Expect ($c.Prov.vmreads -eq $reads -and ($e.lines -join ' ') -match 'operator') "reads $($c.Prov.vmreads - $reads)"
    $req = Read-P9SStopRequest $c
    Expect ($req -and $req.unresolved.Contains('P9S-D12')) 'emergency not recorded as unresolved'
    $sess = Get-Content -Raw -LiteralPath $c.SessionPath | ConvertFrom-Json
    Expect ($sess.lifecycle -eq 'EMERGENCY_UNRESOLVED') "session $($sess.lifecycle)"
    $w = Act $c @{ Action = 'emergency.wait-off'; Vm = 'P9S-D12'; Seconds = 10 }
    Expect ($w.outcome -eq 'REFUSED' -and (Has $w 'VBOX_API_QUARANTINED') -and $c.Prov.vmreads -eq $reads) "wait-off: $($w.outcome)"
    $st = Act $c @{ Action = 'emergency.status' }
    Expect (($st.lines -join ' ') -match 'QUARANTINED' -and ($st.lines -join ' ') -match 'unknown' -and $c.Prov.vmreads -eq $reads) 'status read VM states'
    $d = Act $c @{ Action = 'emergency.acpi-shutdown'; Vm = 'P9S-U24'; DryRun = $true }
    Expect ($d.outcome -eq 'DRYRUN' -and ($d.lines -join ' ') -match 'no request could be sent') 'dry run does not mention the quarantine'
}
Case 'V22' 'a quarantine that appears during an action stops it at the next checkpoint' {
    $n = New-Names
    $vb = New-TestVBox $n
    $c = New-MockCtx @($READY_A) @{ 'P9S-D12' = 'running' } $vb
    $null = Act $c @{ Action = 'session.open'; Kind = 'Test' }
    $c.Prov.onText = { $null = Put-Latch $n 'VBOX_LOCK_TIMEOUT' }.GetNewClosure()
    $t = Act $c @{ Action = 'guest.type'; Vm = 'P9S-D12'; Text = 'abcdefghijkl' }
    Expect ($t.outcome -eq 'STOPPED_AT_CHECKPOINT' -and (Has $t 'VBOX_API_QUARANTINED') -and $c.Calls.Count -eq 1) "$($t.outcome) $($t.reasons -join ',') calls $($c.Calls.Count)"
}
Case 'V23' 'vbox.recover clears through the layer, then the session must be closed and opened again' {
    $n = New-Names
    $vb = New-TestVBox $n
    Set-MockEnv $n 'recover'
    $c = New-MockCtx @($READY_A) @{ 'P9S-D12' = 'running' } $vb
    $null = Act $c @{ Action = 'session.open'; Kind = 'Test' }
    $id = Put-Latch $n
    $bad = Act $c @{ Action = 'vbox.recover'; Operation = 'not-an-id' }
    Expect ($bad.outcome -eq 'REJECTED') "bad id: $($bad.outcome)"
    $r = Act $c @{ Action = 'vbox.recover'; Operation = $id }
    Expect ($r.outcome -eq 'DONE' -and -not (Latch $n) -and (Mocks $n).Count -eq 2) "recover: $($r.outcome) $($r.reasons -join ',') queries $((Mocks $n).Count)"
    $k = Act $c @{ Action = 'guest.key'; Vm = 'P9S-D12'; Key = 'Enter' }
    Expect ($k.outcome -eq 'REFUSED' -and (Has $k 'SESSION_VBOX_RECOVERED') -and $c.Calls.Count -eq 0) "after: $($k.outcome) $($k.reasons -join ',')"
    $null = Act $c @{ Action = 'session.close' }
    $o = Act $c @{ Action = 'session.open'; Kind = 'Test' }
    $k2 = Act $c @{ Action = 'guest.key'; Vm = 'P9S-D12'; Key = 'Enter' }
    Expect ($o.outcome -eq 'DONE' -and $k2.outcome -eq 'DISPATCHED' -and $c.Calls.Count -eq 1) "requalified: $($o.outcome) $($k2.outcome)"
}
Case 'V24' 'vbox.status makes no VirtualBox call and reports the quarantine and its child' {
    $n = New-Names
    $me = Get-Process -Id $PID
    $vb = New-TestVBox $n
    $null = Put-Latch $n 'VBOX_CHILD_UNRESOLVED' ([ordered]@{ operation_id = (New-P9SVBoxOperationId); operation = 'vm.state'; vm = 'P9S-D12'; child_pid = $PID; child_start_ticks = $me.StartTime.ToUniversalTime().Ticks })
    $c = New-MockCtx @($READY_A) @{} $vb
    $s = Act $c @{ Action = 'vbox.status' }
    $t = $s.lines -join ' '
    Expect ($t -match 'QUARANTINED \(VBOX_CHILD_UNRESOLVED' -and $t -match "VBoxManage $PID alive" -and $c.Prov.vmreads -eq 0 -and (Mocks $n).Count -eq 0) "lines: $t"
}
Case 'V25' 'the monitor reads VM states only with a free lock and no quarantine; skipped samples are no observations' {
    $n = New-Names
    $vb = New-TestVBox $n
    $calls = @{ n = 0 }
    $body = { $calls.n++; 'states' }.GetNewClosure()
    $r1 = Invoke-P9SVBoxInProcess -VBox $vb -Operation 'monitor.vmstates' -Opportunistic -Body $body
    Expect ($r1.outcome -eq 'COMPLETED' -and $calls.n -eq 1) "free: $($r1.outcome)"
    $rel = New-Event "$($n.prefix)-rel" -Release; $sa = New-Event "$($n.prefix)-sa"
    $a = Start-Worker $n @{ mock = @{ P9S_MOCK_MODE = 'block'; P9S_MOCK_RELEASE = "$($n.prefix)-rel"; P9S_MOCK_STARTED = "$($n.prefix)-sa" } }
    Wait-Event $sa
    $sw = [Diagnostics.Stopwatch]::StartNew()
    $r2 = Invoke-P9SVBoxInProcess -VBox $vb -Operation 'monitor.vmstates' -Opportunistic -Body $body
    Expect ($r2.outcome -eq 'BUSY' -and $calls.n -eq 1 -and $sw.ElapsedMilliseconds -lt 5000 -and -not $r2.latched -and -not (Latch $n)) "busy: $($r2.outcome) $($sw.ElapsedMilliseconds) ms"
    $null = $rel.Set(); $null = Wait-Worker $a
    $null = Put-Latch $n
    $r3 = Invoke-P9SVBoxInProcess -VBox $vb -Operation 'monitor.vmstates' -Opportunistic -Body $body
    Expect ($r3.outcome -eq 'QUARANTINED' -and $calls.n -eq 1) "quarantined: $($r3.outcome)"
    $samples = @('off', 'busy', 'off', 'quarantined', 'off', 'unreadable', 'off') | ForEach-Object { [pscustomobject]@{ utc = '2026-11-20T00:00:00Z'; d12_state = $_; u24_state = 'off' } }
    Expect (@(Find-OutOfBandVmChanges -Samples $samples -Audit @()).Count -eq 0) 'skipped samples counted as VM state changes'
}

# ---- missing VBoxManage, targets, invocation ----------------------------------------------
Case 'V26' 'missing VBoxManage: unreadable, never off; no exception, no quarantine; dispatch fails; the coordinator refuses' {
    $n = New-Names
    $vb = New-TestVBox $n -Exe (Join-Path $n.dir 'absent\VBoxManage.exe')
    $s = Get-P9SVmStateReal 'P9S-D12' $vb
    Expect ($s -eq 'unreadable' -and $script:P9SLastVBoxResult.reason -eq 'VBOX_EXECUTABLE_UNAVAILABLE') "state $s reason $($script:P9SLastVBoxResult.reason)"
    Expect (@(Get-P9SVmConfigReal 'P9S-D12' $vb).Count -eq 0 -and -not (Latch $n)) 'config read or quarantine'
    $msg = ''; try { Assert-P9SVBoxCompleted (Invoke-P9SVBoxOperation -VBox $vb -Operation 'vm.acpi' -Vm 'P9S-D12') } catch { $msg = $_.Exception.Message }
    Expect ($msg -match 'VBOX_EXECUTABLE_UNAVAILABLE') "dispatch: $msg"
    $c = New-MockCtx @($READY_A) @{} $null
    $c.ReadVmState = { param($vm) Get-P9SVmStateReal $vm $vb }.GetNewClosure()
    $o = Act $c @{ Action = 'session.open'; Kind = 'Test' }
    Expect ($o.outcome -eq 'REFUSED' -and (Has $o 'VM_STATE_UNEXPECTED:P9S-D12')) "open: $($o.outcome) $($o.reasons -join ',')"
    $e = Act $c @{ Action = 'emergency.acpi-shutdown'; Vm = 'P9S-D12' }
    Expect ($e.outcome -ne 'DONE' -and (Has $e 'VM_NOT_RUNNING:unreadable') -and $c.Calls.Count -eq 0) "acpi: $($e.outcome) $($e.reasons -join ',')"
}
Case 'V27' 'unexpected VMs and operations outside the allowlist are rejected before any lock or process' {
    $n = New-Names
    $vb = New-TestVBox $n
    $r1 = Invoke-P9SVBoxOperation -VBox $vb -Operation 'vm.state' -Vm 'other-project-vm'
    $r2 = Invoke-P9SVBoxOperation -VBox $vb -Operation 'vm.poweroff' -Vm 'P9S-D12'
    $r3 = Invoke-P9SVBoxInProcess -VBox $vb -Operation 'guest.type' -Vm 'P9S-D12' -Body { 'x' }
    $r4 = Invoke-P9SVBoxInProcess -VBox $vb -Operation 'guest.keys' -Vm 'other-project-vm' -Body { 'x' }
    Expect ($r1.reason -eq 'VBOX_TARGET_REJECTED' -and $r2.reason -eq 'VBOX_OPERATION_NOT_ALLOWED' -and $r3.reason -eq 'VBOX_OPERATION_NOT_ALLOWED' -and $r4.reason -eq 'VBOX_TARGET_REJECTED') "$($r1.reason) $($r2.reason) $($r3.reason) $($r4.reason)"
    Expect ((Mocks $n).Count -eq 0 -and (Files $n) -notmatch 'other-project-vm') 'a process started or the rejected name was recorded'
    Expect (-not (Test-P9SRequest -Request @{ Action = 'vm.start'; Vm = 'other-project-vm' }).ok) 'coordinator accepted an unexpected VM'
}
Case 'V28' 'the production lock name is one constant, the same in processes with other folders and targets' {
    $n = New-Names
    $d1 = Join-Path $n.dir 'cwd-a'; $d2 = Join-Path $n.dir 'cwd-b'; New-Item -ItemType Directory -Path $d1, $d2 | Out-Null
    $a = Start-Worker $n @{ mode = 'name'; cwd = $d1 }; $b = Start-Worker $n @{ mode = 'name'; cwd = $d2; vm = 'P9S-U24' }
    $ra = Wait-Worker $a; $rb = Wait-Worker $b
    $mine = Get-P9SVBoxMutexName
    Expect ($ra.name -ceq $mine -and $rb.name -ceq $mine -and $ra.cwd -ne $rb.cwd) "names $($ra.name) / $($rb.name)"
    Expect ($mine -ceq 'Global\P9S-VirtualBox-Management-v1' -and $mine -notmatch "$PID|\\\\|:") "name $mine"
    $rc = New-P9SRealContext -StatePath (Join-Path $n.dir 'real\state.json') -KitDir $kit -Config $null
    Expect ($rc.VBox.MutexName -ceq $mine -and $rc.VBox.Dir -eq (Join-Path $n.dir 'real')) "real context: $($rc.VBox.MutexName) $($rc.VBox.Dir)"
}
Case 'V29' 'p9s-action.ps1 started without -File refuses (exit 2) and does nothing; pwsh -File and dot-sourcing work' {
    $n = New-Names
    $cli = Join-Path $kit 'p9s-action.ps1'
    $d1 = Join-Path $n.dir 'amp'; New-Item -ItemType Directory -Path $d1 | Out-Null
    $o = & $pw -NoProfile -NonInteractive -Command "& '$cli' -Action guest.unlock -Vm P9S-D12 -StatePath '$d1\state.json'; exit `$LASTEXITCODE" 2>&1
    Expect ($LASTEXITCODE -eq 2 -and ($o -join ' ') -match 'UNSUPPORTED_INVOCATION' -and ($o -join ' ') -match 'pwsh -NoProfile -File') "&: exit $LASTEXITCODE $($o -join ' ')"
    Expect (@(Get-ChildItem -LiteralPath $d1).Count -eq 0) "files written after the refusal: $(@(Get-ChildItem -LiteralPath $d1 | ForEach-Object Name) -join ',')"
    $d2 = Join-Path $n.dir 'file'; New-Item -ItemType Directory -Path $d2 | Out-Null
    $o2 = & $pw -NoProfile -NonInteractive -File $cli -Action calib.dry-run -Scenario nominal -StatePath (Join-Path $d2 'state.json') 2>&1
    Expect ($LASTEXITCODE -eq 0 -and ($o2 -join ' ') -match 'COMPLETED') "-File: exit $LASTEXITCODE"
    $d3 = Join-Path $n.dir 'dot'; New-Item -ItemType Directory -Path $d3 | Out-Null
    $o3 = & $pw -NoProfile -NonInteractive -Command ". '$cli' -Action calib.dry-run -Scenario nominal -StatePath '$d3\state.json'" 2>&1
    Expect ($LASTEXITCODE -eq 0 -and ($o3 -join ' ') -match 'COMPLETED') "dot-sourced: exit $LASTEXITCODE"
}

# ---- privacy and documentation -------------------------------------------------------------
Case 'V30' 'records and logs hold no secret, command line, configuration or command output' {
    $n = New-Names
    $vb = New-TestVBox $n
    $cfgText = "name=`"P9S-D12`"|UUID=`"00000000-1111-4222-8333-444444444444`"|macaddress1=`"080027AABBCC`"|CfgFile=`"$userPath`"|VMState=`"poweroff`""
    Set-MockEnv $n 'cfg' @{ P9S_MOCK_MODE = 'config'; P9S_MOCK_CONFIG = $cfgText }
    $lines = @(Get-P9SVmConfigReal 'P9S-D12' $vb)
    Expect ($lines.Count -eq 5) "config lines returned to the caller: $($lines.Count)"
    Set-MockEnv $n 'err' @{ P9S_MOCK_MODE = 'fail'; P9S_MOCK_ERRTEXT = "error at $userPath macaddress1=080027AABBCC" }
    $null = Get-P9SVmStateReal 'P9S-D12' $vb
    $fixtureText = 'fixture passphrase for unit tests only'
    $null = Invoke-P9SVBoxInProcess -VBox $vb -Operation 'guest.keys' -Vm 'P9S-D12' -Arguments @('P9S-D12', $fixtureText) -Body { param($v, $s) $null = $s.Length }
    $null = Invoke-P9SVBoxInProcess -VBox $vb -Operation 'guest.keys' -Vm 'P9S-D12' -Arguments @('P9S-D12', $fixtureText) -Body { param($v, $s) throw "failed with $s at $userPath" }.GetNewClosure()
    $all = Files $n
    foreach ($bad in @($fixtureText, 'showvminfo', 'machinereadable', '-File', [IO.Path]::GetFileName($mock), $pw, '080027AABBCC', '00000000-1111', 'someone', 'CfgFile', 'VMState', 'passphrase')) {
        Expect ($all -notmatch [regex]::Escape($bad)) "records contain '$bad'"
    }
    Expect ($all -match 'vm\.config' -and $all -match 'guest\.keys') 'operations not recorded at all'
}
Case 'V31' 'the documentation states that manual VirtualBox use is outside the lock' {
    $readme = Get-Content -Raw -LiteralPath (Join-Path $kit 'README.md')
    $lib = Get-Content -Raw -LiteralPath (Join-Path $kit 'p9s-vbox.ps1')
    Expect ($readme -match '## VirtualBox operations' -and $readme -match '(?s)## VirtualBox operations.*VBoxManage typed by hand.*outside') 'README section missing'
    Expect ($lib -match 'VBoxManage typed by hand' -and $lib -match 'other projects') 'library header missing the boundary'
}

foreach ($e in $script:releases) { try { $null = $e.Set() } catch {} }
$pass = @($results | Where-Object result -eq 'PASS').Count
$fail = @($results | Where-Object result -eq 'FAIL').Count
$notRun = @($results | Where-Object result -ne 'PASS' | Where-Object result -ne 'FAIL').Count
"== $pass passed, $fail failed, $notRun not run"
$results | ConvertTo-Json -Depth 3 | Set-Content -LiteralPath (Join-Path $OutDir 'results.json') -Encoding utf8
$results | ForEach-Object { '{0} {1} {2} ({3} ms){4}' -f $_.result, $_.id, $_.name, $_.ms, $(if ($_.detail) { ' :: ' + $_.detail } else { '' }) } | Set-Content -LiteralPath (Join-Path $OutDir 'results.txt') -Encoding utf8
if ($fail) { exit 1 } elseif ($notRun) { exit 3 } else { exit 0 }
