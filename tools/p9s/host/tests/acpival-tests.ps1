#Requires -Version 7.0
# Synthetic tests of the restricted single-VM ACPI validation
# (p9s-acpival.ps1): its live switch, the authorization file, the monitor
# conditions, the one-start and one-request budgets, recovery when the
# guest does not power off, races between processes, damaged, missing
# and outdated ledgers and stop requests (an unknown request history is
# never an unused budget), and low memory (no new workload, observation
# and recovery stay available). Damage is done to synthetic files in the
# test folder, never to a real ledger. The coordinator
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
$MEM9 = M (Feed (NewMon 'run-A') 0 300 50 9000)
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
# -Version 0: 1 for P9S-D12 (the format of the Debian session), 2 for any
# other target; version 2 ids carry the target code and the memory
# minimum is named in MiB.
function New-Auth([hashtable]$Set = @{}, [string[]]$Remove = @(), $From = $null, $To = $null, [switch]$Real, [string]$Target = 'P9S-D12', [int]$Version = 0) {
    $script:authN++
    $base = if ($Real) { [DateTime]::UtcNow } else { $T0 }
    if ($null -eq $From) { $From = $base.AddMinutes(-10) }; if ($null -eq $To) { $To = $base.AddHours(2) }
    if (-not $Version) { $Version = $(if ($Target -ceq 'P9S-D12') { 1 } else { 2 }) }
    $code = if ($Target -ceq 'P9S-U24') { 'u24' } else { 'd12' }
    $id = if ($Version -eq 1) { 'acpiauth-20261120-{0:x8}' -f (0x10000 + $script:authN) } else { 'acpiauth-{0}-20261120-{1:x8}' -f $code, (0x10000 + $script:authN) }
    $mem = if ($Version -eq 1) { 'provisional_min_available_mb' } else { 'provisional_min_available_mib' }
    $mc = [ordered]@{ mode = 'Monitor'; paging_stop = 'Required'; commit_gate_s = 300; commit_below_pct = 90 }; $mc[$mem] = 8192
    $h = [ordered]@{
        schema = "p9s-acpi-validation-authorization/$Version"; authorization_id = $id; purpose = 'ACPI_VALIDATION'; target_vm = $Target
        permitted_actions = @('acpi.open', 'acpi.start', 'acpi.observe', 'acpi.shutdown', 'acpi.integrity'); max_startups = 1; max_acpi_requests = 1
        valid_from_utc = (Z $From); expires_utc = (Z $To)
        monitor_conditions = $mc
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
        acpiVm = $null; config = @('name="P9S-D12"', 'memory=4096', 'acpi="on"', 'VMState="poweroff"'); configU = @('name="P9S-U24"', 'memory=4096', 'cpus=4', 'acpi="on"', 'VMState="poweroff"') }
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
        if ($prov.acpiSent -and $vm -ceq $prov.acpiVm) { $prov.acpiReads++; if ($prov.offAfter -ge 0 -and $prov.acpiReads -gt $prov.offAfter) { $vmTable[$vm] = 'off' } }
        if ($prov.onVmRead) { & $prov.onVmRead $vm }
        $vmTable[$vm]
    }.GetNewClosure()
    $start = { param($vm) $calls.Add([pscustomobject]@{ kind = 'vm.start'; vm = $vm }); if ($prov.onStart) { & $prov.onStart }; if ($prov.startThrows) { throw 'mock start failure' }; if ($prov.startSets) { $vmTable[$vm] = $prov.startSets } }.GetNewClosure()
    $acpi = { param($vm) $calls.Add([pscustomobject]@{ kind = 'vm.acpi'; vm = $vm }); if ($prov.acpiThrows) { throw 'mock ACPI failure' }; $prov.acpiSent = $true; $prov.acpiVm = $vm; $prov.acpiReads = 0; if ($prov.onAcpi) { & $prov.onAcpi } }.GetNewClosure()
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
    $refPath = Join-Path $Dir 'P9S-D12-reference.txt'; $refPathU = Join-Path $Dir 'P9S-U24-reference.txt'
    if (-not (Test-Path -LiteralPath $refPath)) { $prov.config | Set-Content -LiteralPath $refPath -Encoding ascii }
    if (-not (Test-Path -LiteralPath $refPathU)) { $prov.configU | Set-Content -LiteralPath $refPathU -Encoding ascii }
    [pscustomobject]@{
        StatePath = Join-Path $Dir 'state.json'; SessionPath = Join-Path $Dir 'coordinator-session.json'; LockPath = Join-Path $Dir 'coordinator.lock'
        StopRequestPath = Join-Path $Dir 'coordinator-stop.request'; AuditPath = Join-Path $Dir 'coordinator-audit.jsonl'; ThresholdsPath = Join-Path $kit 'paging-thresholds.json'
        ReadState = $readState; NowQpc = $nowq; QpcFrequency = $freq; IsMonitorAlive = { param($a, $b) $prov.alive }.GetNewClosure()
        ReadVmState = $readVm; ReadVmConfig = { param($vm) if ($vm -ceq 'P9S-U24') { @($prov.configU) } else { @($prov.config) } }.GetNewClosure()
        Dispatch = @{ 'vm.start' = $start; 'vm.acpi' = $acpi; 'guest.text' = $text; 'guest.key' = $key; 'guest.unlock' = $unlock }
        Sleep = $sleep; Pid = $PID; ProcessStartTicks = (Get-Process -Id $PID).StartTime.ToUniversalTime().Ticks
        EmergencyLockWaitMs = 1500; CalibrationHostChecks = { @() }
        ObservationPath = Join-Path $Dir 'vm-observations.jsonl'; LedgerPath = Join-Path $Dir 'acpi-validation-ledger.json'; WriteLedger = $wl; UtcNow = $utc; AcpiValidationLive = $Live
        CountVmProcesses = { [int]$prov.vmprocs }.GetNewClosure()
        Config = [pscustomobject]@{ vm_config_reference = [pscustomobject]@{ debian = $refPath; ubuntu = $refPathU } }
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
function Emerg($c, [string]$Vm = 'P9S-D12') { Act $c @{ Action = 'emergency.acpi-shutdown'; Vm = $Vm } }
function Ledger($c) { $r = Read-P9SAcpiLedger $c; if ($r.ok) { $r.doc } else { $null } }
function Entry($c, [string]$Auth) { $l = Ledger $c; if ($l) { Get-P9SAcpiEntry $l (AuthId $Auth) } else { $null } }
function Sess($c) { (Read-P9SSession $c).session }
function Kinds($c) { (@($c.Calls | ForEach-Object { "$($_.kind) $($_.vm)" })) -join ', ' }
function Count($c, [string]$kind) { @($c.Calls | Where-Object { $_.kind -eq $kind }).Count }
# open and start on a fresh mock context; the VM then runs
function Started([hashtable]$Vms = @{}, $Mon = $ELIG, [string]$Target = 'P9S-D12') {
    $c = New-VCtx -Mon $Mon -Vms $Vms; $a = New-Auth -Target $Target
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
# a recording "VBoxManage" for the VirtualBox layer: every call it gets is
# appended to the log named by its first argument (a tripwire)
$recvbox = Join-Path $OutDir 'recvbox.ps1'
@'
Add-Content -LiteralPath $args[0] -Value (($args | Select-Object -Skip 1) -join ' ')
'name="' + $args[2] + '"'
'VMState="poweroff"'
exit 0
'@ | Set-Content -LiteralPath $recvbox -Encoding ascii
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
    Expect ($ok.ok -and $ok.auth.target_vm -eq 'P9S-D12' -and $ok.auth.min_available_mib -eq 8192 -and $ok.auth.max_startups -eq 1 -and $ok.auth.max_acpi_requests -eq 1) "valid fixture refused: $($ok.reasons -join ',')"
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
        @(@{ schema = 'p9s-acpi-validation-authorization/3' }, 'AUTHORIZATION_SCHEMA_UNSUPPORTED'),
        @(@{ schema = 'p9s-acpi-validation-authorization/2' }, 'AUTHORIZATION_MEMORY_UNIT'), @(@{ schema = 'p9s-acpi-validation-authorization/2' }, 'AUTHORIZATION_ID_INVALID'))
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
# ---- two targets: P9S-U24 next to P9S-D12, each session bound to its own -------------------------
Case 'U01' 'each authorization drives only its own target: a Debian session never touches P9S-U24, an Ubuntu session never touches P9S-D12' {
    foreach ($pair in @(@('P9S-D12', 'P9S-U24'), @('P9S-U24', 'P9S-D12'))) {
        $tv = $pair[0]; $other = $pair[1]
        $c = New-VCtx; $c.Prov.offAfter = 1
        $a = New-Auth -Target $tv
        $steps = @((Act $c @{ Action = 'acpi.preflight'; Authorization = $a }), (AOpen $c $a), (AStart $c), (AObs $c 10), (AShut $c), (Act $c @{ Action = 'acpi.integrity' }), (AFinal $c))
        Expect (@($steps | Where-Object { $_.outcome -notin 'DONE', 'DISPATCHED' -or $_.reasons.Count }).Count -eq 0) "$tv flow: $(@($steps | ForEach-Object { Why $_ }) -join ' / ')"
        Expect ((Kinds $c) -eq "vm.start $tv, vm.acpi $tv" -and $c.VmTable[$other] -eq 'off') "$tv calls: $(Kinds $c)"
        $j = @(Read-P9SVmObservations $c.ObservationPath)
        Expect ($j.Count -gt 0 -and @($j | Where-Object { $_.vm -ne $tv }).Count -eq 0) "$tv observed another VM"
        $e = Entry $c $a
        Expect ($e.target_vm -eq $tv -and $e.outcome -eq 'VALIDATED' -and $e.startups_used -eq 1 -and $e.acpi_used -eq 1 -and $e.integrity.differing -eq 0) "$tv ledger: $($e.target_vm) $($e.outcome)"
        $aud = @(Get-Content -LiteralPath $c.AuditPath | ForEach-Object { $_ | ConvertFrom-Json } | Where-Object { $_.action -like 'acpi.*' })
        Expect (@($aud | Where-Object { $_.vm -ne $tv }).Count -eq 0) "$tv audit names another VM: $(@($aud | ForEach-Object { "$($_.action)=$($_.vm)" }) -join ' ')"
    }
}
Case 'U02' 'a target is one exact name of the table for its role: several, empty, unknown, other-project, wrong-case, wrong-schema, misbound, duplicated and extra fields are refused' {
    $kd = Split-Path -Parent (Join-Path $kit 'paging-thresholds.json')
    $ok = Read-P9SAcpiAuthorization -Path (New-Auth -Target 'P9S-U24') -KitDir $kd
    Expect ($ok.ok -and $ok.auth.target_vm -eq 'P9S-U24' -and $ok.auth.schema -eq 'p9s-acpi-validation-authorization/2' -and $ok.auth.min_available_mib -eq 8192) "valid Ubuntu fixture refused: $($ok.reasons -join ',')"
    $ok2 = Read-P9SAcpiAuthorization -Path (New-Auth -Target 'P9S-D12' -Version 2) -KitDir $kd
    Expect ($ok2.ok -and $ok2.auth.target_vm -eq 'P9S-D12') "valid version-2 Debian fixture refused: $($ok2.reasons -join ',')"
    $cases = @(
        @(@{ target_vm = @('P9S-D12', 'P9S-U24') }, 'AUTHORIZATION_TARGET_NOT_ALLOWED'), @(@{ target_vm = @('P9S-U24') }, 'AUTHORIZATION_TARGET_NOT_ALLOWED'),
        @(@{ target_vm = '' }, 'AUTHORIZATION_TARGET_NOT_ALLOWED'), @(@{ target_vm = $null }, 'AUTHORIZATION_TARGET_NOT_ALLOWED'),
        @(@{ target_vm = 'another-project-vm' }, 'AUTHORIZATION_TARGET_NOT_ALLOWED'), @(@{ target_vm = 'p9s-u24' }, 'AUTHORIZATION_TARGET_NOT_ALLOWED'),
        @(@{ target_vm = ' P9S-U24' }, 'AUTHORIZATION_TARGET_NOT_ALLOWED'), @(@{ target_vm = 'P9S-U24 ' }, 'AUTHORIZATION_TARGET_NOT_ALLOWED'),
        @(@{ target_vm = 'P9S-D12' }, 'AUTHORIZATION_ID_TARGET_MISMATCH'),
        @(@{ target_vms = @('P9S-D12') }, 'AUTHORIZATION_UNKNOWN_FIELD:target_vms'),
        @(@{ authorization_id = 'acpiauth-20261120-0000aaaa' }, 'AUTHORIZATION_ID_INVALID'))
    foreach ($x in $cases) {
        $r = Read-P9SAcpiAuthorization -Path (New-Auth -Target 'P9S-U24' -Set $x[0]) -KitDir $kd
        Expect (-not $r.ok -and ($r.reasons -contains $x[1])) "$(($x[0].Keys) -join ',')=$(@($x[0].Values) -join '|') -> $($r.reasons -join ',') (expected $($x[1]))"
    }
    $v1 = Read-P9SAcpiAuthorization -Path (New-Auth -Target 'P9S-U24' -Version 1) -KitDir $kd
    Expect (-not $v1.ok -and ($v1.reasons -contains 'AUTHORIZATION_TARGET_NOT_ALLOWED')) "version 1 naming Ubuntu: $($v1.reasons -join ',')"
    $dup = Join-Path $authDir 'duplicate-target.json'
    (Get-Content -Raw -LiteralPath (New-Auth -Target 'P9S-D12' -Version 2)) -replace '"target_vm": "P9S-D12"', '"target_vm": "P9S-D12", "target_vm": "P9S-U24"' | Set-Content -LiteralPath $dup -Encoding utf8
    $rd = Read-P9SAcpiAuthorization -Path $dup -KitDir $kd
    Expect (-not $rd.ok -and ($rd.reasons -contains 'AUTHORIZATION_DUPLICATE_FIELD:target_vm')) "two target_vm fields: $($rd.reasons -join ',')"
    $cs = Join-Path $authDir 'case-variant-target.json'
    (Get-Content -Raw -LiteralPath (New-Auth -Target 'P9S-U24')) -replace '"target_vm": "P9S-U24"', '"target_vm": "P9S-U24", "Target_VM": "P9S-D12"' | Set-Content -LiteralPath $cs -Encoding utf8
    $rc2 = Read-P9SAcpiAuthorization -Path $cs -KitDir $kd
    Expect (-not $rc2.ok -and ($rc2.reasons -contains 'AUTHORIZATION_UNKNOWN_FIELD:Target_VM')) "a second target in another case: $($rc2.reasons -join ',')"
    Set-P9SVmNames @('P9S-D12', 'another-project-vm')
    try {
        $rn = Read-P9SAcpiAuthorization -Path (New-Auth -Target 'P9S-U24') -KitDir $kd
        Expect (-not $rn.ok -and ($rn.reasons -contains 'AUTHORIZATION_TARGET_NOT_ALLOWED')) "Ubuntu role configured as another VM: $($rn.reasons -join ',')"
        Expect ($null -eq (Get-P9SAcpiValidationTarget 'another-project-vm') -and $null -eq (Get-P9SAcpiValidationTarget 'P9S-U24') -and $null -ne (Get-P9SAcpiValidationTarget 'P9S-D12')) 'target table follows a foreign configuration'
    } finally { Set-P9SVmNames @(Get-P9SDefaultVmNames) }
    $c = New-VCtx
    $o = AOpen $c (New-Auth -Target 'P9S-U24' -Set @{ target_vm = @('P9S-D12', 'P9S-U24') })
    Expect ($o.outcome -eq 'REFUSED' -and $c.Prov.vmreads -eq 0 -and -not (Test-Path -LiteralPath $c.SessionPath)) "open with two targets: $(Why $o)"
}
Case 'U03' 'the target cannot change after the open: a session or ledger edited to another target or authorization is refused before any VirtualBox read' {
    $c = New-VCtx; $a = New-Auth -Target 'P9S-D12'
    $o = AOpen $c $a
    Expect ($o.outcome -eq 'DONE') "open: $(Why $o)"
    $orig = Get-Content -Raw -LiteralPath $c.SessionPath
    $sr = Read-P9SSession $c; $sr.session.validation.target_vm = 'P9S-U24'; Save-P9SSession $c $sr.session
    $r0 = $c.Prov.vmreads
    foreach ($x in @((AStart $c), (AObs $c 5), (AShut $c), (Act $c @{ Action = 'acpi.integrity' }))) { Expect ($x.outcome -eq 'REFUSED' -and (Has $x 'ACPI_TARGET_MISMATCH')) "session edited to P9S-U24: $(Why $x)" }
    Expect ($c.Prov.vmreads -eq $r0 -and $c.Calls.Count -eq 0) "read or dispatched after the session edit: reads $($c.Prov.vmreads - $r0), $(Kinds $c)"
    $sr = Read-P9SSession $c; $sr.session.validation.target_vm = 'another-project-vm'; Save-P9SSession $c $sr.session
    $y = AStart $c
    Expect ($y.outcome -eq 'REFUSED' -and (Has $y 'ACPI_TARGET_NOT_ALLOWED') -and $c.Calls.Count -eq 0) "session edited to another project's VM: $(Why $y)"
    Set-Content -LiteralPath $c.SessionPath -Value $orig -NoNewline
    $l = Ledger $c; $e = Get-P9SAcpiEntry $l (AuthId $a); $e.target_vm = 'P9S-U24'; $null = Save-P9SAcpiLedger $c $l
    $z = AStart $c
    Expect ($z.outcome -eq 'REFUSED' -and (Has $z 'ACPI_TARGET_MISMATCH') -and $c.Prov.vmreads -eq $r0 -and $c.Calls.Count -eq 0) "ledger edited to P9S-U24: $(Why $z)"
    $l = Ledger $c; $e = Get-P9SAcpiEntry $l (AuthId $a); $e.target_vm = 'P9S-D12'; $e.authorization_sha256 = ('0' * 64); $null = Save-P9SAcpiLedger $c $l
    $w = AStart $c
    Expect ($w.outcome -eq 'REFUSED' -and (Has $w 'ACPI_AUTHORIZATION_MISMATCH') -and $c.Calls.Count -eq 0) "ledger bound to another file: $(Why $w)"
    $l = Ledger $c; $e = Get-P9SAcpiEntry $l (AuthId $a); $e.authorization_sha256 = (Get-FileSha256 $a); $null = Save-P9SAcpiLedger $c $l
    (Get-Content -Raw -LiteralPath $a) -replace '"target_vm": "P9S-D12"', '"target_vm": "P9S-U24"' | Set-Content -LiteralPath $a -Encoding utf8
    $s = AStart $c
    Expect ($s.outcome -eq 'DISPATCHED' -and (Kinds $c) -eq 'vm.start P9S-D12') "after the authorization file was edited: $(Why $s) $(Kinds $c)"
}
Case 'U04' 'a used authorization stays used: the Debian permit cannot open again, relabelled for Ubuntu or moved to version 2' {
    $c = New-VCtx; $c.Prov.offAfter = 1; $a = New-Auth -Target 'P9S-D12'
    $steps = @((AOpen $c $a), (AStart $c), (AShut $c), (AFinal $c))
    Expect ((Entry $c $a).outcome -eq 'VALIDATED') "Debian validation: $(@($steps | ForEach-Object { Why $_ }) -join ' / ')"
    $again = AOpen $c $a
    Expect ($again.outcome -eq 'REFUSED' -and (Has $again 'AUTHORIZATION_ALREADY_USED')) "same permit again: $(Why $again)"
    $h = Get-Content -Raw -LiteralPath $a | ConvertFrom-Json -AsHashtable
    $h.target_vm = 'P9S-U24'; $p1 = Join-Path $authDir 'debian-relabelled.json'; $h | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $p1 -Encoding utf8
    $h.schema = 'p9s-acpi-validation-authorization/2'; $h.monitor_conditions.Remove('provisional_min_available_mb'); $h.monitor_conditions['provisional_min_available_mib'] = 8192
    $p2 = Join-Path $authDir 'debian-relabelled-v2.json'; $h | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $p2 -Encoding utf8
    $d2 = New-Auth -Target 'P9S-D12' -Version 2; $p3 = Join-Path $authDir 'debian-v2-moved.json'
    (Get-Content -Raw -LiteralPath $d2) -replace '"target_vm": "P9S-D12"', '"target_vm": "P9S-U24"' | Set-Content -LiteralPath $p3 -Encoding utf8
    foreach ($x in @(@($p1, 'AUTHORIZATION_TARGET_NOT_ALLOWED'), @($p2, 'AUTHORIZATION_ID_INVALID'), @($p3, 'AUTHORIZATION_ID_TARGET_MISMATCH'))) {
        $r = AOpen $c $x[0]
        Expect ($r.outcome -eq 'REFUSED' -and (Has $r $x[1])) "$([IO.Path]::GetFileName($x[0])): $(Why $r) (expected $($x[1]))"
    }
    Expect (@((Ledger $c).entries).Count -eq 1 -and (Kinds $c) -eq 'vm.start P9S-D12, vm.acpi P9S-D12' -and $c.VmTable['P9S-U24'] -eq 'off') "afterwards: $(@((Ledger $c).entries).Count) entries, $(Kinds $c)"
}
Case 'U05' 'an Ubuntu authorization outside its window is refused at open and before the start, before any VirtualBox read' {
    $c = New-VCtx
    $x = AOpen $c (New-Auth -Target 'P9S-U24' -From $T0.AddMinutes(5) -To $T0.AddHours(1))
    Expect ($x.outcome -eq 'REFUSED' -and (Has $x 'AUTHORIZATION_NOT_YET_VALID') -and $c.Prov.vmreads -eq 0) "not yet valid: $(Why $x)"
    $y = AOpen $c (New-Auth -Target 'P9S-U24' -From $T0.AddHours(-2) -To $T0.AddMinutes(-1))
    Expect ($y.outcome -eq 'REFUSED' -and (Has $y 'AUTHORIZATION_EXPIRED') -and $c.Prov.vmreads -eq 0 -and $c.Prov.reads -eq 0) "expired: $(Why $y)"
    $a = New-Auth -Target 'P9S-U24' -From $T0.AddMinutes(-10) -To $T0.AddMinutes(20)
    $o = AOpen $c $a
    Expect ($o.outcome -eq 'DONE') "open: $(Why $o)"
    $c.Clock.utc = $T0.AddMinutes(21); $r0 = $c.Prov.vmreads
    $s = AStart $c
    Expect ($s.outcome -eq 'REFUSED' -and (@($s.reasons) -join ',') -eq 'AUTHORIZATION_EXPIRED' -and $c.Calls.Count -eq 0 -and $c.Prov.vmreads -eq $r0 -and (Entry $c $a).startups_used -eq 0) "start after expiry: $(Why $s)"
}
Case 'U06' 'no valid Ubuntu authorization: no coordinator lock, no VirtualBox lock, no VM read, no dispatch, no budget; no toolkit file is a permit' {
    $d = Join-Path $OutDir 'realctx-u24'
    $rc = New-P9SRealContext -StatePath (Join-Path $d 'state.json') -KitDir $kit -Config $null
    $log = Join-Path $d 'vboxmanage-calls.log'
    $vb = New-P9SVBoxContext -Dir $d -VmNames @('P9S-D12', 'P9S-U24') -MutexName "Global\P9S-TEST-$run-u06" -ExePath $pw -PrefixArgs @('-NoProfile', '-NonInteractive', '-File', $recvbox, $log) -LockWaitMs 1500
    $rc.ReadVmState = { param($vm) Get-P9SVmStateReal $vm $vb }.GetNewClosure()
    $rc.ReadVmConfig = { param($vm) Get-P9SVmConfigReal $vm $vb }.GetNewClosure()
    $rc.Dispatch = @{ 'vm.start' = { param($vm) Assert-P9SVBoxCompleted (Invoke-P9SVBoxOperation -VBox $vb -Operation 'vm.start' -Vm $vm) }.GetNewClosure(); 'vm.acpi' = { param($vm) Assert-P9SVBoxCompleted (Invoke-P9SVBoxOperation -VBox $vb -Operation 'vm.acpi' -Vm $vm) }.GetNewClosure() }
    $rc.CountVmProcesses = { 0 }
    # no installation of the persistent state: the tests never read the real anchor
    $rc.Installation = { [pscustomobject]@{ known = $true; installed = $false; installation_id = $null; initialized_utc = $null; reason = $null } }
    $ux = Join-Path $kit 'acpi-validation-authorization-u24.example.json'
    $uxOut = Join-Path $authDir 'u24-example-copy.json'; Copy-Item -LiteralPath $ux -Destination $uxOut
    $old = New-Auth -Real -Target 'P9S-U24' -From ([DateTime]::UtcNow.AddHours(-3)) -To ([DateTime]::UtcNow.AddMinutes(-5))
    $early = New-Auth -Real -Target 'P9S-U24' -From ([DateTime]::UtcNow.AddMinutes(30)) -To ([DateTime]::UtcNow.AddHours(2))
    $deb = New-Auth -Real -Target 'P9S-D12'; $debU = Join-Path $authDir 'debian-for-ubuntu.json'
    (Get-Content -Raw -LiteralPath $deb) -replace '"target_vm": "P9S-D12"', '"target_vm": "P9S-U24"' | Set-Content -LiteralPath $debU -Encoding utf8
    $cases = @(
        @(@{ Action = 'acpi.open'; Authorization = $ux }, 'AUTHORIZATION_IN_TOOLKIT'), @(@{ Action = 'acpi.open'; Authorization = $uxOut }, 'AUTHORIZATION_SAFEGUARD_NOT_APPROVED'),
        @(@{ Action = 'acpi.open'; Authorization = $uxOut; DryRun = $true }, 'AUTHORIZATION_ID_INVALID'), @(@{ Action = 'acpi.open'; Authorization = $old }, 'AUTHORIZATION_EXPIRED'),
        @(@{ Action = 'acpi.open'; Authorization = $early }, 'AUTHORIZATION_NOT_YET_VALID'), @(@{ Action = 'acpi.open'; Authorization = $debU }, 'AUTHORIZATION_TARGET_NOT_ALLOWED'),
        @(@{ Action = 'acpi.start' }, 'ACPI_VALIDATION_SESSION_MISSING'), @(@{ Action = 'acpi.start'; DryRun = $true }, 'ACPI_VALIDATION_SESSION_MISSING'),
        @(@{ Action = 'acpi.observe'; Seconds = 5 }, 'ACPI_VALIDATION_SESSION_MISSING'), @(@{ Action = 'acpi.shutdown' }, 'ACPI_VALIDATION_SESSION_MISSING'),
        @(@{ Action = 'acpi.integrity' }, 'ACPI_VALIDATION_SESSION_MISSING'))
    # the coordinator lock is held throughout: an action that went past the
    # authorization check would report COORDINATOR_BUSY
    $lk = [IO.FileStream]::new($rc.LockPath, [IO.FileMode]::OpenOrCreate, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
    try {
        foreach ($x in $cases) {
            $r = Act $rc $x[0]
            Expect ($r.outcome -eq 'REFUSED' -and (Has $r $x[1]) -and -not (Has $r 'COORDINATOR_BUSY')) "$($x[0].Action)$(if ($x[0].DryRun) { ' -DryRun' }): $(Why $r) (expected $($x[1]))"
        }
        foreach ($p in @($null, $uxOut, $old)) {
            $r = Act $rc $(if ($p) { @{ Action = 'acpi.preflight'; Authorization = $p } } else { @{ Action = 'acpi.preflight' } })
            Expect ($r.outcome -eq 'DONE' -and $r.reasons.Count -gt 0 -and ($r.lines -join ' ') -notmatch 'target: P9S') "preflight $(if ($p) { [IO.Path]::GetFileName($p) } else { 'without a file' }): $(Why $r)"
        }
    } finally { $lk.Dispose() }
    # recovery actions take the coordinator lock; with nothing to recover they read nothing either
    foreach ($x in @(@(@{ Action = 'acpi.finalize' }, 'ACPI_VALIDATION_SESSION_MISSING'), @(@{ Action = 'acpi.confirm-off'; Seconds = 5 }, 'ACPI_NOTHING_TO_CONFIRM'))) {
        $r = Act $rc $x[0]
        Expect ($r.outcome -eq 'REFUSED' -and (Has $r $x[1])) "$($x[0].Action): $(Why $r) (expected $($x[1]))"
    }
    Expect (-not (Test-Path -LiteralPath $log)) "VBoxManage was called: $(if (Test-Path -LiteralPath $log) { (Get-Content -LiteralPath $log) -join '; ' })"
    foreach ($n in 'vbox-audit.jsonl', 'vbox-operation.json', 'vbox-quarantine.json', 'acpi-validation-ledger.json', 'coordinator-session.json', 'vm-observations.jsonl') { Expect (-not (Test-Path -LiteralPath (Join-Path $d $n))) "$n written" }
    foreach ($j in Get-ChildItem -LiteralPath $kit -Recurse -Filter '*.json') {
        $r = Read-P9SAcpiAuthorization -Path $j.FullName -KitDir (Join-Path $OutDir 'not-the-kit')
        Expect (-not $r.ok) "$($j.Name) is a valid permit"
    }
    $re = Read-P9SAcpiAuthorization -Path $ux -KitDir (Join-Path $OutDir 'not-the-kit')
    foreach ($w in 'AUTHORIZATION_ID_INVALID', 'AUTHORIZATION_TIME_INVALID', 'AUTHORIZATION_MONITOR_CONDITIONS', 'AUTHORIZATION_SAFEGUARD_NOT_APPROVED', 'AUTHORIZATION_APPROVAL_INCOMPLETE') { Expect ($re.reasons -contains $w) "Ubuntu example: missing $w ($($re.reasons -join ','))" }
}
Case 'U07' 'a new Ubuntu session leaves the Debian ledger entry as it was and starts with fresh budgets' {
    $c = New-VCtx; $c.Prov.offAfter = 1
    $ad = New-Auth -Target 'P9S-D12'
    $null = AOpen $c $ad; $null = AStart $c; $null = AShut $c; $null = Act $c @{ Action = 'acpi.integrity' }; $null = AFinal $c
    $before = Entry $c $ad | ConvertTo-Json -Depth 8 -Compress
    Expect ((Entry $c $ad).outcome -eq 'VALIDATED') "Debian validation: $((Entry $c $ad).outcome)"
    $au = New-Auth -Target 'P9S-U24'
    $o = AOpen $c $au; $eu = Entry $c $au
    Expect ($o.outcome -eq 'DONE' -and $eu.startups_used -eq 0 -and $eu.acpi_used -eq 0 -and $eu.phase -eq 'OPEN' -and $eu.target_vm -eq 'P9S-U24') "Ubuntu open: $(Why $o) $($eu.startups_used)/$($eu.acpi_used) $($eu.phase)"
    $null = AStart $c; $null = AShut $c; $null = AFinal $c
    $after = Entry $c $ad | ConvertTo-Json -Depth 8 -Compress
    Expect ($before -ceq $after) 'the Debian entry changed'
    $l = Ledger $c
    Expect (@($l.entries).Count -eq 2 -and $l.entries[0].authorization_id -eq (AuthId $ad) -and $l.entries[1].authorization_id -eq (AuthId $au)) "entries: $(@($l.entries | ForEach-Object authorization_id) -join ',')"
    Expect ((Entry $c $au).outcome -eq 'VALIDATED' -and (Kinds $c) -eq 'vm.start P9S-D12, vm.acpi P9S-D12, vm.start P9S-U24, vm.acpi P9S-U24') "Ubuntu: $((Entry $c $au).outcome) $(Kinds $c)"
}
Case 'U08' 'one Ubuntu start: no second start, not after the VM is off again, not from a new process' {
    $c = New-VCtx; $c.Prov.offAfter = 0; $a = New-Auth -Target 'P9S-U24'
    $null = AOpen $c $a; $s1 = AStart $c; $sh = AShut $c
    Expect ($s1.outcome -eq 'DISPATCHED' -and $sh.outcome -eq 'DONE' -and $c.VmTable['P9S-U24'] -eq 'off') "start $(Why $s1), shutdown $(Why $sh)"
    $s2 = AStart $c
    Expect ($s2.outcome -eq 'REFUSED' -and (@($s2.reasons) -join ',') -eq 'ACPI_STARTUP_BUDGET_EXHAUSTED' -and (Count $c 'vm.start') -eq 1) "second start: $(Why $s2)"
    $c2 = New-VCtx -Dir $c.Dir
    $s3 = AStart $c2
    Expect ($s3.outcome -eq 'REFUSED' -and (Has $s3 'ACPI_STARTUP_BUDGET_EXHAUSTED') -and $c2.Calls.Count -eq 0 -and (Entry $c $a).startups_used -eq 1) "after a restart: $(Why $s3)"
}
Case 'U09' 'one Ubuntu ACPI request: neither the validation nor the emergency path sends a second one' {
    $x = Started -Target 'P9S-U24'; $c = $x.c
    $t1 = AShut $c 10
    Expect ($t1.outcome -eq 'TIMEOUT' -and (Has $t1 'ACPI_VALIDATION_UNRESOLVED')) "first request: $(Why $t1)"
    $t2 = AShut $c 10
    Expect ($t2.outcome -eq 'REFUSED' -and (@($t2.reasons) -join ',') -eq 'ACPI_BUDGET_EXHAUSTED') "second request: $(Why $t2)"
    $em = Emerg $c 'P9S-U24'
    Expect ($em.outcome -eq 'REFUSED' -and (Has $em 'ACPI_ALREADY_REQUESTED')) "emergency: $(Why $em)"
    Expect ((Count $c 'vm.acpi') -eq 1 -and (Kinds $c) -eq 'vm.start P9S-U24, vm.acpi P9S-U24' -and (Entry $c $x.a).acpi_used -eq 1) "requests: $(Kinds $c)"
}
Case 'U10' 'a crashed Ubuntu coordinator cannot give a budget back: after the start or the request a new process finds it spent' {
    $c = New-VCtx -RealClock; $a = New-Auth -Real -Target 'P9S-U24'
    $null = AOpen $c $a
    $w = Start-AWorker $c @{ Action = 'acpi.start' } @{ 'P9S-D12' = 'off'; 'P9S-U24' = 'off' } -Crash 'start'
    $null = Wait-AWorker $w
    $e = Entry $c $a
    Expect ($w.p.ExitCode -eq 9 -and $e.startups_used -eq 1 -and $e.phase -eq 'STARTING' -and @(DispatchLog $c | Where-Object { $_ -like 'vm.start P9S-U24 *' }).Count -eq 1) "after the start crash: exit $($w.p.ExitCode) $($e.startups_used) $($e.phase)"
    $c2 = New-VCtx -Dir $c.Dir -RealClock -Vms @{ 'P9S-U24' = 'running' }
    $s = AStart $c2
    Expect ((Has $s 'ACPI_STARTUP_BUDGET_EXHAUSTED') -and (Has $s 'INFLIGHT_UNRESOLVED') -and $c2.Calls.Count -eq 0) "after restart: $(Why $s)"
    $c3 = New-VCtx -RealClock; $a3 = New-Auth -Real -Target 'P9S-U24'
    $null = AOpen $c3 $a3; $null = AStart $c3
    $w3 = Start-AWorker $c3 @{ Action = 'acpi.shutdown' } @{ 'P9S-D12' = 'off'; 'P9S-U24' = 'running' } -Crash 'acpi'
    $null = Wait-AWorker $w3
    $c4 = New-VCtx -Dir $c3.Dir -RealClock -Vms @{ 'P9S-U24' = 'running' }
    $sh = AShut $c4; $em = Emerg $c4 'P9S-U24'
    Expect ($w3.p.ExitCode -eq 9 -and (Entry $c3 $a3).acpi_used -eq 1 -and (Has $sh 'ACPI_BUDGET_EXHAUSTED') -and (Has $em 'ACPI_ALREADY_REQUESTED') -and $c4.Calls.Count -eq 0) "after the request crash: $(Why $sh) / $(Why $em)"
}
Case 'U11' 'two processes race for the one Ubuntu start, and for the one Ubuntu request: one dispatch each' {
    $c = New-VCtx -RealClock; $a = New-Auth -Real -Target 'P9S-U24'
    $null = AOpen $c $a
    $go = New-Event "Global\P9S-TEST-$run-u11-go"; $r1 = New-Event "Global\P9S-TEST-$run-u11-r1"; $r2 = New-Event "Global\P9S-TEST-$run-u11-r2"
    $vms = @{ 'P9S-D12' = 'off'; 'P9S-U24' = 'off' }
    $w1 = Start-AWorker $c @{ Action = 'acpi.start' } $vms -DispatchMs 800 -Go "Global\P9S-TEST-$run-u11-go" -Ready "Global\P9S-TEST-$run-u11-r1"
    $w2 = Start-AWorker $c @{ Action = 'acpi.start' } $vms -DispatchMs 800 -Go "Global\P9S-TEST-$run-u11-go" -Ready "Global\P9S-TEST-$run-u11-r2"
    Expect ($r1.WaitOne(60000) -and $r2.WaitOne(60000)) 'start workers not ready'
    $null = $go.Set(); $x1 = Wait-AWorker $w1; $x2 = Wait-AWorker $w2
    Expect (@(DispatchLog $c | Where-Object { $_ -like 'vm.start P9S-U24 *' }).Count -eq 1 -and (Entry $c $a).startups_used -eq 1 -and @(@($x1, $x2) | Where-Object { $_.outcome -eq 'DISPATCHED' }).Count -eq 1) "starts: $(@(DispatchLog $c) -join '; ')"
    $go2 = New-Event "Global\P9S-TEST-$run-u11-go2"; $q1 = New-Event "Global\P9S-TEST-$run-u11-q1"; $q2 = New-Event "Global\P9S-TEST-$run-u11-q2"
    $vms2 = @{ 'P9S-D12' = 'off'; 'P9S-U24' = 'running' }
    $w3 = Start-AWorker $c @{ Action = 'acpi.shutdown'; Seconds = 10 } $vms2 -DispatchMs 500 -Go "Global\P9S-TEST-$run-u11-go2" -Ready "Global\P9S-TEST-$run-u11-q1"
    $w4 = Start-AWorker $c @{ Action = 'acpi.shutdown'; Seconds = 10 } $vms2 -DispatchMs 500 -Go "Global\P9S-TEST-$run-u11-go2" -Ready "Global\P9S-TEST-$run-u11-q2"
    Expect ($q1.WaitOne(60000) -and $q2.WaitOne(60000)) 'request workers not ready'
    $null = $go2.Set(); $x3 = Wait-AWorker $w3; $x4 = Wait-AWorker $w4
    Expect (@(DispatchLog $c | Where-Object { $_ -like 'vm.acpi P9S-U24 *' }).Count -eq 1 -and (Entry $c $a).acpi_used -eq 1 -and @(@($x3, $x4) | Where-Object { $_.outcome -eq 'REFUSED' -and @($_.reasons) -contains 'ACPI_BUDGET_EXHAUSTED' }).Count -eq 1) "requests: $(@(DispatchLog $c) -join '; ')"
    Expect (@(DispatchLog $c | Where-Object { $_ -like '* P9S-D12 *' }).Count -eq 0) 'P9S-D12 was touched'
}
Case 'U12' 'an unresolved Ubuntu shutdown blocks every further start: no new Ubuntu or Debian validation, no test session, until the VM is observed off' {
    $x = Started -Target 'P9S-U24'; $c = $x.c
    $t1 = AShut $c 10; $fin = AFinal $c
    Expect ($t1.outcome -eq 'TIMEOUT' -and (Entry $c $x.a).outcome -eq 'ACPI_VALIDATION_UNRESOLVED' -and (Sess $c).lifecycle -eq 'CLOSED') "unresolved: $(Why $t1) / $(Why $fin)"
    $nu = AOpen $c (New-Auth -Target 'P9S-U24'); $nd = AOpen $c (New-Auth -Target 'P9S-D12'); $ts = Act $c @{ Action = 'session.open'; Kind = 'Test' }
    Expect ((Has $nu 'ACPI_VALIDATION_UNRESOLVED_PREVIOUS') -and (Has $nd 'ACPI_VALIDATION_UNRESOLVED_PREVIOUS') -and (Has $ts 'ACPI_VALIDATION_UNRESOLVED') -and (Count $c 'vm.start') -eq 1) "new sessions: $(Why $nu) / $(Why $nd) / $(Why $ts)"
    $c.VmTable['P9S-U24'] = 'off'
    $k = AConfirm $c 10
    Expect ($k.outcome -eq 'DONE' -and (Test-P9SAcpiEntryResolved (Entry $c $x.a))) "confirmation: $(Why $k) $((Entry $c $x.a).phase)"
    $nd2 = AOpen $c (New-Auth -Target 'P9S-D12')
    Expect ($nd2.outcome -eq 'DONE') "Debian after the confirmation: $(Why $nd2)"
}
Case 'U13' 'STOP and ABORT stop an Ubuntu session: no start, no progression; the one emergency request is counted' {
    $c = New-VCtx; $a = New-Auth -Target 'P9S-U24'; $null = AOpen $c $a
    $c.Prov.mon = $STOP
    $s = AStart $c
    Expect ($s.outcome -eq 'REFUSED' -and (Has $s 'SESSION_HALTED_STOP') -and $c.Calls.Count -eq 0 -and (Entry $c $a).startups_used -eq 0) "start under STOP: $(Why $s)"
    $y = Started -Target 'P9S-U24'; $cy = $y.c; $cy.Prov.mon = $ABORT
    $b = AObs $cy 10; $sh = AShut $cy
    Expect ((Has $b 'SESSION_HALTED_ABORT') -and $sh.outcome -eq 'REFUSED' -and (Count $cy 'vm.acpi') -eq 0) "observe and request under ABORT: $(Why $b) / $(Why $sh)"
    $cy.Prov.offAfter = 0
    $em = Emerg $cy 'P9S-U24'
    Expect ($em.outcome -eq 'DONE' -and (Entry $cy $y.a).acpi_source -eq 'emergency' -and (Entry $cy $y.a).acpi_used -eq 1 -and (Kinds $cy) -eq 'vm.start P9S-U24, vm.acpi P9S-U24') "emergency: $(Why $em) $(Kinds $cy)"
    $sh2 = AShut $cy
    Expect ($sh2.outcome -eq 'REFUSED' -and (Count $cy 'vm.acpi') -eq 1) "after the emergency: $(Why $sh2)"
}
Case 'U14' 'a missing or stale monitor state and a stale VM observation refuse the Ubuntu start' {
    $c = New-VCtx; $a = New-Auth -Target 'P9S-U24'; $null = AOpen $c $a
    $c.Prov.mon = $null
    $s1 = AStart $c
    Expect ($s1.outcome -eq 'REFUSED' -and (Has $s1 'STATE_EMPTY') -and $c.Calls.Count -eq 0) "no monitor state: $(Why $s1)"
    $c.Prov.mon = $ELIG; $c.Prov.age = 13
    $s2 = AStart $c
    Expect ($s2.outcome -eq 'REFUSED' -and (Has $s2 'STATE_STALE') -and $c.Calls.Count -eq 0) "stale monitor state: $(Why $s2)"
    $c.Prov.age = 1
    $k = $c.Clock; $step = 11L * $F; $c.Prov.onVmRead = { param($vm) $k.q += $step }.GetNewClosure()
    $s3 = AStart $c
    Expect ($s3.outcome -eq 'REFUSED' -and (Has $s3 'VM_STATE_STALE:P9S-U24') -and $c.Calls.Count -eq 0 -and (Entry $c $a).startups_used -eq 0) "stale observation: $(Why $s3)"
}
Case 'U15' 'an unreadable Ubuntu state is never taken for off' {
    $x = Started -Target 'P9S-U24'; $c = $x.c
    $tbl = $c.VmTable; $c.Prov.onAcpi = { $tbl['P9S-U24'] = 'unreadable' }.GetNewClosure()
    $r = AShut $c
    Expect ($r.outcome -eq 'FAILED' -and (Has $r 'VM_STATE_UNOBSERVED') -and (Has $r 'ACPI_VALIDATION_UNRESOLVED')) "watch: $(Why $r)"
    $k = AConfirm $c 10
    Expect ($k.outcome -ne 'DONE' -and (Entry $c $x.a).phase -eq 'ACPI_VALIDATION_UNRESOLVED' -and (Sess $c).expected_vm['P9S-U24'] -ne 'off') "confirmation of an unreadable state: $(Why $k) $((Entry $c $x.a).phase)"
}
Case 'U16' 'an Ubuntu authorization grants no normal action: no T-CLEAN step, no test session, no READY' {
    $c = New-VCtx; $null = AOpen $c (New-Auth -Target 'P9S-U24')
    $plan = Join-Path $c.Dir 'plan.json'; [ordered]@{ schema = 'p9s-plan/1'; name = 'p'; steps = @(@{ Action = 'wait'; Seconds = 5 }) } | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath $plan
    foreach ($r in @(@{ Action = 'vm.start'; Vm = 'P9S-U24' }, @{ Action = 'vm.start'; Vm = 'P9S-D12' }, @{ Action = 'guest.type'; Vm = 'P9S-U24'; Text = 'abc' }, @{ Action = 'guest.key'; Vm = 'P9S-U24'; Key = 'Enter' },
                      @{ Action = 'guest.unlock'; Vm = 'P9S-U24' }, @{ Action = 'scenario.begin'; Name = 'tclean-15' }, @{ Action = 'sequence.run'; Plan = $plan })) {
        $x = Act $c $r
        Expect ($x.outcome -eq 'REFUSED' -and (Has $x 'SESSION_KIND_MISMATCH')) "$($r.Action) $($r.Vm): $(Why $x)"
    }
    $ts = Act $c @{ Action = 'session.open'; Kind = 'Test' }
    Expect ($ts.outcome -eq 'REFUSED' -and (Has $ts 'ACPI_VALIDATION_NOT_FINALIZED') -and $c.Calls.Count -eq 0) "test session: $(Why $ts)"
    $g = Get-P9SGate $c
    Expect ($g.status -eq 'UNCALIBRATED' -and -not $g.ok) "normal gate: $($g.status) ok $($g.ok)"
}
Case 'U17' 'the Ubuntu memory minimum: stated by the session in MiB, approved by it, never below the floor, applied before the start and while observing' {
    $kd = Split-Path -Parent (Join-Path $kit 'paging-thresholds.json')
    $base = [ordered]@{ mode = 'Monitor'; paging_stop = 'Required'; commit_gate_s = 300; commit_below_pct = 90 }
    $mk = { param($k, $v) $m = [ordered]@{}; foreach ($i in $base.Keys) { $m[$i] = $base[$i] }; if ($k) { $m[$k] = $v }; $m }
    $cases = @(
        @(@{ monitor_conditions = (& $mk $null $null) }, 'AUTHORIZATION_MONITOR_CONDITIONS'), @(@{ monitor_conditions = (& $mk 'provisional_min_available_mb' 8192) }, 'AUTHORIZATION_MEMORY_UNIT'),
        @(@{ monitor_conditions = (& $mk 'provisional_min_available_mib' 4096) }, 'AUTHORIZATION_MONITOR_CONDITIONS'), @(@{ monitor_conditions = (& $mk 'provisional_min_available_mib' 8191) }, 'AUTHORIZATION_MONITOR_CONDITIONS'),
        @(@{ monitor_conditions = (& $mk 'provisional_min_available_mib' '8192') }, 'AUTHORIZATION_MONITOR_CONDITIONS'), @(@{ monitor_conditions = (& $mk 'provisional_min_available_mib' 8192.5) }, 'AUTHORIZATION_MONITOR_CONDITIONS'),
        @(@{ provisional_safeguard_approved = $false }, 'AUTHORIZATION_SAFEGUARD_NOT_APPROVED'), @(@{ provisional_safeguard_approved = 'true' }, 'AUTHORIZATION_SAFEGUARD_NOT_APPROVED'))
    foreach ($x in $cases) {
        $r = Read-P9SAcpiAuthorization -Path (New-Auth -Target 'P9S-U24' -Set $x[0]) -KitDir $kd
        Expect (-not $r.ok -and ($r.reasons -contains $x[1])) "$($x[0] | ConvertTo-Json -Compress -Depth 4) -> $($r.reasons -join ',') (expected $($x[1]))"
    }
    $r2 = Read-P9SAcpiAuthorization -Path (New-Auth -Target 'P9S-U24' -Remove @('provisional_safeguard_approved')) -KitDir $kd
    Expect (-not $r2.ok -and ($r2.reasons -contains 'AUTHORIZATION_SAFEGUARD_NOT_APPROVED')) "approval missing: $($r2.reasons -join ',')"
    $c = New-VCtx; $c.Prov.offAfter = 1
    $a = New-Auth -Target 'P9S-U24' -Set @{ monitor_conditions = (& $mk 'provisional_min_available_mib' 10240) }
    $p = Act $c @{ Action = 'acpi.preflight'; Authorization = $a }
    Expect (($p.lines -join ' ') -match 'memory minimum 10240 MiB') "preflight: $($p.lines -join ' | ')"
    $null = AOpen $c $a
    $c.Prov.mon = $MEM9
    $s1 = AStart $c
    Expect ($s1.outcome -eq 'REFUSED' -and (Has $s1 'MEMORY_BELOW_PROVISIONAL_MINIMUM') -and $c.Calls.Count -eq 0) "start at 9000 MiB with a 10240 MiB minimum: $(Why $s1)"
    $c.Prov.mon = $ELIG
    $s2 = AStart $c
    Expect ($s2.outcome -eq 'DISPATCHED') "start at 16000 MiB: $(Why $s2)"
    $c.Prov.mon = $MEM9
    $b = AObs $c 10
    Expect ($b.outcome -eq 'REFUSED' -and (Has $b 'MEMORY_BELOW_PROVISIONAL_MINIMUM')) "observe at 9000 MiB: $(Why $b)"
    $sh = AShut $c
    Expect ($sh.outcome -eq 'DONE' -and (Entry $c $a).phase -eq 'OFF_CONFIRMED' -and (Entry $c $a).min_available_mib -eq 10240) "the request below the minimum: $(Why $sh)"
}
Case 'U18' 'a corrupt or locked ledger refuses every validation step and every new validation; it is never read as an empty one' {
    $c = New-VCtx; $a = New-Auth -Target 'P9S-U24'; $null = AOpen $c $a
    $good = Get-Content -Raw -LiteralPath $c.LedgerPath
    $r0 = $c.Prov.vmreads
    Set-Content -LiteralPath $c.LedgerPath -Value '{ "schema": "p9s-acpi-validation-ledger/1", "entries": [' -NoNewline
    $s1 = AStart $c; $r1 = $c.Prov.vmreads; $o1 = AOpen $c (New-Auth -Target 'P9S-U24')
    Expect ((Has $s1 'ACPI_LEDGER_UNREADABLE') -and (Has $o1 'ACPI_LEDGER_UNREADABLE') -and $c.Calls.Count -eq 0 -and $r1 -eq $r0) "corrupt ledger: $(Why $s1) / $(Why $o1)"
    $p1 = Act $c @{ Action = 'acpi.preflight'; Authorization = (New-Auth -Target 'P9S-U24') }
    Expect ((Has $p1 'ACPI_LEDGER_UNREADABLE') -and ($p1.lines -join ' ') -match 'NOT READY TO OPEN') "preflight with a corrupt ledger: $(Why $p1)"
    $r0 = $c.Prov.vmreads
    Set-Content -LiteralPath $c.LedgerPath -Value $good -NoNewline
    $hold = [IO.FileStream]::new($c.LedgerPath, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::None)
    try {
        $s2 = AStart $c; $r2 = $c.Prov.vmreads; $o2 = AOpen $c $a; $o3 = AOpen $c (New-Auth -Target 'P9S-D12')
        Expect ((Has $s2 'ACPI_LEDGER_UNREADABLE') -and (Has $o2 'ACPI_LEDGER_UNREADABLE') -and (Has $o3 'ACPI_LEDGER_UNREADABLE') -and $c.Calls.Count -eq 0 -and $r2 -eq $r0) "locked ledger: $(Why $s2) / $(Why $o2) / $(Why $o3)"
    } finally { $hold.Dispose() }
    $s3 = AStart $c
    Expect ($s3.outcome -eq 'DISPATCHED' -and (Entry $c $a).startups_used -eq 1 -and (Kinds $c) -eq 'vm.start P9S-U24') "once readable again: $(Why $s3)"
}
# ---- ledger failures: an unknown history is never an unused budget ------------------------------
# The request is sent and the guest stays on: the one request is spent,
# the validation unresolved, the VM running.
function Sent([string]$Target = 'P9S-D12', $Mon = $ELIG) {
    $x = Started -Target $Target -Mon $Mon
    $t = AShut $x.c 10
    if ($t.outcome -ne 'TIMEOUT') { throw "setup failed: request $(Why $t)" }
    $x
}
function LText($c) { if (Test-Path -LiteralPath $c.LedgerPath) { [IO.File]::ReadAllText($c.LedgerPath) } else { $null } }
# the ledger and its revision record by digest: what no refusal may change
function LFiles($c) { (@($c.LedgerPath, (Get-P9SAcpiLedgerRevisionPath $c)) | ForEach-Object { if (Test-Path -LiteralPath $_) { (Get-FileHash -LiteralPath $_).Hash } else { 'absent' } }) -join ',' }
# the accidents a ledger can meet
function Break-Ledger($c, [string]$How) {
    $t = LText $c
    switch ($How) {
        'missing' { Remove-Item -LiteralPath $c.LedgerPath }
        'empty' { [IO.File]::WriteAllText($c.LedgerPath, '') }
        'truncated' { [IO.File]::WriteAllText($c.LedgerPath, $t.Substring(0, [int]($t.Length / 2))) }
        'notjson' { [IO.File]::WriteAllText($c.LedgerPath, "`0`0`0 not a ledger") }
        'schema' { [IO.File]::WriteAllText($c.LedgerPath, $t.Replace('p9s-acpi-validation-ledger/1', 'p9s-acpi-validation-ledger/2')) }
        default { throw "unknown damage $How" }
    }
}
# a sent Ubuntu request, the ledger damaged: nothing more is sent by either
# path and the files stay as they are
function Test-BrokenLedger([string]$How, [string]$Reason, [string]$Detail) {
    $x = Sent -Target 'P9S-U24'; $c = $x.c
    Break-Ledger $c $How
    $f0 = LFiles $c
    $l = Read-P9SAcpiLedger $c
    Expect (-not $l.ok -and $l.reason -eq $Reason -and (-not $Detail -or $l.detail -eq $Detail)) "${How}: read as $($l.ok) $($l.reason) ($($l.detail))"
    $m = Emerg $c 'P9S-U24'; $s = AShut $c
    Expect ($m.outcome -eq 'REFUSED' -and (Has $m $Reason) -and (Has $m 'SHUTDOWN_HISTORY_UNCERTAIN') -and (Has $m 'EMERGENCY_UNRESOLVED') -and $m.dispatched -eq 0) "${How}: emergency $(Why $m)"
    Expect ($s.outcome -eq 'REFUSED' -and (Has $s $Reason)) "${How}: validation request $(Why $s)"
    Expect ((Count $c 'vm.acpi') -eq 1 -and (LFiles $c) -eq $f0) "${How}: $(Kinds $c); ledger files changed: $((LFiles $c) -ne $f0)"
}
Case 'L01' 'a missing ledger: empty only where none was ever used; absent after use it refuses every step and every emergency request' {
    $c = New-VCtx; $a = New-Auth -Target 'P9S-U24'
    $l0 = Read-P9SAcpiLedger $c
    Expect ($l0.ok -and @($l0.doc.entries).Count -eq 0 -and -not (Test-Path -LiteralPath $c.LedgerPath)) "never used: $($l0.reason)"
    Expect ((AOpen $c $a).outcome -eq 'DONE' -and (Test-Path -LiteralPath (Get-P9SAcpiLedgerRevisionPath $c))) 'the first validation of a host did not open'
    $x = Sent -Target 'P9S-U24'; $c = $x.c
    Break-Ledger $c 'missing'
    $l1 = Read-P9SAcpiLedger $c
    Expect (-not $l1.ok -and $l1.reason -eq 'ACPI_LEDGER_MISSING') "absent after use: $($l1.reason) ($($l1.detail))"
    foreach ($y in @((AShut $c), (AStart $c), (AOpen $c (New-Auth -Target 'P9S-U24')), (Emerg $c 'P9S-U24'), (AConfirm $c 5), (AFinal $c))) {
        Expect ($y.outcome -eq 'REFUSED' -and (Has $y 'ACPI_LEDGER_MISSING')) "$($y.action): $(Why $y)"
    }
    Expect ((Count $c 'vm.acpi') -eq 1 -and -not (Test-Path -LiteralPath $c.LedgerPath)) "after: $(Kinds $c); ledger recreated: $(Test-Path -LiteralPath $c.LedgerPath)"
    # a ledger from before the revision record: the session record is enough
    Remove-Item -LiteralPath (Get-P9SAcpiLedgerRevisionPath $c)
    $l2 = Read-P9SAcpiLedger $c
    Expect (-not $l2.ok -and $l2.reason -eq 'ACPI_LEDGER_MISSING' -and $l2.detail -match 'session record') "session record only: $($l2.reason) ($($l2.detail))"
}
Case 'L02' 'an empty ledger is unreadable, not empty: no request by either path, nothing rewritten' { Test-BrokenLedger 'empty' 'ACPI_LEDGER_UNREADABLE' 'empty' }
Case 'L03' 'a truncated ledger is unreadable: no request by either path, nothing rewritten' { Test-BrokenLedger 'truncated' 'ACPI_LEDGER_UNREADABLE' 'not JSON' }
Case 'L04' 'a ledger that is not JSON is unreadable: no request by either path, nothing rewritten' { Test-BrokenLedger 'notjson' 'ACPI_LEDGER_UNREADABLE' 'not JSON' }
Case 'L05' 'a ledger of an unsupported schema is unreadable: no request by either path, nothing rewritten' { Test-BrokenLedger 'schema' 'ACPI_LEDGER_UNREADABLE' 'schema' }
Case 'L06' 'a ledger entry naming another session: the validation request and the emergency request are both refused' {
    $x = Started -Target 'P9S-U24'; $c = $x.c
    $l = Ledger $c; $e = Get-P9SAcpiEntry $l (AuthId $x.a); $e.session_id = 'ses-20261120T000000000Z-1'; $null = Save-P9SAcpiLedger $c $l
    $s = AShut $c; $m = Emerg $c 'P9S-U24'
    Expect ((Has $s 'ACPI_AUTHORIZATION_MISMATCH') -and (Has $m 'ACPI_AUTHORIZATION_MISMATCH') -and (Has $m 'SHUTDOWN_HISTORY_UNCERTAIN') -and $m.dispatched -eq 0 -and (Count $c 'vm.acpi') -eq 0) "shutdown $(Why $s) / emergency $(Why $m)"
}
Case 'L07' 'a ledger entry naming another VM: its request is not lost, nothing is sent to either VM' {
    $x = Sent -Target 'P9S-U24'; $c = $x.c
    $l = Ledger $c; $e = Get-P9SAcpiEntry $l (AuthId $x.a); $e.target_vm = 'P9S-D12'; $null = Save-P9SAcpiLedger $c $l
    $c.VmTable['P9S-D12'] = 'running'
    $s = AShut $c; $m1 = Emerg $c 'P9S-U24'; $m2 = Emerg $c 'P9S-D12'
    Expect ((Has $s 'ACPI_TARGET_MISMATCH') -and (Has $m1 'ACPI_TARGET_MISMATCH') -and (Has $m1 'SHUTDOWN_HISTORY_UNCERTAIN') -and $m1.dispatched -eq 0 -and $m2.outcome -eq 'REFUSED' -and $m2.dispatched -eq 0) "shutdown $(Why $s) / P9S-U24 $(Why $m1) / P9S-D12 $(Why $m2)"
    Expect ((Count $c 'vm.acpi') -eq 1 -and @($c.Calls | Where-Object vm -eq 'P9S-D12').Count -eq 0) "dispatches: $(Kinds $c)"
}
Case 'L08' 'a budget count that is not a whole number >= 0 makes the ledger unreadable: never read as zero' {
    foreach ($v in '-1', '"1"', '0.5', 'null') {
        $x = Sent -Target 'P9S-U24'; $c = $x.c
        [IO.File]::WriteAllText($c.LedgerPath, ((LText $c) -replace '"acpi_used":\s*1', ('"acpi_used": ' + $v)))
        $l = Read-P9SAcpiLedger $c
        $m = Emerg $c 'P9S-U24'; $s = AShut $c
        Expect (-not $l.ok -and $l.reason -eq 'ACPI_LEDGER_UNREADABLE' -and $l.detail -eq 'budget count' -and (Has $m 'SHUTDOWN_HISTORY_UNCERTAIN') -and (Has $s 'ACPI_LEDGER_UNREADABLE') -and (Count $c 'vm.acpi') -eq 1) "acpi_used ${v}: $($l.reason) ($($l.detail)) / $(Why $m) / $(Why $s)"
    }
}
Case 'L09' 'a ledger another program holds open cannot be read: no request; once readable the same history refuses again' {
    $x = Sent -Target 'P9S-U24'; $c = $x.c
    $hold = [IO.FileStream]::new($c.LedgerPath, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::None)
    try {
        $m = Emerg $c 'P9S-U24'; $s = AShut $c
        Expect ((Has $m 'ACPI_LEDGER_UNREADABLE') -and (Has $m 'SHUTDOWN_HISTORY_UNCERTAIN') -and $m.dispatched -eq 0 -and (Has $s 'ACPI_LEDGER_UNREADABLE')) "held open: $(Why $m) / $(Why $s)"
    } finally { $hold.Dispose() }
    $m2 = Emerg $c 'P9S-U24'
    Expect ((Has $m2 'ACPI_ALREADY_REQUESTED') -and (Count $c 'vm.acpi') -eq 1 -and (Entry $c $x.a).acpi_used -eq 1) "readable again: $(Why $m2)"
}
Case 'L10' 'a failed ledger write is no proof of a dispatch: nothing unrecorded is sent; a ledger written without its revision record keeps the request spent' {
    $x = Started -Target 'P9S-U24'; $c = $x.c; $c.Prov.mon = $STOP
    $c.Prov.failLedgerAt = $c.Prov.ledgerWrites + 1; $c.Prov.failLedgerTo = $c.Prov.ledgerWrites + 1
    $m = Emerg $c 'P9S-U24'
    $q = Read-P9SStopRequestState $c
    Expect ($m.outcome -eq 'FAILED' -and (Has $m 'EVIDENCE_WRITE_FAILED') -and (Has $m 'ACPI_REQUEST_NOT_SENT') -and $m.dispatched -eq 0 -and (Count $c 'vm.acpi') -eq 0) "write failed: $(Why $m)"
    Expect ((Entry $c $x.a).acpi_used -eq 0 -and $q.ok -and -not $q.doc.acpi.Contains('P9S-U24') -and $q.doc.unresolved.Contains('P9S-U24')) 'a request that was not sent is recorded as sent'
    $c.Prov.offAfter = 0
    $m2 = Emerg $c 'P9S-U24'
    Expect ($m2.outcome -eq 'DONE' -and (Count $c 'vm.acpi') -eq 1 -and (Entry $c $x.a).acpi_used -eq 1) "once the ledger can be written: $(Why $m2)"
    $y = Started -Target 'P9S-U24'; $d = $y.c
    $d | Add-Member -NotePropertyName WriteLedgerRevision -NotePropertyValue { param($p, $t) throw 'injected revision record failure' }
    $s = AShut $d
    $d.WriteLedgerRevision = $null
    Expect ($s.outcome -eq 'REFUSED' -and (Has $s 'EVIDENCE_WRITE_FAILED') -and (Count $d 'vm.acpi') -eq 0 -and (Entry $d $y.a).acpi_used -eq 1) "revision record not written: $(Why $s)"
    $s2 = AShut $d; $m3 = Emerg $d 'P9S-U24'
    Expect ((Has $s2 'ACPI_BUDGET_EXHAUSTED') -and (Has $m3 'ACPI_ALREADY_REQUESTED') -and (Count $d 'vm.acpi') -eq 0) "afterwards: $(Why $s2) / $(Why $m3)"
}
Case 'L11' 'the reported sequence: one request sent, the guest stays on, the ledger becomes unreadable; no second request, and the state stays unresolved' {
    $x = Sent -Target 'P9S-U24'; $c = $x.c
    Expect ((Count $c 'vm.acpi') -eq 1 -and (Entry $c $x.a).acpi_used -eq 1 -and $c.VmTable['P9S-U24'] -eq 'running') "setup: $(Kinds $c)"
    Break-Ledger $c 'truncated'
    $f0 = LFiles $c
    $e = Emerg $c 'P9S-U24'
    $t = $e.lines -join ' '
    Expect ($e.outcome -eq 'REFUSED' -and (Has $e 'ACPI_LEDGER_UNREADABLE') -and (Has $e 'SHUTDOWN_HISTORY_UNCERTAIN') -and (Has $e 'EMERGENCY_UNRESOLVED') -and $e.dispatched -eq 0) "emergency: $(Why $e)"
    Expect ($t -match 'NO REQUEST IS SENT' -and $t -match 'P9S-U24: P9S-U24 running' -and $t -match 'acpi-validation-ledger\.json \d+ bytes' -and $t -match 'separately authorized recovery') "report: $t"
    $q = Read-P9SStopRequestState $c
    Expect ($q.ok -and $q.doc.unresolved.Contains('P9S-U24') -and -not $q.doc.acpi.Contains('P9S-U24') -and (Sess $c).lifecycle -eq 'EMERGENCY_UNRESOLVED') "recorded: stop request $($q.ok), session $((Sess $c).lifecycle)"
    $au = @(Get-Content -LiteralPath $c.AuditPath | ForEach-Object { $_ | ConvertFrom-Json } | Where-Object { $_.action -eq 'emergency.acpi-shutdown' })[-1]
    Expect ($au.note -match 'request history uncertain' -and $au.note -match 'sha256 [0-9a-f]{16}' -and $au.dispatched -eq 0) "audit: $($au.note)"
    $e2 = Emerg $c 'P9S-U24'; $sh = AShut $c; $st = AStart $c; $fi = AFinal $c; $op = AOpen $c (New-Auth -Target 'P9S-D12'); $cl = Act $c @{ Action = 'session.close' }
    Expect ((Has $e2 'SHUTDOWN_HISTORY_UNCERTAIN') -and $e2.dispatched -eq 0) "second emergency: $(Why $e2)"
    foreach ($r in $sh, $st, $fi, $op) { Expect ($r.outcome -eq 'REFUSED' -and (Has $r 'ACPI_LEDGER_UNREADABLE')) "$($r.action): $(Why $r)" }
    Expect ($cl.outcome -eq 'REFUSED' -and (Test-Path -LiteralPath $c.StopRequestPath)) "session.close: $(Why $cl)"
    $c2 = New-VCtx -Dir $c.Dir -Vms @{ 'P9S-U24' = 'running' }
    $e3 = Emerg $c2 'P9S-U24'
    Expect ((Has $e3 'SHUTDOWN_HISTORY_UNCERTAIN') -and $c2.Calls.Count -eq 0) "after a restart: $(Why $e3)"
    Expect ((Count $c 'vm.acpi') -eq 1 -and (LFiles $c) -eq $f0) "afterwards: $(Kinds $c); ledger files changed: $((LFiles $c) -ne $f0)"
}
Case 'L12' 'the ledger breaks while an emergency waits for the lock: decided again under the lock, nothing is sent' {
    $c = New-VCtx -RealClock; $a = New-Auth -Real -Target 'P9S-U24'
    $null = AOpen $c $a; $s = AStart $c
    Expect ($s.outcome -eq 'DISPATCHED') "start: $(Why $s)"
    $lock = [IO.FileStream]::new($c.LockPath, [IO.FileMode]::OpenOrCreate, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
    try {
        $w = Start-AWorker $c @{ Action = 'emergency.acpi-shutdown'; Vm = 'P9S-U24'; Seconds = 4 } @{ 'P9S-D12' = 'off'; 'P9S-U24' = 'running' }
        $sw = [Diagnostics.Stopwatch]::StartNew()
        while (-not (Test-Path -LiteralPath $c.StopRequestPath) -and $sw.Elapsed.TotalSeconds -lt 60) { Start-Sleep -Milliseconds 50 }
        Expect (Test-Path -LiteralPath $c.StopRequestPath) 'the emergency did not get past its first check'
        Break-Ledger $c 'notjson'
    } finally { $lock.Dispose() }
    $r = Wait-AWorker $w
    Expect ($r.outcome -eq 'REFUSED' -and @($r.reasons) -contains 'ACPI_LEDGER_UNREADABLE' -and @($r.reasons) -contains 'SHUTDOWN_HISTORY_UNCERTAIN' -and $r.dispatched -eq 0) "emergency: $($r.outcome) $(@($r.reasons) -join ',')"
    Expect (@(DispatchLog $c | Where-Object { $_ -like 'vm.acpi *' }).Count -eq 0) "dispatches: $(@(DispatchLog $c) -join '; ')"
}
Case 'L13' 'the emergency request is recorded twice over: a lost stop request does not give it back' {
    $x = Started -Target 'P9S-U24'; $c = $x.c; $c.Prov.mon = $STOP
    $m = Emerg $c 'P9S-U24'
    Expect ($m.dispatched -eq 1 -and (Entry $c $x.a).acpi_used -eq 1 -and (Entry $c $x.a).acpi_source -eq 'emergency') "first: $(Why $m)"
    $m2 = Emerg $c 'P9S-U24'
    Remove-Item -LiteralPath $c.StopRequestPath
    $m3 = Emerg $c 'P9S-U24'; $s = AShut $c
    Expect ((Has $m2 'ACPI_ALREADY_REQUESTED') -and (Has $m3 'ACPI_ALREADY_REQUESTED') -and (Has $s 'ACPI_BUDGET_EXHAUSTED') -and (Count $c 'vm.acpi') -eq 1) "second $(Why $m2) / without the stop request $(Why $m3) / validation $(Why $s)"
}
Case 'L14' 'the validation path refuses a second request: after its own, after a restart, and after the emergency path sent it' {
    $x = Sent -Target 'P9S-U24'; $c = $x.c
    $s1 = AShut $c
    $c2 = New-VCtx -Dir $c.Dir -Vms @{ 'P9S-U24' = 'running' }; $s2 = AShut $c2
    Expect ((Has $s1 'ACPI_BUDGET_EXHAUSTED') -and (Has $s2 'ACPI_BUDGET_EXHAUSTED') -and $c2.Calls.Count -eq 0 -and (Count $c 'vm.acpi') -eq 1) "own $(Why $s1) / restart $(Why $s2)"
    $y = Started -Target 'P9S-U24'; $d = $y.c
    $m = Emerg $d 'P9S-U24'; $s3 = AShut $d
    Expect ($m.dispatched -eq 1 -and (Has $s3 'ACPI_BUDGET_EXHAUSTED') -and (Count $d 'vm.acpi') -eq 1) "after the emergency: $(Why $m) / $(Why $s3)"
}
Case 'L15' 'a request recorded by another emergency while this one waited for the lock is seen under the lock: no second request' {
    $c = New-VCtx -RealClock
    $lock = [IO.FileStream]::new($c.LockPath, [IO.FileMode]::OpenOrCreate, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
    try {
        $w = Start-AWorker $c @{ Action = 'emergency.acpi-shutdown'; Vm = 'P9S-D12'; Seconds = 4 } @{ 'P9S-D12' = 'running'; 'P9S-U24' = 'off' }
        $sw = [Diagnostics.Stopwatch]::StartNew()
        while (-not (Test-Path -LiteralPath $c.StopRequestPath) -and $sw.Elapsed.TotalSeconds -lt 60) { Start-Sleep -Milliseconds 50 }
        Expect (Test-Path -LiteralPath $c.StopRequestPath) 'the emergency did not get past its first check'
        # meanwhile another emergency records its request (what it writes just before its dispatch)
        $q = Read-P9SStopRequest $c; $q.acpi['P9S-D12'] = [DateTime]::UtcNow.ToString('o'); Save-P9SStopRequest $c $q
    } finally { $lock.Dispose() }
    $r = Wait-AWorker $w
    Expect ($r.outcome -eq 'REFUSED' -and @($r.reasons) -contains 'ACPI_ALREADY_REQUESTED' -and $r.dispatched -eq 0) "emergency: $($r.outcome) $(@($r.reasons) -join ',')"
    Expect (@(DispatchLog $c | Where-Object { $_ -like 'vm.acpi *' }).Count -eq 0) "dispatches: $(@(DispatchLog $c) -join '; ')"
}
Case 'L16' 'two emergencies for the same VM at once: exactly one request, and its record is not overwritten' {
    $c = New-VCtx -RealClock
    $go = New-Event "Global\P9S-TEST-$run-l16-go"; $r1 = New-Event "Global\P9S-TEST-$run-l16-r1"; $r2 = New-Event "Global\P9S-TEST-$run-l16-r2"
    $vms = @{ 'P9S-D12' = 'running'; 'P9S-U24' = 'off' }
    $w1 = Start-AWorker $c @{ Action = 'emergency.acpi-shutdown'; Vm = 'P9S-D12'; Seconds = 4 } $vms -DispatchMs 500 -Go "Global\P9S-TEST-$run-l16-go" -Ready "Global\P9S-TEST-$run-l16-r1"
    $w2 = Start-AWorker $c @{ Action = 'emergency.acpi-shutdown'; Vm = 'P9S-D12'; Seconds = 4 } $vms -DispatchMs 500 -Go "Global\P9S-TEST-$run-l16-go" -Ready "Global\P9S-TEST-$run-l16-r2"
    Expect ($r1.WaitOne(60000) -and $r2.WaitOne(60000)) 'workers not ready'
    $null = $go.Set()
    $outs = @((Wait-AWorker $w1), (Wait-AWorker $w2))
    Expect (@(DispatchLog $c | Where-Object { $_ -like 'vm.acpi *' }).Count -eq 1) "dispatches: $(@(DispatchLog $c) -join '; ')"
    Expect (@($outs | Where-Object { $_.dispatched -eq 1 }).Count -eq 1 -and @($outs | Where-Object { $_.outcome -eq 'REFUSED' -and @($_.reasons) -contains 'ACPI_ALREADY_REQUESTED' }).Count -eq 1) "outcomes: $(@($outs | ForEach-Object { "$($_.outcome) $(@($_.reasons) -join ',')" }) -join ' / ')"
    $q = Read-P9SStopRequestState $c
    Expect ($q.ok -and $q.doc.acpi.Contains('P9S-D12')) 'the record of the request was lost'
}
Case 'L17' 'a restart gives nothing back: a coordinator that ended after spending the request but before its dispatch leaves it spent' {
    $x = Started -Target 'P9S-U24'; $c = $x.c
    # what acpi.shutdown writes before it dispatches; then its process ends
    $b = Get-P9SAcpiBundle $c
    $b.entry.acpi_used = 1; $b.entry.acpi_source = 'validation'; $b.entry.phase = 'SHUTDOWN_REQUESTING'; $b.entry.deadline_utc = (Get-P9SUtcNow $c).AddSeconds(180).ToString('o')
    $b.session.inflight = [ordered]@{ action = 'acpi.shutdown'; vm = 'P9S-U24'; pid = 999999; start_ticks = 1; utc = [DateTime]::UtcNow.ToString('o') }
    Expect (Save-P9SAcpiState $c $b -BeforeDispatch) 'write-ahead not written'
    $c2 = New-VCtx -Dir $c.Dir -Vms @{ 'P9S-U24' = 'running' }
    $s = AShut $c2; $m = Emerg $c2 'P9S-U24'; $st = AStart $c2
    Expect ((Has $s 'ACPI_BUDGET_EXHAUSTED') -and (Has $s 'INFLIGHT_UNRESOLVED') -and (Has $m 'ACPI_ALREADY_REQUESTED') -and (Has $st 'ACPI_STARTUP_BUDGET_EXHAUSTED') -and $c2.Calls.Count -eq 0) "after a restart: $(Why $s) / $(Why $m) / $(Why $st)"
    $c2.VmTable['P9S-U24'] = 'off'
    $k = AConfirm $c2 10; $fin = AFinal $c2
    Expect ($k.outcome -eq 'DONE' -and (Entry $c2 $x.a).phase -eq 'OFF_REQUEST_UNCONFIRMED' -and $fin.outcome -eq 'DONE' -and (Entry $c2 $x.a).outcome -eq 'OFF_AFTER_UNCONFIRMED_REQUEST') "resolved read only: $(Why $k) / $(Why $fin)"
}
Case 'L18' 'no action repairs, resets or recreates a damaged or missing ledger' {
    foreach ($how in 'notjson', 'missing') {
        $x = Sent -Target 'P9S-U24'; $c = $x.c
        Break-Ledger $c $how
        $f0 = LFiles $c; $n0 = @(Get-ChildItem -LiteralPath $c.Dir -Name -Force | Where-Object { $_ -like '*ledger*' }) -join ','
        $acts = @(
            @{ Action = 'acpi.preflight'; Authorization = (New-Auth -Target 'P9S-U24') }, @{ Action = 'acpi.open'; Authorization = (New-Auth -Target 'P9S-U24') }, @{ Action = 'acpi.start' },
            @{ Action = 'acpi.observe'; Seconds = 5 }, @{ Action = 'acpi.shutdown' }, @{ Action = 'acpi.confirm-off'; Seconds = 5 }, @{ Action = 'acpi.integrity' }, @{ Action = 'acpi.finalize' },
            @{ Action = 'emergency.status' }, @{ Action = 'emergency.acpi-shutdown'; Vm = 'P9S-U24' }, @{ Action = 'emergency.wait-off'; Vm = 'P9S-U24'; Seconds = 4 },
            @{ Action = 'session.status' }, @{ Action = 'session.open'; Kind = 'Test' }, @{ Action = 'session.close' })
        $out = @(foreach ($r in $acts) { Act $c $r })
        $n1 = @(Get-ChildItem -LiteralPath $c.Dir -Name -Force | Where-Object { $_ -like '*ledger*' }) -join ','
        Expect ((LFiles $c) -eq $f0 -and $n1 -eq $n0 -and (Count $c 'vm.acpi') -eq 1) "${how}: files $n0 -> $n1, changed $((LFiles $c) -ne $f0), $(Kinds $c)"
        Expect (@($out | Where-Object { $_.dispatched -gt 0 }).Count -eq 0) "${how}: something was dispatched"
    }
}
Case 'L19' 'a damaged ledger leaves the safe diagnostics: VM state, monitor, session and what is wrong with the files, read only' {
    $x = Sent -Target 'P9S-U24'; $c = $x.c
    Break-Ledger $c 'truncated'
    $r0 = $c.Prov.vmreads
    $st = Act $c @{ Action = 'emergency.status' }
    $t = $st.lines -join ' | '
    Expect ($st.outcome -eq 'DONE' -and (Has $st 'SHUTDOWN_HISTORY_UNCERTAIN') -and $t -match 'P9S-U24 running' -and $t -match 'ACPI_LEDGER_UNREADABLE \(not JSON\)' -and $t -match 'acpi-validation-ledger\.json \d+ bytes, written \S+, sha256 [0-9a-f]{16}' -and $t -match 'monitor: status UNCALIBRATED' -and $c.Prov.vmreads -gt $r0) "status: $t"
    $w = Act $c @{ Action = 'emergency.wait-off'; Vm = 'P9S-U24'; Seconds = 4 }
    Expect ($w.outcome -eq 'TIMEOUT' -and ($w.lines -join ' ') -match 'still running') "wait-off while on: $(Why $w)"
    $ss = Act $c @{ Action = 'session.status' }
    Expect ($ss.outcome -eq 'DONE' -and ($ss.lines -join ' ') -match 'ledger ACPI_LEDGER_UNREADABLE') "session.status: $($ss.lines -join ' | ')"
    $k = AConfirm $c 5
    Expect ($k.outcome -eq 'REFUSED' -and ($k.lines -join ' ') -match 'emergency\.wait-off') "confirm-off: $(Why $k)"
    $c.VmTable['P9S-U24'] = 'off'
    $w2 = Act $c @{ Action = 'emergency.wait-off'; Vm = 'P9S-U24'; Seconds = 4 }
    Expect ($w2.outcome -eq 'DONE' -and ($w2.lines -join ' ') -match 'P9S-U24 is off' -and (Sess $c).expected_vm['P9S-U24'] -eq 'off' -and (Count $c 'vm.acpi') -eq 1) "wait-off once off: $(Why $w2)"
}
Case 'L20' 'a damaged ledger authorizes nothing for the other target: no Debian request, no Debian session, no fallback' {
    $x = Sent -Target 'P9S-U24'; $c = $x.c
    Break-Ledger $c 'notjson'
    $c.VmTable['P9S-D12'] = 'running'
    $m = Emerg $c 'P9S-D12'; $o = AOpen $c (New-Auth -Target 'P9S-D12'); $p = Act $c @{ Action = 'acpi.preflight'; Authorization = (New-Auth -Target 'P9S-D12') }
    Expect ((Has $m 'SHUTDOWN_HISTORY_UNCERTAIN') -and $m.dispatched -eq 0 -and (Has $o 'ACPI_LEDGER_UNREADABLE') -and (Has $p 'ACPI_LEDGER_UNREADABLE')) "P9S-D12: emergency $(Why $m) / open $(Why $o) / preflight $(Why $p)"
    Expect ((Count $c 'vm.acpi') -eq 1 -and (Count $c 'vm.start') -eq 1 -and @($c.Calls | Where-Object vm -eq 'P9S-D12').Count -eq 0) "dispatches: $(Kinds $c)"
}
Case 'L21' 'a dispatch whose outcome is unknown stays unresolved and spent, on both paths and after a restart' {
    $x = Started -Target 'P9S-U24'; $c = $x.c; $c.Prov.acpiThrows = $true
    $r = AShut $c
    Expect ($r.outcome -eq 'FAILED' -and (Has $r 'ACPI_VALIDATION_UNRESOLVED') -and (Entry $c $x.a).acpi_used -eq 1 -and (Entry $c $x.a).phase -eq 'ACPI_VALIDATION_UNRESOLVED') "validation: $(Why $r)"
    $c2 = New-VCtx -Dir $c.Dir -Vms @{ 'P9S-U24' = 'running' }
    $m = Emerg $c2 'P9S-U24'; $s = AShut $c2
    Expect ((Has $m 'ACPI_ALREADY_REQUESTED') -and (Has $s 'ACPI_BUDGET_EXHAUSTED') -and $c2.Calls.Count -eq 0) "after a restart: $(Why $m) / $(Why $s)"
    $d = New-VCtx -Vms @{ 'P9S-D12' = 'running' }; $d.Prov.acpiThrows = $true
    $e1 = Emerg $d 'P9S-D12'
    $q = Read-P9SStopRequestState $d
    $d2 = New-VCtx -Dir $d.Dir -Vms @{ 'P9S-D12' = 'running' }
    $e2 = Emerg $d2 'P9S-D12'
    Expect ($e1.outcome -eq 'FAILED' -and (Has $e1 'EMERGENCY_UNRESOLVED') -and $q.doc.acpi.Contains('P9S-D12') -and $q.doc.unresolved.Contains('P9S-D12') -and (Has $e2 'ACPI_ALREADY_REQUESTED') -and $d2.Calls.Count -eq 0) "emergency: $(Why $e1) / after a restart $(Why $e2)"
}
Case 'L22' 'Ubuntu recovery leaves the Debian record as it was: refused emergencies, a damaged and restored ledger, a confirmation and a finalization change no Debian field' {
    $c = New-VCtx; $c.Prov.offAfter = 1; $d = New-Auth -Target 'P9S-D12'
    $steps = @((AOpen $c $d), (AStart $c), (AShut $c), (AFinal $c))
    Expect (@($steps | Where-Object { $_.outcome -notin 'DONE', 'DISPATCHED' }).Count -eq 0 -and (Entry $c $d).outcome -eq 'VALIDATED') "Debian: $(@($steps | ForEach-Object { Why $_ }) -join ' / ')"
    $deb = (Entry $c $d) | ConvertTo-Json -Depth 8 -Compress
    $c.Prov.offAfter = -1; $u = New-Auth -Target 'P9S-U24'
    $null = AOpen $c $u; $null = AStart $c; $t = AShut $c 10
    $m = Emerg $c 'P9S-U24'
    $good = LText $c
    Break-Ledger $c 'notjson'; $m2 = Emerg $c 'P9S-U24'
    [IO.File]::WriteAllText($c.LedgerPath, $good)
    $c.VmTable['P9S-U24'] = 'off'
    $k = AConfirm $c 10; $fin = AFinal $c
    Expect ($t.outcome -eq 'TIMEOUT' -and (Has $m 'ACPI_ALREADY_REQUESTED') -and (Has $m2 'SHUTDOWN_HISTORY_UNCERTAIN') -and $k.outcome -eq 'DONE' -and $fin.outcome -eq 'DONE' -and (Entry $c $u).outcome -eq 'SHUTDOWN_LATE') "Ubuntu: $(Why $t) / $(Why $m) / $(Why $m2) / $(Why $k) / $(Why $fin)"
    Expect (((Entry $c $d) | ConvertTo-Json -Depth 8 -Compress) -eq $deb) 'the Debian entry changed'
    Expect (@($c.Calls | Where-Object { $_.kind -eq 'vm.acpi' -and $_.vm -eq 'P9S-U24' }).Count -eq 1 -and (Count $c 'vm.acpi') -eq 2) "dispatches: $(Kinds $c)"
}
Case 'L23' 'a ledger replaced by an older copy (a restored backup) is refused: the budget it no longer shows is not given back' {
    $x = Started -Target 'P9S-U24'; $c = $x.c
    $old = LText $c
    $t = AShut $c 10
    Expect ($t.outcome -eq 'TIMEOUT') "request: $(Why $t)"
    [IO.File]::WriteAllText($c.LedgerPath, $old)
    $l = Read-P9SAcpiLedger $c
    $s = AShut $c; $m = Emerg $c 'P9S-U24'
    Expect (-not $l.ok -and $l.reason -eq 'ACPI_LEDGER_STALE' -and (Has $s 'ACPI_LEDGER_STALE') -and (Has $m 'ACPI_LEDGER_STALE') -and (Has $m 'SHUTDOWN_HISTORY_UNCERTAIN') -and (Count $c 'vm.acpi') -eq 1) "older copy: $($l.reason) ($($l.detail)) / $(Why $s) / $(Why $m)"
}
Case 'L24' 'the same revision with other content, or an unreadable revision record: inconsistent and refused' {
    $x = Sent -Target 'P9S-U24'; $c = $x.c
    $good = LText $c
    [IO.File]::WriteAllText($c.LedgerPath, $good.Replace('"acpi_used": 1', '"acpi_used": 0'))
    $l = Read-P9SAcpiLedger $c; $m = Emerg $c 'P9S-U24'
    Expect (-not $l.ok -and $l.reason -eq 'ACPI_LEDGER_INCONSISTENT' -and (Has $m 'ACPI_LEDGER_INCONSISTENT') -and $m.dispatched -eq 0) "edited in place: $($l.reason) ($($l.detail)) / $(Why $m)"
    [IO.File]::WriteAllText($c.LedgerPath, $good)
    [IO.File]::WriteAllText((Get-P9SAcpiLedgerRevisionPath $c), '{ "schema": "p9s-acpi-validation-ledger-revision/1", "revision": ')
    $l2 = Read-P9SAcpiLedger $c; $s = AShut $c
    Expect (-not $l2.ok -and $l2.reason -eq 'ACPI_LEDGER_INCONSISTENT' -and (Has $s 'ACPI_LEDGER_INCONSISTENT') -and (Count $c 'vm.acpi') -eq 1) "revision record damaged: $($l2.reason) ($($l2.detail)) / $(Why $s)"
}
Case 'L25' 'an unreadable stop request: its requests are unknown, so the emergency sends none and session.close keeps it' {
    $c = New-VCtx -Vms @{ 'P9S-D12' = 'running' }
    [IO.File]::WriteAllText($c.StopRequestPath, '{ "reason": "EMERGENCY_ACPI_SHUTDOWN", "acpi": { "P9S-D12": ')
    $h0 = (Get-FileHash -LiteralPath $c.StopRequestPath).Hash
    $m = Emerg $c 'P9S-D12'; $cl = Act $c @{ Action = 'session.close' }; $st = Act $c @{ Action = 'emergency.status' }
    Expect ($m.outcome -eq 'REFUSED' -and (Has $m 'STOP_REQUEST_UNREADABLE') -and (Has $m 'SHUTDOWN_HISTORY_UNCERTAIN') -and $m.dispatched -eq 0 -and ($m.lines -join ' ') -match 'P9S-D12: P9S-D12 running') "emergency: $(Why $m)"
    Expect ($cl.outcome -eq 'REFUSED' -and (Has $cl 'STOP_REQUEST_UNREADABLE') -and (Get-FileHash -LiteralPath $c.StopRequestPath).Hash -eq $h0) "close: $(Why $cl)"
    Expect ($st.outcome -eq 'DONE' -and (Has $st 'STOP_REQUEST_UNREADABLE') -and ($st.lines -join ' ') -match 'P9S-D12 running') "status: $($st.lines -join ' | ')"
    [IO.File]::WriteAllText($c.StopRequestPath, '{ "acpi": "P9S-D12" }')
    $m2 = Emerg $c 'P9S-D12'
    Expect ((Has $m2 'STOP_REQUEST_UNREADABLE') -and (Count $c 'vm.acpi') -eq 0) "another shape: $(Why $m2)"
}
Case 'L26' 'an emergency for one VM never erases the record of the request sent to the other: neither is repeated' {
    $c = New-VCtx -Vms @{ 'P9S-D12' = 'running'; 'P9S-U24' = 'running' }
    $m1 = Emerg $c 'P9S-D12'; $m2 = Emerg $c 'P9S-U24'; $m3 = Emerg $c 'P9S-D12'; $m4 = Emerg $c 'P9S-U24'
    $q = Read-P9SStopRequestState $c
    Expect ($m1.dispatched -eq 1 -and $m2.dispatched -eq 1 -and (Has $m3 'ACPI_ALREADY_REQUESTED') -and (Has $m4 'ACPI_ALREADY_REQUESTED') -and (Count $c 'vm.acpi') -eq 2) "$(Why $m1) / $(Why $m2) / $(Why $m3) / $(Why $m4); $(Kinds $c)"
    Expect ($q.ok -and $q.doc.acpi.Contains('P9S-D12') -and $q.doc.acpi.Contains('P9S-U24') -and $q.doc.unresolved.Contains('P9S-D12') -and $q.doc.unresolved.Contains('P9S-U24')) 'a record of the stop request was lost'
}
# Records present but inaccessible: deny entries for this account on
# synthetic records only, removed by each case. 'inaccessible' denies a
# record its data and attributes and its folder the listing (an existence
# test then answers false), 'data' denies only its data.
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
# a step's result, or the exception it raised
function TryA([scriptblock]$Step) { try { $x = & $Step; [pscustomobject]@{ outcome = [string]$x.outcome; reasons = @($x.reasons); dispatched = [int]$x.dispatched; lines = @($x.lines) } } catch { [pscustomobject]@{ outcome = "EXCEPTION $($_.Exception.Message)"; reasons = @(); dispatched = 0; lines = @() } } }
Case 'L27' 'a ledger restored to an older revision while its revision record cannot be read: the validation request and the emergency refuse, the older copy is never read as current; readable again it is stale' {
    $x = Started -Target 'P9S-U24'; $c = $x.c
    $old = LText $c
    $t = AShut $c 10
    if ($t.outcome -ne 'TIMEOUT') { throw "setup: request $(Why $t)" }
    [IO.File]::WriteAllText($c.LedgerPath, $old)
    $f0 = LFiles $c
    foreach ($how in 'data', 'inaccessible') {
        $l = Lock-Record (Get-P9SAcpiLedgerRevisionPath $c) $how
        try { $r = Read-P9SAcpiLedger $c; $s = TryA { AShut $c }; $m = TryA { Emerg $c 'P9S-U24' } } finally { Unlock-Record $l }
        Expect (-not $r.ok -and $r.reason -eq 'ACPI_LEDGER_INCONSISTENT') "${how}: read as $($r.ok) $($r.reason) ($($r.detail))"
        Expect ($s.outcome -eq 'REFUSED' -and (Has $s 'ACPI_LEDGER_INCONSISTENT')) "${how}: validation request $(Why $s)"
        Expect ($m.outcome -eq 'REFUSED' -and (Has $m 'ACPI_LEDGER_INCONSISTENT') -and (Has $m 'SHUTDOWN_HISTORY_UNCERTAIN') -and $m.dispatched -eq 0) "${how}: emergency $(Why $m)"
        Expect ((Count $c 'vm.acpi') -eq 1 -and (LFiles $c) -eq $f0) "${how}: $(Kinds $c); ledger files changed $((LFiles $c) -ne $f0)"
    }
    $r2 = Read-P9SAcpiLedger $c
    Expect (-not $r2.ok -and $r2.reason -eq 'ACPI_LEDGER_STALE') "readable again: $($r2.reason) ($($r2.detail))"
}
Case 'L28' 'a ledger from before the revision record (no revision, no revision record, no session record) that cannot be read is never "absent, never used": the emergency and a new validation refuse; readable again its request stays spent' {
    $x = Sent -Target 'P9S-U24'; $c = $x.c
    $d = LText $c | ConvertFrom-Json -AsHashtable; $d.Remove('revision')
    [IO.File]::WriteAllText($c.LedgerPath, ($d | ConvertTo-Json -Depth 8))
    Remove-Item -LiteralPath (Get-P9SAcpiLedgerRevisionPath $c), $c.SessionPath
    $r0 = Read-P9SAcpiLedger $c
    if (-not $r0.ok -or $r0.detail -cne 'revision 0') { throw "setup: $($r0.reason) ($($r0.detail))" }
    $b0 = (Get-FileHash -LiteralPath $c.LedgerPath).Hash
    foreach ($how in 'data', 'inaccessible') {
        $l = Lock-Record $c.LedgerPath $how
        try { $r = Read-P9SAcpiLedger $c; $m = TryA { Emerg $c 'P9S-U24' }; $o = TryA { AOpen $c (New-Auth -Target 'P9S-U24') } } finally { Unlock-Record $l }
        Expect (-not $r.ok -and $r.reason -eq 'ACPI_LEDGER_UNREADABLE') "${how}: read as $($r.ok) $($r.reason) ($($r.detail))"
        Expect ($m.outcome -eq 'REFUSED' -and (Has $m 'ACPI_LEDGER_UNREADABLE') -and (Has $m 'SHUTDOWN_HISTORY_UNCERTAIN') -and $m.dispatched -eq 0) "${how}: emergency $(Why $m)"
        Expect ($o.outcome -eq 'REFUSED' -and (Has $o 'ACPI_LEDGER_UNREADABLE')) "${how}: new validation $(Why $o)"
        Expect ((Count $c 'vm.acpi') -eq 1 -and (Get-FileHash -LiteralPath $c.LedgerPath).Hash -eq $b0 -and -not (Test-Path -LiteralPath (Get-P9SAcpiLedgerRevisionPath $c)) -and -not (Test-Path -LiteralPath $c.SessionPath)) "${how}: $(Kinds $c); ledger changed $((Get-FileHash -LiteralPath $c.LedgerPath).Hash -ne $b0); revision record $(Test-Path -LiteralPath (Get-P9SAcpiLedgerRevisionPath $c)); session record $(Test-Path -LiteralPath $c.SessionPath)"
    }
    $m2 = Emerg $c 'P9S-U24'
    Expect ($m2.outcome -eq 'REFUSED' -and (Has $m2 'ACPI_ALREADY_REQUESTED') -and (Count $c 'vm.acpi') -eq 1) "readable again: $(Why $m2); $(Kinds $c)"
}
Case 'L29' 'a stop request that cannot be read halts the validation steps and refuses a new validation whatever ErrorActionPreference the caller set; nothing is dispatched, no error is written' {
    foreach ($pref in 'Stop', 'Continue', 'SilentlyContinue') {
        $x = Started; $c = $x.c
        $n = New-VCtx
        foreach ($y in $c, $n) { $q = New-P9SStopRequestDoc $y; $q.acpi['P9S-U24'] = '2026-11-25T10:00:00.0000000Z'; Save-P9SStopRequest $y $q }
        $a = New-Auth
        $l1 = Lock-Record $c.StopRequestPath; $l2 = Lock-Record $n.StopRequestPath
        try {
            $all = @(& { $ErrorActionPreference = $pref; [pscustomobject]@{ obs = (TryA { AObs $c 5 }); shut = (TryA { AShut $c 5 }); open = (TryA { AOpen $n $a }) } } 2>&1)
        } finally { Unlock-Record $l2; Unlock-Record $l1 }
        $errs = @($all | Where-Object { $_ -is [Management.Automation.ErrorRecord] }); $w = @($all | Where-Object { $_ -isnot [Management.Automation.ErrorRecord] })[-1]
        Expect ($w.obs.outcome -eq 'REFUSED' -and (Has $w.obs 'EMERGENCY_STOP_REQUESTED')) "${pref}: observation $(Why $w.obs)"
        Expect ($w.shut.outcome -eq 'REFUSED' -and (Has $w.shut 'EMERGENCY_STOP_REQUESTED') -and $w.shut.dispatched -eq 0) "${pref}: validation request $(Why $w.shut)"
        Expect ($w.open.outcome -eq 'REFUSED' -and (Has $w.open 'STOP_REQUEST_PENDING') -and -not (Test-Path -LiteralPath $n.SessionPath)) "${pref}: new validation $(Why $w.open)"
        Expect ((Count $c 'vm.acpi') -eq 0 -and $n.Calls.Count -eq 0) "${pref}: $(Kinds $c)"
        Expect ($errs.Count -eq 0) "${pref}: $($errs.Count) errors written: $(@($errs | ForEach-Object { $_.Exception.Message }) -join ' / ')"
    }
}
Case 'L30' 'the ledger diagnosis never calls a file that cannot be read absent' {
    $x = Sent -Target 'P9S-U24'; $c = $x.c
    $l = Lock-Record $c.LedgerPath
    try { $r = Read-P9SAcpiLedger $c; $dg = Get-P9SAcpiLedgerDiagnosis $c $r; $m = TryA { Emerg $c 'P9S-U24' } } finally { Unlock-Record $l }
    Expect ($dg -notmatch 'acpi-validation-ledger\.json absent' -and $dg -match 'acpi-validation-ledger\.json present, not inspected') "diagnosis: $dg"
    Expect ($m.outcome -eq 'REFUSED' -and ($m.lines -join ' ') -notmatch 'acpi-validation-ledger\.json absent') "emergency: $(Why $m) $($m.lines -join ' | ')"
    Remove-Item -LiteralPath $c.LedgerPath
    $dg2 = Get-P9SAcpiLedgerDiagnosis $c $null
    Expect ($dg2 -match 'acpi-validation-ledger\.json absent') "an absent ledger: $dg2"
}

# ---- an installation of the persistent state (state.init) and the authorizations ----
# The context's Installation member answers what Get-P9SStateInstallation
# would: no installation, one initialized at a time, or one that cannot be
# established.
function Set-Installation($c, $Init, [switch]$Unknown) {
    $i = if ($Unknown) { [pscustomobject]@{ known = $false; installed = $false; installation_id = $null; initialized_utc = $null; reason = 'ANCHOR_UNREADABLE' } }
         elseif ($null -eq $Init) { [pscustomobject]@{ known = $true; installed = $false; installation_id = $null; initialized_utc = $null; reason = $null } }
         else { [pscustomobject]@{ known = $true; installed = $true; installation_id = 'inst-test-0001'; initialized_utc = ([datetime]$Init).ToUniversalTime(); reason = $null } }
    $c | Add-Member -NotePropertyName Installation -NotePropertyValue ({ $i }.GetNewClosure()) -Force
}
Case 'L31' 'after an installation every authorization approved or valid before it is void: acpi.open and acpi.preflight refuse it, nothing read or sent; one approved and valid after it opens; no installation changes nothing; an installation that cannot be established refuses' {
    $c = New-VCtx; Set-Installation $c $T0.AddMinutes(-5)
    $x = AOpen $c (New-Auth)
    Expect ($x.outcome -eq 'REFUSED' -and (Has $x 'AUTHORIZATION_BEFORE_INSTALLATION') -and $c.Calls.Count -eq 0 -and $null -eq (Sess $c)) "approved and valid before: $(Why $x)"
    $x = AOpen $c (New-Auth -From $T0.AddMinutes(-1))
    Expect ($x.outcome -eq 'REFUSED' -and (Has $x 'AUTHORIZATION_BEFORE_INSTALLATION')) "valid after, approved before: $(Why $x)"
    $x = AOpen $c (New-Auth -From $T0.AddMinutes(-10) -Set @{ approved_utc = (Z $T0.AddMinutes(-2)) })
    Expect ($x.outcome -eq 'REFUSED' -and (Has $x 'AUTHORIZATION_BEFORE_INSTALLATION')) "approved after, valid before: $(Why $x)"
    $p = Act $c @{ Action = 'acpi.preflight'; Authorization = (New-Auth) }
    Expect ((Has $p 'AUTHORIZATION_BEFORE_INSTALLATION') -and $c.Calls.Count -eq 0) "preflight: $(Why $p)"
    $o = AOpen $c (New-Auth -From $T0.AddMinutes(-1) -Set @{ approved_utc = (Z $T0.AddMinutes(-2)) })
    Expect ($o.outcome -eq 'DONE') "approved and valid after: $(Why $o)"
    $c2 = New-VCtx; Set-Installation $c2 $null; $o2 = AOpen $c2 (New-Auth)
    Expect ($o2.outcome -eq 'DONE') "no installation: $(Why $o2)"
    $c3 = New-VCtx; Set-Installation $c3 $null -Unknown; $o3 = AOpen $c3 (New-Auth)
    Expect ($o3.outcome -eq 'REFUSED' -and (Has $o3 'AUTHORIZATION_INSTALLATION_UNKNOWN') -and $c3.Calls.Count -eq 0) "installation unknown: $(Why $o3)"
}
Case 'L32' 'a validation opened before an installation stops at its next step: acpi.start, acpi.observe and acpi.shutdown refuse the stored authorization, nothing is started or sent' {
    $c = New-VCtx; Set-Installation $c $null; $a = New-Auth
    $o = AOpen $c $a; Expect ($o.outcome -eq 'DONE') "open: $(Why $o)"
    Set-Installation $c $T0.AddMinutes(1); $c.Clock.utc = $T0.AddMinutes(2)
    $s = AStart $c
    Expect ($s.outcome -eq 'REFUSED' -and (Has $s 'AUTHORIZATION_BEFORE_INSTALLATION') -and (Count $c 'vm.start') -eq 0 -and (Entry $c $a).startups_used -eq 0) "start: $(Why $s)"
    $b = AObs $c 10; Expect ($b.outcome -eq 'REFUSED' -and (Has $b 'AUTHORIZATION_BEFORE_INSTALLATION')) "observe: $(Why $b)"
    $h = AShut $c; Expect ($h.outcome -eq 'REFUSED' -and (Has $h 'AUTHORIZATION_BEFORE_INSTALLATION') -and (Count $c 'vm.acpi') -eq 0) "shutdown: $(Why $h)"
    $x = Started; Set-Installation $x.c $T0.AddMinutes(1); $x.c.Clock.utc = $T0.AddMinutes(2)
    $h = AShut $x.c; Expect ($h.outcome -eq 'REFUSED' -and (Has $h 'AUTHORIZATION_BEFORE_INSTALLATION') -and (Count $x.c 'vm.acpi') -eq 0) "shutdown of a started validation: $(Why $h)"
}
Case 'L33' 'the installation rule only adds refusals: the emergency, acpi.confirm-off and the outcome of every step without an installation are the same with and without the member' {
    $x1 = Sent; $x2 = Sent; Set-Installation $x2.c $null -Unknown
    $e1 = Emerg $x1.c; $e2 = Emerg $x2.c
    Expect ($e1.outcome -eq $e2.outcome -and (@($e1.reasons) -join ',') -ceq (@($e2.reasons) -join ',') -and (Count $x1.c 'vm.acpi') -eq (Count $x2.c 'vm.acpi')) "emergency: $(Why $e1) / $(Why $e2)"
    $f1 = AConfirm $x1.c 5; $f2 = AConfirm $x2.c 5
    Expect ($f1.outcome -eq $f2.outcome -and (@($f1.reasons) -join ',') -ceq (@($f2.reasons) -join ',')) "confirm-off: $(Why $f1) / $(Why $f2)"
    $c1 = New-VCtx; $c2 = New-VCtx; Set-Installation $c2 $null
    $a = New-Auth -From $T0.AddMinutes(5)
    $y1 = AOpen $c1 $a; $y2 = AOpen $c2 $a
    Expect ((Why $y1) -ceq (Why $y2) -and (Has $y1 'AUTHORIZATION_NOT_YET_VALID')) "not yet valid: $(Why $y1) / $(Why $y2)"
}

# ---- interruptions: a request without a recorded end, an unconfirmed dispatch, a finished validation ----
# The state the Debian session left: its ledger entry and its closed
# session record. Returns its authorization.
function Finished-Debian($c) {
    $a = New-Auth
    $c.Prov.offAfter = 1
    $r = @((AOpen $c $a), (AStart $c), (AShut $c), (AFinal $c))
    if (@($r | Where-Object { $_.outcome -notin 'DONE', 'DISPATCHED' }).Count -or (Entry $c $a).outcome -ne 'VALIDATED') { throw "setup failed: $(@($r | ForEach-Object { Why $_ }) -join ' / ')" }
    $c.Prov.offAfter = -1; $c.Prov.acpiSent = $false
    $a
}
function EntryJson($c, [string]$Auth) { (Entry $c $Auth) | ConvertTo-Json -Depth 8 -Compress }
# what the emergency path or acpi.shutdown writes just before its dispatch,
# left by a coordinator that then ended
function Set-Recorded($c, [string]$Vm) { $q = New-P9SStopRequestDoc $c; $q.acpi[$Vm] = [DateTime]::UtcNow.ToString('o'); Save-P9SStopRequest $c $q }
function Set-Requesting($c, [string]$Source, [switch]$Inflight) {
    $b = Get-P9SAcpiBundle $c
    $b.entry.acpi_used = 1; $b.entry.acpi_source = $Source; $b.entry.phase = 'SHUTDOWN_REQUESTING'; $b.entry.deadline_utc = (Get-P9SUtcNow $c).AddSeconds(180).ToString('o')
    if ($Inflight) { $b.session.inflight = [ordered]@{ action = 'acpi.shutdown'; vm = [string]$b.entry.target_vm; pid = 999999; start_ticks = 1; utc = [DateTime]::UtcNow.ToString('o') } }
    if (-not (Save-P9SAcpiState $c $b -BeforeDispatch)) { throw 'write-ahead not written' }
}
Case 'I01' 'an emergency request without a recorded end keeps its stop request: session.close waits until the VM is seen off, after a restart and during the watch, so no second request follows' {
    $c = New-VCtx; $null = Finished-Debian $c
    Set-Recorded $c 'P9S-U24'
    $c2 = New-VCtx -Dir $c.Dir -Vms @{ 'P9S-U24' = 'running' }
    $m1 = Emerg $c2 'P9S-U24'; $cl = Act $c2 @{ Action = 'session.close' }; $m2 = Emerg $c2 'P9S-U24'
    Expect ((Has $m1 'ACPI_ALREADY_REQUESTED') -and $cl.outcome -eq 'REFUSED' -and (Has $cl 'EMERGENCY_UNRESOLVED') -and (Test-Path -LiteralPath $c2.StopRequestPath) -and (Has $m2 'ACPI_ALREADY_REQUESTED') -and $c2.Calls.Count -eq 0) "after a restart: $(Why $m1) / close $(Why $cl) / $(Why $m2)"
    $c2.VmTable['P9S-U24'] = 'aborted'
    $cl2 = Act $c2 @{ Action = 'session.close' }
    Expect ($cl2.outcome -eq 'DONE' -and -not (Test-Path -LiteralPath $c2.StopRequestPath)) "once the VM has ended: $(Why $cl2)"
    # another coordinator closes the session while this emergency watches
    $d = New-VCtx; $null = Finished-Debian $d
    $d.VmTable['P9S-D12'] = 'running'
    $d2 = New-VCtx -Dir $d.Dir -Vms @{ 'P9S-D12' = 'running' }
    $box = @{ out = $null }
    $d.Prov.onSleep = { if (-not $box.out) { $box.out = @((Act $d2 @{ Action = 'session.close' }), (Emerg $d2 'P9S-D12')) } }.GetNewClosure()
    $m = Act $d @{ Action = 'emergency.acpi-shutdown'; Vm = 'P9S-D12'; Seconds = 10 }
    $q = Read-P9SStopRequestState $d
    Expect ($m.dispatched -eq 1 -and $box.out[0].outcome -eq 'REFUSED' -and (Has $box.out[0] 'EMERGENCY_UNRESOLVED') -and (Has $box.out[1] 'ACPI_ALREADY_REQUESTED') -and $d2.Calls.Count -eq 0) "during the watch: close $(Why $box.out[0]) / emergency $(Why $box.out[1])"
    Expect ($q.ok -and $q.doc.acpi.Contains('P9S-D12') -and $q.doc.unresolved.Contains('P9S-D12')) 'the record of the request was lost'
}
Case 'I02' 'a dispatch never confirmed stays unconfirmed: a confirmation that times out and the finalization keep SHUTDOWN_REQUESTING; the power-off is then OFF_REQUEST_UNCONFIRMED, never a late shutdown' {
    foreach ($order in 'confirm-first', 'finalize-first') {
        $x = Started -Target 'P9S-U24'
        Set-Requesting $x.c 'validation' -Inflight
        $c = New-VCtx -Dir $x.c.Dir -Vms @{ 'P9S-U24' = 'running' }
        if ($order -eq 'confirm-first') { $k1 = AConfirm $c 10; Expect ($k1.outcome -eq 'TIMEOUT' -and (Entry $c $x.a).phase -eq 'SHUTDOWN_REQUESTING') "${order}: confirmation while on: $(Why $k1) $((Entry $c $x.a).phase)" }
        $fin = AFinal $c
        Expect ($fin.outcome -eq 'DONE' -and (Entry $c $x.a).phase -eq 'SHUTDOWN_REQUESTING' -and (Entry $c $x.a).outcome -eq 'ACPI_VALIDATION_UNRESOLVED') "${order}: finalize: $(Why $fin) $((Entry $c $x.a).phase)"
        $c.VmTable['P9S-U24'] = 'off'
        $k2 = AConfirm $c 10; $e = Entry $c $x.a
        Expect ($k2.outcome -eq 'DONE' -and $e.phase -eq 'OFF_REQUEST_UNCONFIRMED' -and (Get-P9SAcpiOutcome $e) -eq 'OFF_AFTER_UNCONFIRMED_REQUEST' -and -not (Has $k2 'SHUTDOWN_AFTER_DEADLINE') -and $c.Calls.Count -eq 0) "${order}: power-off: $(Why $k2) $($e.phase)"
    }
    # a request the emergency path recorded for the validation
    $y = Started -Target 'P9S-U24'
    Set-Requesting $y.c 'emergency'; Set-Recorded $y.c 'P9S-U24'
    $d = New-VCtx -Dir $y.c.Dir -Vms @{ 'P9S-U24' = 'running' }
    $k = AConfirm $d 10; $d.VmTable['P9S-U24'] = 'off'; $k3 = AConfirm $d 10
    Expect ($k.outcome -eq 'TIMEOUT' -and $k3.outcome -eq 'DONE' -and (Entry $d $y.a).phase -eq 'OFF_REQUEST_UNCONFIRMED' -and $d.Calls.Count -eq 0) "emergency record: $(Why $k) / $(Why $k3) $((Entry $d $y.a).phase)"
}
Case 'I03' 'a request recorded before its dispatch is reported as recorded, not as sent, until its dispatch is confirmed' {
    $x = Started -Target 'P9S-U24'
    Set-Requesting $x.c 'validation'
    $m = Emerg $x.c 'P9S-U24'; $t = $m.lines -join ' '
    Expect ((Has $m 'ACPI_ALREADY_REQUESTED') -and $t -match 'dispatch is not confirmed' -and $t -notmatch 'already sent') "unconfirmed: $t"
    $y = Sent -Target 'P9S-U24'; $m2 = Emerg $y.c 'P9S-U24'
    Expect ((Has $m2 'ACPI_ALREADY_REQUESTED') -and ($m2.lines -join ' ') -match 'already sent its request') "confirmed: $($m2.lines -join ' ')"
    $d = New-VCtx -Vms @{ 'P9S-D12' = 'running' }
    Set-Recorded $d 'P9S-D12'
    $m3 = Emerg $d 'P9S-D12'; $st = Act $d @{ Action = 'emergency.status' }
    $t3 = $m3.lines -join ' '; $ts = $st.lines -join ' '
    Expect ((Has $m3 'ACPI_ALREADY_REQUESTED') -and $t3 -match 'recorded before its dispatch' -and $t3 -notmatch 'was already sent' -and $ts -match 'ACPI requests recorded for P9S-D12' -and $d.Calls.Count -eq 0) "stop request: $t3 / status: $ts"
}
Case 'I04' 'a finished validation is never finalized again: after an unresolved emergency, also under STOP, acpi.finalize closes the session record and keeps the entry as it is' {
    $c = New-VCtx; $a = Finished-Debian $c
    $deb = EntryJson $c $a; $f0 = LFiles $c
    $c.VmTable['P9S-D12'] = 'running'; $c.Prov.mon = $STOP
    $m = Act $c @{ Action = 'emergency.acpi-shutdown'; Vm = 'P9S-D12'; Seconds = 10 }
    $s1 = Sess $c
    Expect ($m.outcome -eq 'TIMEOUT' -and $s1.lifecycle -eq 'EMERGENCY_UNRESOLVED' -and $s1.halt.status -eq 'STOP_REQUIRED') "emergency: $(Why $m), session $($s1.lifecycle) halt $($s1.halt.status)"
    $c.VmTable['P9S-D12'] = 'off'
    $cl = Act $c @{ Action = 'session.close' }; $fin = AFinal $c; $cl2 = Act $c @{ Action = 'session.close' }
    Expect ((Has $cl 'ACPI_VALIDATION_USE_FINALIZE') -and $fin.outcome -eq 'DONE' -and ($fin.lines -join ' ') -match 'finalized before' -and $cl2.outcome -eq 'DONE' -and -not (Test-Path -LiteralPath $c.StopRequestPath)) "close $(Why $cl) / finalize $(Why $fin) / close $(Why $cl2)"
    Expect ((EntryJson $c $a) -eq $deb -and (LFiles $c) -eq $f0) "the finished validation was rewritten: $(EntryJson $c $a)"
}
Case 'I05' 'an ABORT gives no request back: an aborted test session keeps the stop request while the VM runs, after a restart, during the watch and after the abort is acknowledged; an aborted validation keeps its request spent in the ledger after finalize and close' {
    # a coordinator that ended after recording its emergency request, in
    # an aborted test session and with the record the abort itself creates
    $readyB = M (Feed (NewMon 'run-B' $fxApproved) 0 300)
    foreach ($open in $true, $false) {
        $tag = if ($open) { 'aborted test session' } else { 'record created by the abort' }
        $c = New-VCtx -Mon $READYAPP -Vms @{ 'P9S-D12' = 'running' }
        if ($open) { $o = Act $c @{ Action = 'session.open'; Kind = 'Test' }; if ($o.outcome -ne 'DONE') { throw "setup failed: $(Why $o)" } }
        $c.Prov.mon = $ABORT; $null = Act $c @{ Action = 'wait'; Seconds = 5 }
        Set-Recorded $c 'P9S-D12'
        $c2 = New-VCtx -Dir $c.Dir -Mon $ABORT -Vms @{ 'P9S-D12' = 'running' }
        $cl = Act $c2 @{ Action = 'session.close' }; $m = Emerg $c2 'P9S-D12'; $s = Sess $c2
        Expect ($s.halt.status -eq 'ABORT_REQUIRED' -and $s.lifecycle -eq $(if ($open) { 'HALTED_ABORT' } else { 'CLOSED' }) -and $cl.outcome -eq 'REFUSED' -and (Has $cl 'EMERGENCY_UNRESOLVED') -and (Test-Path -LiteralPath $c2.StopRequestPath) -and (Has $m 'ACPI_ALREADY_REQUESTED') -and $c2.Calls.Count -eq 0) "${tag}, after a restart: session $($s.lifecycle) halt $($s.halt.status) / close $(Why $cl) / emergency $(Why $m)"
        $c2.Prov.mon = $readyB
        $ack = Act $c2 @{ Action = 'session.open'; Kind = 'Test'; AcknowledgeAbort = [string]$s.halt.abort_id }; $m2 = Emerg $c2 'P9S-D12'
        Expect ($ack.outcome -eq 'REFUSED' -and (Has $ack 'STOP_REQUEST_PENDING') -and -not (Sess $c2).halt.acknowledged -and (Has $m2 'ACPI_ALREADY_REQUESTED') -and $c2.Calls.Count -eq 0) "${tag}, abort acknowledged on a new monitor run: open $(Why $ack) / emergency $(Why $m2)"
        $c2.VmTable['P9S-D12'] = 'off'
        $cl2 = Act $c2 @{ Action = 'session.close' }
        Expect ($cl2.outcome -eq 'DONE' -and -not (Test-Path -LiteralPath $c2.StopRequestPath)) "${tag}, once the VM is off: $(Why $cl2)"
    }
    # the session is aborted while its emergency request is watched
    $d = New-VCtx -Mon $READYAPP -Vms @{ 'P9S-D12' = 'running' }
    $o = Act $d @{ Action = 'session.open'; Kind = 'Test' }; if ($o.outcome -ne 'DONE') { throw "setup failed: $(Why $o)" }
    $d.Prov.mon = $ABORT
    $d2 = New-VCtx -Dir $d.Dir -Mon $ABORT -Vms @{ 'P9S-D12' = 'running' }
    $box = @{ out = $null }
    $d.Prov.onSleep = { if (-not $box.out) { $box.out = @((Sess $d2), (Act $d2 @{ Action = 'session.close' }), (Emerg $d2 'P9S-D12')) } }.GetNewClosure()
    $w = Act $d @{ Action = 'emergency.acpi-shutdown'; Vm = 'P9S-D12'; Seconds = 10 }
    $q = Read-P9SStopRequestState $d
    Expect ($w.dispatched -eq 1 -and $box.out[0].lifecycle -eq 'HALTED_ABORT' -and $box.out[1].outcome -eq 'REFUSED' -and (Has $box.out[1] 'EMERGENCY_UNRESOLVED') -and (Has $box.out[2] 'ACPI_ALREADY_REQUESTED') -and $d2.Calls.Count -eq 0) "during the watch: session $($box.out[0].lifecycle) / close $(Why $box.out[1]) / emergency $(Why $box.out[2])"
    Expect ($q.ok -and $q.doc.acpi.Contains('P9S-D12') -and $q.doc.unresolved.Contains('P9S-D12')) 'the record of the request was lost'
    # an aborted validation whose request was recorded before its dispatch
    $y = Started -Target 'P9S-U24'
    Set-Requesting $y.c 'validation' -Inflight
    $v = New-VCtx -Dir $y.c.Dir -Mon $ABORT -Vms @{ 'P9S-U24' = 'running' }
    $null = AObs $v 10; $h = (Sess $v).halt
    $cl3 = Act $v @{ Action = 'session.close' }; $fin = AFinal $v; $cl4 = Act $v @{ Action = 'session.close' }
    $v.Prov.mon = $ELIG_B
    $m3 = Emerg $v 'P9S-U24'; $sh = AShut $v; $op = AOpen $v (New-Auth -Target 'P9S-U24'); $e = Entry $v $y.a
    Expect ($h.status -eq 'ABORT_REQUIRED' -and (Has $cl3 'ACPI_VALIDATION_USE_FINALIZE') -and $fin.outcome -eq 'DONE' -and $cl4.outcome -eq 'DONE') "abort: halt $($h.status) / close $(Why $cl3) / finalize $(Why $fin) / close $(Why $cl4)"
    Expect ((Has $m3 'ACPI_ALREADY_REQUESTED') -and $sh.outcome -eq 'REFUSED' -and (Has $op 'ACPI_VALIDATION_UNRESOLVED_PREVIOUS') -and [int]$e.acpi_used -eq 1 -and $e.halt -eq 'ABORT_REQUIRED' -and $v.Calls.Count -eq 0) "after the abort: emergency $(Why $m3) / request $(Why $sh) / open $(Why $op) / entry $($e.phase) used $($e.acpi_used) halt $($e.halt)"
}
# session.close reads the stop request, observes its VMs and removes it
# holding the coordinator lock; every record of a request and every
# dispatch takes the same lock. Another coordinator acting inside that
# window is run here at the two points between the check and the removal.
Case 'I06' 'session.close decides and removes under the coordinator lock: an emergency or a session opened between its check and its removal records and sends nothing; the next request is recorded before its dispatch and sent once' {
    # the check observes a VM whose emergency ended unresolved, now off
    $c = New-VCtx -Mon $READYAPP -Vms @{ 'P9S-U24' = 'running' }
    $o = Act $c @{ Action = 'session.open'; Kind = 'Test' }; if ($o.outcome -ne 'DONE') { throw "setup failed: $(Why $o)" }
    $q0 = New-P9SStopRequestDoc $c; $q0.acpi['P9S-D12'] = $q0.utc; $q0.unresolved['P9S-D12'] = $q0.utc; Save-P9SStopRequest $c $q0
    $text0 = [IO.File]::ReadAllText($c.StopRequestPath)
    $c2 = New-VCtx -Dir $c.Dir -Mon $READYAPP -Vms @{ 'P9S-U24' = 'running' }
    $box = @{ out = $null; rec = $null }
    $c.Prov.onVmRead = { param($vm) if ($vm -ceq 'P9S-D12' -and -not $box.out) { $box.out = @((Emerg $c2 'P9S-U24'), (Emerg $c2 'P9S-D12'), (Act $c2 @{ Action = 'session.open'; Kind = 'Test' }), [IO.File]::ReadAllText($c2.StopRequestPath)) } }.GetNewClosure()
    $cl = Act $c @{ Action = 'session.close' }
    Expect ($cl.outcome -eq 'DONE' -and $box.out -and (Has $box.out[0] 'EMERGENCY_LOCK_TIMEOUT') -and (Has $box.out[1] 'ACPI_ALREADY_REQUESTED') -and (Has $box.out[2] 'COORDINATOR_BUSY') -and $box.out[3] -ceq $text0 -and $c2.Calls.Count -eq 0) "while the record is checked: close $(Why $cl) / $(if ($box.out) { "emergency $(Why $box.out[0]) / emergency $(Why $box.out[1]) / open $(Why $box.out[2])" } else { 'no VM observed' })"
    $c2.Prov.onAcpi = { $box.rec = (Read-P9SStopRequestState $c2).doc.acpi.Contains('P9S-U24') }.GetNewClosure()
    $m = Emerg $c2 'P9S-U24'; $cl2 = Act $c2 @{ Action = 'session.close' }; $m2 = Emerg $c2 'P9S-U24'
    Expect ($m.dispatched -eq 1 -and $box.rec -and $cl2.outcome -eq 'REFUSED' -and (Has $m2 'ACPI_ALREADY_REQUESTED') -and (Count $c2 'vm.acpi') -eq 1) "after the close: $(Why $m) recorded first $($box.rec) / close $(Why $cl2) / $(Why $m2)"
    # no record at the check: the window is the session write before the
    # removal. A stop request raised there, before any request is
    # recorded, may go with the session; the request itself is recorded
    # under the lock before its dispatch.
    $d = New-VCtx -Mon $READYAPP -Vms @{ 'P9S-U24' = 'running' }
    $o = Act $d @{ Action = 'session.open'; Kind = 'Test' }; if ($o.outcome -ne 'DONE') { throw "setup failed: $(Why $o)" }
    $d2 = New-VCtx -Dir $d.Dir -Mon $READYAPP -Vms @{ 'P9S-U24' = 'running' }
    $probe = @{ out = $null; rec = $null }
    # in this case only, the session write runs the other coordinator first
    $save0 = ${function:Save-P9SSession}
    function Save-P9SSession($Ctx, $Session) { if (-not $probe.out -and $Session.lifecycle -eq 'CLOSED') { $probe.out = @((Emerg $d2 'P9S-U24'), (Act $d2 @{ Action = 'session.open'; Kind = 'Test' })) }; & $save0 $Ctx $Session }
    $cl3 = Act $d @{ Action = 'session.close' }
    Expect ($cl3.outcome -eq 'DONE' -and $probe.out -and (Has $probe.out[0] 'EMERGENCY_LOCK_TIMEOUT') -and (Has $probe.out[1] 'COORDINATOR_BUSY') -and $d2.Calls.Count -eq 0) "while the session is written: close $(Why $cl3) / $(if ($probe.out) { "emergency $(Why $probe.out[0]) / open $(Why $probe.out[1])" } else { 'no session write' })"
    $d2.Prov.onAcpi = { $probe.rec = (Read-P9SStopRequestState $d2).doc.acpi.Contains('P9S-U24') }.GetNewClosure()
    $m3 = Emerg $d2 'P9S-U24'; $cl4 = Act $d2 @{ Action = 'session.close' }; $m4 = Emerg $d2 'P9S-U24'
    Expect ($m3.dispatched -eq 1 -and $probe.rec -and $cl4.outcome -eq 'REFUSED' -and (Has $m4 'ACPI_ALREADY_REQUESTED') -and (Count $d2 'vm.acpi') -eq 1) "after the close: $(Why $m3) recorded first $($probe.rec) / close $(Why $cl4) / $(Why $m4)"
}

# ---- low memory: no new workload, observation and recovery stay available ---------------------
$MEM8192 = M (Feed (NewMon 'run-A') 0 300 50 8192)
$MEM8191 = M (Feed (NewMon 'run-A') 0 300 50 8191)
Case 'M01' 'enough memory permits the start: above the minimum and exactly at it' {
    foreach ($mon in $ELIG, $MEM8192) {
        $c = New-VCtx; $a = New-Auth -Target 'P9S-U24'; $null = AOpen $c $a
        $c.Prov.mon = $mon
        $s = AStart $c
        Expect ($s.outcome -eq 'DISPATCHED' -and (Entry $c $a).startups_used -eq 1) "start at $($mon.doc.memory.avail_mb) MiB: $(Why $s)"
    }
}
Case 'M02' 'too little memory refuses the start: one MiB below the minimum and further, nothing dispatched or spent' {
    foreach ($mon in $MEM8191, $LOWMEM) {
        $c = New-VCtx; $a = New-Auth -Target 'P9S-U24'; $null = AOpen $c $a
        $c.Prov.mon = $mon
        $s = AStart $c
        Expect ($s.outcome -eq 'REFUSED' -and (Has $s 'MEMORY_BELOW_PROVISIONAL_MINIMUM') -and (Count $c 'vm.start') -eq 0 -and (Entry $c $a).startups_used -eq 0) "start at $($mon.doc.memory.avail_mb) MiB: $(Why $s)"
    }
}
Case 'M03' 'below the minimum after the start the VM stays observable: status, wait-off and session status read it; the observation step reports it and stops' {
    $x = Started -Target 'P9S-U24'; $c = $x.c; $c.Prov.mon = $LOWMEM
    $r0 = $c.Prov.vmreads
    $st = Act $c @{ Action = 'emergency.status' }; $ob = AObs $c 10
    $w = Act $c @{ Action = 'emergency.wait-off'; Vm = 'P9S-U24'; Seconds = 4 }; $ss = Act $c @{ Action = 'session.status' }
    Expect ($st.outcome -eq 'DONE' -and ($st.lines -join ' ') -match 'P9S-U24 running') "status: $($st.lines -join ' | ')"
    Expect ($ob.outcome -eq 'REFUSED' -and (Has $ob 'MEMORY_BELOW_PROVISIONAL_MINIMUM') -and ($ob.lines -join ' ') -match 'begin: P9S-U24 running' -and ($ob.lines -join ' ') -match 'stays available') "observe: $(Why $ob) $($ob.lines -join ' | ')"
    Expect ($w.outcome -eq 'TIMEOUT' -and $ss.outcome -eq 'DONE' -and $c.Prov.vmreads -gt $r0 -and (Count $c 'vm.acpi') -eq 0) "wait-off $(Why $w) / session.status $(Why $ss)"
}
Case 'M04' 'low memory does not stop the monitor: every measurement keeps coming, the STOP and ABORT limits are unchanged, READY still needs approved paging values' {
    $d = Feed (NewMon 'run-L') 0 300 50 1500 40
    Expect ($d.memory.avail_mb -eq 1500 -and $d.commit.valid -and (Test-Finite $d.commit.pct) -and (Test-Finite $d.memory.pages_in) -and (Test-Finite $d.memory.pages_out) -and (Test-Finite $d.memory.pagefile_used_mb) -and (Test-Finite $d.memory.pagefile_pct)) "measurements: $($d.memory | ConvertTo-Json -Compress)"
    Expect ((Test-Finite $d.disks.system.read_ms) -and (Test-Finite $d.disks.external.write_ms) -and $d.sampling.samples -eq 61 -and $d.sampling.invalid_samples -eq 0 -and $d.sampling.last_gap_s -le 12 -and $d.gate.qualified) "sampling: $($d.sampling | ConvertTo-Json -Compress)"
    Expect ($d.status -eq 'UNCALIBRATED' -and -not $d.latched) "low memory alone: $($d.status) $($d.reasons -join ',')"
    $lim = $d.limits
    Expect ($lim.interval_s -eq 5 -and $lim.max_gap_s -eq 12 -and $lim.pretest_seconds -eq 300 -and $lim.sustained_pct -eq 95 -and $lim.sustained_seconds -eq 30 -and $lim.critical_pct -eq 97) "limits: $($lim | ConvertTo-Json -Compress)"
    $m3 = NewMon 'run-L'; $null = Feed $m3 0 300 50 1500; $s = Feed $m3 305 335 95 1500
    $m4 = NewMon 'run-L'; $null = Feed $m4 0 300 50 1500; $ab = Feed $m4 305 305 97 1500
    Expect ($s.status -eq 'STOP_REQUIRED' -and $ab.status -eq 'ABORT_REQUIRED' -and $s.memory.avail_mb -eq 1500 -and $ab.memory.avail_mb -eq 1500) "STOP $($s.status) / ABORT $($ab.status)"
    $m5 = NewMon 'run-L'; $null = Feed $m5 0 300 50 1500; $gap = Step-Monitor -State $m5 -Sample (S 313 50 1500)
    $g = Test-SafetyState -Text (JM (M $gap) $NOW $F) -NowQpc $NOW -QpcFrequency $F -PagingStop NotRequired -IsMonitorAlive { param($a, $b) $true }
    Expect ($gap.sampling.gaps_over_limit -eq 1 -and $gap.status -eq 'UNKNOWN' -and @($g.reasons) -contains 'SAMPLING_GAP') "13 s gap: $($gap.status) $(@($g.reasons) -join ',')"
    $ready = Feed (NewMon 'run-L' $fxApproved) 0 300 50 16000
    Expect ($ready.status -eq 'READY' -and $d.paging.calibration -eq 'UNCALIBRATED') "READY with approved values: $($ready.status); kit values $($d.paging.calibration)"
}
Case 'M05' 'below the minimum a shutdown is still confirmed: acpi.confirm-off records the observed power-off and finalize closes the validation' {
    $x = Sent -Target 'P9S-U24'; $c = $x.c; $c.Prov.mon = $LOWMEM
    $c.VmTable['P9S-U24'] = 'off'
    $k = AConfirm $c 10; $fin = AFinal $c
    Expect ($k.outcome -eq 'DONE' -and (Entry $c $x.a).phase -eq 'OFF_AFTER_DEADLINE' -and $fin.outcome -eq 'DONE' -and (Entry $c $x.a).outcome -eq 'SHUTDOWN_LATE' -and (Sess $c).lifecycle -eq 'CLOSED') "confirm $(Why $k) / finalize $(Why $fin)"
}
Case 'M06' 'refusals below the minimum write nothing: the budgets, the ledger revision and the session stay as they were' {
    $c = New-VCtx; $a = New-Auth -Target 'P9S-U24'; $null = AOpen $c $a
    $c.Prov.mon = $LOWMEM
    $f0 = LFiles $c; $s0 = Get-Content -Raw -LiteralPath $c.SessionPath
    $r = @((AStart $c), (AStart $c))
    Expect (@($r | Where-Object { $_.outcome -eq 'REFUSED' -and (Has $_ 'MEMORY_BELOW_PROVISIONAL_MINIMUM') }).Count -eq 2 -and (LFiles $c) -eq $f0 -and (Get-Content -Raw -LiteralPath $c.SessionPath) -eq $s0 -and $c.Calls.Count -eq 0) "before the start: $(@($r | ForEach-Object { Why $_ }) -join ' / ')"
    $c.Prov.mon = $ELIG; $s = AStart $c; $c.Prov.mon = $LOWMEM
    $f1 = LFiles $c
    $o = AObs $c 10
    Expect ($s.outcome -eq 'DISPATCHED' -and $o.outcome -eq 'REFUSED' -and (LFiles $c) -eq $f1 -and (Entry $c $a).startups_used -eq 1 -and (Entry $c $a).acpi_used -eq 0) "after the start: $(Why $s) / $(Why $o)"
}
Case 'M07' 'below the minimum nothing becomes READY: no test session, no normal action, the gate stays uncalibrated' {
    $x = Started -Target 'P9S-U24'; $c = $x.c; $c.Prov.mon = $LOWMEM
    $g = Get-P9SGate $c
    $st = Act $c @{ Action = 'emergency.status' }; $so = Act $c @{ Action = 'session.open'; Kind = 'Test' }; $k = Act $c @{ Action = 'guest.key'; Vm = 'P9S-U24'; Key = 'Enter' }
    Expect (-not $g.ok -and $g.status -eq 'UNCALIBRATED' -and ($st.lines -join ' ') -notmatch 'READY for test actions' -and $so.outcome -eq 'REFUSED' -and $k.outcome -eq 'REFUSED' -and @($c.Calls | Where-Object { $_.kind -like 'guest.*' }).Count -eq 0) "gate $($g.status) / test session $(Why $so) / key $(Why $k)"
}
Case 'M08' 'below the minimum there is no second request: both paths refuse once the one request is spent' {
    $x = Sent -Target 'P9S-U24'; $c = $x.c; $c.Prov.mon = $LOWMEM
    $s = AShut $c; $m = Emerg $c 'P9S-U24'
    Expect ((Has $s 'ACPI_BUDGET_EXHAUSTED') -and (Has $m 'ACPI_ALREADY_REQUESTED') -and (Count $c 'vm.acpi') -eq 1) "validation $(Why $s) / emergency $(Why $m)"
}
Case 'M09' 'below the minimum the unused request is checked on its own: shown unused, offered by dry runs, then sent once by the validation' {
    $x = Started -Target 'P9S-U24'; $c = $x.c; $c.Prov.mon = $LOWMEM; $c.Prov.offAfter = 1
    $ss = Act $c @{ Action = 'session.status' }; $d = Act $c @{ Action = 'acpi.shutdown'; DryRun = $true }; $dm = Act $c @{ Action = 'emergency.acpi-shutdown'; Vm = 'P9S-U24'; DryRun = $true }
    Expect (($ss.lines -join ' ') -match 'ACPI requests 0 of 1' -and $d.outcome -eq 'DRYRUN' -and $dm.outcome -eq 'DRYRUN' -and ($dm.lines -join ' ') -notmatch 'no request could be sent now' -and $c.Calls.Count -eq 1) "status / dry runs: $(Why $d) / $(Why $dm)"
    $s = AShut $c
    Expect ($s.outcome -eq 'DONE' -and (Count $c 'vm.acpi') -eq 1 -and (Entry $c $x.a).phase -eq 'OFF_CONFIRMED') "the request below the minimum: $(Why $s)"
}
Case 'M10' 'below the minimum with an unreadable ledger nothing is sent: low memory grants no request' {
    $x = Started -Target 'P9S-U24'; $c = $x.c; $c.Prov.mon = $LOWMEM
    Break-Ledger $c 'notjson'
    $s = AShut $c; $m = Emerg $c 'P9S-U24'
    Expect ((Has $s 'ACPI_LEDGER_UNREADABLE') -and (Has $m 'SHUTDOWN_HISTORY_UNCERTAIN') -and $m.dispatched -eq 0 -and (Count $c 'vm.acpi') -eq 0) "validation $(Why $s) / emergency $(Why $m)"
}
function Test-HaltDiagnostics($Mon, [string]$Label) {
    $x = Sent -Target 'P9S-U24'; $c = $x.c; $c.Prov.mon = $Mon
    $r0 = $c.Prov.vmreads
    $st = Act $c @{ Action = 'emergency.status' }; $w = Act $c @{ Action = 'emergency.wait-off'; Vm = 'P9S-U24'; Seconds = 4 }; $ss = Act $c @{ Action = 'session.status' }
    $c.VmTable['P9S-U24'] = 'off'
    $k = AConfirm $c 10
    Expect ($st.outcome -eq 'DONE' -and ($st.lines -join ' ') -match 'P9S-U24 running' -and $w.outcome -eq 'TIMEOUT' -and $ss.outcome -eq 'DONE' -and $c.Prov.vmreads -gt $r0) "${Label}: status $(Why $st) / wait-off $(Why $w) / session $(Why $ss)"
    Expect ($k.outcome -eq 'DONE' -and (Entry $c $x.a).phase -eq 'OFF_AFTER_DEADLINE' -and (Count $c 'vm.acpi') -eq 1) "${Label}: confirmation $(Why $k)"
}
Case 'M11' 'a STOP leaves the read-only diagnostics: status, wait-off, session status and the confirmation still read the VM' { Test-HaltDiagnostics $STOP 'STOP' }
Case 'M12' 'an ABORT leaves the read-only diagnostics: status, wait-off, session status and the confirmation still read the VM' { Test-HaltDiagnostics $ABORT 'ABORT' }
Case 'M13' 'a stale or missing monitor state is named, refuses every progressing step, and leaves the read-only paths and the one emergency request' {
    $x = Started -Target 'P9S-U24'; $c = $x.c; $c.Prov.age = 30
    $s = AShut $c; $o = AObs $c 5; $st = Act $c @{ Action = 'emergency.status' }
    Expect ((Has $s 'STATE_STALE') -and ($s.lines -join ' ') -match 'emergency\.acpi-shutdown' -and (Has $o 'STATE_STALE') -and ($st.lines -join ' ') -match 'STATE_STALE' -and ($st.lines -join ' ') -match 'P9S-U24 running') "stale: $(Why $s) / $(Why $o)"
    $c.Prov.mon = $null
    $s2 = AShut $c; $st2 = Act $c @{ Action = 'emergency.status' }
    Expect ((Has $s2 'STATE_EMPTY') -and ($st2.lines -join ' ') -match 'STATE_MISSING' -and (Count $c 'vm.acpi') -eq 0) "missing: $(Why $s2)"
    $c.Prov.offAfter = 0
    $m = Emerg $c 'P9S-U24'
    Expect ($m.outcome -eq 'DONE' -and (Count $c 'vm.acpi') -eq 1 -and (Entry $c $x.a).acpi_source -eq 'emergency' -and (Entry $c $x.a).phase -eq 'OFF_CONFIRMED') "emergency without a monitor: $(Why $m)"
}
Case 'M14' 'a missing or unreadable VM state is never taken for off: no request, no confirmation, no resolved outcome' {
    foreach ($bad in 'unreadable', $null) {
        $x = Sent -Target 'P9S-U24'; $c = $x.c
        $c.VmTable['P9S-U24'] = $bad
        $w = Act $c @{ Action = 'emergency.wait-off'; Vm = 'P9S-U24'; Seconds = 4 }; $k = AConfirm $c 10; $fin = AFinal $c
        Expect ($w.outcome -eq 'TIMEOUT' -and $k.outcome -ne 'DONE' -and (Has $fin 'ACPI_VALIDATION_UNRESOLVED') -and (Entry $c $x.a).outcome -eq 'ACPI_VALIDATION_UNRESOLVED') "'$bad': wait-off $(Why $w) / confirm $(Why $k) / finalize $(Why $fin)"
        $d = New-VCtx -Vms @{ 'P9S-D12' = $bad }
        $m = Emerg $d 'P9S-D12'
        Expect ($m.outcome -eq 'REFUSED' -and (Has $m 'VM_NOT_RUNNING:unreadable') -and $d.Calls.Count -eq 0) "'$bad': emergency $(Why $m)"
    }
}
Case 'M15' 'a finalization that cannot be written keeps the session and the validation open; a damaged ledger refuses it before anything is written' {
    $x = Sent -Target 'P9S-U24'; $c = $x.c
    $c.Prov.failLedgerAt = $c.Prov.ledgerWrites + 1
    $fin = AFinal $c
    $c.Prov.failLedgerAt = 0
    Expect ($fin.outcome -eq 'REFUSED' -and (Has $fin 'EVIDENCE_WRITE_FAILED') -and (Sess $c).lifecycle -eq 'ACTIVE' -and -not (Entry $c $x.a).outcome) "finalize: $(Why $fin)"
    $o = AOpen $c (New-Auth -Target 'P9S-D12'); $so = Act $c @{ Action = 'session.open'; Kind = 'Test' }
    Expect ((Has $o 'ACPI_VALIDATION_UNRESOLVED_PREVIOUS') -and $so.outcome -eq 'REFUSED' -and $c.Calls.Count -eq 2) "afterwards: open $(Why $o) / test session $(Why $so)"
    Break-Ledger $c 'notjson'; $f0 = LFiles $c; $s0 = Get-Content -Raw -LiteralPath $c.SessionPath
    $f2 = AFinal $c
    Expect ((Has $f2 'ACPI_LEDGER_UNREADABLE') -and (LFiles $c) -eq $f0 -and (Get-Content -Raw -LiteralPath $c.SessionPath) -eq $s0) "damaged: $(Why $f2)"
}
Case 'M16' 'scenario, readable ledger: memory falls below the minimum after the start; no second start, the VM stays observable, the unused request is sent once and the power-off recorded; never READY' {
    $x = Started -Target 'P9S-U24'; $c = $x.c; $c.Prov.mon = $LOWMEM
    $s = AStart $c; $st = Act $c @{ Action = 'emergency.status' }; $ss = Act $c @{ Action = 'session.status' }
    Expect ((Has $s 'ACPI_STARTUP_BUDGET_EXHAUSTED') -and (Has $s 'MEMORY_BELOW_PROVISIONAL_MINIMUM') -and (Count $c 'vm.start') -eq 1) "second start: $(Why $s)"
    Expect (($st.lines -join ' ') -match 'P9S-U24 running' -and ($ss.lines -join ' ') -match 'ACPI requests 0 of 1') "observation: $($st.lines -join ' | ')"
    $c.Prov.offAfter = 1
    $m = Emerg $c 'P9S-U24'; $fin = AFinal $c
    Expect ($m.outcome -eq 'DONE' -and (Count $c 'vm.acpi') -eq 1 -and (Entry $c $x.a).acpi_used -eq 1 -and (Entry $c $x.a).acpi_source -eq 'emergency') "emergency: $(Why $m)"
    Expect ($fin.outcome -eq 'DONE' -and (Entry $c $x.a).outcome -eq 'OFF_AFTER_EMERGENCY_ACPI' -and ($fin.lines -join ' ') -match 'final state P9S-U24 off' -and (Get-P9SGate $c).status -ne 'READY') "finalize: $(Why $fin) $($fin.lines -join ' | ')"
}
Case 'M17' 'scenario, damaged ledger: memory below the minimum and the ledger unreadable; nothing is sent, the state stays unresolved and observable, never READY' {
    $x = Started -Target 'P9S-U24'; $c = $x.c; $c.Prov.mon = $LOWMEM
    Break-Ledger $c 'truncated'
    $s = AStart $c; $sh = AShut $c; $m = Emerg $c 'P9S-U24'; $st = Act $c @{ Action = 'emergency.status' }; $fin = AFinal $c
    Expect ((Has $s 'ACPI_LEDGER_UNREADABLE') -and (Has $sh 'ACPI_LEDGER_UNREADABLE') -and (Has $m 'SHUTDOWN_HISTORY_UNCERTAIN') -and (Has $m 'EMERGENCY_UNRESOLVED') -and (Count $c 'vm.acpi') -eq 0 -and (Count $c 'vm.start') -eq 1) "start $(Why $s) / request $(Why $sh) / emergency $(Why $m)"
    Expect (($st.lines -join ' ') -match 'P9S-U24 running' -and (Has $fin 'ACPI_LEDGER_UNREADABLE') -and (Sess $c).lifecycle -eq 'EMERGENCY_UNRESOLVED' -and (Get-P9SGate $c).status -ne 'READY') "status / finalize $(Why $fin) / session $((Sess $c).lifecycle)"
    $c.VmTable['P9S-U24'] = 'off'
    $w = Act $c @{ Action = 'emergency.wait-off'; Vm = 'P9S-U24'; Seconds = 4 }; $f2 = AFinal $c
    Expect ($w.outcome -eq 'DONE' -and (Sess $c).expected_vm['P9S-U24'] -eq 'off' -and (Has $f2 'ACPI_LEDGER_UNREADABLE') -and (Sess $c).lifecycle -ne 'CLOSED') "powered off, history still unknown: $(Why $w) / $(Why $f2)"
}
foreach ($e in $script:events) { try { $null = $e.Set() } catch {} }
$pass = @($results | Where-Object result -eq 'PASS').Count
$fail = @($results | Where-Object result -eq 'FAIL').Count
$notRun = @($results | Where-Object result -ne 'PASS' | Where-Object result -ne 'FAIL').Count
"== $pass passed, $fail failed, $notRun not run"
$results | ConvertTo-Json -Depth 3 | Set-Content -LiteralPath (Join-Path $OutDir 'results.json') -Encoding utf8
$results | ForEach-Object { '{0} {1} {2} ({3} ms){4}' -f $_.result, $_.id, $_.name, $_.ms, $(if ($_.detail) { ' :: ' + $_.detail } else { '' }) } | Set-Content -LiteralPath (Join-Path $OutDir 'results.txt') -Encoding utf8
if ($fail) { exit 1 } elseif ($notRun) { exit 3 } else { exit 0 }
