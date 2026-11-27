#Requires -Version 7.0
# Synthetic, deterministic tests of the stage C additions: local
# configuration, credential handling, the ACPI emergency shutdown, retired
# actions, the calibration preparation and the safety of the published
# source tree. No VM is started, no guest input or ACPI request is sent:
# in-process cases use mock dispatchers and synthetic monitor states built
# with the real policy of hostmon-lib.ps1; command-line cases use -DryRun
# or the simulated calibration; secrets are fixture files written here.
#   pwsh -NoProfile -File tests\stagec-tests.ps1 -OutDir <new folder>
param([Parameter(Mandatory)][string]$OutDir)
$ErrorActionPreference = 'Stop'
$kit = Split-Path $PSScriptRoot -Parent
. (Join-Path $kit 'hostmon-lib.ps1')
. (Join-Path $kit 'p9s-config.ps1')
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
function Has($res, [string]$code) { @($res.reasons) -contains $code }

# ---- monitor states (real policy, synthetic samples) ---------------------
$GiB = [math]::Pow(2, 30)
function S([double]$t, $commit = 50) {
    [pscustomobject]@{
        seq = 0; mono = $t; utc = 'x'; read_ok = $true; read_error = $null; committed_bytes = $(if ($null -eq $commit) { $null } else { [double]$commit * $GiB }); commit_limit_bytes = 100 * $GiB
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
$kitThr = Get-PagingThresholds -Path (Join-Path $kit 'paging-thresholds.json')
$F = 10000000L; $NOW = 1000L * $F
function J($doc, [double]$age = 1, $mode = 'Monitor') {
    ConvertTo-StateJson -Doc $doc -Monitor ([ordered]@{ pid = 4242; process_start_utc = 'x'; process_start_ticks = 1L; mode = $mode; paging_stop = 'Required'; thresholds_sha256 = 'x'; thresholds_approved = $true; test = $false; kit = 'test' }) -WrittenQpc ([long]($NOW - $age * $F)) -QpcFrequency $F -WrittenUtc 'x'
}
function NewMon([string]$run, $th = $fxApproved, [string]$mode = 'Monitor') { New-MonitorState -RunId $run -Config (New-HostMonConfig -PagingStop Required -Mode $mode -Thresholds $th) }
$mA = NewMon 'run-A'; $READY_A = J (Feed $mA 0 300)
$STOP_A = J (Feed $mA 305 335 95)
$mA2 = NewMon 'run-A'; $null = Feed $mA2 0 300; $ABORT_A = J (Feed $mA2 305 305 97)
$mA4 = NewMon 'run-A'; $null = Feed $mA4 0 300; $UNKNOWN_A = J (Step-Monitor -State $mA4 -Sample (S 305 $null))
$UNCAL_A = J (Feed (NewMon 'run-A' $kitThr) 0 300)

# ---- fixtures: configuration and secrets -----------------------------------
$fxDir = Join-Path $OutDir 'fixture'
$secDir = Join-Path $fxDir 'secrets'
New-Item -ItemType Directory -Path $secDir | Out-Null
$fxText = 'fixture passphrase for unit tests only'
Set-Content -LiteralPath (Join-Path $secDir 'P9S-D12-profile-passphrase.txt') -Value @('# test fixture', $fxText) -Encoding ascii
# malformed on purpose; QXZW cannot occur by chance in timestamps or ids
Set-Content -LiteralPath (Join-Path $secDir 'P9S-U24-profile-passphrase.txt') -Value 'Bad-Format QXZW' -Encoding ascii
function New-Config([hashtable]$Override = @{}) {
    $o = [ordered]@{ schema = 'p9s-host-config/1'; vms = [ordered]@{ debian = 'P9S-D12'; ubuntu = 'P9S-U24' }
        external_disk = [ordered]@{ drive_letter = 'X'; model = 'Example Disk'; probe_path = (Join-Path $fxDir 'lab') }
        secrets_dir = $secDir; evidence_dir = (Join-Path $fxDir 'evidence'); vm_config_reference = [ordered]@{ debian = $null; ubuntu = $null } }
    foreach ($k in $Override.Keys) { $o[$k] = $Override[$k] }
    $p = Join-Path $fxDir ("config-{0}.json" -f [guid]::NewGuid().ToString('N').Substring(0, 8))
    $o | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $p -Encoding ascii
    $p
}
$cfgPath = New-Config
$cfg = Read-P9SConfig -Path $cfgPath
$me = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
function Acl([string[]]$Sids, [bool]$Protected = $true) {
    [pscustomobject]@{ Owner = $me; AreAccessRulesProtected = $Protected
        Access = @(foreach ($s in $Sids) { [pscustomobject]@{ IdentityReference = $s; AccessControlType = 'Allow'; FileSystemRights = 'FullControl'; IsInherited = $false } }) }
}
$aclGood = { param($p) Acl @('S-1-5-18', $me) }.GetNewClosure()
$aclOpen = { param($p) Acl @('S-1-5-18', $me, 'S-1-1-0') }.GetNewClosure()

# ---- mock context -------------------------------------------------------------
function New-Recorder($kind, $list) { { param($vm, $arg) $list.Add([pscustomobject]@{ kind = $kind; vm = $vm; arg = $arg }) }.GetNewClosure() }
function New-AcpiRecorder($list, $tbl, $eff) {
    { param($vm) $list.Add([pscustomobject]@{ kind = 'vm.acpi'; vm = $vm; arg = $null }); switch ([string]$eff[$vm]) { 'throw' { throw 'VBoxManage exited with 1' } 'off' { $tbl[$vm] = 'off' } default {} } }.GetNewClosure()
}
function New-UnlockSender($list) { { param($vm, $groups) $list.Add([pscustomobject]@{ kind = 'guest.unlock'; vm = $vm; arg = $null; groups = @($groups).Count }) }.GetNewClosure() }
$script:n = 0
function New-MockCtx([object[]]$States, [hashtable]$Vms = @{}, [hashtable]$AcpiEffect = @{}, $Config = $cfg, $GetAcl = $aclGood) {
    $script:n++
    $dir = Join-Path $OutDir ('ctx-{0:D2}' -f $script:n); New-Item -ItemType Directory -Path $dir | Out-Null
    $calls = [System.Collections.Generic.List[object]]::new()
    $q = [System.Collections.Generic.Queue[object]]::new(); foreach ($x in $States) { $q.Enqueue($x) }
    $prov = @{ queue = $q; reads = 0 }
    $vmTable = @{ 'P9S-D12' = 'off'; 'P9S-U24' = 'off' }; foreach ($k in $Vms.Keys) { $vmTable[$k] = $Vms[$k] }
    $eff = @{}; foreach ($k in $AcpiEffect.Keys) { $eff[$k] = $AcpiEffect[$k] }
    $readState = { param($p) $prov.reads++; if ($prov.queue.Count -gt 1) { $x = $prov.queue.Dequeue() } else { $x = $prov.queue.Peek() }
        if ($null -eq $x) { [pscustomobject]@{ ok = $false; text = $null; error = 'missing' } } else { [pscustomobject]@{ ok = $true; text = $x; error = $null } } }.GetNewClosure()
    [pscustomobject]@{
        StatePath = Join-Path $dir 'state.json'; SessionPath = Join-Path $dir 'coordinator-session.json'; LockPath = Join-Path $dir 'coordinator.lock'
        StopRequestPath = Join-Path $dir 'coordinator-stop.request'; AuditPath = Join-Path $dir 'coordinator-audit.jsonl'; ThresholdsPath = Join-Path $kit 'paging-thresholds.json'
        ReadState = $readState; NowQpc = { $NOW }.GetNewClosure(); QpcFrequency = $F; IsMonitorAlive = { param($a, $b) $true }
        ReadVmState = { param($vm) $vmTable[$vm] }.GetNewClosure(); ReadVmConfig = { param($vm) @() }
        Dispatch = @{ 'vm.start' = (New-Recorder 'vm.start' $calls); 'guest.text' = (New-Recorder 'guest.text' $calls); 'guest.key' = (New-Recorder 'guest.key' $calls)
            'guest.unlock' = (New-P9SUnlockDispatcher -Config $Config -GetAcl $GetAcl -Sender (New-UnlockSender $calls) -ForbiddenRoots @()); 'vm.acpi' = (New-AcpiRecorder $calls $vmTable $eff) }
        Sleep = { param($ms) }; Pid = $PID; ProcessStartTicks = (Get-Process -Id $PID).StartTime.ToUniversalTime().Ticks
        EmergencyLockWaitMs = 1500; CalibrationHostChecks = { @() }; Config = $Config
        Calls = $calls; Prov = $prov; VmTable = $vmTable; Dir = $dir
    }
}
function Act($ctx, [hashtable]$r) { Invoke-P9SAction -Ctx $ctx -Request $r }
function Open($ctx) { Act $ctx @{ Action = 'session.open'; Kind = 'Test' } }
function Files($ctx) { (Get-ChildItem -LiteralPath $ctx.Dir -File | ForEach-Object { Get-Content -Raw -LiteralPath $_.FullName }) -join "`n" }

"== P9-S stage C synthetic tests, $((Get-Date).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ')), pwsh $($PSVersionTable.PSVersion)"

# ---- configuration ----------------------------------------------------------------
Case 'H01' 'local configuration: valid file read, invalid ones refused, missing one reported' {
    Expect ($cfg.ok -and ($cfg.vm_names -join ',') -eq 'P9S-D12,P9S-U24' -and $cfg.secrets_dir -eq $secDir) "valid: $($cfg.ok) $($cfg.errors -join ',')"
    $m = Read-P9SConfig -Path (Join-Path $fxDir 'nothing.json'); Expect (-not $m.ok -and @($m.errors) -contains 'CONFIG_MISSING') "missing: $($m.errors -join ',')"
    $d = Read-P9SConfig -Path (New-Config @{ vms = [ordered]@{ debian = 'P9S-D12'; ubuntu = 'P9S-D12' } }); Expect (-not $d.ok -and @($d.errors) -contains 'CONFIG_VM_NAMES') "duplicate VMs: $($d.errors -join ',')"
    $b = Read-P9SConfig -Path (New-Config @{ vms = [ordered]@{ debian = 'P9S D12; x'; ubuntu = 'P9S-U24' } }); Expect (-not $b.ok) 'bad VM name accepted'
    $r = Read-P9SConfig -Path (New-Config @{ secrets_dir = 'relative\secrets' }); Expect (-not $r.ok -and @($r.errors) -contains 'CONFIG_PATH_NOT_ABSOLUTE:secrets_dir') "relative path: $($r.errors -join ',')"
    Expect ((Get-P9SConfigPath -ConfigPath '' -ToolDir $kit) -like '*p9s-local.json') 'default path'
}

# ---- credentials ----------------------------------------------------------------------
Case 'H02' 'secret store in a session-log or temporary location is refused' {
    # each rule on its own: a hidden (dot) folder with no forbidden root, then the temporary folder by default
    $dot = [pscustomobject]@{ secrets_dir = (Join-Path $fxDir ('.cla' + 'ude\projects')) }
    $msg = ''; try { $null = Read-P9SSecret -Config $dot -Name 'P9S-D12-profile-passphrase' -GetAcl $aclGood -ForbiddenRoots @() } catch { $msg = $_.Exception.Message }
    Expect ($msg -eq 'SECRET_STORE_FORBIDDEN_LOCATION') "dot folder -> '$msg'"
    $tmp = [pscustomobject]@{ secrets_dir = (Join-Path ([IO.Path]::GetTempPath()) 'p9s-secrets') }
    $msg = ''; try { $null = Read-P9SSecret -Config $tmp -Name 'P9S-D12-profile-passphrase' -GetAcl $aclGood } catch { $msg = $_.Exception.Message }
    Expect ($msg -eq 'SECRET_STORE_FORBIDDEN_LOCATION') "temporary folder -> '$msg'"
}
Case 'H03' 'missing secret or unconfigured store fails safely, nothing typed' {
    $c = New-MockCtx @($READY_A) @{ 'P9S-U24' = 'running' } @{} (Read-P9SConfig -Path (New-Config @{ secrets_dir = (Join-Path $fxDir 'empty-store') }))
    New-Item -ItemType Directory -Path (Join-Path $fxDir 'empty-store') -Force | Out-Null
    $null = Open $c; $r = Act $c @{ Action = 'guest.unlock'; Vm = 'P9S-U24' }
    Expect ($r.outcome -eq 'FAILED' -and ($r.lines -join ' ') -match 'SECRET_MISSING' -and $c.Calls.Count -eq 0) "missing: $($r.outcome) $($r.lines -join ' ')"
    $c2 = New-MockCtx @($READY_A) @{ 'P9S-U24' = 'running' } @{} ([pscustomobject]@{ ok = $false; secrets_dir = $null; vm_names = @('P9S-D12', 'P9S-U24') })
    $null = Open $c2; $r2 = Act $c2 @{ Action = 'guest.unlock'; Vm = 'P9S-U24' }
    Expect ($r2.outcome -eq 'FAILED' -and ($r2.lines -join ' ') -match 'SECRET_STORE_NOT_CONFIGURED' -and $c2.Calls.Count -eq 0) "unconfigured: $($r2.outcome)"
}
Case 'H04' 'secret store readable by others is detected and refused' {
    $c = New-MockCtx @($READY_A) @{ 'P9S-D12' = 'running' } @{} $cfg $aclOpen
    $null = Open $c; $r = Act $c @{ Action = 'guest.unlock'; Vm = 'P9S-D12' }
    Expect ($r.outcome -eq 'FAILED' -and ($r.lines -join ' ') -match 'SECRET_STORE_EXPOSED' -and $c.Calls.Count -eq 0) "$($r.outcome) $($r.lines -join ' ')"
    $t = Test-P9SSecretStoreAcl -Path $secDir -GetAcl $aclOpen
    Expect (-not $t.ok -and (@($t.reasons) -join ' ') -match 'S-1-1-0') "acl check: $($t.reasons -join ',')"
    $u = Test-P9SSecretStoreAcl -Path $secDir -GetAcl { param($p) Acl @('S-1-5-18', $me) $false }.GetNewClosure()
    Expect (-not $u.ok -and @($u.reasons) -contains 'SECRET_STORE_INHERITS_ACL') "inheritance: $($u.reasons -join ',')"
}
Case 'H05' 'a valid secret is typed but never appears in any coordinator file or line' {
    $c = New-MockCtx @($READY_A) @{ 'P9S-D12' = 'running' }
    $null = Open $c; $r = Act $c @{ Action = 'guest.unlock'; Vm = 'P9S-D12' }
    Expect ($r.outcome -eq 'DISPATCHED' -and $c.Calls.Count -eq 1 -and $c.Calls[0].groups -eq $fxText.Length) "$($r.outcome) calls $($c.Calls.Count)"
    Expect ((Files $c) -notmatch [regex]::Escape($fxText) -and ($r.lines -join ' ') -notmatch [regex]::Escape($fxText)) 'secret in a file or line'
}
Case 'H06' 'diagnostics of a malformed secret do not reveal it' {
    $c = New-MockCtx @($READY_A) @{ 'P9S-U24' = 'running' }
    $null = Open $c; $r = Act $c @{ Action = 'guest.unlock'; Vm = 'P9S-U24' }
    Expect ($r.outcome -eq 'FAILED' -and ($r.lines -join ' ') -match 'SECRET_FORMAT' -and $c.Calls.Count -eq 0) "$($r.outcome) $($r.lines -join ' ')"
    $all = (Files $c) + ($r.lines -join ' ') + ($r.reasons -join ' ')
    Expect ($all -notmatch 'Bad-Format' -and $all -cnotmatch 'QXZW') 'malformed secret echoed'
}
Case 'H07' 'retired actions and retired scripts cannot be reached through the coordinator' {
    $c = New-MockCtx @($READY_A) @{ 'P9S-D12' = 'running' }
    foreach ($a in 'guest.password', 'guest.sudo', 'guest.login', 'emergency.guest-poweroff') {
        $r = Act $c @{ Action = $a; Vm = 'P9S-D12' }; Expect ($r.outcome -eq 'REJECTED' -and (Has $r "ACTION_RETIRED:$a")) "$a $($r.outcome) $($r.reasons -join ',')"
    }
    $retired = 'pw-enter.ps1', 'pw-u24.ps1', 'pw-u24-com.ps1', 'pw-vm-com.ps1', 'pp-vm-com.ps1', 'pp-type.ps1', 'pp-type-u24.ps1', 'type-it.ps1', 'type-u24.ps1', 'probe.ps1', 'wait-keep.ps1', 'watch-gui.ps1', 'com-keys-vm.ps1', 'com-keys.ps1'
    foreach ($f in 'coordinator-lib.ps1', 'p9s-action.ps1', 'p9s-config.ps1', 'com-type.ps1', 'vm-action.ps1', 'host-monitor.ps1') {
        $t = Get-Content -Raw -LiteralPath (Join-Path $kit $f)
        foreach ($r in $retired) { Expect ($t -notmatch [regex]::Escape($r)) "$f references $r" }
        Expect ($t -notmatch ('\.cla' + 'ude') -and $t -notmatch 'transcript' -and $t -notmatch 'keyboardputstring') "$f references a session log or raw string typing"
    }
    Expect ($c.Calls.Count -eq 0) 'dispatched'
}
Case 'H08' 'secret names cannot escape the store' {
    foreach ($n in @('..\x', 'a/b', 'P9S-D12-profile-passphrase.txt', '', ('x' * 100))) {
        $msg = ''; try { $null = Read-P9SSecret -Config $cfg -Name $n -GetAcl $aclGood } catch { $msg = $_.Exception.Message }
        Expect ($msg -eq 'SECRET_NAME_INVALID') "'$n' -> '$msg'"
    }
}

# ---- ACPI emergency shutdown ----------------------------------------------------------
Case 'H10' 'STOP allows the ACPI shutdown; success only once the VM is off' {
    $c = New-MockCtx @($STOP_A) @{ 'P9S-D12' = 'running' } @{ 'P9S-D12' = 'off' }
    $r = Act $c @{ Action = 'emergency.acpi-shutdown'; Vm = 'P9S-D12'; Seconds = 30 }
    Expect ($r.outcome -eq 'DONE' -and $c.Calls.Count -eq 1 -and $c.Calls[0].kind -eq 'vm.acpi') "$($r.outcome) $($r.reasons -join ',') calls $($c.Calls.Count)"
    Expect (Test-Path -LiteralPath $c.StopRequestPath) 'no stop request'
}
Case 'H11' 'ABORT and UNKNOWN allow the ACPI shutdown but no normal action' {
    foreach ($s in $ABORT_A, $UNKNOWN_A) {
        $c = New-MockCtx @($READY_A, $READY_A, $s) @{ 'P9S-U24' = 'running' } @{ 'P9S-U24' = 'off' }; $null = Open $c
        $n = Act $c @{ Action = 'guest.key'; Vm = 'P9S-U24'; Key = 'Enter' }
        Expect ($n.outcome -eq 'REFUSED') "normal action: $($n.outcome)"
        $r = Act $c @{ Action = 'emergency.acpi-shutdown'; Vm = 'P9S-U24' }
        Expect ($r.outcome -eq 'DONE' -and @($c.Calls | Where-Object kind -ne 'vm.acpi').Count -eq 0) "acpi: $($r.outcome)"
    }
}
Case 'H12' 'ACPI request built as an argument array for the validated VM' {
    $a = Get-VBoxAcpiArgs 'P9S-U24'
    Expect ($a -is [array] -and ($a -join '|') -eq 'controlvm|P9S-U24|acpipowerbutton') "args $($a -join '|')"
    $c = New-MockCtx @($STOP_A)
    foreach ($v in 'P9S-X', 'p9s-u24', 'P9S-U24 ', 'P9S-U24; calc') { $r = Act $c @{ Action = 'emergency.acpi-shutdown'; Vm = $v }; Expect ($r.outcome -eq 'REJECTED') "'$v' $($r.outcome)" }
    Expect ($c.Calls.Count -eq 0) 'dispatched'
}
Case 'H13' 'request failure is reported, nothing else is sent' {
    $c = New-MockCtx @($STOP_A) @{ 'P9S-D12' = 'running' } @{ 'P9S-D12' = 'throw' }
    $r = Act $c @{ Action = 'emergency.acpi-shutdown'; Vm = 'P9S-D12' }
    Expect ($r.outcome -eq 'FAILED' -and (Has $r 'ACPI_REQUEST_FAILED')) "$($r.outcome) $($r.reasons -join ',')"
    Expect ($c.Calls.Count -eq 1 -and $c.Calls[0].kind -eq 'vm.acpi') "calls $(($c.Calls | ForEach-Object kind) -join ',')"
    Expect ($null -ne (Get-Content -Raw -LiteralPath $c.StopRequestPath | ConvertFrom-Json).unresolved.'P9S-D12') 'unresolved emergency not recorded'
}
Case 'H14' 'timeout without power-off is UNRESOLVED, never success; close refused while the VM runs' {
    $c = New-MockCtx @($READY_A, $READY_A, $STOP_A) @{ 'P9S-D12' = 'running' } @{ 'P9S-D12' = 'stays' }; $null = Open $c
    $r = Act $c @{ Action = 'emergency.acpi-shutdown'; Vm = 'P9S-D12'; Seconds = 10 }
    Expect ($r.outcome -eq 'TIMEOUT' -and (Has $r 'EMERGENCY_UNRESOLVED') -and $c.Calls.Count -eq 1) "$($r.outcome) $($r.reasons -join ',') calls $($c.Calls.Count)"
    $s = Get-Content -Raw -LiteralPath $c.SessionPath | ConvertFrom-Json
    Expect ($s.lifecycle -eq 'EMERGENCY_UNRESOLVED') "session $($s.lifecycle)"
    $cl = Act $c @{ Action = 'session.close' }
    Expect ($cl.outcome -eq 'REFUSED' -and (Has $cl 'EMERGENCY_UNRESOLVED')) "close while running: $($cl.outcome)"
    $c.VmTable['P9S-D12'] = 'off'
    $cl2 = Act $c @{ Action = 'session.close' }
    Expect ($cl2.outcome -eq 'DONE' -and -not (Test-Path -LiteralPath $c.StopRequestPath)) "close after off: $($cl2.outcome)"
}
Case 'H15' 'no forced power-off and no keyboard fallback, ever' {
    $c = New-MockCtx @($ABORT_A) @{ 'P9S-D12' = 'running'; 'P9S-U24' = 'running' } @{ 'P9S-D12' = 'throw'; 'P9S-U24' = 'stays' }
    $null = Act $c @{ Action = 'emergency.acpi-shutdown'; Vm = 'P9S-D12' }
    $null = Act $c @{ Action = 'emergency.acpi-shutdown'; Vm = 'P9S-U24'; Seconds = 6 }
    Expect (@($c.Calls | Where-Object kind -ne 'vm.acpi').Count -eq 0) "other dispatches: $(($c.Calls | ForEach-Object kind) -join ',')"
    Expect (-not @((Get-P9SRealDispatch).Keys | Where-Object { $_ -match 'poweroff|reset|force' }).Count) "dispatcher keys $((Get-P9SRealDispatch).Keys -join ',')"
    $src = Get-Content -Raw -LiteralPath (Join-Path $kit 'coordinator-lib.ps1')
    Expect ($src -notmatch "'controlvm',\s*\`$\w+,\s*'(poweroff|reset)'" -and $src -notmatch 'shutdown --force') 'forced power commands in the source'
}
Case 'H16' 'emergency path cannot start a VM; an off VM gets no request' {
    $c = New-MockCtx @($STOP_A)
    $r = Act $c @{ Action = 'emergency.acpi-shutdown'; Vm = 'P9S-D12' }
    Expect ($r.outcome -eq 'DONE' -and $c.Calls.Count -eq 0) "off VM: $($r.outcome) calls $($c.Calls.Count)"
    foreach ($a in 'emergency.vm-start', 'emergency.start') { $x = Act $c @{ Action = $a; Vm = 'P9S-D12' }; Expect ($x.outcome -eq 'REJECTED') "$a $($x.outcome)" }
}
Case 'H17' 'ordinary input and the ACPI request never overlap' {
    $c = New-MockCtx @($STOP_A) @{ 'P9S-D12' = 'running' } @{ 'P9S-D12' = 'off' }
    $hold = [IO.FileStream]::new($c.LockPath, [IO.FileMode]::OpenOrCreate, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
    try { $r = Act $c @{ Action = 'emergency.acpi-shutdown'; Vm = 'P9S-D12' } } finally { $hold.Dispose() }
    Expect ($r.outcome -eq 'REFUSED' -and (Has $r 'EMERGENCY_LOCK_TIMEOUT') -and $c.Calls.Count -eq 0) "held lock: $($r.outcome)"
    $c2 = New-MockCtx @($READY_A) @{ 'P9S-D12' = 'running' } @{ 'P9S-D12' = 'off' }
    $job = Start-ThreadJob -ArgumentList $c2.LockPath, $c2.StopRequestPath -ScriptBlock {
        param($lockPath, $req)
        $h = [IO.FileStream]::new($lockPath, [IO.FileMode]::OpenOrCreate, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
        $sw = [Diagnostics.Stopwatch]::StartNew(); while (-not (Test-Path -LiteralPath $req) -and $sw.ElapsedMilliseconds -lt 10000) { Start-Sleep -Milliseconds 50 }
        Start-Sleep -Milliseconds 300; $h.Dispose()
    }
    Start-Sleep -Milliseconds 500
    $r2 = Act $c2 @{ Action = 'emergency.acpi-shutdown'; Vm = 'P9S-D12' }
    Receive-Job $job -Wait | Out-Null; Remove-Job $job
    Expect ($r2.outcome -eq 'DONE' -and $c2.Calls.Count -eq 1) "yielding holder: $($r2.outcome)"
}
Case 'H18' 'one ACPI request per VM per emergency, no repetition' {
    $c = New-MockCtx @($STOP_A) @{ 'P9S-D12' = 'running' } @{ 'P9S-D12' = 'stays' }
    $null = Act $c @{ Action = 'emergency.acpi-shutdown'; Vm = 'P9S-D12'; Seconds = 4 }
    $r = Act $c @{ Action = 'emergency.acpi-shutdown'; Vm = 'P9S-D12'; Seconds = 4 }
    Expect ($r.outcome -eq 'REFUSED' -and (Has $r 'ACPI_ALREADY_REQUESTED') -and $c.Calls.Count -eq 1) "$($r.outcome) calls $($c.Calls.Count)"
}
Case 'H19' 'dry run describes the ACPI request and sends nothing' {
    $c = New-MockCtx @($STOP_A) @{ 'P9S-D12' = 'running' }
    $r = Act $c @{ Action = 'emergency.acpi-shutdown'; Vm = 'P9S-D12'; DryRun = $true }
    Expect ($r.outcome -eq 'DRYRUN' -and ($r.lines -join ' ') -match 'acpipowerbutton' -and $c.Calls.Count -eq 0 -and -not (Test-Path -LiteralPath $c.StopRequestPath)) "$($r.outcome) $($r.lines -join ' ')"
}

# ---- coordinator enforcement ----------------------------------------------------------
Case 'H20' 'historical actions and safety-bypass parameters are rejected' {
    $c = New-MockCtx @($UNCAL_A) @{ 'P9S-D12' = 'running' }
    foreach ($a in @(@{ Action = 'guest.string'; Vm = 'P9S-D12' }, @{ Action = 'vm.keys'; Vm = 'P9S-D12' }, @{ Action = 'guest.mouse'; Vm = 'P9S-D12' },
                     @{ Action = 'session.open'; Kind = 'Test'; Force = $true }, @{ Action = 'session.open'; Kind = 'Test'; PagingStop = 'NotRequired' },
                     @{ Action = 'vm.start'; Vm = 'P9S-D12'; IgnoreSafety = $true }, @{ Action = 'calib.run' }, @{ Action = 'calib.live' })) {
        $r = Act $c $a; Expect ($r.outcome -eq 'REJECTED') "$(($a.GetEnumerator() | ForEach-Object { "$($_.Key)=$($_.Value)" }) -join ' '): $($r.outcome)"
    }
    $o = Open $c; Expect ($o.outcome -eq 'REFUSED' -and (Has $o 'STATUS_UNCALIBRATED')) "uncalibrated gate: $($o.outcome)"
    Expect ($c.Calls.Count -eq 0) 'dispatched'
}

# ---- calibration preparation -------------------------------------------------------------
$nom = Invoke-P9SCalibrationDryRun -Scenario nominal -Thresholds $kitThr
function StepT($d, [string]$name) { @($d.steps | Where-Object step -eq $name)[0].t }
Case 'H30' 'dry run: VM order, settling, operator assistance, qualification and observation timing' {
    $names = @($nom.steps | ForEach-Object step)
    $iD = [array]::IndexOf($names, 'vm-start P9S-D12'); $iU = [array]::IndexOf($names, 'vm-start P9S-U24')
    Expect ($nom.verdict -eq 'COMPLETED' -and $iD -ge 0 -and $iU -gt $iD) "order $($names -join ' > ')"
    Expect ((StepT $nom 'settle P9S-D12') - (StepT $nom 'vm-start P9S-D12') -eq 120 -and (StepT $nom 'settle P9S-U24') - (StepT $nom 'vm-start P9S-U24') -eq 120) 'settle 120 s'
    $tor = StepT $nom 'operator-assist ubuntu-tor-browser'
    Expect ((StepT $nom 'qualify-300s') - $tor -eq 300) "qualification $((StepT $nom 'qualify-300s') - $tor) s after Tor"
    Expect ((StepT $nom 'observe') - (StepT $nom 'qualify-300s') -eq 1800) 'observation 1800 s'
    Expect ([array]::IndexOf($names, 'memory-check') -lt $iU -and [array]::IndexOf($names, 'memory-check') -gt $iD) 'memory check not before the second VM'
}
Case 'H31' 'memory below the proposed 8 GiB stops the second VM start' {
    $d = Invoke-P9SCalibrationDryRun -Scenario low-memory -Thresholds $kitThr
    Expect ($d.verdict -eq 'HALTED' -and $d.halted_at -eq 'memory-check' -and @($d.reasons) -contains 'MEMORY_BELOW_PROPOSED_MINIMUM') "$($d.verdict) $($d.halted_at) $($d.reasons -join ',')"
    Expect (@($d.steps | Where-Object { $_.step -eq 'vm-start P9S-U24' -and $_.outcome -eq 'SIMULATED' }).Count -eq 0) 'second VM started'
    Expect ((Get-P9SCalibrationLimits).second_vm_min_available_mb -eq 8192) 'threshold is not 8192 MB'
}
Case 'H32' 'missing measurements block progression' {
    $d = Invoke-P9SCalibrationDryRun -Scenario missing-counters -Thresholds $kitThr
    Expect ($d.verdict -eq 'HALTED' -and (@($d.reasons) -contains 'MEASUREMENTS_INCOMPLETE')) "$($d.verdict) $($d.halted_at) $($d.reasons -join ',')"
}
Case 'H33' 'a paging WARNING blocks further workload' {
    $d = Invoke-P9SCalibrationDryRun -Scenario paging-warning -Thresholds $kitThr
    Expect ($d.verdict -eq 'HALTED' -and $d.halted_at -eq 'settle P9S-D12' -and @($d.reasons) -contains 'STATUS_WARNING') "$($d.verdict) $($d.halted_at) $($d.reasons -join ',')"
    Expect (@($d.steps | Where-Object { $_.step -eq 'vm-start P9S-U24' -and $_.outcome -eq 'CANCELLED' }).Count -eq 1) 'second VM not cancelled'
}
Case 'H34' 'Tor steps are operator-assisted and explicit; Monolith and transfers are excluded' {
    $oa = @($nom.steps | Where-Object kind -eq 'operator-assist')
    Expect ($oa.Count -eq 2 -and @($oa | Where-Object step -eq 'operator-assist ubuntu-tor-browser').Count -eq 1 -and @($oa | Where-Object { $_.outcome -like 'OPERATOR-ASSISTED*' }).Count -eq 2) "operator steps: $(($oa | ForEach-Object step) -join ',')"
    Expect (-not @($nom.steps | Where-Object { $_.step -match 'monolith|transfer|message|unlock|type' }).Count) 'excluded step present'
    foreach ($k in 'monolith-start', 'file-transfer', 'message', 'guest.type', 'tor-interrupt', 'snapshot-restore') { Expect (-not (Test-P9SCalibrationStepKind $k)) "$k accepted as a calibration step" }
    foreach ($k in Get-P9SCalibrationStepKinds) { Expect (Test-P9SCalibrationStepKind $k) "$k refused" }
}
Case 'H35' 'live calibration stays disabled, whatever file exists' {
    $c = New-MockCtx @($READY_A)
    '{"approved": true, "scope": "calibration"}' | Set-Content -LiteralPath (Join-Path $c.Dir 'calibration-authorization.json')
    '{"approved": true}' | Set-Content -LiteralPath (Join-Path $fxDir 'calibration-authorization.json')
    $r = Act $c @{ Action = 'session.open'; Kind = 'Calibration' }
    Expect ($r.outcome -eq 'REFUSED' -and (Has $r 'CALIBRATION_LIVE_DISABLED')) "$($r.outcome) $($r.reasons -join ',')"
    Expect (-not @(Get-Command -CommandType Function | Where-Object { $_.Name -match 'Calibration(Live|Run|Start)|Start-P9SCalib' }).Count) 'a live calibration function exists'
}
Case 'H36' 'qualification cap and unresolved ACPI shutdown in the dry run' {
    $t = Invoke-P9SCalibrationDryRun -Scenario nominal -Thresholds $kitThr -QualifyTimeoutSeconds 200
    Expect ($t.verdict -eq 'HALTED' -and @($t.reasons) -contains 'QUALIFICATION_TIMEOUT') "cap: $($t.verdict) $($t.reasons -join ',')"
    $u = Invoke-P9SCalibrationDryRun -Scenario acpi-timeout -Thresholds $kitThr
    Expect ($u.verdict -eq 'SHUTDOWN_UNRESOLVED' -and @($u.steps | Where-Object { $_.step -eq 'wait-off P9S-U24' -and $_.outcome -like 'UNRESOLVED*' }).Count -eq 1) "acpi timeout: $($u.verdict)"
    Expect (@($u.steps | Where-Object { $_.step -eq 'acpi-shutdown P9S-U24' }).Count -eq 1) 'ACPI request repeated in the dry run'
}

# ---- publication safety of this tree --------------------------------------------------------
$dist = 'README.md', 'assert-host-safe.ps1', 'com-type.ps1', 'coordinator-lib.ps1', 'host-monitor.ps1', 'hostmon-lib.ps1', 'p9s-action.ps1', 'p9s-config.ps1',
        'p9s-local.example.json', 'paging-thresholds.json', 'vm-action.ps1', 'tests\coordinator-tests.ps1', 'tests\hostmon-tests.ps1', 'tests\stagec-tests.ps1',
        'tests\run-all.ps1', 'tests\mutants.json', 'p9s-vbox.ps1', 'tests\vbox-tests.ps1', 'p9s-vmstate.ps1', 'p9s-acpival.ps1',
        'acpi-validation-authorization.example.json', 'tests\decouple-tests.ps1', 'tests\acpival-tests.ps1', 'acpi-validation-authorization-u24.example.json',
        'p9s-state.ps1', 'tests\persist-tests.ps1', 'p9s-state-init.ps1', 'tests\setup-tests.ps1',
        'p9s-state-migrate.ps1', 'tests\migrate-tests.ps1', 'tests\migproc-tests.ps1'
$localOnly = 'p9s-local.json', 'KIT-SHA256SUMS.txt'
$distFiles = @($dist | ForEach-Object { Join-Path $kit $_ } | Where-Object { Test-Path -LiteralPath $_ })
Case 'H40' 'the tree holds exactly the distribution files (plus known local-only files)' {
    $have = @(Get-ChildItem -LiteralPath $kit -Recurse -File | ForEach-Object { $_.FullName.Substring($kit.Length + 1) })
    foreach ($f in $dist) { Expect ($have -contains $f) "missing $f" }
    foreach ($f in $have) { Expect (($dist -contains $f) -or ($localOnly -contains $f)) "unexpected file $f" }
    $readme = Get-Content -Raw -LiteralPath (Join-Path $kit 'README.md')
    foreach ($f in $dist | Where-Object { $_ -like '*.ps1' -or $_ -like '*.json' }) { Expect ($readme -match [regex]::Escape(($f -replace '\\', '/'))) "README does not mention $f" }
}
Case 'H41' 'no machine-specific path, account or host name in the distribution' {
    $pats = @(('(?i)[a-z]:\\us' + 'ers\\'), ('(?i)[a-z]:\\monolith-' + 'p9s'), ('(?i)\\app' + 'data\\'), ('(?i)\b' + [regex]::Escape($env:COMPUTERNAME) + '\b'), ('(?i)\\' + [regex]::Escape($env:USERNAME) + '\\'))
    foreach ($f in $distFiles) { $t = Get-Content -Raw -LiteralPath $f; foreach ($p in $pats) { Expect ($t -notmatch $p) "$([IO.Path]::GetFileName($f)) matches $p" } }
}
Case 'H42' 'no credential, key, token or onion address in the distribution' {
    $pats = @('(?i)(password|passphrase|secret|token|apikey)\s*[:=]\s*[''"][^''"\s$][^''"]{5,}[''"]', '-----BEGIN [A-Z ]*PRIVATE KEY', '\bghp_[A-Za-z0-9]{20,}', '\bAKIA[0-9A-Z]{16}\b', '\b16:[0-9A-F]{40,}', '\b[a-z2-7]{56}\.onion\b', '(?i)HashedControlPassword\s+\S')
    foreach ($f in $distFiles) { $t = Get-Content -Raw -LiteralPath $f; foreach ($p in $pats) { Expect ($t -notmatch $p) "$([IO.Path]::GetFileName($f)) matches $p" } }
}
Case 'H43' 'no evidence, runtime state or binary in the distribution; ASCII only; no tool attributions' {
    foreach ($f in $distFiles) {
        $n = [IO.Path]::GetFileName($f)
        Expect ($n -notmatch '(?i)\.(csv|png|jpg|pcap|pcapng|jsonl|log|lock|request|vdi|vhd|iso|zip)$' -and $n -notin 'state.json', 'coordinator-session.json', 'p9s-local.json') "runtime or evidence file $n"
        $t = Get-Content -Raw -LiteralPath $f
        Expect ($t -notmatch '[^\x09\x0a\x0d\x20-\x7e]') "$n is not plain ASCII"
        foreach ($w in @(('cla' + 'ude'), ('anthr' + 'opic'), ('chat' + 'gpt'), ('open' + 'ai'), ('co' + 'pilot'), ('generated ' + 'by'))) { Expect ($t -notmatch "(?i)$w") "$n mentions $w" }
    }
}
Case 'H44' 'example configuration is valid and holds only placeholders' {
    $e = Read-P9SConfig -Path (Join-Path $kit 'p9s-local.example.json')
    Expect ($e.ok) "example invalid: $($e.errors -join ',')"
    $t = Get-Content -Raw -LiteralPath (Join-Path $kit 'p9s-local.example.json')
    Expect ($t -match '(?i)example' -and $t -notmatch '(?i)WD Elements') 'example holds lab values'
}

# ---- command line (dry runs and the simulation only) ---------------------------------------
$cli = Join-Path $kit 'p9s-action.ps1'
$pw = (Get-Process -Id $PID).Path
$vb = Join-Path $env:ProgramFiles 'Oracle\VirtualBox\VBoxManage.exe'
# VM states for the "nothing changed" check, read only, through the
# toolkit's VirtualBox layer (host-wide lock, one query at a time, no
# retry); its records go to this run's folder.
$oracleVBox = New-P9SVBoxContext -Dir (Join-Path $OutDir 'vbox-oracle') -VmNames @('P9S-D12', 'P9S-U24')
function VmStates {
    if (-not (Test-Path -LiteralPath $vb)) { return 'no VirtualBox' }
    @(foreach ($v in 'P9S-D12', 'P9S-U24') { "$v $(Get-P9SVmStateReal $v $oracleVBox)" }) -join ' '
}
Case 'H50' 'command line with a configuration file: ACPI dry run, calibration simulation, retired shutdown typing' {
    $before = VmStates
    $d = Join-Path $OutDir 'cli'; New-Item -ItemType Directory -Path $d | Out-Null
    $o = & $pw -NoProfile -File $cli -Action emergency.acpi-shutdown -Vm P9S-D12 -DryRun -ConfigPath $cfgPath -StatePath (Join-Path $d 'state.json') 2>&1
    Expect ($LASTEXITCODE -eq 0 -and ($o -join ' ') -match 'acpipowerbutton') "acpi dry run: $LASTEXITCODE $($o -join ' ')"
    $o2 = & $pw -NoProfile -File $cli -Action calib.dry-run -Scenario nominal -ConfigPath $cfgPath -StatePath (Join-Path $d 'state.json') 2>&1
    Expect ($LASTEXITCODE -eq 0 -and ($o2 -join ' ') -match 'OPERATOR-ASSISTED' -and ($o2 -join ' ') -match 'COMPLETED') "calibration: $LASTEXITCODE"
    $msg = ''; try { & (Join-Path $kit 'com-type.ps1') -Vm P9S-D12 -Text 'systemctl poweroff' -Shutdown -DryRun -StatePath (Join-Path $d 'state.json') } catch { $msg = $_.Exception.Message }
    Expect ($msg -match 'retired' -and $msg -match 'emergency.acpi-shutdown') "shim: $msg"
    Expect ((VmStates) -eq $before) "VM states changed: $before -> $(VmStates)"
}

$pass = @($results | Where-Object result -eq 'PASS').Count
$fail = @($results | Where-Object result -eq 'FAIL').Count
$notRun = @($results | Where-Object result -ne 'PASS' | Where-Object result -ne 'FAIL').Count
"== $pass passed, $fail failed, $notRun not run"
$results | ConvertTo-Json -Depth 3 | Set-Content -LiteralPath (Join-Path $OutDir 'results.json') -Encoding utf8
$results | ForEach-Object { '{0} {1} {2} ({3} ms){4}' -f $_.result, $_.id, $_.name, $_.ms, $(if ($_.detail) { ' :: ' + $_.detail } else { '' }) } | Set-Content -LiteralPath (Join-Path $OutDir 'results.txt') -Encoding utf8
if ($fail) { exit 1 } elseif ($notRun) { exit 3 } else { exit 0 }
