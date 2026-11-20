#Requires -Version 7.0
# Synthetic tests of the separation between the host resource monitor and
# the VM-state observer: the monitor makes no VirtualBox call and keeps
# sampling while VirtualBox is missing, hung or quarantined; VM states are
# observations made on demand, timestamped, never taken for off when
# unreadable and rejected when stale. One part runs the real monitor (the
# real Windows performance classes, no fixture) for about 24 s while this
# test holds the production VirtualBox lock name, a quarantine and an
# unfinished operation record sit in the monitor's state folder and two
# worker processes hang inside mocked VirtualBox calls; that lock name is
# the only production name used, and only to show that the monitor never
# needs it. No VirtualBox command runs: "VBoxManage" is a mock script
# started with pwsh.
#   pwsh -NoProfile -File tests\decouple-tests.ps1 -OutDir <new folder>
param([Parameter(Mandatory)][string]$OutDir)
$ErrorActionPreference = 'Stop'
$kit = Split-Path $PSScriptRoot -Parent
. (Join-Path $kit 'hostmon-lib.ps1')
. (Join-Path $kit 'coordinator-lib.ps1')
if (Test-Path -LiteralPath $OutDir) { throw "$OutDir exists" }
New-Item -ItemType Directory -Path $OutDir | Out-Null
$OutDir = (Resolve-Path -LiteralPath $OutDir).Path
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
function Has($res, [string]$code) { @($res.reasons) -contains $code }

# ---- mock VBoxManage and workers --------------------------------------------------------
$pw = (Get-Process -Id $PID).Path
$run = [guid]::NewGuid().ToString('N').Substring(0, 12)
$mock = Join-Path $OutDir 'mockvbox.ps1'
$worker = Join-Path $OutDir 'worker.ps1'
@'
$ErrorActionPreference = 'Stop'
$mode = $env:P9S_MOCK_MODE; $dir = $env:P9S_MOCK_DIR
$t0 = [Diagnostics.Stopwatch]::GetTimestamp()
$ov = $null; $overlap = $false
if ($env:P9S_MOCK_OVERLAP) { $ov = [Threading.Mutex]::new($false, $env:P9S_MOCK_OVERLAP); try { $overlap = -not $ov.WaitOne(0) } catch { $overlap = $false } }
if ($env:P9S_MOCK_STARTED) { try { $null = [Threading.EventWaitHandle]::OpenExisting($env:P9S_MOCK_STARTED).Set() } catch {} }
if ($mode -eq 'block') { $e = [Threading.EventWaitHandle]::OpenExisting($env:P9S_MOCK_RELEASE); $null = $e.WaitOne(120000) }
if ($env:P9S_MOCK_HOLD_MS) { Start-Sleep -Milliseconds ([int]$env:P9S_MOCK_HOLD_MS) }
$t1 = [Diagnostics.Stopwatch]::GetTimestamp()
if ($ov -and -not $overlap) { $ov.ReleaseMutex() }
[ordered]@{ pid = $PID; first = [string]$args[0]; start = $t0; end = $t1; overlap = $overlap } | ConvertTo-Json -Compress | Set-Content -LiteralPath (Join-Path $dir "mock-$PID.json")
switch ($mode) {
    'fail' { [Console]::Error.WriteLine('VBoxManage: error: Could not find a registered machine named the one asked for (VBOX_E_OBJECT_NOT_FOUND)'); exit 1 }
    'othername' { 'name="OTHER-VM"'; 'VMState="poweroff"'; exit 0 }
    default { 'name="' + $args[1] + '"'; 'VMState="' + $(if ($env:P9S_MOCK_STATE) { $env:P9S_MOCK_STATE } else { 'poweroff' }) + '"'; exit 0 }
}
'@ | Set-Content -LiteralPath $mock -Encoding ascii
@'
param([Parameter(Mandatory)][string]$Spec)
$ErrorActionPreference = 'Stop'
$s = Get-Content -Raw -LiteralPath $Spec | ConvertFrom-Json -AsHashtable
. (Join-Path $s.kit 'hostmon-lib.ps1')
. (Join-Path $s.kit 'coordinator-lib.ps1')
foreach ($k in @($s.env.Keys)) { [Environment]::SetEnvironmentVariable($k, [string]$s.env[$k]) }
$vb = New-P9SVBoxContext -Dir $s.dir -VmNames @('P9S-D12', 'P9S-U24') -MutexName $s.mutex -ExePath $s.exe -PrefixArgs @($s.prefix) -LockWaitMs ([int]$s.lock_ms) -ExecMs @{ 'vm.state' = [int]$s.exec_ms } -ConfirmMs 2000
if ($s.ready) { $null = [Threading.EventWaitHandle]::OpenExisting($s.ready).Set() }
if ($s.go) { $null = [Threading.EventWaitHandle]::OpenExisting($s.go).WaitOne(60000) }
$r = switch ($s.mode) {
    'observe' {
        $ctx = [pscustomobject]@{ NowQpc = { [Diagnostics.Stopwatch]::GetTimestamp() }; QpcFrequency = [Diagnostics.Stopwatch]::Frequency
            ReadVmState = { param($vm) Get-P9SVmStateReal $vm $vb }.GetNewClosure(); ObservationPath = $s.journal }
        Get-P9SVmObservation $ctx 'P9S-D12' 'status'
    }
    # a VirtualBox COM call that does not return (mocked: the body waits for an event)
    'inproc-hang' { $ev = $s.release; Invoke-P9SVBoxInProcess -VBox $vb -Operation 'guest.keys' -Vm 'P9S-D12' -Body { param($n) $null = [Threading.EventWaitHandle]::OpenExisting($n).WaitOne(120000); 'released' } -Arguments @($ev) }
    default { Invoke-P9SVBoxOperation -VBox $vb -Operation 'vm.state' -Vm 'P9S-D12' }
}
[ordered]@{ result = $r; pid = $PID } | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $s.out
'@ | Set-Content -LiteralPath $worker -Encoding ascii
$script:events = [System.Collections.Generic.List[object]]::new()
$script:n = 0
function New-Names {
    $script:n++
    $p = "Global\P9S-TEST-$run-$($script:n)"
    $d = Join-Path $OutDir ('case-{0:D2}' -f $script:n); New-Item -ItemType Directory -Path $d | Out-Null
    [pscustomobject]@{ mutex = "$p-lock"; overlap = "$p-overlap"; prefix = $p; dir = $d; w = 0 }
}
function New-Event([string]$Name) { $e = [Threading.EventWaitHandle]::new($false, [Threading.EventResetMode]::ManualReset, $Name); $script:events.Add($e); $e }
function MockEnv($N, [hashtable]$More = @{}) {
    $h = [ordered]@{ P9S_MOCK_MODE = 'ok'; P9S_MOCK_DIR = $N.dir; P9S_MOCK_OVERLAP = $N.overlap; P9S_MOCK_STARTED = ''; P9S_MOCK_RELEASE = ''; P9S_MOCK_HOLD_MS = ''; P9S_MOCK_STATE = '' }
    foreach ($k in $More.Keys) { $h[$k] = $More[$k] }
    $h
}
function Set-MockEnv($N, [hashtable]$More = @{}) { $h = MockEnv $N $More; foreach ($k in $h.Keys) { [Environment]::SetEnvironmentVariable($k, [string]$h[$k]) } }
function New-TestVBox($N, [int]$LockMs = 60000, [int]$ExecMs = 60000, [string]$Exe = $pw) {
    New-P9SVBoxContext -Dir $N.dir -VmNames @('P9S-D12', 'P9S-U24') -MutexName $N.mutex -ExePath $Exe -PrefixArgs @('-NoProfile', '-NonInteractive', '-File', $mock) -LockWaitMs $LockMs -ExecMs @{ 'vm.state' = $ExecMs; 'vm.config' = $ExecMs } -ConfirmMs 2000
}
function Start-Worker($N, [hashtable]$Spec) {
    $N.w++
    $s = [ordered]@{ kit = $kit; dir = $N.dir; mutex = $N.mutex; exe = $pw; prefix = @('-NoProfile', '-NonInteractive', '-File', $mock); lock_ms = 60000; exec_ms = 60000
        mode = 'op'; env = (MockEnv $N); out = (Join-Path $N.dir "worker-$($N.w).json"); journal = (Join-Path $N.dir 'vm-observations.jsonl') }
    foreach ($k in $Spec.Keys) { if ($k -eq 'mock') { foreach ($m in $Spec.mock.Keys) { $s.env[$m] = $Spec.mock[$m] } } else { $s[$k] = $Spec[$k] } }
    $sp = Join-Path $N.dir "spec-$($N.w).json"
    $s | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $sp
    $p = Start-Process -FilePath $pw -PassThru -WindowStyle Hidden -ArgumentList @('-NoProfile', '-NonInteractive', '-File', $worker, '-Spec', $sp)
    $null = $p.Handle
    [pscustomobject]@{ p = $p; out = $s.out }
}
function Wait-Worker($W, [int]$ms = 120000) {
    if (-not $W.p.WaitForExit($ms)) { throw 'a worker did not end in time' }
    $W.p.WaitForExit()
    if (Test-Path -LiteralPath $W.out) { (Get-Content -Raw -LiteralPath $W.out | ConvertFrom-Json).result } else { $null }
}
function Mocks($N) { @(Get-ChildItem -LiteralPath $N.dir -Filter 'mock-*.json' -ErrorAction SilentlyContinue | ForEach-Object { Get-Content -Raw -LiteralPath $_.FullName | ConvertFrom-Json }) }
function Latch($N) { $q = Read-P9SVBoxJson (Join-Path $N.dir 'vbox-quarantine.json') 'p9s-vbox-quarantine/1'; if ($q.present) { $q.doc } else { $null } }

# ---- monitor states and a mock coordinator context ---------------------------------------
$GiB = [math]::Pow(2, 30)
function S([double]$t, $commit = 50) {
    [pscustomobject]@{
        seq = 0; mono = $t; utc = 'x'; read_ok = $true; read_error = $null; committed_bytes = [double]$commit * $GiB; commit_limit_bytes = 100 * $GiB
        avail_mb = 16000.0; modified_list_mb = 200.0; pages = 100.0; pages_in = 100.0; pages_out = 0.0; page_reads = 90.0; page_writes = 0.0
        page_faults = 1000.0; transition_faults = 500.0; cache_faults = 100.0; demand_zero_faults = 300.0; rate_issues = @()
        pagefile_used_mb = 100.0; pagefile_alloc_mb = 5888.0; pagefile_pct = 1.7; pagefile_disks = @(0); system_disk = 0; external_disk = 2
        disks = @{ 0 = [pscustomobject]@{ read_ms = 0.5; write_ms = 0.3; read_mbps = 5.0; write_mbps = 1.0; queue = 0; avg_queue = 0.01 }
                   2 = [pscustomobject]@{ read_ms = 2.0; write_ms = 1.0; read_mbps = 0.1; write_mbps = 0.1; queue = 0; avg_queue = 0.01 } }
        external_present = $true; external_identity_ok = $true; eventlog_ok = $true; storage_events_new = 0; storage_event_ids = @(); cpu_pct = 10.0
    }
}
function Feed($st, [double]$from, [double]$to, $commit = 50) { $d = $null; for ($t = $from; $t -le $to + 1e-9; $t += 5) { $d = Step-Monitor -State $st -Sample (S $t $commit) }; $d }
$thr = [ordered]@{
    suspect = [ordered]@{ page_out_per_s = 500; avail_mb_low = 4096; avail_decline_mb = 4096; pagefile_growth_mb = 128; pagefile_disk_read_ms = 20; page_in_per_s_with_low_memory = 2000 }
    confirm = [ordered]@{ page_out_per_s = 2000; avail_mb_low = 2048; avail_decline_mb = 6144; pagefile_growth_mb = 300; pagefile_disk_read_ms = 50; page_in_per_s_with_low_memory = 5000 }
}
$fxApproved = ConvertTo-PagingThresholds -Json ([ordered]@{ schema = 'p9s-hostmon-paging/1'; approved = $true; approved_by = 'test fixture only'; approved_utc = '2026-11-19T00:00:00Z'; source = 'test'; suspect = $thr.suspect; confirm = $thr.confirm } | ConvertTo-Json -Depth 5)
$F = 10000000L; $NOW = 1000L * $F
function J($doc, [double]$age = 1) { ConvertTo-StateJson -Doc $doc -Monitor ([ordered]@{ pid = 4242; process_start_utc = 'x'; process_start_ticks = 1L; mode = 'Monitor'; paging_stop = 'Required'; thresholds_sha256 = 'x'; thresholds_approved = $true; test = $false; kit = 'test' }) -WrittenQpc ([long]($NOW - $age * $F)) -QpcFrequency $F -WrittenUtc 'x' }
$mA = New-MonitorState -RunId 'run-A' -Config (New-HostMonConfig -PagingStop Required -Mode Monitor -Thresholds $fxApproved); $READY_A = J (Feed $mA 0 300)
$script:ctxN = 0
function New-MockCtx([object[]]$States, [hashtable]$Vms = @{}) {
    $script:ctxN++
    $dir = Join-Path $OutDir ('ctx-{0:D2}' -f $script:ctxN); New-Item -ItemType Directory -Path $dir | Out-Null
    $calls = [System.Collections.Generic.List[object]]::new()
    $prov = @{ text = $States[0]; vmreads = 0 }
    $vmTable = @{ 'P9S-D12' = 'off'; 'P9S-U24' = 'off' }; foreach ($k in $Vms.Keys) { $vmTable[$k] = $Vms[$k] }
    $nowq = $NOW; $freq = $F
    $rec = { param($kind, $list) { param($vm, $arg) $list.Add([pscustomobject]@{ kind = $kind; vm = $vm; arg = $arg }) }.GetNewClosure() }
    [pscustomobject]@{
        StatePath = Join-Path $dir 'state.json'; SessionPath = Join-Path $dir 'coordinator-session.json'; LockPath = Join-Path $dir 'coordinator.lock'
        StopRequestPath = Join-Path $dir 'coordinator-stop.request'; AuditPath = Join-Path $dir 'coordinator-audit.jsonl'; ThresholdsPath = Join-Path $kit 'paging-thresholds.json'
        ReadState = { param($p) [pscustomobject]@{ ok = $true; text = $prov.text; error = $null } }.GetNewClosure(); NowQpc = { $nowq }.GetNewClosure(); QpcFrequency = $freq; IsMonitorAlive = { param($a, $b) $true }
        ReadVmState = { param($vm) $prov.vmreads++; $vmTable[$vm] }.GetNewClosure(); ReadVmConfig = { param($vm) @() }
        Dispatch = @{ 'vm.start' = (& $rec 'vm.start' $calls); 'guest.text' = (& $rec 'guest.text' $calls); 'guest.key' = (& $rec 'guest.key' $calls); 'guest.unlock' = (& $rec 'guest.unlock' $calls); 'vm.acpi' = (& $rec 'vm.acpi' $calls) }
        Sleep = { param($ms) }; Pid = $PID; ProcessStartTicks = (Get-Process -Id $PID).StartTime.ToUniversalTime().Ticks
        EmergencyLockWaitMs = 1500; CalibrationHostChecks = { @() }
        ObservationPath = Join-Path $dir 'vm-observations.jsonl'; LedgerPath = Join-Path $dir 'acpi-validation-ledger.json'; AcpiValidationLive = $false
        Calls = $calls; Prov = $prov; VmTable = $vmTable; Dir = $dir
    }
}
function Act($ctx, [hashtable]$r) { Invoke-P9SAction -Ctx $ctx -Request $r }

"== P9-S monitor decoupling and VM-state observer synthetic tests, $((Get-Date).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ')), pwsh $($PSVersionTable.PSVersion), run $run"

# ---- the resource monitor: no VirtualBox in its code ----------------------------------------
Case 'M01' 'the monitor loads no VirtualBox code: no COM object, no VBoxManage, no VirtualBox lock, no VM-state read' {
    $mon = Join-Path $kit 'host-monitor.ps1'
    $tok = $null; $err = $null
    $ast = [Management.Automation.Language.Parser]::ParseFile($mon, [ref]$tok, [ref]$err)
    $dots = @($ast.FindAll({ param($x) $x -is [Management.Automation.Language.CommandAst] -and $x.InvocationOperator -eq [Management.Automation.Language.TokenKind]::Dot }, $true) | ForEach-Object { [regex]::Match($_.Extent.Text, "'([^']+\.ps1)'").Groups[1].Value })
    Expect ((@($dots) | Sort-Object) -join ',' -eq 'hostmon-lib.ps1,p9s-config.ps1') "dot-sourced: $($dots -join ',')"
    $forbidden = 'VirtualBox\.VirtualBoxClient|ComObject|VBoxManage|Get-VBoxManagePath|P9S-VirtualBox-Management|p9s-vbox|p9s-vmstate|Invoke-P9SVBox|Enter-P9SVBoxMutex|New-P9SVBoxContext|Get-P9SVBoxMutexName|Get-P9SVmStateReal|Get-P9SVmObservation|ReadVmState|Threading\.Mutex|Oracle'
    foreach ($file in 'host-monitor.ps1', 'hostmon-lib.ps1', 'p9s-config.ps1') {
        $t2 = $null; $e2 = $null
        $null = [Management.Automation.Language.Parser]::ParseFile((Join-Path $kit $file), [ref]$t2, [ref]$e2)
        $code = (@($t2 | Where-Object { $_.Kind -ne [Management.Automation.Language.TokenKind]::Comment } | ForEach-Object { $_.Text })) -join ' '
        $m = [regex]::Match($code, $forbidden)
        Expect (-not $m.Success) "$file has $($m.Value) in its code"
    }
    $cmds = @($ast.FindAll({ param($x) $x -is [Management.Automation.Language.CommandAst] }, $true) | ForEach-Object { $_.GetCommandName() } | Where-Object { $_ } | Sort-Object -Unique)
    Expect (-not @($cmds | Where-Object { $_ -match 'VBox|VmState|VmObservation|Mutex' }).Count) "commands: $($cmds -join ',')"
}

# ---- the real monitor while VirtualBox is in every kind of trouble ------------------------
# set up once, then several cases read what happened
$rt = Join-Path $OutDir 'realmon'; New-Item -ItemType Directory -Path $rt | Out-Null
$stateDir = Join-Path $rt 'state'; New-Item -ItemType Directory -Path $stateDir | Out-Null
$statePath = Join-Path $stateDir 'state.json'
$nq = New-Names
$vbq = New-P9SVBoxContext -Dir $stateDir -VmNames @('P9S-D12', 'P9S-U24') -MutexName $nq.mutex
$null = Set-P9SVBoxLatch $vbq 'VBOX_COMMAND_TIMEOUT' $null 'test fixture: a quarantine next to the monitor state'
$recDoc = [ordered]@{ schema = 'p9s-vbox-operation/1'; operation_id = 'vbox-20261120T000000000Z-4000000-0badc0de'; operation = 'vm.state'; vm = 'P9S-D12'; phase = 'RUNNING'
    owner_pid = 4000000; owner_start_ticks = 1L; child_pid = 4000001; child_start_ticks = 1L; started_utc = '2026-11-20T00:00:00Z'; updated_utc = $null; exec_ms = 30000 }
Write-StateAtomic -Path (Join-Path $stateDir 'vbox-operation.json') -Text ($recDoc | ConvertTo-Json)
function StateDirHash { (Get-ChildItem -LiteralPath $stateDir -Filter 'vbox-*' | Sort-Object Name | ForEach-Object { "$($_.Name)=$(Get-FileSha256 $_.FullName)" }) -join ';' }
$vboxBefore = StateDirHash
# the production lock name, held by this process for the whole run, as a
# hung VirtualBox call in another toolkit process would hold it
$prod = [Threading.Mutex]::new($false, (Get-P9SVBoxMutexName))
$prodHeld = $false
try { $prodHeld = $prod.WaitOne(120000) } catch [Threading.AbandonedMutexException] { $prodHeld = $true }
# two workers stuck in VirtualBox calls of their own: a COM call that does
# not return and a VBoxManage child that does not end
$nh = New-Names
$relIn = New-Event "$($nh.prefix)-relin"; $relOp = New-Event "$($nh.prefix)-relop"; $stOp = New-Event "$($nh.prefix)-stop"; $rdIn = New-Event "$($nh.prefix)-rdin"
$wIn = Start-Worker $nh @{ mode = 'inproc-hang'; release = "$($nh.prefix)-relin"; ready = "$($nh.prefix)-rdin"; mutex = "$($nh.mutex)-a" }
$wOp = Start-Worker $nh @{ mode = 'op'; mutex = "$($nh.mutex)-b"; mock = @{ P9S_MOCK_MODE = 'block'; P9S_MOCK_RELEASE = "$($nh.prefix)-relop"; P9S_MOCK_STARTED = "$($nh.prefix)-stop" } }
$hungReady = $rdIn.WaitOne(60000) -and $stOp.WaitOne(60000)
Start-Sleep -Milliseconds 300
# the observer, during the run: VBoxManage missing
$nm = New-Names
$vbMissing = New-TestVBox $nm -Exe (Join-Path $nm.dir 'absent\VBoxManage.exe')
$obsCtx = [pscustomobject]@{ NowQpc = { [Diagnostics.Stopwatch]::GetTimestamp() }; QpcFrequency = [Diagnostics.Stopwatch]::Frequency
    ReadVmState = { param($vm) Get-P9SVmStateReal $vm $vbMissing }.GetNewClosure(); ObservationPath = Join-Path $nm.dir 'vm-observations.jsonl' }
$monOut = Join-Path $rt 'run'
# warm up the Windows performance data provider, which may start on demand
try { foreach ($cl in 'Win32_PerfRawData_PerfOS_Memory', 'Win32_PerfRawData_PerfOS_Processor', 'Win32_PerfRawData_PerfOS_PagingFile', 'Win32_PerfRawData_PerfDisk_PhysicalDisk') { $null = Get-CimInstance $cl -ErrorAction Stop } } catch {}
$mp = Start-Process -FilePath $pw -PassThru -WindowStyle Hidden -ArgumentList @('-NoProfile', '-NonInteractive', '-File', (Join-Path $kit 'host-monitor.ps1'), '-OutDir', $monOut, '-StatePath', $statePath,
    '-DurationSeconds', '24', '-ProcessSnapshotSeconds', '0', '-ConfigPath', (Join-Path $rt 'no-local-config.json'))
$null = $mp.Handle
$seen = [System.Collections.Generic.List[object]]::new(); $obsDuring = [System.Collections.Generic.List[object]]::new()
$swm = [Diagnostics.Stopwatch]::StartNew()
while (-not $mp.HasExited -and $swm.Elapsed.TotalSeconds -lt 60) {
    $r = Read-StateFile -Path $statePath
    if ($r.ok) { try { $d = $r.text | ConvertFrom-Json; $seen.Add([pscustomobject]@{ t = $swm.Elapsed.TotalSeconds; seq = $d.seq; status = [string]$d.status; age = ([Diagnostics.Stopwatch]::GetTimestamp() - [long]$d.written_qpc) / [double][Diagnostics.Stopwatch]::Frequency; keys = @($d.PSObject.Properties.Name) }) } catch {} }
    if ($obsDuring.Count -lt 3 -and $swm.Elapsed.TotalSeconds -ge (2 + 4 * $obsDuring.Count)) { $obsDuring.Add((Get-P9SVmObservation $obsCtx 'P9S-D12' 'status')) }
    Start-Sleep -Milliseconds 200
}
$monExited = $mp.WaitForExit(30000)
$hungStill = (-not $wIn.p.HasExited) -and (-not $wOp.p.HasExited)
$null = $relIn.Set(); $null = $relOp.Set()
$rIn = Wait-Worker $wIn; $rOp = Wait-Worker $wOp
$csvRows = @(if (Test-Path -LiteralPath (Join-Path $monOut 'samples.csv')) { Import-Csv -LiteralPath (Join-Path $monOut 'samples.csv') })
$csvHeader = if (Test-Path -LiteralPath (Join-Path $monOut 'samples.csv')) { (Get-Content -LiteralPath (Join-Path $monOut 'samples.csv') -TotalCount 1) } else { '' }
$runDoc = if (Test-Path -LiteralPath (Join-Path $monOut 'run.json')) { Get-Content -Raw -LiteralPath (Join-Path $monOut 'run.json') | ConvertFrom-Json } else { $null }
$cimOk = $csvRows.Count -gt 0 -and -not @($csvRows | Where-Object { $_.reasons -match 'SAMPLE_READ_FAILED' }).Count
"   real monitor: exited $monExited code $(if ($monExited) { $mp.ExitCode }), samples $($csvRows.Count), state reads $($seen.Count), performance classes readable $cimOk, production lock held $prodHeld, hung workers ready $hungReady"

Case 'M02' 'real sampling keeps its 5-second heartbeat while the production VirtualBox lock is held and a quarantine is in place' {
    Expect $prodHeld 'the production lock name could not be held by the test'
    Expect ($monExited -and $mp.ExitCode -eq 0) "monitor exit $($mp.ExitCode)"
    # the first sample of a process can be slow (the Windows performance
    # data provider may start on demand); every later one keeps the cadence
    Expect ($csvRows.Count -ge 3) "samples $($csvRows.Count) (3 or more expected in 24 s)"
    $mono = @($csvRows | ForEach-Object { [double]::Parse($_.mono_s, $inv) })
    for ($i = 2; $i -lt $mono.Count; $i++) { $g = $mono[$i] - $mono[$i - 1]; Expect ($g -ge 4.5 -and $g -le 6.0) "gap $g s between samples $i and $($i + 1)" }
    $ms = @($csvRows | ForEach-Object { [double]::Parse($_.sample_ms, $inv) })
    $later = @($ms | Select-Object -Skip 1 | Sort-Object)
    $median = if ($later.Count) { $later[[math]::Floor(($later.Count - 1) / 2)] } else { $null }
    Expect ($null -ne $median -and $median -lt 1000) "median of samples 2 to $($ms.Count): $median ms (a lock wait in every sample would add at least 1000 ms)"
    Expect ($ms.Count -and $ms[0] -lt 20000 -and @($later | Where-Object { $_ -ge 4000 }).Count -eq 0) "sample times $($ms -join ', ') ms"
    $fresh = @($seen | Where-Object { $_.seq -ge 2 -and $_.status -ne 'STOPPED' })
    Expect ($fresh.Count -gt 10) "state read only $($fresh.Count) times while sampling"
    Expect (@($fresh | Where-Object { $_.age -gt 7.0 }).Count -eq 0) "state older than 7 s seen while sampling: $((($fresh | Measure-Object -Property age -Maximum).Maximum))"
    Expect (@($seen | ForEach-Object seq | Where-Object { $_ } | Sort-Object -Unique).Count -eq $csvRows.Count) 'not every sample was published'
    Expect ((StateDirHash) -eq $vboxBefore) 'the monitor changed the VirtualBox records of its state folder'
}
Case 'M03' 'a hung VirtualBox COM call and a hung VBoxManage in other processes do not block the monitor' {
    Expect $hungReady 'the hung workers did not start'
    Expect $hungStill 'a hung worker ended before the monitor run did'
    Expect ($rIn.outcome -eq 'COMPLETED' -and $rOp.outcome -eq 'COMPLETED') "workers: $($rIn.outcome) $($rOp.outcome)"
    Expect ($csvRows.Count -ge 3 -and $monExited -and $mp.ExitCode -eq 0) 'the monitor did not finish its run'
}
Case 'M04' 'VM-state failures during the run do not change the resource samples; the state holds no VM field' {
    Expect ($obsDuring.Count -ge 2 -and -not @($obsDuring | Where-Object { $_.observed -or $_.state -ne 'unreadable' }).Count) "observations during the run: $(@($obsDuring | ForEach-Object state) -join ',')"
    Expect ($csvHeader -match 'vbox_vm_procs,vbox_vm_ws_mb,wsl_ws_mb' -and $csvHeader -notmatch 'd12_|u24_') "header: $csvHeader"
    Expect ($runDoc -and $runDoc.samples_schema -eq 'p9s-hostmon-samples/2' -and [string]$runDoc.virtualbox -match '^none: the monitor makes no VirtualBox call' -and -not ($runDoc.PSObject.Properties.Name -contains 'vm_states_readable')) 'run.json does not declare layout 2 without VirtualBox'
    foreach ($r in $csvRows) {
        Expect ($r.status -and $r.vbox_vm_procs -match '^\d+$') "row $($r.seq): status '$($r.status)', VM processes '$($r.vbox_vm_procs)'"
        if ($cimOk) { Expect ($r.commit_pct -and $r.commit_limit_mb -and $r.avail_mb) "row $($r.seq): commit or memory missing" }
    }
    $keys = @($seen | ForEach-Object { $_.keys } | Sort-Object -Unique)
    Expect (-not @($keys | Where-Object { $_ -match '^vm' }).Count) "state fields: $($keys -join ',')"
}
Case 'M06' 'paging measurements continue while every VM observation is unreadable' {
    if (-not $cimOk) { Expect ($csvRows.Count -ge 3 -and @($csvRows | Where-Object { $_.reasons -match 'SAMPLE_READ_FAILED' }).Count -eq $csvRows.Count) 'performance classes unreadable here, and not reported as such'; return }
    foreach ($r in @($csvRows | Select-Object -Skip 1)) { Expect ($r.pages_in_per_s -ne '' -and $r.pages_out_per_s -ne '' -and $r.page_reads_per_s -ne '' -and $r.pagefile_used_mb -ne '') "row $($r.seq): paging columns empty" }
}
Case 'M05' 'commit STOP and ABORT are still classified while VirtualBox is quarantined and its lock is held' {
    Expect $prodHeld 'the production lock name is not held'
    foreach ($case in @(@('stop', 95, 'STOP_REQUIRED'), @('abort', 97, 'ABORT_REQUIRED'))) {
        $fx = Join-Path $rt "fixture-$($case[0]).json"
        @(for ($t = 0; $t -le 400; $t += 5) { $x = S $t $(if ($t -ge 310) { $case[1] } else { 50 }); $x | Add-Member -NotePropertyName procs -NotePropertyValue $null; $x.disks = @{ '0' = $x.disks[0]; '2' = $x.disks[2] }; $x }) | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $fx -Encoding utf8
        $sp = Join-Path $stateDir "state-$($case[0]).json"
        $p = Start-Process -FilePath $pw -PassThru -WindowStyle Hidden -ArgumentList @('-NoProfile', '-NonInteractive', '-File', (Join-Path $kit 'host-monitor.ps1'), '-OutDir', (Join-Path $rt "mon-$($case[0])"), '-StatePath', $sp,
            '-TestFixture', $fx, '-TestIntervalMs', '40', '-PagingStop', 'NotRequired', '-DurationSeconds', '60', '-ConfigPath', (Join-Path $rt 'no-local-config.json'))
        $sw = [Diagnostics.Stopwatch]::StartNew(); $got = $null
        while ($sw.Elapsed.TotalSeconds -lt 30) { $g = Invoke-HostGate -StatePath $sp -Step 'test' -PagingStop NotRequired -AllowTestMonitor; if ($g.status -eq $case[2]) { $got = $g; break }; Start-Sleep -Milliseconds 100 }
        Expect ($null -ne $got -and -not $got.ok) "$($case[2]) not published"
        New-Item -ItemType File -Path (Join-Path $rt "mon-$($case[0])\STOP") | Out-Null
        Expect ($p.WaitForExit(20000)) "$($case[0]) monitor did not stop"
    }
    Expect ((StateDirHash) -eq $vboxBefore) 'VirtualBox records changed'
}
try { $prod.ReleaseMutex() } catch {}
$prod.Dispose()

Case 'M07' 'the resource state stays fresh while a VM observation goes stale, and each is judged on its own' {
    $o = [pscustomobject]@{ schema = 'p9s-vm-observation/1'; vm = 'P9S-D12'; state = 'off'; observed = $true; observed_qpc = $NOW - 30L * $F }
    $v = Test-P9SVmObservation -Observation $o -NowQpc $NOW -QpcFrequency $F -Vm 'P9S-D12'
    $m = Test-SafetyState -Text $READY_A -NowQpc $NOW -QpcFrequency $F -PagingStop Required -IsMonitorAlive { param($a, $b) $true }
    Expect ($m.ok -and -not ($m.reasons -contains 'STATE_STALE')) "monitor: $($m.reasons -join ',')"
    Expect (-not $v.ok -and ($v.reasons -contains 'VM_STATE_STALE:P9S-D12') -and [math]::Round($v.age_s) -eq 30) "observation: $($v.reasons -join ',') age $($v.age_s)"
}
Case 'M08' 'a qualified monitor does not make an unreadable VM state usable' {
    $c = New-MockCtx @($READY_A) @{ 'P9S-D12' = 'unreadable' }
    $o = Act $c @{ Action = 'session.open'; Kind = 'Test' }
    Expect ($o.outcome -eq 'REFUSED' -and (Has $o 'VM_STATE_UNEXPECTED:P9S-D12') -and -not (Test-Path -LiteralPath $c.SessionPath)) "open: $($o.outcome) $($o.reasons -join ',')"
    $st = Act $c @{ Action = 'emergency.status' }
    Expect (($st.lines -join ' ') -match 'P9S-D12 unreadable \(not observed' -and ($st.lines -join ' ') -match 'never taken for off') "status: $($st.lines -join ' | ')"
    $doc = $READY_A | ConvertFrom-Json
    Expect (-not @($doc.PSObject.Properties.Name | Where-Object { $_ -match '^vm' }).Count) 'the monitor state has a VM field'
}
Case 'M09' 'no VM-state polling at the sampling interval: the monitor never observes, waits do not observe' {
    $j = @(Get-ChildItem -LiteralPath $stateDir -Filter 'vm-observations.jsonl' -ErrorAction SilentlyContinue)
    $a = @(Get-ChildItem -LiteralPath $stateDir -Filter 'vbox-audit.jsonl' -ErrorAction SilentlyContinue)
    Expect ($j.Count -eq 0 -and $a.Count -eq 0) 'the monitor run left VM observations or VirtualBox audit records'
    $c = New-MockCtx @($READY_A)
    $o = Act $c @{ Action = 'session.open'; Kind = 'Test' }
    $before = $c.Prov.vmreads
    $w = Act $c @{ Action = 'wait'; Seconds = 60 }
    Expect ($o.outcome -eq 'DONE' -and $w.outcome -eq 'DONE') "open $($o.outcome), wait $($w.outcome) $($w.reasons -join ',')"
    Expect (($c.Prov.vmreads - $before) -eq 2) "a 60-s wait made $($c.Prov.vmreads - $before) VM-state reads (2 expected: one per VM before the action)"
    $n0 = $c.Prov.vmreads; $null = Get-P9SVmObservation $c 'P9S-D12' 'status'
    Expect (($c.Prov.vmreads - $n0) -eq 1) 'one observation is not one query'
}

# ---- the VM-state observer --------------------------------------------------------------------
Case 'O01' 'observations from two processes go through the serialized layer and never overlap' {
    $n = New-Names
    $ws = @(1..2 | ForEach-Object { Start-Worker $n @{ mode = 'observe'; mock = @{ P9S_MOCK_HOLD_MS = '700' } } })
    $rs = @($ws | ForEach-Object { Wait-Worker $_ })
    Expect (@($rs | Where-Object { $_.observed -and $_.state -eq 'off' -and $_.operation_id }).Count -eq 2) "results: $(@($rs | ForEach-Object { "$($_.state)/$($_.observed)" }) -join ',')"
    $m = @(Mocks $n | Sort-Object start)
    Expect ($m.Count -eq 2 -and -not @($m | Where-Object overlap).Count -and $m[1].start -ge $m[0].end) 'mock commands overlapped'
    $jr = @(Read-P9SVmObservations (Join-Path $n.dir 'vm-observations.jsonl'))
    $au = @(Get-Content -LiteralPath (Join-Path $n.dir 'vbox-audit.jsonl') | ForEach-Object { $_ | ConvertFrom-Json })
    Expect ($jr.Count -eq 2 -and @($jr | Where-Object { $_.operation_id -in @($au.operation_id) }).Count -eq 2) 'journal and VirtualBox audit do not match'
}
Case 'O02' 'an observation carries its own times: query start, end, duration and UTC' {
    $n = New-Names
    Set-MockEnv $n @{ P9S_MOCK_HOLD_MS = '600' }
    $vb = New-TestVBox $n
    $c = [pscustomobject]@{ NowQpc = { [Diagnostics.Stopwatch]::GetTimestamp() }; QpcFrequency = [Diagnostics.Stopwatch]::Frequency; ReadVmState = { param($vm) Get-P9SVmStateReal $vm $vb }.GetNewClosure(); ObservationPath = Join-Path $n.dir 'vm-observations.jsonl' }
    $u0 = [DateTime]::UtcNow; $q0 = [Diagnostics.Stopwatch]::GetTimestamp()
    $o = Get-P9SVmObservation $c 'P9S-D12' 'status'
    $q1 = [Diagnostics.Stopwatch]::GetTimestamp(); $u1 = [DateTime]::UtcNow
    Expect ($o.observed -and $o.state -eq 'off' -and $o.observed_qpc -ge $q0 -and $o.completed_qpc -le $q1 -and $o.completed_qpc -gt $o.observed_qpc) 'query times out of order'
    Expect ($o.elapsed_ms -ge 550 -and $o.elapsed_ms -le (1000.0 * ($q1 - $q0) / [Diagnostics.Stopwatch]::Frequency + 1)) "elapsed $($o.elapsed_ms) ms"
    $ut = [datetime]::Parse($o.observed_utc, $inv, [Globalization.DateTimeStyles]::AdjustToUniversal)
    Expect ($ut -ge $u0.AddMilliseconds(-5) -and $ut -le $u1) "utc $($o.observed_utc)"
    $jr = @(Read-P9SVmObservations $c.ObservationPath)
    Expect ($jr.Count -eq 1 -and $jr[0].vm -eq 'P9S-D12' -and $jr[0].state -eq 'off' -and $jr[0].purpose -eq 'status' -and $jr[0].operation_id -eq $o.operation_id) 'journal entry'
    Expect ((Get-Content -Raw -LiteralPath $c.ObservationPath) -notmatch 'VMState|name=') 'command output in the journal'
}
Case 'O03' 'stale, future, unobserved and foreign observations are rejected for decisions' {
    $mk = { param([double]$age, [bool]$obs = $true, [string]$vm = 'P9S-D12') [pscustomobject]@{ schema = 'p9s-vm-observation/1'; vm = $vm; state = $(if ($obs) { 'off' } else { 'unreadable' }); observed = $obs; observed_qpc = [long]($NOW - $age * $F) } }
    Expect ((Test-P9SVmObservation -Observation (& $mk 0) -NowQpc $NOW -QpcFrequency $F -Vm 'P9S-D12').ok) 'fresh refused'
    Expect ((Test-P9SVmObservation -Observation (& $mk 10) -NowQpc $NOW -QpcFrequency $F -Vm 'P9S-D12').ok) '10 s refused'
    $s = Test-P9SVmObservation -Observation (& $mk 10.5) -NowQpc $NOW -QpcFrequency $F -Vm 'P9S-D12'
    Expect (-not $s.ok -and ($s.reasons -contains 'VM_STATE_STALE:P9S-D12')) "10.5 s: $($s.reasons -join ',')"
    $fu = Test-P9SVmObservation -Observation (& $mk -1) -NowQpc $NOW -QpcFrequency $F -Vm 'P9S-D12'
    Expect (-not $fu.ok -and ($fu.reasons -contains 'VM_STATE_FROM_FUTURE:P9S-D12')) "future: $($fu.reasons -join ',')"
    $u = Test-P9SVmObservation -Observation (& $mk 0 $false) -NowQpc $NOW -QpcFrequency $F -Vm 'P9S-D12'
    Expect (-not $u.ok -and ($u.reasons -contains 'VM_STATE_UNOBSERVED:P9S-D12')) "unobserved: $($u.reasons -join ',')"
    $x = Test-P9SVmObservation -Observation (& $mk 0 $true 'P9S-U24') -NowQpc $NOW -QpcFrequency $F -Vm 'P9S-D12'
    Expect (-not $x.ok -and ($x.reasons -contains 'VM_IDENTITY_MISMATCH:P9S-D12')) "foreign: $($x.reasons -join ',')"
    $z = Test-P9SVmObservation -Observation $null -NowQpc $NOW -QpcFrequency $F -Vm 'P9S-D12'
    Expect (-not $z.ok -and ($z.reasons -contains 'VM_STATE_UNOBSERVED:P9S-D12')) "none: $($z.reasons -join ',')"
}
Case 'O04' 'missing VBoxManage: unreadable, not observed, with its reason; never off and no quarantine' {
    $n = New-Names
    $vb = New-TestVBox $n -Exe (Join-Path $n.dir 'absent\VBoxManage.exe')
    $c = [pscustomobject]@{ NowQpc = { [Diagnostics.Stopwatch]::GetTimestamp() }; QpcFrequency = [Diagnostics.Stopwatch]::Frequency; ReadVmState = { param($vm) Get-P9SVmStateReal $vm $vb }.GetNewClosure(); ObservationPath = Join-Path $n.dir 'vm-observations.jsonl' }
    $o = Get-P9SVmObservation $c 'P9S-D12' 'status'
    Expect (-not $o.observed -and $o.state -eq 'unreadable' -and $o.reason -eq 'VBOX_EXECUTABLE_UNAVAILABLE' -and -not (Latch $n)) "$($o.state) $($o.observed) $($o.reason)"
}
Case 'O05' 'quarantine: no query, unreadable with the quarantine as reason' {
    $n = New-Names
    $vb = New-TestVBox $n
    $null = Set-P9SVBoxLatch $vb 'VBOX_COMMAND_TIMEOUT' $null 'test fixture'
    $c = [pscustomobject]@{ NowQpc = { [Diagnostics.Stopwatch]::GetTimestamp() }; QpcFrequency = [Diagnostics.Stopwatch]::Frequency; ReadVmState = { param($vm) Get-P9SVmStateReal $vm $vb }.GetNewClosure(); ObservationPath = $null }
    $o = Get-P9SVmObservation $c 'P9S-D12' 'status'
    Expect (-not $o.observed -and $o.state -eq 'unreadable' -and $o.reason -eq 'VBOX_API_QUARANTINED' -and (Mocks $n).Count -eq 0) "$($o.state) $($o.reason) processes $((Mocks $n).Count)"
}
Case 'O06' 'a missing VM or an answer about another VM is unreadable, never off' {
    $n = New-Names
    $vb = New-TestVBox $n
    $c = [pscustomobject]@{ NowQpc = { [Diagnostics.Stopwatch]::GetTimestamp() }; QpcFrequency = [Diagnostics.Stopwatch]::Frequency; ReadVmState = { param($vm) Get-P9SVmStateReal $vm $vb }.GetNewClosure(); ObservationPath = $null }
    Set-MockEnv $n @{ P9S_MOCK_MODE = 'fail' }
    $a = Get-P9SVmObservation $c 'P9S-D12' 'status'
    Expect (-not $a.observed -and $a.state -eq 'unreadable' -and $a.reason -eq 'VBOX_COMMAND_FAILED' -and -not (Latch $n)) "missing VM: $($a.state) $($a.reason)"
    Set-MockEnv $n @{ P9S_MOCK_MODE = 'othername' }
    $b = Get-P9SVmObservation $c 'P9S-D12' 'status'
    Expect (-not $b.observed -and $b.state -eq 'unreadable') "other VM: $($b.state) $($b.observed)"
    Set-MockEnv $n @{}
    $d = Get-P9SVmObservation $c 'P9S-D12' 'status'
    Expect ($d.observed -and $d.state -eq 'off') "normal answer: $($d.state)"
}
Case 'O07' 'no unknown is ever turned into off, and decisions refuse it' {
    foreach ($raw in @('unreadable', 'unknown', '', $null, 'busy', 'quarantined', 'none', 'OFF', 'off ', 'poweroff"', 42)) {
        $c = [pscustomobject]@{ NowQpc = { $NOW }.GetNewClosure(); QpcFrequency = $F; ReadVmState = { param($vm) $raw }.GetNewClosure(); ObservationPath = $null }
        $o = Get-P9SVmObservation $c 'P9S-D12' 'status'
        Expect (-not $o.observed -and $o.state -eq 'unreadable') "'$raw' became $($o.state) observed $($o.observed)"
    }
    $c2 = [pscustomobject]@{ NowQpc = { $NOW }.GetNewClosure(); QpcFrequency = $F; ReadVmState = { param($vm) @('off', 'off') }; ObservationPath = $null }
    Expect (-not (Get-P9SVmObservation $c2 'P9S-D12' 'status').observed) 'two answers taken as one'
    $c3 = [pscustomobject]@{ NowQpc = { $NOW }.GetNewClosure(); QpcFrequency = $F; ReadVmState = { param($vm) throw 'broken reader' }; ObservationPath = $null }
    Expect ((Get-P9SVmObservation $c3 'P9S-D12' 'status').state -eq 'unreadable') 'a failing reader gave a state'
    $m = New-MockCtx @($READY_A) @{ 'P9S-D12' = 'unknown' }
    $e = Act $m @{ Action = 'emergency.acpi-shutdown'; Vm = 'P9S-D12' }
    Expect ($e.outcome -ne 'DONE' -and (Has $e 'VM_NOT_RUNNING:unreadable') -and $m.Calls.Count -eq 0) "acpi: $($e.outcome) $($e.reasons -join ',')"
    $w = Act $m @{ Action = 'emergency.wait-off'; Vm = 'P9S-D12'; Seconds = 4 }
    Expect ($w.outcome -eq 'TIMEOUT') "wait-off: $($w.outcome)"
}
Case 'O08' 'the coordinator reads VM states only through the observer' {
    $hits = @()
    foreach ($file in 'coordinator-lib.ps1', 'p9s-acpival.ps1', 'p9s-vmstate.ps1') {
        $tk = $null; $e = $null
        $null = [Management.Automation.Language.Parser]::ParseFile((Join-Path $kit $file), [ref]$tk, [ref]$e)
        $code = @($tk | Where-Object { $_.Kind -ne [Management.Automation.Language.TokenKind]::Comment })
        for ($i = 0; $i -lt $code.Count; $i++) { if ($code[$i].Text -eq 'ReadVmState' -and $i -gt 0 -and $code[$i - 1].Text -eq '.') { $hits += "$file line $($code[$i].Extent.StartLineNumber)" } }
    }
    Expect ($hits.Count -eq 1 -and $hits[0] -like 'p9s-vmstate.ps1 *') "ReadVmState called at: $($hits -join '; ')"
}

foreach ($e in $script:events) { try { $null = $e.Set() } catch {} }
$pass = @($results | Where-Object result -eq 'PASS').Count
$fail = @($results | Where-Object result -eq 'FAIL').Count
$notRun = @($results | Where-Object result -ne 'PASS' | Where-Object result -ne 'FAIL').Count
"== $pass passed, $fail failed, $notRun not run"
$results | ConvertTo-Json -Depth 3 | Set-Content -LiteralPath (Join-Path $OutDir 'results.json') -Encoding utf8
$results | ForEach-Object { '{0} {1} {2} ({3} ms){4}' -f $_.result, $_.id, $_.name, $_.ms, $(if ($_.detail) { ' :: ' + $_.detail } else { '' }) } | Set-Content -LiteralPath (Join-Path $OutDir 'results.txt') -Encoding utf8
if ($fail) { exit 1 } elseif ($notRun) { exit 3 } else { exit 0 }
