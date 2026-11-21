#Requires -Version 7.0
# Synthetic tests of the restricted single-VM ACPI validation
# (p9s-acpival.ps1): its live switch, the authorization file, the monitor
# conditions, the one-start and one-request budgets, recovery when the
# guest does not power off, and races between processes. The coordinator
# runs on mock contexts: monitor states come from the real policy of
# hostmon-lib.ps1, VM states from a table, dispatchers only record calls,
# and the clock is simulated; cross-process cases start worker processes
# on file-backed mock contexts (real clock, mock dispatchers that may end
# their own process to simulate a crash). One case contends for a
# test-only VirtualBox lock with a mock VBoxManage. No VM, no VirtualBox
# command, no production lock or file.
#   pwsh -NoProfile -File tests\acpival-tests.ps1 -OutDir <new folder>
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
function Why($res) { "$($res.outcome) [$(@($res.reasons) -join ',')]" }

# ---- monitor states from the real policy ------------------------------------------------
$GiB = [math]::Pow(2, 30)
function S([double]$t, $commit = 50, [double]$avail = 16000, [double]$out = 0) {
    [pscustomobject]@{
        seq = 0; mono = $t; utc = 'x'; read_ok = $true; read_error = $null; committed_bytes = [double]$commit * $GiB; commit_limit_bytes = 100 * $GiB
        avail_mb = $avail; modified_list_mb = 200.0; pages = 100.0 + $out; pages_in = 100.0; pages_out = $out; page_reads = 90.0; page_writes = $out / 10
        page_faults = 1000.0; transition_faults = 500.0; cache_faults = 100.0; demand_zero_faults = 300.0; rate_issues = @()
        pagefile_used_mb = 100.0; pagefile_alloc_mb = 5888.0; pagefile_pct = 1.7; pagefile_disks = @(0); system_disk = 0; external_disk = 2
        disks = @{ 0 = [pscustomobject]@{ read_ms = 0.5; write_ms = 0.3; read_mbps = 5.0; write_mbps = 1.0; queue = 0; avg_queue = 0.01 }
                   2 = [pscustomobject]@{ read_ms = 2.0; write_ms = 1.0; read_mbps = 0.1; write_mbps = 0.1; queue = 0; avg_queue = 0.01 } }
        external_present = $true; external_identity_ok = $true; eventlog_ok = $true; storage_events_new = 0; storage_event_ids = @(); cpu_pct = 10.0
    }
}
function Feed($st, [double]$from, [double]$to, $commit = 50, [double]$avail = 16000, [double]$out = 0) { $d = $null; for ($t = $from; $t -le $to + 1e-9; $t += 5) { $d = Step-Monitor -State $st -Sample (S $t $commit $avail $out) }; $d }
$thr = [ordered]@{
    suspect = [ordered]@{ page_out_per_s = 500; avail_mb_low = 4096; avail_decline_mb = 4096; pagefile_growth_mb = 128; pagefile_disk_read_ms = 20; page_in_per_s_with_low_memory = 2000 }
    confirm = [ordered]@{ page_out_per_s = 2000; avail_mb_low = 2048; avail_decline_mb = 6144; pagefile_growth_mb = 300; pagefile_disk_read_ms = 50; page_in_per_s_with_low_memory = 5000 }
}
$fxApproved = ConvertTo-PagingThresholds -Json ([ordered]@{ schema = 'p9s-hostmon-paging/1'; approved = $true; approved_by = 'test fixture only'; approved_utc = '2026-11-19T00:00:00Z'; source = 'test'; suspect = $thr.suspect; confirm = $thr.confirm } | ConvertTo-Json -Depth 5)
$kitThr = Get-PagingThresholds -Path (Join-Path $kit 'paging-thresholds.json')
function NewMon([string]$run, $th = $kitThr, [string]$mode = 'Monitor', [string]$paging = 'Required') { New-MonitorState -RunId $run -Config (New-HostMonConfig -PagingStop $paging -Mode $mode -Thresholds $th) }
function M($doc, [string]$mode = 'Monitor', [string]$paging = 'Required') { [pscustomobject]@{ doc = $doc; mode = $mode; paging = $paging } }
function JM($m, [long]$q, [long]$f, [double]$age = 1) {
    ConvertTo-StateJson -Doc $m.doc -Monitor ([ordered]@{ pid = 4242; process_start_utc = 'x'; process_start_ticks = 1L; mode = $m.mode; paging_stop = $m.paging; thresholds_sha256 = 'x'; thresholds_approved = $false; test = $false; kit = 'test' }) `
        -WrittenQpc ([long]($q - $age * $f)) -QpcFrequency $f -WrittenUtc 'x'
}
$ELIG = M (Feed (NewMon 'run-A') 0 300)
$ELIG_B = M (Feed (NewMon 'run-B') 0 300)
$LOWMEM = M (Feed (NewMon 'run-A') 0 300 50 7000)
$m1 = NewMon 'run-A'; $null = Feed $m1 0 300; $WARN = M (Feed $m1 305 305 92)
$m2 = NewMon 'run-A'; $null = Feed $m2 0 300; $PAGWARN = M (Feed $m2 305 360 50 3000 3000)
$m3 = NewMon 'run-A'; $null = Feed $m3 0 300; $STOP = M (Feed $m3 305 335 95)
$m4 = NewMon 'run-A'; $null = Feed $m4 0 300; $ABORT = M (Feed $m4 305 305 97)
$QUAL = M (Feed (NewMon 'run-A') 0 100)
$CALIB = M (Feed (NewMon 'run-C' $kitThr 'Calibrate') 0 300) 'Calibrate'
$NOTREQ = M (Feed (NewMon 'run-A' $kitThr 'Monitor' 'NotRequired') 0 300) 'Monitor' 'NotRequired'
$READYAPP = M (Feed (NewMon 'run-A' $fxApproved) 0 300)
$F = 10000000L; $NOW = 1000L * $F
$T0 = [datetime]::new(2026, 11, 20, 12, 0, 0, [DateTimeKind]::Utc)
function Z([datetime]$d) { $d.ToUniversalTime().ToString("yyyy-MM-dd'T'HH:mm:ss'Z'", $inv) }

# ---- authorizations ---------------------------------------------------------------------
$authDir = Join-Path $OutDir 'authorizations'; New-Item -ItemType Directory -Path $authDir | Out-Null
$script:authN = 0
function New-Auth([hashtable]$Set = @{}, [string[]]$Remove = @(), $From = $null, $To = $null, [switch]$Real) {
    $script:authN++
    $base = if ($Real) { [DateTime]::UtcNow } else { $T0 }
    if ($null -eq $From) { $From = $base.AddMinutes(-10) }; if ($null -eq $To) { $To = $base.AddHours(2) }
    $h = [ordered]@{
        schema = 'p9s-acpi-validation-authorization/1'; authorization_id = ('acpiauth-20261120-{0:x8}' -f (0x10000 + $script:authN)); purpose = 'ACPI_VALIDATION'; target_vm = 'P9S-D12'
        permitted_actions = @('acpi.open', 'acpi.start', 'acpi.observe', 'acpi.shutdown', 'acpi.integrity'); max_startups = 1; max_acpi_requests = 1
        valid_from_utc = (Z $From); expires_utc = (Z $To)
        monitor_conditions = [ordered]@{ mode = 'Monitor'; paging_stop = 'Required'; commit_gate_s = 300; commit_below_pct = 90; provisional_min_available_mb = 8192 }
        preflight_checks = @(Get-P9SAcpiPreflightChecks); supervision = 'operator-present'; on_stop_or_abort = 'halt-then-emergency-acpi-shutdown-no-repeat'
        provisional_safeguard_approved = $true; approved_by = 'test fixture only'; approved_utc = (Z $base.AddHours(-1))
    }
    foreach ($k in $Set.Keys) { $h[$k] = $Set[$k] }
    foreach ($k in $Remove) { $h.Remove($k) }
    $p = Join-Path $authDir "auth-$($script:authN).json"
    $h | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $p -Encoding utf8
    $p
}
function AuthId([string]$Path) { (Get-Content -Raw -LiteralPath $Path | ConvertFrom-Json).authorization_id }

# ---- mock context --------------------------------------------------------------------------
$script:ctxN = 0
function New-VCtx {
    param($Mon = $ELIG, [hashtable]$Vms = @{}, [bool]$Live = $true, [string]$Dir, [switch]$RealClock)
    $script:ctxN++
    if (-not $Dir) { $Dir = Join-Path $OutDir ('ctx-{0:D2}' -f $script:ctxN); New-Item -ItemType Directory -Path $Dir | Out-Null }
    $calls = [System.Collections.Generic.List[object]]::new()
    $clk = @{ q = $NOW; utc = $T0; real = [bool]$RealClock }
    $freq = if ($RealClock) { [Diagnostics.Stopwatch]::Frequency } else { $F }
    $prov = @{ mon = $Mon; age = 1.0; reads = 0; switchAt = 0; switchTo = $null; vmreads = 0; onVmRead = $null; acpiSent = $false; acpiReads = 0; offAfter = -1
        onStart = $null; onAcpi = $null; onSleep = $null; startThrows = $false; acpiThrows = $false; startSets = 'running'; ledgerWrites = 0; failLedgerAt = 0; failLedgerTo = 0; alive = $true; vmprocs = 0
        config = @('name="P9S-D12"', 'memory=4096', 'acpi="on"', 'VMState="poweroff"') }
    $vmTable = @{ 'P9S-D12' = 'off'; 'P9S-U24' = 'off' }; foreach ($k in $Vms.Keys) { $vmTable[$k] = $Vms[$k] }
    $nowq = { if ($clk.real) { [Diagnostics.Stopwatch]::GetTimestamp() } else { $clk.q } }.GetNewClosure()
    $readState = {
        param($p)
        $prov.reads++
        if ($prov.switchAt -gt 0 -and $prov.reads -ge $prov.switchAt) { $prov.mon = $prov.switchTo; $prov.switchAt = 0 }
        if ($null -eq $prov.mon) { return [pscustomobject]@{ ok = $false; text = $null; error = 'missing' } }
        [pscustomobject]@{ ok = $true; text = (JM $prov.mon (& $nowq) $freq $prov.age); error = $null }
    }.GetNewClosure()
    $readVm = {
        param($vm)
        $prov.vmreads++
        if ($prov.acpiSent -and $vm -ceq 'P9S-D12') { $prov.acpiReads++; if ($prov.offAfter -ge 0 -and $prov.acpiReads -gt $prov.offAfter) { $vmTable[$vm] = 'off' } }
        if ($prov.onVmRead) { & $prov.onVmRead $vm }
        $vmTable[$vm]
    }.GetNewClosure()
    $start = { param($vm) $calls.Add([pscustomobject]@{ kind = 'vm.start'; vm = $vm }); if ($prov.onStart) { & $prov.onStart }; if ($prov.startThrows) { throw 'mock start failure' }; if ($prov.startSets) { $vmTable[$vm] = $prov.startSets } }.GetNewClosure()
    $acpi = { param($vm) $calls.Add([pscustomobject]@{ kind = 'vm.acpi'; vm = $vm }); if ($prov.acpiThrows) { throw 'mock ACPI failure' }; $prov.acpiSent = $true; $prov.acpiReads = 0; if ($prov.onAcpi) { & $prov.onAcpi } }.GetNewClosure()
    $text = { param($vm, $arg) $calls.Add([pscustomobject]@{ kind = 'guest.text'; vm = $vm }) }.GetNewClosure()
    $key = { param($vm, $arg) $calls.Add([pscustomobject]@{ kind = 'guest.key'; vm = $vm }) }.GetNewClosure()
    $unlock = { param($vm) $calls.Add([pscustomobject]@{ kind = 'guest.unlock'; vm = $vm }) }.GetNewClosure()
    $wl = {
        param($path, $txt)
        $prov.ledgerWrites++
        if ($prov.failLedgerAt -gt 0 -and $prov.ledgerWrites -ge $prov.failLedgerAt -and ($prov.failLedgerTo -eq 0 -or $prov.ledgerWrites -le $prov.failLedgerTo)) { throw 'injected ledger write failure' }
        Write-StateAtomic -Path $path -Text $txt
    }.GetNewClosure()
    $sleep = { param($ms) if ($clk.real) { Start-Sleep -Milliseconds ([math]::Max(1, [int]($ms / 50))) } else { $clk.q += [long]($ms * $freq / 1000); $clk.utc = $clk.utc.AddMilliseconds($ms) }; if ($prov.onSleep) { & $prov.onSleep } }.GetNewClosure()
    $utc = { if ($clk.real) { [DateTime]::UtcNow } else { $clk.utc } }.GetNewClosure()
    $refPath = Join-Path $Dir 'P9S-D12-reference.txt'
    if (-not (Test-Path -LiteralPath $refPath)) { $prov.config | Set-Content -LiteralPath $refPath -Encoding ascii }
    [pscustomobject]@{
        StatePath = Join-Path $Dir 'state.json'; SessionPath = Join-Path $Dir 'coordinator-session.json'; LockPath = Join-Path $Dir 'coordinator.lock'
        StopRequestPath = Join-Path $Dir 'coordinator-stop.request'; AuditPath = Join-Path $Dir 'coordinator-audit.jsonl'; ThresholdsPath = Join-Path $kit 'paging-thresholds.json'
        ReadState = $readState; NowQpc = $nowq; QpcFrequency = $freq; IsMonitorAlive = { param($a, $b) $prov.alive }.GetNewClosure()
        ReadVmState = $readVm; ReadVmConfig = { param($vm) @($prov.config) }.GetNewClosure()
        Dispatch = @{ 'vm.start' = $start; 'vm.acpi' = $acpi; 'guest.text' = $text; 'guest.key' = $key; 'guest.unlock' = $unlock }
        Sleep = $sleep; Pid = $PID; ProcessStartTicks = (Get-Process -Id $PID).StartTime.ToUniversalTime().Ticks
        EmergencyLockWaitMs = 1500; CalibrationHostChecks = { @() }
        ObservationPath = Join-Path $Dir 'vm-observations.jsonl'; LedgerPath = Join-Path $Dir 'acpi-validation-ledger.json'; WriteLedger = $wl; UtcNow = $utc; AcpiValidationLive = $Live
        CountVmProcesses = { [int]$prov.vmprocs }.GetNewClosure()
        Config = [pscustomobject]@{ vm_config_reference = [pscustomobject]@{ debian = $refPath; ubuntu = $null } }
        Calls = $calls; Prov = $prov; VmTable = $vmTable; Dir = $Dir; Clock = $clk
    }
}
function Act($ctx, [hashtable]$r) { Invoke-P9SAction -Ctx $ctx -Request $r }
function AOpen($c, [string]$a, [hashtable]$More = @{}) { $r = @{ Action = 'acpi.open'; Authorization = $a }; foreach ($k in $More.Keys) { $r[$k] = $More[$k] }; Act $c $r }
function AStart($c) { Act $c @{ Action = 'acpi.start' } }
function AObs($c, [int]$s = 30) { Act $c @{ Action = 'acpi.observe'; Seconds = $s } }
function AShut($c, [int]$s = 0) { if ($s) { Act $c @{ Action = 'acpi.shutdown'; Seconds = $s } } else { Act $c @{ Action = 'acpi.shutdown' } } }
function AConfirm($c, [int]$s = 30) { Act $c @{ Action = 'acpi.confirm-off'; Seconds = $s } }
function AFinal($c) { Act $c @{ Action = 'acpi.finalize' } }
function Emerg($c) { Act $c @{ Action = 'emergency.acpi-shutdown'; Vm = 'P9S-D12' } }
function Ledger($c) { $r = Read-P9SAcpiLedger $c; if ($r.ok) { $r.doc } else { $null } }
function Entry($c, [string]$Auth) { $l = Ledger $c; if ($l) { Get-P9SAcpiEntry $l (AuthId $Auth) } else { $null } }
function Sess($c) { (Read-P9SSession $c).session }
function Kinds($c) { (@($c.Calls | ForEach-Object { "$($_.kind) $($_.vm)" })) -join ', ' }
function Count($c, [string]$kind) { @($c.Calls | Where-Object { $_.kind -eq $kind }).Count }
# open and start on a fresh mock context; the VM then runs
function Started([hashtable]$Vms = @{}, $Mon = $ELIG) {
    $c = New-VCtx -Mon $Mon -Vms $Vms; $a = New-Auth
    $o = AOpen $c $a; $s = AStart $c
    if ($o.outcome -ne 'DONE' -or $s.outcome -ne 'DISPATCHED') { throw "setup failed: open $(Why $o), start $(Why $s)" }
    [pscustomobject]@{ c = $c; a = $a }
}

# ---- workers (separate processes on a file-backed mock context) ---------------------------------
$pw = (Get-Process -Id $PID).Path
$run = [guid]::NewGuid().ToString('N').Substring(0, 12)
$worker = Join-Path $OutDir 'worker.ps1'
@'
param([Parameter(Mandatory)][string]$Spec)
$ErrorActionPreference = 'Stop'
$s = Get-Content -Raw -LiteralPath $Spec | ConvertFrom-Json -AsHashtable
. (Join-Path $s.kit 'hostmon-lib.ps1')
. (Join-Path $s.kit 'coordinator-lib.ps1')
$f = [Diagnostics.Stopwatch]::Frequency
$tmpl = Get-Content -Raw -LiteralPath $s.state
$vms = @{}; foreach ($k in $s.vms.Keys) { $vms[$k] = [string]$s.vms[$k] }
$log = $s.log; $crash = [string]$s.crash; $hold = [int]$s.dispatch_ms
$me = Get-Process -Id $PID
$ctx = [pscustomobject]@{
    StatePath = Join-Path $s.dir 'state.json'; SessionPath = Join-Path $s.dir 'coordinator-session.json'; LockPath = Join-Path $s.dir 'coordinator.lock'
    StopRequestPath = Join-Path $s.dir 'coordinator-stop.request'; AuditPath = Join-Path $s.dir 'coordinator-audit.jsonl'; ThresholdsPath = Join-Path $s.kit 'paging-thresholds.json'
    ReadState = { param($p) $o = $tmpl | ConvertFrom-Json; $o.written_qpc = [Diagnostics.Stopwatch]::GetTimestamp() - $f; $o.qpc_frequency = $f; [pscustomobject]@{ ok = $true; text = ($o | ConvertTo-Json -Depth 12); error = $null } }.GetNewClosure()
    NowQpc = { [Diagnostics.Stopwatch]::GetTimestamp() }; QpcFrequency = $f; IsMonitorAlive = { param($a, $b) $true }
    ReadVmState = { param($vm) $vms[$vm] }.GetNewClosure(); ReadVmConfig = { param($vm) @() }
    Dispatch = @{
        'vm.start' = { param($vm) Add-Content -LiteralPath $log -Value "vm.start $vm $PID"; if ($crash -eq 'start') { [Environment]::Exit(9) }; Start-Sleep -Milliseconds $hold }.GetNewClosure()
        'vm.acpi' = { param($vm) Add-Content -LiteralPath $log -Value "vm.acpi $vm $PID"; if ($crash -eq 'acpi') { [Environment]::Exit(9) }; Start-Sleep -Milliseconds $hold }.GetNewClosure()
    }
    Sleep = { param($ms) Start-Sleep -Milliseconds ([math]::Max(1, [int]($ms / 20))) }; Pid = $PID; ProcessStartTicks = $me.StartTime.ToUniversalTime().Ticks
    EmergencyLockWaitMs = 10000; CalibrationHostChecks = { @() }
    ObservationPath = Join-Path $s.dir 'vm-observations.jsonl'; LedgerPath = Join-Path $s.dir 'acpi-validation-ledger.json'; UtcNow = { [DateTime]::UtcNow }; AcpiValidationLive = $true
    CountVmProcesses = { 0 }
}
if ($s.ready) { $null = [Threading.EventWaitHandle]::OpenExisting($s.ready).Set() }
if ($s.go) { $null = [Threading.EventWaitHandle]::OpenExisting($s.go).WaitOne(60000) }
$req = @{}; foreach ($k in $s.request.Keys) { $req[$k] = $s.request[$k] }
$r = $null
for ($i = 0; $i -lt [int]$s.retries; $i++) { $r = Invoke-P9SAction -Ctx $ctx -Request $req; if (-not (@($r.reasons) -contains 'COORDINATOR_BUSY')) { break }; Start-Sleep -Milliseconds 100 }
[ordered]@{ outcome = $r.outcome; reasons = @($r.reasons); dispatched = $r.dispatched; pid = $PID } | ConvertTo-Json | Set-Content -LiteralPath $s.out
'@ | Set-Content -LiteralPath $worker -Encoding ascii
$holder = Join-Path $OutDir 'holder.ps1'
@'
param([Parameter(Mandatory)][string]$Name, [Parameter(Mandatory)][string]$Ready, [Parameter(Mandatory)][string]$Release)
$m = [Threading.Mutex]::new($false, $Name); $null = $m.WaitOne(30000)
$null = [Threading.EventWaitHandle]::OpenExisting($Ready).Set()
$null = [Threading.EventWaitHandle]::OpenExisting($Release).WaitOne(120000)
$m.ReleaseMutex()
'@ | Set-Content -LiteralPath $holder -Encoding ascii
$mock = Join-Path $OutDir 'mockvbox.ps1'
@'
'name="' + $args[1] + '"'
'VMState="poweroff"'
exit 0
'@ | Set-Content -LiteralPath $mock -Encoding ascii
$script:events = [System.Collections.Generic.List[object]]::new()
function New-Event([string]$Name) { $e = [Threading.EventWaitHandle]::new($false, [Threading.EventResetMode]::ManualReset, $Name); $script:events.Add($e); $e }
$script:wN = 0
function Start-AWorker($c, [hashtable]$Request, [hashtable]$Vms, [string]$Crash = '', [int]$DispatchMs = 300, [string]$Go = '', [string]$Ready = '') {
    $script:wN++
    $tmpl = Join-Path $c.Dir 'state-template.json'
    if (-not (Test-Path -LiteralPath $tmpl)) { JM $c.Prov.mon ([Diagnostics.Stopwatch]::GetTimestamp()) ([Diagnostics.Stopwatch]::Frequency) 1 | Set-Content -LiteralPath $tmpl -Encoding utf8 }
    $s = [ordered]@{ kit = $kit; dir = $c.Dir; state = $tmpl; vms = $Vms; log = (Join-Path $c.Dir 'dispatch.log'); crash = $Crash; dispatch_ms = $DispatchMs; request = $Request; retries = 200
        go = $Go; ready = $Ready; out = (Join-Path $c.Dir "worker-$($script:wN).json") }
    $sp = Join-Path $c.Dir "spec-$($script:wN).json"
    $s | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $sp
    $p = Start-Process -FilePath $pw -PassThru -WindowStyle Hidden -ArgumentList @('-NoProfile', '-NonInteractive', '-File', $worker, '-Spec', $sp)
    $null = $p.Handle
    [pscustomobject]@{ p = $p; out = $s.out }
}
function Wait-AWorker($W, [int]$ms = 120000) {
    if (-not $W.p.WaitForExit($ms)) { throw 'a worker did not end in time' }
    $W.p.WaitForExit()
    if (Test-Path -LiteralPath $W.out) { Get-Content -Raw -LiteralPath $W.out | ConvertFrom-Json } else { $null }
}
function DispatchLog($c) { @(if (Test-Path -LiteralPath (Join-Path $c.Dir 'dispatch.log')) { Get-Content -LiteralPath (Join-Path $c.Dir 'dispatch.log') }) }

"== P9-S restricted ACPI validation synthetic tests, $((Get-Date).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ')), pwsh $($PSVersionTable.PSVersion), run $run"

# ---- the capability ------------------------------------------------------------------------------
Case 'A01' 'nothing live by default: without an owner authorization every live step is refused before any VirtualBox query, dry runs too' {
    $cap = Get-P9SAcpiValidationCapability
    Expect ($cap.name -eq 'ACPI_VALIDATION' -and $cap.live_enabled -eq $true) "capability: $($cap.live_enabled)"
    $d = Join-Path $OutDir 'realctx'
    $rc = New-P9SRealContext -StatePath (Join-Path $d 'state.json') -KitDir $kit -Config $null
    Expect ($rc.AcpiValidationLive -eq $true) 'the real context does not follow the capability'
    $n = & $rc.CountVmProcesses
    Expect ($n -is [int] -and $n -ge 0) "the real count of host VM processes: $n"
    $trip = [System.Collections.Generic.List[string]]::new()
    $rc.ReadState = { param($p) $trip.Add('state'); [pscustomobject]@{ ok = $false; text = $null; error = 'missing' } }.GetNewClosure()
    $rc.ReadVmState = { param($vm) $trip.Add("vm $vm"); 'off' }.GetNewClosure()
    $rc.ReadVmConfig = { param($vm) $trip.Add("config $vm"); @() }.GetNewClosure()
    $rc.Dispatch = @{ 'vm.start' = { param($vm) $trip.Add('start') }.GetNewClosure(); 'vm.acpi' = { param($vm) $trip.Add('acpi') }.GetNewClosure() }
    $rc.CountVmProcesses = { $trip.Add('processes'); 0 }.GetNewClosure()
    $ex = Join-Path $kit 'acpi-validation-authorization.example.json'
    $exOut = Join-Path $authDir 'example-copy.json'; Copy-Item -LiteralPath $ex -Destination $exOut
    $cases = @(
        @(@{ Action = 'acpi.open'; Authorization = $ex }, 'AUTHORIZATION_IN_TOOLKIT'), @(@{ Action = 'acpi.open'; Authorization = $exOut }, 'AUTHORIZATION_SAFEGUARD_NOT_APPROVED'),
        @(@{ Action = 'acpi.open'; Authorization = (New-Auth -Set @{ max_acpi_requests = 2 }) }, 'AUTHORIZATION_BUDGET:max_acpi_requests'),
        @(@{ Action = 'acpi.open'; Authorization = $exOut; DryRun = $true }, 'AUTHORIZATION_SAFEGUARD_NOT_APPROVED'),
        @(@{ Action = 'acpi.start' }, 'ACPI_VALIDATION_SESSION_MISSING'), @(@{ Action = 'acpi.start'; DryRun = $true }, 'ACPI_VALIDATION_SESSION_MISSING'),
        @(@{ Action = 'acpi.observe'; Seconds = 5 }, 'ACPI_VALIDATION_SESSION_MISSING'), @(@{ Action = 'acpi.shutdown' }, 'ACPI_VALIDATION_SESSION_MISSING'),
        @(@{ Action = 'acpi.shutdown'; DryRun = $true }, 'ACPI_VALIDATION_SESSION_MISSING'), @(@{ Action = 'acpi.integrity' }, 'ACPI_VALIDATION_SESSION_MISSING'),
        @(@{ Action = 'acpi.finalize' }, 'ACPI_VALIDATION_SESSION_MISSING'))
    foreach ($x in $cases) {
        $r = Act $rc $x[0]
        Expect ($r.outcome -eq 'REFUSED' -and (Has $r $x[1])) "$($x[0].Action)$(if ($x[0].DryRun) { ' -DryRun' }): $(Why $r) (expected $($x[1]))"
    }
    $j = Act $rc @{ Action = 'acpi.open'; Authorization = (Join-Path $authDir 'absent.json') }
    Expect ($j.outcome -eq 'REJECTED') "open without a file: $(Why $j)"
    Expect ($trip.Count -eq 0) "read or dispatched without an authorization: $($trip -join ',')"
    Expect (-not (Test-Path -LiteralPath (Join-Path $d 'acpi-validation-ledger.json')) -and -not (Test-Path -LiteralPath (Join-Path $d 'coordinator-session.json'))) 'a ledger or a session was written'
    # a context with the live switch off refuses before anything is read
    $m = New-VCtx -Live $false
    $y = AOpen $m (New-Auth)
    Expect ($y.outcome -eq 'REFUSED' -and (Has $y 'ACPI_VALIDATION_LIVE_DISABLED') -and $m.Prov.vmreads -eq 0 -and $m.Prov.reads -eq 0) "mock with the switch off: $(Why $y)"
    # a valid authorization is the way in
    $v = New-VCtx; $va = New-Auth
    $o = AOpen $v $va; $s = AStart $v
    Expect ($o.outcome -eq 'DONE' -and $s.outcome -eq 'DISPATCHED' -and (Count $v 'vm.start') -eq 1) "with an authorization: open $(Why $o), start $(Why $s)"
}
Case 'A02' 'the authorization file: the example and every boundary violation are refused' {
    $kd = Split-Path -Parent (Join-Path $kit 'paging-thresholds.json')
    $ok = Read-P9SAcpiAuthorization -Path (New-Auth) -KitDir $kd
    Expect ($ok.ok -and $ok.auth.target_vm -eq 'P9S-D12' -and $ok.auth.min_available_mb -eq 8192 -and $ok.auth.max_startups -eq 1 -and $ok.auth.max_acpi_requests -eq 1) "valid fixture refused: $($ok.reasons -join ',')"
    $ex = Read-P9SAcpiAuthorization -Path (Join-Path $kit 'acpi-validation-authorization.example.json') -KitDir (Join-Path $OutDir 'not-the-kit')
    foreach ($r in 'AUTHORIZATION_ID_INVALID', 'AUTHORIZATION_TIME_INVALID', 'AUTHORIZATION_SAFEGUARD_NOT_APPROVED', 'AUTHORIZATION_APPROVAL_INCOMPLETE') { Expect (-not $ex.ok -and ($ex.reasons -contains $r)) "example: missing $r ($($ex.reasons -join ','))" }
    $inKit = Read-P9SAcpiAuthorization -Path (Join-Path $kit 'acpi-validation-authorization.example.json') -KitDir $kd
    Expect (-not $inKit.ok -and ($inKit.reasons -contains 'AUTHORIZATION_IN_TOOLKIT')) "file in the toolkit folder: $($inKit.reasons -join ',')"
    $cases = @(
        @(@{ purpose = 'CALIBRATION' }, 'AUTHORIZATION_PURPOSE'), @(@{ target_vm = 'P9S-U24' }, 'AUTHORIZATION_TARGET_NOT_ALLOWED'), @(@{ target_vm = 'other-project-vm' }, 'AUTHORIZATION_TARGET_NOT_ALLOWED'),
        @(@{ max_startups = 2 }, 'AUTHORIZATION_BUDGET:max_startups'), @(@{ max_startups = '1' }, 'AUTHORIZATION_BUDGET:max_startups'), @(@{ max_acpi_requests = 2 }, 'AUTHORIZATION_BUDGET:max_acpi_requests'),
        @(@{ max_acpi_requests = 0 }, 'AUTHORIZATION_BUDGET:max_acpi_requests'),
        @(@{ permitted_actions = @('acpi.open', 'guest.type') }, 'AUTHORIZATION_ACTIONS_INVALID'), @(@{ permitted_actions = @('acpi.start') }, 'AUTHORIZATION_ACTIONS_INVALID'),
        @(@{ permitted_actions = @('acpi.open', 'acpi.open') }, 'AUTHORIZATION_ACTIONS_INVALID'), @(@{ permitted_actions = @('acpi.open', 'vm.start') }, 'AUTHORIZATION_ACTIONS_INVALID'),
        @(@{ valid_from_utc = '2026-11-20T11:50:00' }, 'AUTHORIZATION_TIME_INVALID'), @(@{ expires_utc = (Z $T0.AddMinutes(-20)) }, 'AUTHORIZATION_TIME_INVALID'),
        @(@{ expires_utc = (Z $T0.AddHours(5)) }, 'AUTHORIZATION_WINDOW_TOO_LONG'),
        @(@{ monitor_conditions = [ordered]@{ mode = 'Monitor'; paging_stop = 'Required'; commit_gate_s = 300; commit_below_pct = 90; provisional_min_available_mb = 4096 } }, 'AUTHORIZATION_MONITOR_CONDITIONS'),
        @(@{ monitor_conditions = [ordered]@{ mode = 'Calibrate'; paging_stop = 'Required'; commit_gate_s = 300; commit_below_pct = 90; provisional_min_available_mb = 8192 } }, 'AUTHORIZATION_MONITOR_CONDITIONS'),
        @(@{ monitor_conditions = [ordered]@{ mode = 'Monitor'; paging_stop = 'NotRequired'; commit_gate_s = 300; commit_below_pct = 90; provisional_min_available_mb = 8192 } }, 'AUTHORIZATION_MONITOR_CONDITIONS'),
        @(@{ preflight_checks = @('monitor-eligible') }, 'AUTHORIZATION_PREFLIGHT_CHECKS'), @(@{ supervision = 'unattended' }, 'AUTHORIZATION_SUPERVISION'),
        @(@{ on_stop_or_abort = 'continue' }, 'AUTHORIZATION_STOP_POLICY'), @(@{ provisional_safeguard_approved = $false }, 'AUTHORIZATION_SAFEGUARD_NOT_APPROVED'),
        @(@{ provisional_safeguard_approved = 'true' }, 'AUTHORIZATION_SAFEGUARD_NOT_APPROVED'), @(@{ approved_by = '' }, 'AUTHORIZATION_APPROVAL_INCOMPLETE'),
        @(@{ approved_by = '<owner>' }, 'AUTHORIZATION_APPROVAL_INCOMPLETE'), @(@{ authorization_id = 'acpiauth-1' }, 'AUTHORIZATION_ID_INVALID'),
        @(@{ schema = 'p9s-acpi-validation-authorization/2' }, 'AUTHORIZATION_SCHEMA_UNSUPPORTED'))
    foreach ($x in $cases) {
        $r = Read-P9SAcpiAuthorization -Path (New-Auth -Set $x[0]) -KitDir $kd
        Expect (-not $r.ok -and ($r.reasons -contains $x[1])) "$(($x[0].Keys) -join ',') -> $($r.reasons -join ',') (expected $($x[1]))"
    }
    $miss = Read-P9SAcpiAuthorization -Path (New-Auth -Remove @('monitor_conditions')) -KitDir $kd
    Expect (-not $miss.ok -and ($miss.reasons -contains 'AUTHORIZATION_MONITOR_CONDITIONS')) "no monitor conditions: $($miss.reasons -join ',')"
    $bad = Join-Path $authDir 'broken.json'; '{ "schema": ' | Set-Content -LiteralPath $bad
    Expect ((Read-P9SAcpiAuthorization -Path $bad -KitDir $kd).reasons -contains 'AUTHORIZATION_INVALID_JSON') 'broken JSON'
    $big = Join-Path $authDir 'big.json'; ('{"schema":"p9s-acpi-validation-authorization/1","pad":"' + ('x' * 17000) + '"}') | Set-Content -LiteralPath $big
    Expect ((Read-P9SAcpiAuthorization -Path $big -KitDir $kd).reasons -contains 'AUTHORIZATION_TOO_LARGE') 'large file'
    Expect ((Read-P9SAcpiAuthorization -Path (Join-Path $authDir 'absent.json') -KitDir $kd).reasons -contains 'AUTHORIZATION_MISSING') 'missing file'
    $c = New-VCtx
    $o = AOpen $c (New-Auth -Set @{ max_startups = 3 })
    Expect ($o.outcome -eq 'REFUSED' -and (Has $o 'AUTHORIZATION_BUDGET:max_startups') -and -not (Test-Path -LiteralPath $c.LedgerPath) -and -not (Test-Path -LiteralPath $c.SessionPath)) "open with a bad file: $(Why $o)"
}
Case 'A03' 'an authorization outside its window is refused at open and at every progressing step' {
    $c = New-VCtx
    $x = AOpen $c (New-Auth -From $T0.AddMinutes(5) -To $T0.AddHours(1))
    Expect ($x.outcome -eq 'REFUSED' -and (Has $x 'AUTHORIZATION_NOT_YET_VALID')) "not yet valid: $(Why $x)"
    $y = AOpen $c (New-Auth -From $T0.AddHours(-2) -To $T0.AddMinutes(-1))
    Expect ($y.outcome -eq 'REFUSED' -and (Has $y 'AUTHORIZATION_EXPIRED')) "expired: $(Why $y)"
    $a = New-Auth -From $T0.AddMinutes(-10) -To $T0.AddMinutes(20)
    $o = AOpen $c $a
    Expect ($o.outcome -eq 'DONE') "open: $(Why $o)"
    $c.Clock.utc = $T0.AddMinutes(21)
    $s = AStart $c
    Expect ($s.outcome -eq 'REFUSED' -and (@($s.reasons) -join ',') -eq 'AUTHORIZATION_EXPIRED' -and $c.Calls.Count -eq 0 -and (Entry $c $a).startups_used -eq 0) "start after expiry: $(Why $s)"
    $b = AObs $c 10
    Expect ($b.outcome -eq 'REFUSED' -and (Has $b 'AUTHORIZATION_EXPIRED')) "observe after expiry: $(Why $b)"
}
Case 'A04' 'one VM only: no action takes a VM name, another target is refused, and only P9S-D12 is ever touched' {
    $c = New-VCtx
    foreach ($a in 'acpi.preflight', 'acpi.open', 'acpi.start', 'acpi.observe', 'acpi.shutdown', 'acpi.confirm-off', 'acpi.integrity', 'acpi.finalize') {
        $r = @{ Action = $a; Vm = 'P9S-U24' }; if ($a -in 'acpi.observe', 'acpi.confirm-off') { $r.Seconds = 5 }; if ($a -eq 'acpi.open') { $r.Authorization = (New-Auth) }
        $x = Act $c $r
        Expect ($x.outcome -eq 'REJECTED' -and (Has $x 'PARAMETER_NOT_ALLOWED:Vm')) "$a -Vm: $(Why $x)"
    }
    $u = AOpen $c (New-Auth -Set @{ target_vm = 'P9S-U24' })
    Expect ($u.outcome -eq 'REFUSED' -and (Has $u 'AUTHORIZATION_TARGET_NOT_ALLOWED') -and -not (Test-Path -LiteralPath $c.SessionPath)) "U24 target: $(Why $u)"
    $c.Prov.offAfter = 1
    $a = New-Auth
    $steps = @((AOpen $c $a), (AStart $c), (AObs $c 10), (AShut $c), (Act $c @{ Action = 'acpi.integrity' }), (AFinal $c))
    Expect (@($steps | Where-Object { $_.outcome -notin 'DONE', 'DISPATCHED' }).Count -eq 0) "flow: $(@($steps | ForEach-Object { Why $_ }) -join ' / ')"
    Expect (@($c.Calls | Where-Object { $_.vm -ne 'P9S-D12' }).Count -eq 0 -and $c.Calls.Count -eq 2) "calls: $(Kinds $c)"
    $j = @(Read-P9SVmObservations $c.ObservationPath)
    Expect ($j.Count -gt 0 -and @($j | Where-Object { $_.vm -ne 'P9S-D12' }).Count -eq 0) 'another VM was observed'
}
Case 'A05' 'one start: no second start, not after the VM is off again, not from a new process' {
    $c = New-VCtx; $c.Prov.offAfter = 0
    $a = New-Auth
    $null = AOpen $c $a; $s1 = AStart $c; $sh = AShut $c
    Expect ($s1.outcome -eq 'DISPATCHED' -and $sh.outcome -eq 'DONE' -and $c.VmTable['P9S-D12'] -eq 'off') "start $(Why $s1), shutdown $(Why $sh)"
    $s2 = AStart $c
    Expect ($s2.outcome -eq 'REFUSED' -and (@($s2.reasons) -join ',') -eq 'ACPI_STARTUP_BUDGET_EXHAUSTED' -and (Count $c 'vm.start') -eq 1) "second start: $(Why $s2)"
    $c2 = New-VCtx -Dir $c.Dir
    $s3 = AStart $c2
    Expect ($s3.outcome -eq 'REFUSED' -and (Has $s3 'ACPI_STARTUP_BUDGET_EXHAUSTED') -and $c2.Calls.Count -eq 0) "after a restart: $(Why $s3)"
    Expect ((Entry $c $a).startups_used -eq 1) 'start budget not recorded'
}
Case 'A06' 'one ACPI request: neither the validation nor the emergency path sends a second one' {
    $x = Started; $c = $x.c
    $t1 = AShut $c 10
    Expect ($t1.outcome -eq 'TIMEOUT' -and (Has $t1 'ACPI_VALIDATION_UNRESOLVED')) "first request: $(Why $t1)"
    $t2 = AShut $c 10
    Expect ($t2.outcome -eq 'REFUSED' -and (@($t2.reasons) -join ',') -eq 'ACPI_BUDGET_EXHAUSTED') "second request: $(Why $t2)"
    $e = Emerg $c
    Expect ($e.outcome -eq 'REFUSED' -and (Has $e 'ACPI_ALREADY_REQUESTED') -and -not (Test-Path -LiteralPath $c.StopRequestPath)) "emergency: $(Why $e)"
    Expect ((Count $c 'vm.acpi') -eq 1 -and (Entry $c $x.a).acpi_used -eq 1) "requests: $(Kinds $c)"
}
Case 'A07' 'no acceptance-test action works in a validation session, and it cannot be closed away' {
    $c = New-VCtx
    $null = AOpen $c (New-Auth)
    $plan = Join-Path $c.Dir 'plan.json'; [ordered]@{ schema = 'p9s-plan/1'; name = 'p'; steps = @(@{ Action = 'wait'; Seconds = 5 }) } | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath $plan
    foreach ($r in @(@{ Action = 'vm.start'; Vm = 'P9S-D12' }, @{ Action = 'guest.type'; Vm = 'P9S-D12'; Text = 'abc' }, @{ Action = 'guest.key'; Vm = 'P9S-D12'; Key = 'Enter' }, @{ Action = 'guest.unlock'; Vm = 'P9S-D12' },
                      @{ Action = 'scenario.begin'; Name = 'x' }, @{ Action = 'phase.advance'; Name = 'y' }, @{ Action = 'wait'; Seconds = 5 }, @{ Action = 'sequence.run'; Plan = $plan })) {
        $x = Act $c $r
        Expect ($x.outcome -eq 'REFUSED' -and (Has $x 'SESSION_KIND_MISMATCH')) "$($r.Action): $(Why $x)"
    }
    Expect ($c.Calls.Count -eq 0) "dispatched: $(Kinds $c)"
    $o = Act $c @{ Action = 'session.open'; Kind = 'Test' }
    Expect ($o.outcome -eq 'REFUSED' -and (Has $o 'ACPI_VALIDATION_NOT_FINALIZED')) "test session: $(Why $o)"
    $cl = Act $c @{ Action = 'session.close' }
    Expect ($cl.outcome -eq 'REFUSED' -and (Has $cl 'ACPI_VALIDATION_USE_FINALIZE') -and (Sess $c).lifecycle -eq 'ACTIVE') "close: $(Why $cl)"
}
Case 'A08' 'nothing outside the allowlist: no input, snapshot, reset or forced power-off action or parameter exists' {
    $c = New-VCtx
    foreach ($a in 'acpi.poweroff', 'acpi.reset', 'acpi.snapshot', 'acpi.type', 'acpi.key', 'acpi.run', 'acpi.force-off', 'acpi.restart', 'acpi.unlock', 'acpi.start-u24', 'acpi.command') {
        $x = Act $c @{ Action = $a }
        Expect ($x.outcome -eq 'REJECTED' -and (Has $x 'UNKNOWN_ACTION')) "$a`: $(Why $x)"
    }
    foreach ($p in 'Text', 'Key', 'Plan', 'Force', 'IgnoreSafety', 'Command', 'Kind') {
        $x = Act $c @{ Action = 'acpi.start'; $p = 'x' }
        Expect ($x.outcome -eq 'REJECTED' -and (Has $x "PARAMETER_NOT_ALLOWED:$p")) "acpi.start -$p`: $(Why $x)"
    }
    $src = Get-Content -Raw -LiteralPath (Join-Path $kit 'p9s-acpival.ps1')
    $keys = @([regex]::Matches($src, "Dispatch\['([^']+)'\]") | ForEach-Object { $_.Groups[1].Value } | Sort-Object -Unique)
    Expect (($keys -join ',') -eq 'vm.acpi,vm.start') "dispatchers used by the capability: $($keys -join ',')"
    $c.Prov.offAfter = 0
    $null = AOpen $c (New-Auth); $null = AStart $c; $null = AObs $c 10; $null = AShut $c; $null = AFinal $c
    Expect ((Kinds $c) -eq 'vm.start P9S-D12, vm.acpi P9S-D12') "calls of a whole validation: $(Kinds $c)"
}
Case 'A09' 'monitor conditions: only a fresh, qualified, uncalibrated Monitor-mode run with no warning is eligible' {
    $alive = { param($a, $b) $true }; $dead = { param($a, $b) $false }
    $t = { param($m, $min = 0, [switch]$sd, $al = $alive, [double]$age = 1) Test-P9SAcpiValidationState -Text $(if ($m) { JM $m $NOW $F $age } else { $null }) -NowQpc $NOW -QpcFrequency $F -IsMonitorAlive $al -MinAvailableMb $min -ForShutdown:$sd }
    $e = & $t $ELIG 8192
    Expect ($e.ok -and $e.state -eq 'ACPI_VALIDATION_ELIGIBLE' -and $e.status -eq 'UNCALIBRATED' -and $e.paging_calibration -eq 'UNCALIBRATED') "eligible: $($e.reasons -join ',') $($e.status)"
    $want = @(@($null, 'STATE_EMPTY'), @($WARN, 'STATUS_WARNING'), @($PAGWARN, 'STATUS_WARNING'), @($QUAL, 'GATE_INCOMPLETE'), @($STOP, 'STATUS_STOP_REQUIRED'), @($STOP, 'LATCHED'), @($ABORT, 'STATUS_ABORT_REQUIRED'),
        @($CALIB, 'ACPI_VALIDATION_REQUIRES_MONITOR_MODE'), @($NOTREQ, 'ACPI_VALIDATION_REQUIRES_PAGING_STOP_REQUIRED'))
    foreach ($w in $want) { $r = & $t $w[0]; Expect (-not $r.ok -and ($r.reasons -contains $w[1])) "expected $($w[1]): $($r.reasons -join ',')" }
    $s = & $t $ELIG 0 -age 13; Expect (-not $s.ok -and ($s.reasons -contains 'STATE_STALE')) "stale: $($s.reasons -join ',')"
    $d = & $t $ELIG 0 -al $dead; Expect (-not $d.ok -and ($d.reasons -contains 'MONITOR_NOT_RUNNING')) "dead: $($d.reasons -join ',')"
    $l = & $t $LOWMEM 8192; Expect (-not $l.ok -and ($l.reasons -contains 'MEMORY_BELOW_PROVISIONAL_MINIMUM')) "low memory: $($l.reasons -join ',')"
    Expect ((& $t $LOWMEM 0).ok) 'low memory refused without a minimum'
    Expect ((& $t $WARN 0 -sd).ok -and (& $t $QUAL 0 -sd).ok) 'the shutdown refuses a warning or a broken qualification'
    Expect (-not (& $t $STOP 0 -sd).ok -and -not (& $t $ABORT 0 -sd).ok -and -not (& $t $ELIG 0 -sd -age 13).ok) 'the shutdown accepts STOP, ABORT or a stale monitor'
    foreach ($m in @($null, $WARN, $STOP, $QUAL, $LOWMEM)) {
        $c = New-VCtx -Mon $m
        $o = AOpen $c (New-Auth)
        Expect ($o.outcome -eq 'REFUSED' -and -not (Test-Path -LiteralPath $c.LedgerPath)) "open with $(if ($m) { $m.doc.status } else { 'no monitor' }): $(Why $o)"
    }
    $c = New-VCtx; $null = AOpen $c (New-Auth); $c.Prov.mon = $LOWMEM
    $s2 = AStart $c
    Expect ($s2.outcome -eq 'REFUSED' -and (Has $s2 'MEMORY_BELOW_PROVISIONAL_MINIMUM') -and $c.Calls.Count -eq 0) "start with low memory: $(Why $s2)"
}
Case 'A10' 'paging stays uncalibrated and normal READY stays unavailable; the monitor state is never written' {
    $c = New-VCtx
    $p = Act $c @{ Action = 'acpi.preflight'; Authorization = (New-Auth) }
    $txt = $p.lines -join "`n"
    Expect ($p.outcome -eq 'DONE' -and $p.reasons.Count -eq 0 -and $txt -match 'ACPI_VALIDATION_ELIGIBLE' -and $txt -match 'paging: UNCALIBRATED \(recorded, never a paging pass') "preflight: $(Why $p) $txt"
    $s = Test-SafetyState -Text (JM $ELIG $NOW $F) -NowQpc $NOW -QpcFrequency $F -PagingStop Required -IsMonitorAlive { param($a, $b) $true }
    Expect (-not $s.ok -and ($s.reasons -contains 'PAGING_STOP_NOT_ENFORCED') -and ($s.reasons -contains 'STATUS_UNCALIBRATED')) "normal reader: $($s.reasons -join ',')"
    $n = Act $c @{ Action = 'session.open'; Kind = 'Test' }
    Expect ($n.outcome -eq 'REFUSED' -and (Has $n 'PAGING_STOP_NOT_ENFORCED')) "test session on an eligible monitor: $(Why $n)"
    $c2 = New-VCtx -Mon $NOTREQ
    $n2 = Act $c2 @{ Action = 'session.open'; Kind = 'Test' }
    Expect ($n2.outcome -eq 'REFUSED' -and (Has $n2 'PAGING_STOP_NOT_ENFORCED')) "READY without an enforced paging stop: $(Why $n2)"
    $c3 = New-VCtx -Mon $READYAPP
    $n3 = Act $c3 @{ Action = 'session.open'; Kind = 'Test' }
    Expect ($n3.outcome -eq 'DONE') "approved paging (control): $(Why $n3)"
    Expect (-not (Test-Path -LiteralPath $c.StatePath) -and -not (Test-Path -LiteralPath $c2.StatePath)) 'the coordinator wrote a monitor state'
}
Case 'A11' 'a paging WARNING (candidate values) blocks every progressing step' {
    $c = New-VCtx -Mon $PAGWARN
    $o = AOpen $c (New-Auth)
    Expect ($o.outcome -eq 'REFUSED' -and (Has $o 'STATUS_WARNING')) "open: $(Why $o)"
    $c2 = New-VCtx; $null = AOpen $c2 (New-Auth); $c2.Prov.mon = $PAGWARN
    $s = AStart $c2
    Expect ($s.outcome -eq 'REFUSED' -and (Has $s 'STATUS_WARNING') -and $c2.Calls.Count -eq 0) "start: $(Why $s)"
    $x = Started; $x.c.Prov.switchAt = $x.c.Prov.reads + 3; $x.c.Prov.switchTo = $PAGWARN
    $b = AObs $x.c 60
    Expect ($b.outcome -eq 'STOPPED_AT_CHECKPOINT' -and (Has $b 'STATUS_WARNING')) "observe: $(Why $b)"
}
Case 'A12' 'an unresolved validation blocks every new validation and test session until the VM is observed off' {
    $x = Started; $c = $x.c
    $null = AShut $c 10
    $fin = AFinal $c
    Expect ($fin.outcome -eq 'DONE' -and (Has $fin 'ACPI_VALIDATION_UNRESOLVED') -and (Entry $c $x.a).outcome -eq 'ACPI_VALIDATION_UNRESOLVED' -and (Sess $c).lifecycle -eq 'CLOSED') "finalize: $(Why $fin)"
    $c.VmTable['P9S-D12'] = 'off'
    $a2 = New-Auth
    $o = AOpen $c $a2
    Expect ($o.outcome -eq 'REFUSED' -and (@($o.reasons) -join ',') -eq 'ACPI_VALIDATION_UNRESOLVED_PREVIOUS') "new validation: $(Why $o)"
    $c.Prov.mon = $READYAPP
    $t = Act $c @{ Action = 'session.open'; Kind = 'Test' }
    Expect ($t.outcome -eq 'REFUSED' -and (Has $t 'ACPI_VALIDATION_UNRESOLVED')) "test session: $(Why $t)"
    $c.Prov.mon = $ELIG
    $k = AConfirm $c 10
    Expect ($k.outcome -eq 'DONE' -and (Entry $c $x.a).phase -eq 'OFF_AFTER_DEADLINE') "confirm: $(Why $k) phase $((Entry $c $x.a).phase)"
    $o2 = AOpen $c $a2
    Expect ($o2.outcome -eq 'DONE' -and (Entry $c $a2).startups_used -eq 0) "after the confirmation: $(Why $o2)"
    $o3 = AOpen (New-VCtx -Dir $c.Dir) $x.a
    Expect ($o3.outcome -eq 'REFUSED' -and (Has $o3 'AUTHORIZATION_ALREADY_USED')) "old authorization again: $(Why $o3)"
}
Case 'A13' 'nominal validation: preflight, open, start, observe, one request, observed off, integrity, finalize' {
    $c = New-VCtx; $c.Prov.offAfter = 2
    $a = New-Auth
    $p = Act $c @{ Action = 'acpi.preflight'; Authorization = $a }
    Expect ($p.outcome -eq 'DONE' -and $p.reasons.Count -eq 0 -and ($p.lines -join ' ') -match 'all checks passed') "preflight: $(Why $p)"
    $o = AOpen $c $a; $s = AStart $c; $b = AObs $c 30; $sh = AShut $c; $in = Act $c @{ Action = 'acpi.integrity' }; $fi = AFinal $c
    Expect ($o.outcome -eq 'DONE' -and $s.outcome -eq 'DISPATCHED' -and $b.outcome -eq 'DONE' -and $sh.outcome -eq 'DONE' -and $in.outcome -eq 'DONE' -and $fi.outcome -eq 'DONE') "flow: $(@($o, $s, $b, $sh, $in, $fi | ForEach-Object { Why $_ }) -join ' / ')"
    Expect (($sh.lines -join ' ') -match 'observed off about 10 s after the ACPI request') "shutdown: $($sh.lines -join ' | ')"
    $e = Entry $c $a
    Expect ($e.startups_used -eq 1 -and $e.acpi_used -eq 1 -and $e.acpi_source -eq 'validation' -and $e.phase -eq 'OFF_CONFIRMED' -and $e.outcome -eq 'VALIDATED' -and $e.integrity.differing -eq 0) "ledger: $($e.phase) $($e.outcome)"
    Expect ((Kinds $c) -eq 'vm.start P9S-D12, vm.acpi P9S-D12') "calls: $(Kinds $c)"
    $aud = @(Get-Content -LiteralPath $c.AuditPath | ForEach-Object { $_ | ConvertFrom-Json })
    Expect (@($aud | Where-Object { $_.action -like 'acpi.*' }).Count -eq 7 -and @($aud | Where-Object { $_.action -eq 'acpi.start' -and $_.dispatched -eq 1 -and $_.vm -eq 'P9S-D12' }).Count -eq 1) 'audit records'
    $j = @(Read-P9SVmObservations $c.ObservationPath)
    # preflight 1, open 1, start 1, observe 2, shutdown 1 + watch 3, integrity 1, finalize 1
    Expect ($j.Count -eq 11) "VM observations: $($j.Count) ($(@($j | ForEach-Object purpose) -join ','))"
    Expect ((Sess $c).lifecycle -eq 'CLOSED') 'session not closed'
}
Case 'A14' 'dry runs dispatch nothing and spend nothing' {
    $c = New-VCtx
    $a = New-Auth
    $d1 = Act $c @{ Action = 'acpi.open'; Authorization = $a; DryRun = $true }
    Expect ($d1.outcome -eq 'DRYRUN' -and -not (Test-Path -LiteralPath $c.LedgerPath) -and -not (Test-Path -LiteralPath $c.SessionPath)) "open: $(Why $d1)"
    $null = AOpen $c $a
    $d2 = Act $c @{ Action = 'acpi.start'; DryRun = $true }
    Expect ($d2.outcome -eq 'DRYRUN' -and $c.Calls.Count -eq 0 -and (Entry $c $a).startups_used -eq 0) "start: $(Why $d2)"
    $null = AStart $c
    foreach ($r in @(@{ Action = 'acpi.shutdown'; DryRun = $true }, @{ Action = 'acpi.observe'; Seconds = 30; DryRun = $true }, @{ Action = 'acpi.finalize'; DryRun = $true })) {
        $x = Act $c $r
        Expect ($x.outcome -eq 'DRYRUN') "$($r.Action): $(Why $x)"
    }
    Expect ((Count $c 'vm.acpi') -eq 0 -and (Entry $c $a).acpi_used -eq 0 -and (Sess $c).lifecycle -eq 'ACTIVE') 'a dry run sent or closed something'
    $m = New-VCtx -Live $false
    $d3 = Act $m @{ Action = 'acpi.open'; Authorization = (New-Auth); DryRun = $true }
    Expect ($d3.outcome -eq 'DRYRUN') "dry run with the live path off: $(Why $d3)"
}
Case 'A15' 'a VirtualBox VM process of any VM on the host refuses the open and the start, also when it appears just before the dispatch' {
    $c = New-VCtx; $a = New-Auth; $p = $c.Prov
    $p.vmprocs = 1
    $o = AOpen $c $a
    Expect ($o.outcome -eq 'REFUSED' -and (Has $o 'VM_PROCESS_RUNNING:1') -and -not (Entry $c $a)) "open with a VM process: $(Why $o)"
    $p.vmprocs = 0
    $o2 = AOpen $c $a
    Expect ($o2.outcome -eq 'DONE') "open: $(Why $o2)"
    $p.vmprocs = 2
    $s = AStart $c
    Expect ($s.outcome -eq 'REFUSED' -and (Has $s 'VM_PROCESS_RUNNING:2') -and (Count $c 'vm.start') -eq 0 -and (Entry $c $a).startups_used -eq 0) "start with VM processes: $(Why $s)"
    $p.vmprocs = 0
    $p.onVmRead = { param($vm) $p.vmprocs = 1 }.GetNewClosure()
    $s2 = AStart $c
    Expect ($s2.outcome -eq 'REFUSED' -and (Has $s2 'STATE_CHANGED_BEFORE_DISPATCH') -and (Has $s2 'VM_PROCESS_RUNNING:1') -and (Count $c 'vm.start') -eq 0 -and (Entry $c $a).startups_used -eq 0) "a VM process appearing before the dispatch: $(Why $s2)"
    $p.onVmRead = $null; $p.vmprocs = 0
    $c.CountVmProcesses = { throw 'no process list' }
    $s3 = AStart $c
    Expect ($s3.outcome -eq 'REFUSED' -and (Has $s3 'VM_PROCESSES_UNKNOWN') -and (Count $c 'vm.start') -eq 0) "unknown count: $(Why $s3)"
    $c.CountVmProcesses = { 0 }
    $s4 = AStart $c
    Expect ($s4.outcome -eq 'DISPATCHED' -and (Count $c 'vm.start') -eq 1) "start with the host free of VMs: $(Why $s4)"
}
Case 'A16' 'available memory below the provisional minimum while the VM runs: the observation ends, the one ACPI request stays available' {
    $x = Started; $c = $x.c; $a = $x.a; $p = $c.Prov
    $p.mon = $LOWMEM
    $b = AObs $c 30
    Expect ($b.outcome -eq 'REFUSED' -and (Has $b 'MEMORY_BELOW_PROVISIONAL_MINIMUM') -and @(@($b.lines) -match 'stays available').Count -eq 1) "observe below the minimum: $(Why $b)"
    $p.mon = $ELIG; $p.switchAt = $p.reads + 3; $p.switchTo = $LOWMEM
    $b2 = AObs $c 60
    Expect ($b2.outcome -eq 'STOPPED_AT_CHECKPOINT' -and (Has $b2 'MEMORY_BELOW_PROVISIONAL_MINIMUM') -and @(@($b2.lines) -match 'stays available').Count -eq 1) "observation crossing the minimum: $(Why $b2)"
    $p.offAfter = 1
    $s = AShut $c
    Expect ($s.outcome -eq 'DONE' -and (Count $c 'vm.acpi') -eq 1 -and (Entry $c $a).phase -eq 'OFF_CONFIRMED') "the request below the minimum: $(Why $s)"
}
Case 'A17' 'a valid authorization permits only the actions it lists: an unlisted step is refused and dispatches nothing' {
    $c = New-VCtx; $a = New-Auth -Set @{ permitted_actions = @('acpi.open') }
    $o = AOpen $c $a
    Expect ($o.outcome -eq 'DONE') "open: $(Why $o)"
    $s = AStart $c
    Expect ($s.outcome -eq 'REFUSED' -and (Has $s 'ACTION_NOT_AUTHORIZED:acpi.start') -and $c.Calls.Count -eq 0 -and (Entry $c $a).startups_used -eq 0) "start not listed: $(Why $s)"
    $fin = AFinal $c
    Expect ($fin.outcome -eq 'DONE' -and (Entry $c $a).outcome -eq 'NOT_STARTED') "finalize: $(Why $fin)"
    $c2 = New-VCtx; $a2 = New-Auth -Set @{ permitted_actions = @('acpi.open', 'acpi.start', 'acpi.integrity') }
    $null = AOpen $c2 $a2; $s2 = AStart $c2
    Expect ($s2.outcome -eq 'DISPATCHED') "start listed: $(Why $s2)"
    $b = AObs $c2 10
    Expect ($b.outcome -eq 'REFUSED' -and (Has $b 'ACTION_NOT_AUTHORIZED:acpi.observe')) "observe not listed: $(Why $b)"
    $d = AShut $c2
    Expect ($d.outcome -eq 'REFUSED' -and (Has $d 'ACTION_NOT_AUTHORIZED:acpi.shutdown') -and (Count $c2 'vm.acpi') -eq 0 -and (Entry $c2 $a2).acpi_used -eq 0) "shutdown not listed: $(Why $d)"
}

# ---- recovery when the guest does not power off ------------------------------------------------
Case 'R01' 'ACPI request failure: unresolved, nothing else sent, the budget is spent' {
    $x = Started; $c = $x.c; $c.Prov.acpiThrows = $true
    $r = AShut $c
    Expect ($r.outcome -eq 'FAILED' -and (Has $r 'ACPI_REQUEST_FAILED') -and (Has $r 'ACPI_VALIDATION_UNRESOLVED')) "request: $(Why $r)"
    $e = Entry $c $x.a
    Expect ($e.acpi_used -eq 1 -and $e.phase -eq 'ACPI_VALIDATION_UNRESOLVED') "ledger: $($e.acpi_used) $($e.phase)"
    $c.Prov.acpiThrows = $false
    $r2 = AShut $c; $em = Emerg $c
    Expect ((Has $r2 'ACPI_BUDGET_EXHAUSTED') -and (Has $em 'ACPI_ALREADY_REQUESTED')) "again: $(Why $r2) / $(Why $em)"
    Expect ((Kinds $c) -eq 'vm.start P9S-D12, vm.acpi P9S-D12') "calls: $(Kinds $c)"
    $fin = AFinal $c
    Expect ((Entry $c $x.a).outcome -eq 'ACPI_VALIDATION_UNRESOLVED' -and (Has $fin 'ACPI_VALIDATION_UNRESOLVED')) "finalize: $(Why $fin)"
}
Case 'R02' 'the request is dispatched but the guest keeps running: unresolved after a bounded watch, never success' {
    $x = Started; $c = $x.c
    $r = AShut $c
    Expect ($r.outcome -eq 'TIMEOUT' -and (Has $r 'ACPI_VALIDATION_UNRESOLVED') -and $r.dispatched -eq 1) "request: $(Why $r)"
    Expect ($c.Prov.acpiReads -eq 37 -and ($r.lines -join ' ') -match '37 queries') "queries: $($c.Prov.acpiReads)"
    $k = AConfirm $c 30
    Expect ($k.outcome -eq 'TIMEOUT' -and $c.Prov.acpiReads -eq 44 -and (Entry $c $x.a).phase -eq 'ACPI_VALIDATION_UNRESOLVED') "confirmation: $(Why $k) reads $($c.Prov.acpiReads)"
    Expect ((Kinds $c) -eq 'vm.start P9S-D12, vm.acpi P9S-D12') "calls: $(Kinds $c)"
}
Case 'R03' 'success only on an observed power-off' {
    $x = Started; $c = $x.c; $c.Prov.offAfter = 2
    $r = AShut $c
    Expect ($r.outcome -eq 'DONE' -and (Entry $c $x.a).phase -eq 'OFF_CONFIRMED' -and $c.Prov.acpiReads -eq 3) "request: $(Why $r) reads $($c.Prov.acpiReads)"
    $y = Started; $y.c.Prov.onAcpi = { }
    $y.c.Prov.onVmRead = { param($vm) }
    $r2 = AShut $y.c 5
    Expect ($r2.outcome -ne 'DONE' -and (Entry $y.c $y.a).phase -ne 'OFF_CONFIRMED') "a sent request without power-off: $(Why $r2)"
}
Case 'R04' 'deadline boundary: off at the last observation is in time, one observation later is late' {
    $x = Started; $x.c.Prov.offAfter = 36
    $r = AShut $x.c
    Expect ($r.outcome -eq 'DONE' -and (Entry $x.c $x.a).phase -eq 'OFF_CONFIRMED') "at the deadline: $(Why $r)"
    $y = Started; $y.c.Prov.offAfter = 37
    $r2 = AShut $y.c
    Expect ($r2.outcome -eq 'TIMEOUT' -and (Entry $y.c $y.a).phase -eq 'ACPI_VALIDATION_UNRESOLVED') "after the deadline: $(Why $r2)"
    $k = AConfirm $y.c 30
    Expect ($k.outcome -eq 'DONE' -and (Has $k 'SHUTDOWN_AFTER_DEADLINE') -and (Entry $y.c $y.a).phase -eq 'OFF_AFTER_DEADLINE') "late confirmation: $(Why $k)"
    $null = AFinal $y.c
    Expect ((Entry $y.c $y.a).outcome -eq 'SHUTDOWN_LATE') "outcome $((Entry $y.c $y.a).outcome)"
}
Case 'R05' 'an unreadable VM state during the watch: unresolved, never off' {
    $x = Started; $c = $x.c
    $tbl = $c.VmTable; $c.Prov.onAcpi = { $tbl['P9S-D12'] = 'unreadable' }.GetNewClosure()
    $r = AShut $c
    Expect ($r.outcome -eq 'FAILED' -and (Has $r 'VM_STATE_UNOBSERVED') -and (Has $r 'ACPI_VALIDATION_UNRESOLVED') -and $c.Prov.acpiReads -eq 1) "watch: $(Why $r) reads $($c.Prov.acpiReads)"
    Expect ((Entry $c $x.a).phase -eq 'ACPI_VALIDATION_UNRESOLVED' -and (Sess $c).expected_vm['P9S-D12'] -ne 'off') 'taken for off'
}
Case 'R06' 'a coordinator that dies after dispatching the start: no second start, nothing proceeds, the emergency path counts' {
    $c = New-VCtx -RealClock; $a = New-Auth -Real
    $o = AOpen $c $a
    Expect ($o.outcome -eq 'DONE') "open: $(Why $o)"
    $w = Start-AWorker $c @{ Action = 'acpi.start' } @{ 'P9S-D12' = 'off'; 'P9S-U24' = 'off' } -Crash 'start'
    $wr = Wait-AWorker $w
    Expect ($null -eq $wr -and $w.p.ExitCode -eq 9 -and @(DispatchLog $c).Count -eq 1) "worker: exit $($w.p.ExitCode), log $(@(DispatchLog $c).Count)"
    $e = Entry $c $a
    Expect ($e.startups_used -eq 1 -and $e.phase -eq 'STARTING' -and (Sess $c).inflight.action -eq 'acpi.start') "after the crash: $($e.startups_used) $($e.phase)"
    $c2 = New-VCtx -Dir $c.Dir -RealClock -Vms @{ 'P9S-D12' = 'running' }
    $s = AStart $c2; $sh = AShut $c2; $b = AObs $c2 5
    Expect ((Has $s 'ACPI_STARTUP_BUDGET_EXHAUSTED') -and (Has $s 'INFLIGHT_UNRESOLVED') -and (Has $sh 'INFLIGHT_UNRESOLVED') -and (Has $b 'INFLIGHT_UNRESOLVED') -and $c2.Calls.Count -eq 0) "after restart: $(Why $s) / $(Why $sh) / $(Why $b)"
    $c2.Prov.offAfter = 0
    $em = Emerg $c2
    Expect ($em.outcome -eq 'DONE' -and (Count $c2 'vm.acpi') -eq 1) "emergency: $(Why $em)"
    $e2 = Entry $c2 $a
    Expect ($e2.acpi_used -eq 1 -and $e2.acpi_source -eq 'emergency' -and $e2.phase -eq 'OFF_CONFIRMED') "ledger after the emergency: $($e2.acpi_used) $($e2.acpi_source) $($e2.phase)"
    $fin = AFinal $c2
    Expect ($fin.outcome -eq 'DONE' -and (Entry $c2 $a).outcome -eq 'OFF_AFTER_EMERGENCY_ACPI' -and ($fin.lines -join ' ') -match 'discarded') "finalize: $(Why $fin)"
}
Case 'R07' 'a coordinator that dies after dispatching the ACPI request: no second request; a read-only confirmation resolves it' {
    $c = New-VCtx -RealClock; $a = New-Auth -Real
    $null = AOpen $c $a; $s = AStart $c
    Expect ($s.outcome -eq 'DISPATCHED') "start: $(Why $s)"
    $w = Start-AWorker $c @{ Action = 'acpi.shutdown' } @{ 'P9S-D12' = 'running'; 'P9S-U24' = 'off' } -Crash 'acpi'
    $null = Wait-AWorker $w
    $e = Entry $c $a
    Expect ($w.p.ExitCode -eq 9 -and $e.acpi_used -eq 1 -and $e.phase -eq 'SHUTDOWN_REQUESTING') "after the crash: exit $($w.p.ExitCode) $($e.acpi_used) $($e.phase)"
    $c2 = New-VCtx -Dir $c.Dir -RealClock -Vms @{ 'P9S-D12' = 'off' }
    $sh = AShut $c2; $em = Emerg $c2
    Expect ((Has $sh 'ACPI_BUDGET_EXHAUSTED') -and (Has $em 'ACPI_ALREADY_REQUESTED') -and $c2.Calls.Count -eq 0) "after restart: $(Why $sh) / $(Why $em)"
    $k = AConfirm $c2 10
    Expect ($k.outcome -eq 'DONE' -and (Entry $c2 $a).phase -eq 'OFF_REQUEST_UNCONFIRMED') "confirmation: $(Why $k) $((Entry $c2 $a).phase)"
    $null = AFinal $c2
    Expect ((Entry $c2 $a).outcome -eq 'OFF_AFTER_UNCONFIRMED_REQUEST') "outcome: $((Entry $c2 $a).outcome) (an unconfirmed request is never VALIDATED)"
}
Case 'R08' 'host STOP during the session: progression refused, one emergency request allowed and counted' {
    $x = Started; $c = $x.c; $c.Prov.mon = $STOP
    $b = AObs $c 10
    Expect ($b.outcome -eq 'REFUSED' -and (Has $b 'SESSION_HALTED_STOP') -and (Sess $c).lifecycle -eq 'HALTED_STOP') "observe: $(Why $b)"
    $sh = AShut $c; $st = AStart $c
    Expect ($sh.outcome -eq 'REFUSED' -and ($sh.lines -join ' ') -match 'emergency\.acpi-shutdown' -and $st.outcome -eq 'REFUSED' -and (Count $c 'vm.acpi') -eq 0) "shutdown $(Why $sh) start $(Why $st)"
    $c.Prov.offAfter = 0
    $em = Emerg $c
    Expect ($em.outcome -eq 'DONE' -and (Entry $c $x.a).acpi_source -eq 'emergency' -and (Entry $c $x.a).phase -eq 'OFF_CONFIRMED') "emergency: $(Why $em)"
    $sh2 = AShut $c
    Expect ((Has $sh2 'ACPI_BUDGET_EXHAUSTED') -and (Count $c 'vm.acpi') -eq 1) "after the emergency: $(Why $sh2)"
    $null = AFinal $c
    Expect ((Entry $c $x.a).outcome -eq 'OFF_AFTER_EMERGENCY_ACPI' -and (Entry $c $x.a).halt -eq 'STOP_REQUIRED') "outcome $((Entry $c $x.a).outcome) halt $((Entry $c $x.a).halt)"
}
Case 'R09' 'host ABORT: the same, and a new validation needs a new monitor run and the abort id' {
    $x = Started; $c = $x.c; $c.Prov.mon = $ABORT
    $b = AObs $c 10
    Expect ((Has $b 'SESSION_HALTED_ABORT') -and (Sess $c).halt.abort_id) "observe: $(Why $b)"
    $c.Prov.offAfter = 0
    $em = Emerg $c; $null = AFinal $c
    Expect ($em.outcome -eq 'DONE' -and (Entry $c $x.a).outcome -eq 'OFF_AFTER_EMERGENCY_ACPI') "emergency $(Why $em)"
    $null = Act $c @{ Action = 'session.close' }
    $id = (Sess $c).halt.abort_id
    $c.Prov.mon = $ELIG
    $a2 = New-Auth
    $o = AOpen $c $a2
    Expect ($o.outcome -eq 'REFUSED' -and (Has $o 'STOP_REQUIRES_NEW_MONITOR_RUN') -and (Has $o 'ABORT_NOT_ACKNOWLEDGED')) "same run: $(Why $o)"
    $c.Prov.mon = $ELIG_B
    $o2 = AOpen $c $a2
    Expect ($o2.outcome -eq 'REFUSED' -and (Has $o2 'ABORT_NOT_ACKNOWLEDGED')) "no acknowledgement: $(Why $o2)"
    $o3 = AOpen $c $a2 @{ AcknowledgeAbort = $id }
    Expect ($o3.outcome -eq 'DONE') "acknowledged on a new run: $(Why $o3)"
}
Case 'R10' 'a VirtualBox quarantine during the session: nothing proceeds, nothing is sent, nothing is claimed' {
    $x = Started; $c = $x.c
    $vb = New-P9SVBoxContext -Dir $c.Dir -VmNames @('P9S-D12', 'P9S-U24') -MutexName "Global\P9S-TEST-$run-r10"
    $c | Add-Member -NotePropertyName VBox -NotePropertyValue $vb
    $c.Prov.onSleep = { $null = Set-P9SVBoxLatch $vb 'VBOX_COMMAND_TIMEOUT' $null 'test fixture' }.GetNewClosure()
    $b = AObs $c 30
    Expect ($b.outcome -eq 'STOPPED_AT_CHECKPOINT' -and (Has $b 'VBOX_API_QUARANTINED')) "observe: $(Why $b)"
    $sh = AShut $c
    Expect ($sh.outcome -eq 'REFUSED' -and (Has $sh 'VBOX_API_QUARANTINED')) "shutdown: $(Why $sh)"
    $em = Emerg $c
    Expect ($em.outcome -eq 'FAILED' -and (Has $em 'EMERGENCY_UNRESOLVED') -and (Count $c 'vm.acpi') -eq 0 -and (Entry $c $x.a).acpi_used -eq 0) "emergency: $(Why $em)"
    $fin = AFinal $c
    Expect ((Entry $c $x.a).outcome -eq 'ACPI_VALIDATION_UNRESOLVED' -and ($fin.lines -join ' ') -match 'not read') "finalize: $(Why $fin)"
}
Case 'R11' 'a failed ledger write: before the dispatch nothing is sent; after it the budget stays spent and the outcome unresolved' {
    $c = New-VCtx; $a = New-Auth
    $null = AOpen $c $a
    $c.Prov.failLedgerAt = $c.Prov.ledgerWrites + 1
    $s = AStart $c
    Expect ($s.outcome -eq 'REFUSED' -and (Has $s 'EVIDENCE_WRITE_FAILED') -and $c.Calls.Count -eq 0 -and (Entry $c $a).startups_used -eq 0) "write-ahead failed: $(Why $s)"
    $c.Prov.failLedgerAt = 0
    $s2 = AStart $c
    Expect ($s2.outcome -eq 'DISPATCHED') "start once the ledger works: $(Why $s2)"
    $d = New-VCtx; $b = New-Auth
    $null = AOpen $d $b
    $d.Prov.failLedgerAt = $d.Prov.ledgerWrites + 2; $d.Prov.failLedgerTo = $d.Prov.ledgerWrites + 2
    $t = AStart $d
    Expect ($t.outcome -eq 'DISPATCHED' -and (Has $t 'EVIDENCE_WRITE_FAILED') -and (Entry $d $b).phase -eq 'STARTING' -and (Entry $d $b).startups_used -eq 1) "outcome write failed: $(Why $t) $((Entry $d $b).phase)"
    Expect ((Has (AStart $d) 'ACPI_STARTUP_BUDGET_EXHAUSTED') -and (Has (AOpen (New-VCtx -Dir $d.Dir) (New-Auth)) 'ACPI_VALIDATION_UNRESOLVED_PREVIOUS')) 'the half-written start is not blocking'
    $g = Started; $gc = $g.c
    $gc.Prov.failLedgerAt = $gc.Prov.ledgerWrites + 3; $gc.Prov.failLedgerTo = $gc.Prov.ledgerWrites + 3
    $r = AShut $gc 10
    Expect ($r.outcome -eq 'TIMEOUT' -and (Has $r 'EVIDENCE_WRITE_FAILED') -and (Entry $gc $g.a).phase -eq 'SHUTDOWN_REQUESTED' -and (Entry $gc $g.a).acpi_used -eq 1) "watch result not written: $(Why $r) $((Entry $gc $g.a).phase)"
    Expect ((Has (AShut $gc 10) 'ACPI_BUDGET_EXHAUSTED') -and (Has (Emerg $gc) 'ACPI_ALREADY_REQUESTED') -and (Count $gc 'vm.acpi') -eq 1) 'a second request after the failed write'
}

# ---- races -----------------------------------------------------------------------------------------
Case 'Z01' 'STOP just before the start: the last check refuses, nothing is spent' {
    $c = New-VCtx; $a = New-Auth; $null = AOpen $c $a
    $c.Prov.switchAt = $c.Prov.reads + 2; $c.Prov.switchTo = $STOP
    $s = AStart $c
    Expect ($s.outcome -eq 'REFUSED' -and (Has $s 'STATE_CHANGED_BEFORE_DISPATCH') -and $c.Calls.Count -eq 0 -and (Entry $c $a).startups_used -eq 0) "start: $(Why $s)"
}
Case 'Z02' 'STOP during the start: the start counts, the session halts, only the emergency path remains' {
    $c = New-VCtx; $a = New-Auth; $null = AOpen $c $a
    $p = $c.Prov; $c.Prov.onStart = { $p.mon = $STOP }.GetNewClosure()
    $s = AStart $c
    Expect ($s.outcome -eq 'DISPATCHED' -and (Sess $c).lifecycle -eq 'HALTED_STOP' -and ($s.lines -join ' ') -match 'emergency\.acpi-shutdown') "start: $(Why $s)"
    $b = AObs $c 10
    Expect ((Has $b 'SESSION_HALTED_STOP') -and (Entry $c $a).startups_used -eq 1) "observe: $(Why $b)"
    $c.Prov.offAfter = 0
    Expect ((Emerg $c).outcome -eq 'DONE') 'emergency refused'
}
Case 'Z03' 'ABORT while the start outcome is uncertain: unresolved, halted, nothing more is started or claimed' {
    $c = New-VCtx; $a = New-Auth; $null = AOpen $c $a
    $p = $c.Prov; $tbl = $c.VmTable
    $c.Prov.startThrows = $true; $c.Prov.onStart = { $p.mon = $ABORT; $tbl['P9S-D12'] = 'unreadable' }.GetNewClosure()
    $s = AStart $c
    Expect ($s.outcome -eq 'FAILED' -and (Has $s 'DISPATCH_FAILED') -and (Has $s 'ACPI_VALIDATION_UNRESOLVED') -and (Entry $c $a).phase -eq 'START_UNCERTAIN' -and (Sess $c).lifecycle -eq 'HALTED_ABORT') "start: $(Why $s) $((Entry $c $a).phase)"
    $c.Prov.startThrows = $false
    $s2 = AStart $c; $em = Emerg $c
    Expect ((Has $s2 'ACPI_STARTUP_BUDGET_EXHAUSTED') -and (Has $em 'VM_NOT_RUNNING:unreadable') -and (Count $c 'vm.start') -eq 1 -and (Count $c 'vm.acpi') -eq 0) "afterwards: $(Why $s2) / $(Why $em)"
    $null = AFinal $c
    $c.Prov.mon = $READYAPP
    Expect ((Entry $c $a).outcome -eq 'ACPI_VALIDATION_UNRESOLVED' -and (Has (Act $c @{ Action = 'session.open'; Kind = 'Test' }) 'ACPI_VALIDATION_UNRESOLVED')) 'unresolved start not blocking'
}
Case 'Z04' 'a stale monitor heartbeat refuses the start and stops an observation' {
    $c = New-VCtx; $a = New-Auth; $null = AOpen $c $a
    $c.Prov.age = 13
    $s = AStart $c
    Expect ($s.outcome -eq 'REFUSED' -and (Has $s 'STATE_STALE') -and $c.Calls.Count -eq 0) "start: $(Why $s)"
    $c.Prov.age = 1
    $null = AStart $c
    $p = $c.Prov; $c.Prov.onSleep = { $p.age = 13 }.GetNewClosure()
    $b = AObs $c 30
    Expect ($b.outcome -eq 'STOPPED_AT_CHECKPOINT' -and (Has $b 'STATE_STALE')) "observe: $(Why $b)"
}
Case 'Z05' 'a VM observation that is stale by the decision refuses the start' {
    $c = New-VCtx; $a = New-Auth; $null = AOpen $c $a
    $k = $c.Clock; $step = 11L * $F; $c.Prov.onVmRead = { param($vm) $k.q += $step }.GetNewClosure()
    $s = AStart $c
    Expect ($s.outcome -eq 'REFUSED' -and (Has $s 'VM_STATE_STALE:P9S-D12') -and $c.Calls.Count -eq 0 -and (Entry $c $a).startups_used -eq 0) "start: $(Why $s)"
}
Case 'Z06' 'the VirtualBox lock held by another toolkit process: the observation times out and quarantines, nothing starts' {
    $c = New-VCtx; $a = New-Auth
    $mx = "Global\P9S-TEST-$run-z06"
    $vb = New-P9SVBoxContext -Dir $c.Dir -VmNames @('P9S-D12', 'P9S-U24') -MutexName $mx -ExePath $pw -PrefixArgs @('-NoProfile', '-NonInteractive', '-File', $mock) -LockWaitMs 1500 -ExecMs @{ 'vm.state' = 20000 } -ConfirmMs 2000
    $c | Add-Member -NotePropertyName VBox -NotePropertyValue $vb
    $c.ReadVmState = { param($vm) Get-P9SVmStateReal $vm $vb }.GetNewClosure()
    $o = AOpen $c $a
    Expect ($o.outcome -eq 'DONE') "open (lock free): $(Why $o)"
    $rd = New-Event "Global\P9S-TEST-$run-z06-ready"; $rl = New-Event "Global\P9S-TEST-$run-z06-release"
    $h = Start-Process -FilePath $pw -PassThru -WindowStyle Hidden -ArgumentList @('-NoProfile', '-NonInteractive', '-File', $holder, '-Name', $mx, '-Ready', "Global\P9S-TEST-$run-z06-ready", '-Release', "Global\P9S-TEST-$run-z06-release")
    Expect ($rd.WaitOne(30000)) 'holder did not take the lock'
    $s = AStart $c
    $null = $rl.Set(); $null = $h.WaitForExit(30000)
    Expect ($s.outcome -eq 'REFUSED' -and (Has $s 'VM_STATE_UNOBSERVED:P9S-D12') -and $c.Calls.Count -eq 0 -and (Entry $c $a).startups_used -eq 0) "start: $(Why $s)"
    $q = Read-P9SVBoxLatch $vb
    Expect ($q.present -and $q.doc.reason -eq 'VBOX_LOCK_TIMEOUT') 'no quarantine after the lock timeout'
    $s2 = AStart $c
    Expect ((Has $s2 'VBOX_API_QUARANTINED') -and $c.Calls.Count -eq 0) "afterwards: $(Why $s2)"
}
Case 'Z07' 'the authorization expires during the session: observation stops, the validation request is refused, the emergency path works' {
    $c = New-VCtx; $a = New-Auth -From $T0.AddMinutes(-10) -To $T0.AddSeconds(40)
    $null = AOpen $c $a; $s = AStart $c
    $b = AObs $c 60
    Expect ($s.outcome -eq 'DISPATCHED' -and $b.outcome -eq 'STOPPED_AT_CHECKPOINT' -and (Has $b 'AUTHORIZATION_EXPIRED')) "observe: $(Why $b)"
    $sh = AShut $c
    Expect ($sh.outcome -eq 'REFUSED' -and (Has $sh 'AUTHORIZATION_EXPIRED') -and ($sh.lines -join ' ') -match 'emergency\.acpi-shutdown') "shutdown: $(Why $sh)"
    $c.Prov.offAfter = 0
    $em = Emerg $c
    Expect ($em.outcome -eq 'DONE' -and (Entry $c $a).acpi_used -eq 1 -and (Count $c 'vm.acpi') -eq 1) "emergency: $(Why $em)"
}
Case 'Z08' 'two processes race for the one start: exactly one dispatch' {
    $c = New-VCtx -RealClock; $a = New-Auth -Real
    $o = AOpen $c $a
    Expect ($o.outcome -eq 'DONE') "open: $(Why $o)"
    $go = New-Event "Global\P9S-TEST-$run-z08-go"; $r1 = New-Event "Global\P9S-TEST-$run-z08-r1"; $r2 = New-Event "Global\P9S-TEST-$run-z08-r2"
    $vms = @{ 'P9S-D12' = 'off'; 'P9S-U24' = 'off' }
    $w1 = Start-AWorker $c @{ Action = 'acpi.start' } $vms -DispatchMs 800 -Go "Global\P9S-TEST-$run-z08-go" -Ready "Global\P9S-TEST-$run-z08-r1"
    $w2 = Start-AWorker $c @{ Action = 'acpi.start' } $vms -DispatchMs 800 -Go "Global\P9S-TEST-$run-z08-go" -Ready "Global\P9S-TEST-$run-z08-r2"
    Expect ($r1.WaitOne(60000) -and $r2.WaitOne(60000)) 'workers not ready'
    $null = $go.Set()
    $x1 = Wait-AWorker $w1; $x2 = Wait-AWorker $w2
    $outs = @($x1, $x2)
    Expect (@(DispatchLog $c | Where-Object { $_ -like 'vm.start *' }).Count -eq 1) "dispatches: $(@(DispatchLog $c) -join '; ')"
    Expect (@($outs | Where-Object { $_.outcome -eq 'DISPATCHED' }).Count -eq 1 -and @($outs | Where-Object { $_.outcome -eq 'REFUSED' -and @($_.reasons) -contains 'ACPI_STARTUP_BUDGET_EXHAUSTED' }).Count -eq 1) "outcomes: $(@($outs | ForEach-Object { "$($_.outcome) $(@($_.reasons) -join ',')" }) -join ' / ')"
    Expect ((Entry $c $a).startups_used -eq 1) 'start budget'
}
Case 'Z09' 'two processes race for the one ACPI request: exactly one dispatch' {
    $c = New-VCtx -RealClock; $a = New-Auth -Real
    $null = AOpen $c $a; $s = AStart $c
    Expect ($s.outcome -eq 'DISPATCHED') "start: $(Why $s)"
    $go = New-Event "Global\P9S-TEST-$run-z09-go"; $r1 = New-Event "Global\P9S-TEST-$run-z09-r1"; $r2 = New-Event "Global\P9S-TEST-$run-z09-r2"
    $vms = @{ 'P9S-D12' = 'running'; 'P9S-U24' = 'off' }
    $w1 = Start-AWorker $c @{ Action = 'acpi.shutdown'; Seconds = 10 } $vms -DispatchMs 500 -Go "Global\P9S-TEST-$run-z09-go" -Ready "Global\P9S-TEST-$run-z09-r1"
    $w2 = Start-AWorker $c @{ Action = 'acpi.shutdown'; Seconds = 10 } $vms -DispatchMs 500 -Go "Global\P9S-TEST-$run-z09-go" -Ready "Global\P9S-TEST-$run-z09-r2"
    Expect ($r1.WaitOne(60000) -and $r2.WaitOne(60000)) 'workers not ready'
    $null = $go.Set()
    $x1 = Wait-AWorker $w1; $x2 = Wait-AWorker $w2
    $outs = @($x1, $x2)
    Expect (@(DispatchLog $c | Where-Object { $_ -like 'vm.acpi *' }).Count -eq 1) "dispatches: $(@(DispatchLog $c) -join '; ')"
    Expect (@($outs | Where-Object { $_.dispatched -eq 1 }).Count -eq 1 -and @($outs | Where-Object { $_.outcome -eq 'REFUSED' -and @($_.reasons) -contains 'ACPI_BUDGET_EXHAUSTED' }).Count -eq 1) "outcomes: $(@($outs | ForEach-Object { "$($_.outcome) $(@($_.reasons) -join ',')" }) -join ' / ')"
    Expect ((Entry $c $a).acpi_used -eq 1) 'ACPI budget'
}

Case 'Z10' 'an emergency that waited for the lock while the validation sent its request sends none' {
    $c = New-VCtx -RealClock; $a = New-Auth -Real
    $null = AOpen $c $a; $s = AStart $c
    Expect ($s.outcome -eq 'DISPATCHED') "start: $(Why $s)"
    $lock = [IO.FileStream]::new($c.LockPath, [IO.FileMode]::OpenOrCreate, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
    try {
        $w = Start-AWorker $c @{ Action = 'emergency.acpi-shutdown'; Vm = 'P9S-D12' } @{ 'P9S-D12' = 'running'; 'P9S-U24' = 'off' }
        $sw = [Diagnostics.Stopwatch]::StartNew()
        while (-not (Test-Path -LiteralPath $c.StopRequestPath) -and $sw.Elapsed.TotalSeconds -lt 60) { Start-Sleep -Milliseconds 50 }
        Expect (Test-Path -LiteralPath $c.StopRequestPath) 'the emergency did not get past its first check'
        # meanwhile the validation spends its one request (what acpi.shutdown records before its dispatch)
        $l = Ledger $c; $e = Get-P9SAcpiEntry $l (AuthId $a)
        $e.acpi_used = 1; $e.acpi_source = 'validation'; $e.phase = 'SHUTDOWN_REQUESTED'
        Expect (Save-P9SAcpiLedger $c $l) 'ledger not written'
    } finally { $lock.Dispose() }
    $r = Wait-AWorker $w
    Expect ($r.outcome -eq 'REFUSED' -and @($r.reasons) -contains 'ACPI_ALREADY_REQUESTED') "emergency: $($r.outcome) $(@($r.reasons) -join ',')"
    Expect (@(DispatchLog $c | Where-Object { $_ -like 'vm.acpi *' }).Count -eq 0) "dispatches: $(@(DispatchLog $c) -join '; ')"
}
foreach ($e in $script:events) { try { $null = $e.Set() } catch {} }
$pass = @($results | Where-Object result -eq 'PASS').Count
$fail = @($results | Where-Object result -eq 'FAIL').Count
$notRun = @($results | Where-Object result -ne 'PASS' | Where-Object result -ne 'FAIL').Count
"== $pass passed, $fail failed, $notRun not run"
$results | ConvertTo-Json -Depth 3 | Set-Content -LiteralPath (Join-Path $OutDir 'results.json') -Encoding utf8
$results | ForEach-Object { '{0} {1} {2} ({3} ms){4}' -f $_.result, $_.id, $_.name, $_.ms, $(if ($_.detail) { ' :: ' + $_.detail } else { '' }) } | Set-Content -LiteralPath (Join-Path $OutDir 'results.txt') -Encoding utf8
if ($fail) { exit 1 } elseif ($notRun) { exit 3 } else { exit 0 }
