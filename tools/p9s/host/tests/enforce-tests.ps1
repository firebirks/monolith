#Requires -Version 7.0
# Tests of the persistent state as the coordinator enforces it
# (coordinator-lib.ps1): on synthetic state folders, with the real
# classifier (p9s-state.ps1), the anchor written with the real registry API
# into test keys and the real ledger save. The anchor mark of every ledger
# save and its order; the faults and the crash points of a save (in
# process, and a child process ended by TerminateProcess at each point);
# the state folder changed between the gate and the dispatch, in process
# and from another process; state.reanchor; the monitor and the real
# context on the default location; the earlier folder after a migration;
# STOP, ABORT and a restart. The VM states are mocked: no VirtualBox call.
# The cases that start the entry point without -StatePath read the real
# state locations of the machine: they run only in the main run of
# run-all.ps1 -CleanRunner. Folders under -OutDir (or under the system
# temporary folder when -OutDir is not on a local NTFS volume) and test
# registry keys under HKCU\Software\Monolith-P9S-Test\<run>, removed at the
# end; the real state root and anchor key must be as they were at the start.
# The gate itself, status by status, is tested by tests/coordinator-tests.ps1
# (K56-K63) and tests/acpival-tests.ps1 (A18-A22).
#   pwsh -NoProfile -File tests\enforce-tests.ps1 -OutDir <new folder>
param([Parameter(Mandatory)][string]$OutDir)
$ErrorActionPreference = 'Stop'
$kit = Split-Path $PSScriptRoot -Parent
. (Join-Path $kit 'hostmon-lib.ps1')
. (Join-Path $kit 'coordinator-lib.ps1')
. (Join-Path $kit 'p9s-state.ps1')
. (Join-Path $kit 'p9s-state-init.ps1')
. (Join-Path $kit 'p9s-state-migrate.ps1')
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
function NotRun([string]$id, [string]$name, [string]$why) {
    $results.Add([pscustomobject]@{ id = $id; name = $name; result = 'NOT_RUN'; ms = 0; detail = $why })
    "NOT_RUN $id $name :: $why"
}
function Expect([bool]$cond, [string]$what) { if (-not $cond) { $script:fails.Add($what) } }

$pw = (Get-Process -Id $PID).Path
$me = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
$run = [guid]::NewGuid().ToString('N').Substring(0, 12)
$keyRoot = "Software\Monolith-P9S-Test\$run"
if (-not (Initialize-P9SStateNative)) { throw 'the native checks of p9s-state.ps1 could not be compiled' }
$names = Get-P9SStateNames; $classes = Get-P9SStateFileClasses

# ---- the synthetic base, as in tests/setup-tests.ps1 ----------------------------------------
function Remove-Tree([string[]]$Path) {
    foreach ($p in $Path) {
        $last = $null
        for ($i = 0; $i -lt 20 -and (Test-Path -LiteralPath $p); $i++) {
            try { Remove-Item -LiteralPath $p -Recurse -Force -ErrorAction Stop } catch { $last = $_; Start-Sleep -Milliseconds 100 }
        }
        if (Test-Path -LiteralPath $p) { throw "could not remove $p ($($last.Exception.Message))" }
    }
}
function Test-UsableBase([string]$B) {
    New-Item -ItemType Directory -Force -Path $B | Out-Null
    $fin = (Get-P9SStateEntry $B).final
    $probe = Join-Path $fin 'probe'
    New-Item -ItemType Directory -Force -Path (Join-Path $probe 'LocalAppData'), (Join-Path $probe 'Temp') | Out-Null
    $sc = New-P9SStateContext -LocalAppData (Join-Path $probe 'LocalAppData') -TempPath (Join-Path $probe 'Temp') -AnchorKey "$keyRoot\probe"
    $why = @(Get-P9SStateLocationReasons $sc)
    Remove-Tree $probe
    [pscustomobject]@{ base = $fin; reasons = $why }
}
$pick = Test-UsableBase (Join-Path $OutDir 'synthetic')
if ($pick.reasons.Count) {
    $alt = Test-UsableBase (Join-Path ([IO.Path]::GetTempPath()) "p9s-enforce-tests-$run")
    if (-not $alt.reasons.Count) { $pick = $alt } else { Remove-Tree $alt.base }
}
$base = $pick.base
"== synthetic base $base$(if ($pick.reasons.Count) { ' (location reasons: ' + ($pick.reasons -join ',') + ')' })"
$realLad = [P9SStateNative]::LocalAppData()
function Get-RealSnap {
    $k = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey($names.anchor_key, $false)
    $kp = [bool]$k; if ($k) { $k.Dispose() }
    "real root $(Test-Path -LiteralPath (Join-Path $realLad $names.root)); real anchor key $kp"
}
$realAtStart = Get-RealSnap

# ---- synthetic environments ---------------------------------------------------------------------
function New-Env([string]$Name) {
    $b = Join-Path $base $Name
    $lad = Join-Path $b 'LocalAppData'
    New-Item -ItemType Directory -Force -Path $lad, (Join-Path $b 'Temp') | Out-Null
    [pscustomobject]@{ base = $b; lad = $lad; tmp = Join-Path $b 'Temp'; key = "$keyRoot\$Name"; root = Join-Path $lad $names.root
        state = Join-Path (Join-Path $lad $names.root) $names.state; legacy = Join-Path (Join-Path $b 'Temp') $names.legacy
        legacy2 = Join-Path (Join-Path $lad 'Temp') $names.legacy; id = $null; mid = $null; manifest = $null }
}
function New-EnvCtx($E) { New-P9SStateContext -LocalAppData $E.lad -TempPath $E.tmp -AnchorKey $E.key }
function Get-Status($E) { Get-P9SStateStatus (New-EnvCtx $E) }
function Test-Status($E, [string]$Want, [string[]]$Reasons = @(), [string]$Label = '') {
    $st = Get-Status $E
    $p = if ($Label) { "$($Label): " } else { '' }
    Expect ($st.status -ceq $Want) "${p}status $($st.status) (reasons $(@($st.reasons) -join ','); notes $(@($st.notes) -join ',')), expected $Want"
    foreach ($r in $Reasons) { Expect (@($st.reasons | Where-Object { $_ -clike $r }).Count -gt 0) "${p}reason $r missing (have $(@($st.reasons) -join ','))" }
    $st
}
function Write-Doc([string]$Path, $Obj) { [IO.File]::WriteAllBytes($Path, [Text.UTF8Encoding]::new($false).GetBytes(($Obj | ConvertTo-Json -Depth 8))) }
function Write-Text([string]$Path, [string]$T) { [IO.File]::WriteAllText($Path, $T, [Text.UTF8Encoding]::new($false)) }
function Add-Line([string]$Path, $Obj) { [IO.File]::AppendAllText($Path, ($Obj | ConvertTo-Json -Compress -Depth 5) + "`n") }
function Set-ProtectedAcl([string]$Dir) {
    $acl = [Security.AccessControl.DirectorySecurity]::new(); $acl.SetAccessRuleProtection($true, $false)
    foreach ($sid in @($me, 'S-1-5-18', 'S-1-5-32-544')) { $acl.AddAccessRule([Security.AccessControl.FileSystemAccessRule]::new([Security.Principal.SecurityIdentifier]::new($sid), 'FullControl', 'ContainerInherit,ObjectInherit', 'None', 'Allow')) }
    [IO.FileSystemAclExtensions]::SetAccessControl([IO.DirectoryInfo]::new($Dir), $acl)
}
function Get-Utc { [DateTime]::UtcNow.ToString('yyyy-MM-ddTHH:mm:ss.fffZ', [Globalization.CultureInfo]::InvariantCulture) }
function New-Id([string]$P = 'inst') { "$P-" + [guid]::NewGuid().ToString('N').Substring(0, 16) }
function Get-Mark([string]$LedgerPath) {
    $d = [IO.File]::ReadAllText($LedgerPath) | ConvertFrom-Json -AsHashtable
    [ordered]@{ revision = [long]$(if ($d.Contains('revision')) { $d.revision } else { 0 }); sha256 = (Get-FileHash -LiteralPath $LedgerPath -Algorithm SHA256).Hash.ToLowerInvariant() }
}
function Write-Anchor($E, $Doc, [string]$Raw) {
    $k = [Microsoft.Win32.Registry]::CurrentUser.CreateSubKey($E.key)
    try { $k.SetValue($names.anchor_value, $(if ($PSBoundParameters.ContainsKey('Raw')) { $Raw } else { $Doc | ConvertTo-Json -Depth 6 -Compress }), [Microsoft.Win32.RegistryValueKind]::String) } finally { $k.Dispose() }
}
function Get-Anchor($E) { $k = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey($E.key, $false); if (-not $k) { return $null }; try { $v = $k.GetValue($names.anchor_value); if ($null -eq $v) { $null } else { ConvertFrom-P9SStateJson ([Text.UTF8Encoding]::new($false).GetBytes([string]$v)) } } finally { $k.Dispose() } }
function Remove-Anchor($E) { [Microsoft.Win32.Registry]::CurrentUser.DeleteSubKeyTree($E.key, $false) }
function New-AnchorDoc($E, [string]$Id, [string]$Phase = 'READY', [string]$Origin = 'new', $Mark, [string]$Source, [string]$Manifest) {
    [ordered]@{ schema = 'p9s-state-anchor/1'; phase = $Phase; op_id = (New-Id 'op'); installation_id = $Id; account = $me; state_dir = $E.state; origin = $Origin
        source_dir = $(if ($Origin -eq 'migrated') { $Source } else { $null }); manifest_sha256 = $(if ($Origin -eq 'migrated') { $Manifest } else { $null }); supersedes = $null
        created_utc = (Get-Utc); initialized_utc = $(if ($Phase -eq 'READY') { Get-Utc } else { $null }); ledger = $(if ($Phase -eq 'READY') { $Mark } else { $null }) }
}
function New-InstallDoc($E, [string]$Id, [string]$Origin = 'new', [string]$Mid) {
    [ordered]@{ schema = 'p9s-state-install/1'; installation_id = $Id; account = $me; state_dir = $E.state; origin = $Origin; migration_id = $(if ($Origin -eq 'migrated') { $Mid } else { $null }); supersedes = $null; created_utc = (Get-Utc) }
}
function New-Root($E) { [IO.Directory]::CreateDirectory($E.root) | Out-Null; Set-ProtectedAcl $E.root; [IO.Directory]::CreateDirectory($E.state) | Out-Null }
function Get-LedgerPath($E) { Join-Path $E.state 'acpi-validation-ledger.json' }
function Get-RevisionPath($E) { Join-Path $E.state 'acpi-validation-ledger.revision.json' }
function Save-Ledger([string]$Path) {
    $lc = [pscustomobject]@{ LedgerPath = $Path; SessionPath = (Join-Path (Split-Path -Parent $Path) 'coordinator-session.json') }; $lg = Read-P9SAcpiLedger $lc
    if (-not (Save-P9SAcpiLedger $lc $lg.doc)) { throw 'ledger save failed' }
}
# A new installation in READY: protected root, install record, empty ledger
# at revision 1 with its revision record, anchor with the ledger's mark.
function Install-New($E) {
    New-Root $E; $E.id = New-Id
    Write-Doc (Join-Path $E.state $names.install) (New-InstallDoc $E $E.id)
    Save-Ledger (Get-LedgerPath $E)
    Write-Anchor $E (New-AnchorDoc $E $E.id -Mark (Get-Mark (Get-LedgerPath $E)))
}
# An earlier coordinator folder with the laboratory's shape (synthetic content).
function New-Legacy([string]$Dir) {
    New-Item -ItemType Directory -Force -Path $Dir | Out-Null
    Write-Doc (Join-Path $Dir 'acpi-validation-ledger.json') ([ordered]@{ schema = 'p9s-acpi-validation-ledger/1'; entries = @([ordered]@{ authorization_id = 'acpiauth-20260101-00000000'; target_vm = 'P9S-D12'; phase = 'OFF_CONFIRMED'; startups_used = 1; acpi_used = 1; events = @() }) })
    Write-Doc (Join-Path $Dir 'coordinator-session.json') ([ordered]@{ schema = 'p9s-coordinator-session/1'; session_id = 'ses-synthetic'; kind = 'AcpiValidation'; lifecycle = 'CLOSED'; expected_vm = @{} })
    Add-Line (Join-Path $Dir 'coordinator-audit.jsonl') ([ordered]@{ action = 'session.close'; outcome = 'DONE' })
    Add-Line (Join-Path $Dir 'vm-observations.jsonl') ([ordered]@{ vm = 'P9S-D12'; state = 'poweroff' })
    Add-Line (Join-Path $Dir 'vbox-audit.jsonl') ([ordered]@{ operation = 'vm.state'; outcome = 'DONE' })
    Write-Text (Join-Path $Dir 'coordinator.lock') ''; Write-Text (Join-Path $Dir 'state.json') '{}'
}
# The migration of that folder by state.migrate itself (VMs mocked off):
# READY, the marker and the tombstone in the earlier folder.
function Install-Migrated($E) {
    New-Legacy $E.legacy
    $i = New-P9SStateMigrateContext -State (New-EnvCtx $E) -VmNames @('P9S-D12', 'P9S-U24') -ReadVmStateIn { param($vm, $dir) 'off' } -CountVmProcesses { 0 } -FreeBytes { param($d) 107374182400L }
    $r = Invoke-P9SStateMigrate -I $i
    if ($r.outcome -cne 'DONE') { throw "fixture migration: $($r.outcome) $(@($r.reasons) -join ',')" }
    $E.id = $r.installation_id
}
function Why($R) { "$($R.outcome) [$(@($R.reasons) -join ',')]" }
function Has($R, [string]$Code) { @($R.reasons) -ccontains $Code }
function Test-Refused($R, [string]$Reason, [string]$Label) { Expect ($R.outcome -ceq 'REFUSED' -and @($R.reasons | Where-Object { $_ -clike $Reason }).Count -gt 0) "${Label}: $(Why $R), expected $Reason" }
function Get-Files([string]$Root) {
    $h = [ordered]@{}
    if (Test-Path -LiteralPath $Root) { foreach ($f0 in @(Get-ChildItem -LiteralPath $Root -Recurse -Force -File | Sort-Object FullName)) { $h[$f0.FullName.Substring($Root.Length)] = (Get-FileHash -LiteralPath $f0.FullName -Algorithm SHA256).Hash } }
    ($h.Keys | ForEach-Object { "$_ $($h[$_])" }) -join '; '
}
function Get-KeyText($E) { $k = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey($E.key, $false); if (-not $k) { return 'absent' }; try { [string]$k.GetValue($names.anchor_value) } finally { $k.Dispose() } }
# A kit function replaced for one fault and put back; it is replaced where it
# is defined (the global scope when the suite is dot-sourced or started with
# -File, which is where the toolkit's closures look).
function Set-Override([string]$Name, [scriptblock]$Body) {
    $scope = if (Test-Path -LiteralPath "function:global:$Name") { 'global' } else { 'script' }
    $orig = Get-Content -LiteralPath "function:${scope}:$Name"
    if ($orig -isnot [scriptblock]) { throw "the definition of $Name was not read" }
    Set-Item -LiteralPath "function:${scope}:$Name" -Value $Body
    [pscustomobject]@{ name = $Name; scope = $scope; orig = $orig }
}
function Reset-Override($O) { Set-Item -LiteralPath "function:$($O.scope):$($O.name)" -Value $O.orig }

# ---- monitor states from the real policy (as in tests/coordinator-tests.ps1) --------------------
$GiB = [math]::Pow(2, 30)
function S([double]$t, $commit = 50) {
    [pscustomobject][ordered]@{
        seq = 0; mono = $t; utc = '2026-11-19T00:00:00.000Z'; read_ok = $true; read_error = $null
        committed_bytes = $(if ($null -eq $commit) { $null } else { [double]$commit * $GiB }); commit_limit_bytes = 100 * $GiB
        avail_mb = 16000.0; modified_list_mb = 200.0; pages = 100.0; pages_in = 100.0; pages_out = 0.0; page_reads = 90.0; page_writes = 0.0
        page_faults = 1000.0; transition_faults = 500.0; cache_faults = 100.0; demand_zero_faults = 300.0; rate_issues = @()
        pagefile_used_mb = 100.0; pagefile_alloc_mb = 5888.0; pagefile_pct = 1.7; pagefile_disks = @(0); system_disk = 0; external_disk = 2
        disks = @{ 0 = [pscustomobject]@{ read_ms = 0.5; write_ms = 0.3; read_mbps = 5.0; write_mbps = 1.0; queue = 0; avg_queue = 0.01 }
                   2 = [pscustomobject]@{ read_ms = 2.0; write_ms = 1.0; read_mbps = 0.1; write_mbps = 0.1; queue = 0; avg_queue = 0.01 } }
        external_present = $true; external_identity_ok = $true; eventlog_ok = $true; storage_events_new = 0; storage_event_ids = @(); cpu_pct = 10.0; vm = @{}
    }
}
function Feed($st, [double]$from, [double]$to, $commit = 50) { $dd = $null; for ($t = $from; $t -le $to + 1e-9; $t += 5) { $dd = Step-Monitor -State $st -Sample (S $t $commit) }; $dd }
$thr = [ordered]@{
    suspect = [ordered]@{ page_out_per_s = 500; avail_mb_low = 4096; avail_decline_mb = 4096; pagefile_growth_mb = 128; pagefile_disk_read_ms = 20; page_in_per_s_with_low_memory = 2000 }
    confirm = [ordered]@{ page_out_per_s = 2000; avail_mb_low = 2048; avail_decline_mb = 6144; pagefile_growth_mb = 300; pagefile_disk_read_ms = 50; page_in_per_s_with_low_memory = 5000 }
}
$fxApproved = ConvertTo-PagingThresholds -Json ([ordered]@{ schema = 'p9s-hostmon-paging/1'; approved = $true; approved_by = 'test fixture only'; approved_utc = '2026-11-19T00:00:00Z'; source = 'test'; suspect = $thr.suspect; confirm = $thr.confirm } | ConvertTo-Json -Depth 5)
$QF = 10000000L
$QNOW = 1000L * $QF
function Info { [ordered]@{ pid = 4242; process_start_utc = 'x'; process_start_ticks = 1L; mode = 'Monitor'; paging_stop = 'Required'; thresholds_sha256 = 'x'; thresholds_approved = $true; test = $false; kit = 'test' } }
function J($doc) { ConvertTo-StateJson -Doc $doc -Monitor (Info) -WrittenQpc ([long]($QNOW - $QF)) -QpcFrequency $QF -WrittenUtc 'x' }
function NewMon([string]$RunName) { New-MonitorState -RunId $RunName -Config (New-HostMonConfig -PagingStop Required -Mode Monitor -Thresholds $fxApproved) }
$mA = NewMon 'run-A'; $READY_A = J (Feed $mA 0 300); $STOP_A = J (Feed $mA 305 335 95)
$mA2 = NewMon 'run-A'; $null = Feed $mA2 0 300; $ABORT_A = J (Feed $mA2 305 305 97)
$READY_B = J (Feed (NewMon 'run-B') 0 300)

# ---- a coordinator context on a synthetic state folder ------------------------------------------
# The coordinator files are those of the environment's state folder; the
# gate and the anchor mark are the real ones on that folder (the real
# classifier, the test key); the monitor states, the VM states and the
# dispatchers are mocked. $c.Prov.onVmRead runs once at the next VM read
# (the first read of an action comes after its recheck under the lock).
function New-EnvCoordCtx($E, [object[]]$States = @($READY_A), [hashtable]$Vms = @{}) {
    $sc = New-EnvCtx $E
    $dir = $E.state
    $calls = [System.Collections.Generic.List[object]]::new()
    $q = [System.Collections.Generic.Queue[object]]::new(); foreach ($x in $States) { $q.Enqueue($x) }
    $prov = @{ queue = $q; reads = 0; onVmRead = $null; vmreads = 0 }
    $vmTable = @{ 'P9S-D12' = 'running'; 'P9S-U24' = 'off' }; foreach ($k in $Vms.Keys) { $vmTable[$k] = $Vms[$k] }
    $rec = { param($kind, $list) { param($vm, $arg) $list.Add([pscustomobject]@{ kind = $kind; vm = $vm }) }.GetNewClosure() }
    $acpi = { param($list, $tbl) { param($vm) $list.Add([pscustomobject]@{ kind = 'vm.acpi'; vm = $vm }); $tbl[$vm] = 'off' }.GetNewClosure() }
    [pscustomobject]@{
        StatePath = Join-Path $dir 'state.json'; SessionPath = Join-Path $dir 'coordinator-session.json'; LockPath = Join-Path $dir 'coordinator.lock'
        StopRequestPath = Join-Path $dir 'coordinator-stop.request'; AuditPath = Join-Path $dir 'coordinator-audit.jsonl'; ThresholdsPath = Join-Path $kit 'paging-thresholds.json'
        ReadState = { param($p) $prov.reads++; $x = if ($prov.queue.Count -gt 1) { $prov.queue.Dequeue() } else { $prov.queue.Peek() }; [pscustomobject]@{ ok = $true; text = $x; error = $null } }.GetNewClosure()
        NowQpc = { $QNOW }.GetNewClosure(); QpcFrequency = $QF; IsMonitorAlive = { param($a, $b) $true }
        ReadVmState = { param($vm) $prov.vmreads++; if ($prov.onVmRead) { $h = $prov.onVmRead; $prov.onVmRead = $null; & $h $vm }; $vmTable[$vm] }.GetNewClosure()
        ReadVmConfig = { param($vm) @() }
        Dispatch = @{ 'vm.start' = (& $rec 'vm.start' $calls); 'guest.text' = (& $rec 'guest.text' $calls); 'guest.key' = (& $rec 'guest.key' $calls); 'guest.unlock' = (& $rec 'guest.unlock' $calls); 'vm.acpi' = (& $acpi $calls $vmTable) }
        Sleep = { param($ms) }; Pid = $PID; ProcessStartTicks = (Get-Process -Id $PID).StartTime.ToUniversalTime().Ticks
        EmergencyLockWaitMs = 1500; CalibrationHostChecks = { @() }
        ObservationPath = Join-Path $dir 'vm-observations.jsonl'; LedgerPath = Join-Path $dir 'acpi-validation-ledger.json'; UtcNow = { [DateTime]::UtcNow }; AcpiValidationLive = $true
        CountVmProcesses = { 0 }
        StateGate = { Get-P9SStateGateOf $sc }.GetNewClosure()
        UpdateLedgerMark = { param($rev, $sha) Update-P9SStateLedgerMarkOf $sc $rev $sha }.GetNewClosure()
        Installation = { Get-P9SStateInstallation $sc }.GetNewClosure()
        Calls = $calls; Prov = $prov; VmTable = $vmTable; Dir = $dir; State = $sc
    }
}
function Act($ctx, [hashtable]$Rq) { Invoke-P9SAction -Ctx $ctx -Request $Rq }
function Open($ctx) { Act $ctx @{ Action = 'session.open'; Kind = 'Test' } }
function Sess($ctx) { (Read-P9SSession $ctx).session }
# a ledger saved once more through the context (the anchor mark moves)
function Save-Again($ctx) { $lg = Read-P9SAcpiLedger $ctx; if (-not $lg.ok) { throw "ledger: $($lg.reason)" }; Save-P9SAcpiLedger $ctx $lg.doc }
# the anchor mark against the ledger on disk: 'equal', 'behind' or 'AHEAD'
function Get-MarkPosition($E) {
    $a = Get-Anchor $E; $m = Get-Mark (Get-LedgerPath $E)
    if (-not $a -or -not $a.ledger) { return 'none' }
    if ([long]$a.ledger.revision -eq $m.revision) { if ($a.ledger.sha256 -ceq $m.sha256) { 'equal' } else { 'DIVERGED' } } elseif ([long]$a.ledger.revision -lt $m.revision) { 'behind' } else { 'AHEAD' }
}

"== P9-S persistent state enforcement tests, $((Get-Date).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ')), pwsh $($PSVersionTable.PSVersion)"

# ================================================================ the real context and the monitor
# The default location as the toolkit computes it; the cases that need a
# context or a monitor on it use a synthetic one in its place (a fault in
# the code under test then creates nothing on the machine's real location).
$fakeDefault = Join-Path $base "default\LocalAppData\$($names.root)\state\state.json"
$hostmonLine = "function Get-DefaultStatePath { Join-Path ([Environment]::GetFolderPath([Environment+SpecialFolder]::LocalApplicationData)) 'Monolith-P9S\state\state.json' }"
Case 'N01' 'the real context: without -StatePath the coordinator files are those of the persistent state folder from the known folder API, and nothing is created; the default given as -StatePath is no sandbox; any other -StatePath is a sandbox whose dispatchers refuse, whose ledger has no anchor mark and whose gate reads no state' {
    $def = Get-DefaultStatePath; $ddir = Split-Path -Parent $def
    Expect (Test-P9SStateSamePath $ddir (Join-Path (Join-Path $realLad $names.root) $names.state)) "default $def, known folder $realLad"
    $ov = Set-Override 'Get-DefaultStatePath' ([scriptblock]::Create("'$($fakeDefault.Replace("'", "''"))'"))
    try {
        $fdir = Split-Path -Parent $fakeDefault
        $rc = New-P9SRealContext -KitDir $kit -Config $null
        Expect ($rc.StatePath -ceq $fakeDefault -and $rc.LedgerPath -ceq (Join-Path $fdir 'acpi-validation-ledger.json') -and $rc.VBox.Dir -ceq $fdir) "paths $($rc.StatePath), $($rc.LedgerPath), $($rc.VBox.Dir)"
        Expect ($rc.StateGate.ToString() -match 'Get-P9SStateGateOf \$null' -and $rc.UpdateLedgerMark.ToString() -match 'Update-P9SStateLedgerMarkOf \$null') 'the real context does not gate on the account''s state folder'
        foreach ($p in @($fakeDefault, ($fdir.ToUpperInvariant() + '\state.json'), (Join-Path $fdir '..\state\state.json'))) {
            $x = New-P9SRealContext -StatePath $p -KitDir $kit -Config $null
            Expect ($x.UpdateLedgerMark -and $x.StateGate.ToString() -match 'Get-P9SStateGateOf') "the default given as $p is taken for a sandbox"
        }
        Expect (-not (Test-Path -LiteralPath (Split-Path -Parent $fdir))) 'a real context created the state folder or its root'
    } finally { Reset-Override $ov }
    foreach ($p in @((Join-Path $base 'n01\sandbox\state.json'), (Join-Path $base "n01\Temp\$($names.legacy)\state.json"), (Join-Path $base "n01\LocalAppData\$($names.root)\state\state.json"))) {
        $x = New-P9SRealContext -StatePath $p -KitDir $kit -Config $null
        $g = & $x.StateGate
        Expect ($g.status -ceq 'SANDBOX' -and $null -eq $x.UpdateLedgerMark -and [IO.Directory]::Exists((Split-Path -Parent $p))) "sandbox ${p}: gate $($g.status), mark $([bool]$x.UpdateLedgerMark)"
        # read, never called: each dispatcher of a sandbox only refuses
        foreach ($k in @($x.Dispatch.Keys)) { Expect ($x.Dispatch[$k].ToString().Trim() -ceq "throw 'STATE_SANDBOX: no VirtualBox write from a -StatePath sandbox'") "sandbox dispatcher ${k}: $($x.Dispatch[$k].ToString().Trim())" }
        Expect (@($x.Dispatch.Keys).Count -eq 5) "sandbox dispatchers: $(@($x.Dispatch.Keys) -join ',')"
        $res = New-P9SResult 'vm.start' 'P9S-D12'
        $m = ''; try { Invoke-P9SDispatch $x $res 'vm.start' @('P9S-D12') } catch { $m = $_.Exception.Message }
        Expect ($m -match 'STATE_NOT_READY:SANDBOX') "sandbox dispatch through the recheck: '$m'"
    }
}
Case 'N02' 'the monitor started without -StatePath refuses when the persistent state folder does not exist, and creates nothing' {
    # a copy of the toolkit whose default location is a synthetic one
    $kc = Join-Path $base 'n02-kit'; New-Item -ItemType Directory -Path $kc | Out-Null
    Get-ChildItem -LiteralPath $kit -File -Force | Where-Object { $_.Name -notin 'p9s-local.json', 'KIT-SHA256SUMS.txt' } | Copy-Item -Destination $kc
    $hl = Join-Path $kc 'hostmon-lib.ps1'; $src = [IO.File]::ReadAllText($hl)
    $fd = Join-Path $base "n02-default\LocalAppData\$($names.root)\state\state.json"
    Expect (([regex]::Matches($src, [regex]::Escape($hostmonLine))).Count -eq 1) 'the default location of hostmon-lib.ps1 is not the expected line'
    [IO.File]::WriteAllText($hl, $src.Replace($hostmonLine, "function Get-DefaultStatePath { '$($fd.Replace("'", "''"))' }"), [Text.UTF8Encoding]::new($false))
    $od = Join-Path $base 'n02-run'
    $o = @(& $pw -NoProfile -NonInteractive -File (Join-Path $kc 'host-monitor.ps1') -OutDir $od -DurationSeconds 1 2>&1 | ForEach-Object { "$_" }); $code = $LASTEXITCODE
    Expect ($code -eq 2 -and ($o -join ' ') -match 'state\.init or state\.migrate creates it, the monitor never does') "exit $code; $($o -join ' / ')"
    Expect (-not (Test-Path -LiteralPath (Join-Path $base 'n02-default\LocalAppData')) -and -not (Test-Path -LiteralPath $od)) 'the monitor created a folder'
    Expect (-not (Test-Path -LiteralPath $fd)) 'the monitor wrote a state file'
}
# ================================================================ the anchor mark of a ledger save
Case 'N03' 'every ledger save of an installed folder moves the anchor mark to the ledger on disk, through the registry, flushed and read back; nothing else of the anchor changes; the mark never moves back, never takes other content for the same revision, never moves while a setup holds setup.lock, and only on a READY anchor of this account and folder' {
    $e = New-Env 'n03'; Install-New $e; $c = New-EnvCoordCtx $e
    $a0 = Get-Anchor $e
    for ($i = 1; $i -le 5; $i++) {
        Expect (Save-Again $c) "save $i failed"
        $a = Get-Anchor $e; $m = Get-Mark (Get-LedgerPath $e)
        Expect ([long]$a.ledger.revision -eq $m.revision -and $a.ledger.sha256 -ceq $m.sha256 -and $m.revision -eq $i + 1) "save ${i}: anchor $($a.ledger.revision), ledger $($m.revision)"
        foreach ($k in @($a0.Keys | Where-Object { $_ -cne 'ledger' })) { Expect ([string]$a[$k] -ceq [string]$a0[$k]) "save ${i}: the anchor's $k changed" }
    }
    $null = Test-Status $e 'READY'
    $sc = New-EnvCtx $e; $m = Get-Mark (Get-LedgerPath $e); $before = Get-KeyText $e
    Expect (-not (Update-P9SStateLedgerMark $sc ($m.revision - 1) $m.sha256)) 'a lower revision was accepted'
    Expect ((Update-P9SStateLedgerMark $sc $m.revision $m.sha256)) 'the same mark again was refused'
    Expect (-not (Update-P9SStateLedgerMark $sc $m.revision ('0' * 64))) 'other content for the same revision was accepted'
    Expect (-not (Update-P9SStateLedgerMark $sc ($m.revision + 1) 'not-a-digest')) 'a malformed digest was accepted'
    Expect ((Get-KeyText $e) -ceq $before) 'a refused mark changed the anchor'
    # a setup holds setup.lock for its whole run: no mark meanwhile, the save fails
    $lk = [IO.FileStream]::new((Join-Path $e.root $names.setup_lock), 'OpenOrCreate', 'ReadWrite', 'None')
    try {
        $ok = Save-Again $c
        Expect ($ok -eq $false -and (Get-KeyText $e) -ceq $before -and (Get-MarkPosition $e) -ceq 'behind') "a save while a setup holds the lock: $ok; anchor $(Get-MarkPosition $e) the ledger"
    } finally { $lk.Dispose() }
    Expect (Save-Again $c) 'the save after the setup failed'
    $m = Get-Mark (Get-LedgerPath $e); $before = Get-KeyText $e
    foreach ($how in 'MIGRATING', 'account', 'state_dir') {
        $x = Get-Anchor $e; $y = [ordered]@{}; foreach ($k in $x.Keys) { $y[$k] = $x[$k] }
        switch ($how) { 'MIGRATING' { $y.phase = 'MIGRATING' } 'account' { $y.account = 'S-1-5-21-0-0-0-1001' } 'state_dir' { $y.state_dir = Join-Path $e.base 'elsewhere' } }
        Write-Anchor $e $y; $t0 = Get-KeyText $e
        Expect (-not (Update-P9SStateLedgerMark $sc ($m.revision + 1) $m.sha256) -and (Get-KeyText $e) -ceq $t0) "an anchor with another $how took the mark"
        Write-Anchor $e $null -Raw $before
    }
    Remove-Anchor $e
    Expect (-not (Update-P9SStateLedgerMark $sc ($m.revision + 1) $m.sha256) -and (Get-KeyText $e) -ceq 'absent') 'a missing anchor was created by the mark'
}
Case 'N04' 'a ledger save that fails at any point: at the ledger write, its read-back, the revision record, the registry write refused by its ACL, after the registry write before its flush, or at the anchor read-back: the save fails, the folder stays READY with the anchor at or behind the ledger, never ahead, and the next save puts the mark on the ledger' {
    $faults = [ordered]@{
        'ledger write' = @{ ctx = @{ WriteLedger = { param($p, $t) throw 'injected: ledger not written' } } }
        'ledger read-back' = @{ ctx = @{ WriteLedger = { param($p, $t) Write-StateAtomic -Path $p -Text ($t + ' ') } } }
        'revision record' = @{ ctx = @{ WriteLedgerRevision = { param($p, $t) throw 'injected: revision record not written' } } }
        'registry write refused' = @{ acl = $true }
        'before the flush' = @{ fn = 'Write-P9SStateRegistryValue'; body = { param($C, $Name, $Text) $k = [Microsoft.Win32.Registry]::CurrentUser.CreateSubKey($C.AnchorKey, $true); try { $k.SetValue($Name, $Text, [Microsoft.Win32.RegistryValueKind]::String) } finally { $k.Dispose() }; throw 'injected: flush failed' } }
        'anchor read-back' = @{ fn = 'Write-P9SStateRegistryValue'; body = { param($C, $Name, $Text) throw 'injected: the value read back is not the one written' } }
    }
    $i = 0
    foreach ($fk in $faults.Keys) {
        $i++; $e = New-Env "n04-$i"; Install-New $e; $c = New-EnvCoordCtx $e
        $null = Save-Again $c; $fx = $faults[$fk]; $ov = $null; $rule = $null
        if ($fx.ctx) { foreach ($k in $fx.ctx.Keys) { $c | Add-Member -NotePropertyName $k -NotePropertyValue $fx.ctx[$k] -Force } }
        if ($fx.fn) { $ov = Set-Override $fx.fn $fx.body }
        if ($fx.acl) {
            $kk = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey($e.key, [Microsoft.Win32.RegistryKeyPermissionCheck]::ReadWriteSubTree, [Security.AccessControl.RegistryRights]'ReadKey, ChangePermissions')
            $sec = $kk.GetAccessControl(); $rule = [Security.AccessControl.RegistryAccessRule]::new([Security.Principal.SecurityIdentifier]::new($me), [Security.AccessControl.RegistryRights]::SetValue, [Security.AccessControl.AccessControlType]::Deny)
            $sec.AddAccessRule($rule); $kk.SetAccessControl($sec); $kk.Dispose()
        }
        $ok = $null
        try { $ok = Save-Again $c } finally {
            if ($ov) { Reset-Override $ov }
            if ($rule) { $kk = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey($e.key, [Microsoft.Win32.RegistryKeyPermissionCheck]::ReadWriteSubTree, [Security.AccessControl.RegistryRights]'ReadKey, ChangePermissions'); $sec = $kk.GetAccessControl(); $null = $sec.RemoveAccessRule($rule); $kk.SetAccessControl($sec); $kk.Dispose() }
            foreach ($k in 'WriteLedger', 'WriteLedgerRevision') { $c.PSObject.Properties.Remove($k) }
        }
        $pos = Get-MarkPosition $e
        Expect ($ok -eq $false -and $pos -cin 'equal', 'behind') "${fk}: save returned $ok; anchor $pos the ledger"
        $null = Test-Status $e 'READY' -Label $fk
        Expect (Save-Again $c) "${fk}: the next save failed"
        Expect ((Get-MarkPosition $e) -ceq 'equal') "${fk}: after the next save the anchor is $(Get-MarkPosition $e) the ledger"
        $null = Test-Status $e 'READY' -Label "$fk, then saved"
    }
}

# ================================================================ the state changed under an action
# Between the recheck under the lock and the dispatch, the first VM read of
# the action changes the state folder as another writer could.
$changes = [ordered]@{
    'anchor deleted' = @{ want = 'STATE_NOT_READY:ANCHOR_MISSING'; act = { param($E, $Old) Remove-Anchor $E } }
    'ledger rolled back' = @{ want = 'STATE_NOT_READY:STATE_UNCERTAIN'; act = { param($E, $Old) [IO.File]::WriteAllBytes((Get-LedgerPath $E), $Old.ledger); [IO.File]::WriteAllBytes((Get-RevisionPath $E), $Old.revision) } }
    'ledger replaced, same revision' = @{ want = 'STATE_NOT_READY:STATE_UNCERTAIN'; act = { param($E, $Old)
        $p = Get-LedgerPath $E; $d = [IO.File]::ReadAllText($p) | ConvertFrom-Json -AsHashtable; $d.entries = @([ordered]@{ authorization_id = 'acpiauth-20260101-0000ffff'; target_vm = 'P9S-D12'; phase = 'OFF_CONFIRMED'; startups_used = 0; acpi_used = 0; events = @() })
        $t = $d | ConvertTo-Json -Depth 8; Write-StateAtomic -Path $p -Text $t
        Write-StateAtomic -Path (Get-RevisionPath $E) -Text ([ordered]@{ schema = 'p9s-acpi-validation-ledger-revision/1'; revision = [long]$d.revision; sha256 = (Get-P9STextSha256 $t); utc = [DateTime]::UtcNow.ToString('o') } | ConvertTo-Json) } }
    'ledger deleted' = @{ want = 'STATE_NOT_READY:STATE_UNCERTAIN'; act = { param($E, $Old) [IO.File]::Delete((Get-LedgerPath $E)) } }
    'ledger corrupt' = @{ want = 'STATE_NOT_READY:STATE_UNCERTAIN'; act = { param($E, $Old) Write-Text (Get-LedgerPath $E) '{"schema":"p9s-acpi-validation-ledger/1","revision":' } }
    'anchor of another installation' = @{ want = 'STATE_NOT_READY:STATE_MISMATCH'; act = { param($E, $Old) $a = Get-Anchor $E; $a.installation_id = New-Id; Write-Anchor $E $a } }
    'another installation in READY' = @{ want = 'STATE_INSTALLATION_CHANGED'; act = { param($E, $Old)
        $id = New-Id; Write-Doc (Join-Path $E.state $names.install) (New-InstallDoc $E $id); Write-Anchor $E (New-AnchorDoc $E $id -Mark (Get-Mark (Get-LedgerPath $E))) } }
}
# the ledger and its revision record one save before the current ones
function Get-OlderPair($E, $ctx) {
    $o = @{ ledger = [IO.File]::ReadAllBytes((Get-LedgerPath $E)); revision = [IO.File]::ReadAllBytes((Get-RevisionPath $E)) }
    if (-not (Save-Again $ctx)) { throw 'fixture save failed' }
    $o
}
Case 'N05' 'the state folder changed between the recheck under the lock and the dispatch, on the real classifier: the anchor deleted, the ledger rolled back, replaced with the same revision, deleted or corrupt, the anchor of another installation, another installation: nothing is dispatched, and the next action is refused by the status' {
    $i = 0
    foreach ($ck in $changes.Keys) {
        $i++; $e = New-Env "n05-$i"; Install-New $e; $c = New-EnvCoordCtx $e
        $o = Open $c; Expect ($o.outcome -ceq 'DONE') "${ck}: open $(Why $o)"
        $old = Get-OlderPair $e $c; $act = $changes[$ck].act
        $c.Prov.onVmRead = { param($vm) & $act $e $old }.GetNewClosure()
        $r = Act $c @{ Action = 'guest.key'; Vm = 'P9S-D12'; Key = 'Enter' }
        Expect ($r.outcome -ceq 'REFUSED' -and (Has $r 'STATE_CHANGED_BEFORE_DISPATCH') -and (Has $r $changes[$ck].want) -and $c.Calls.Count -eq 0) "${ck}: $(Why $r); calls $($c.Calls.Count)"
        $r2 = Act $c @{ Action = 'guest.key'; Vm = 'P9S-D12'; Key = 'Enter' }
        if ($changes[$ck].want -ceq 'STATE_INSTALLATION_CHANGED') { Expect ($r2.outcome -ceq 'DISPATCHED') "${ck}: the next action on the new installation: $(Why $r2)" }
        else { Expect ($r2.outcome -ceq 'REFUSED' -and (Has $r2 $changes[$ck].want) -and $c.Calls.Count -eq 0) "${ck}: the next action: $(Why $r2)" }
    }
}
Case 'N06' 'an emergency whose state folder changes after its recheck: with no validation the stop request records the request and the dispatch is refused (spent, unresolved); with an unresolved validation the ledger cannot take the mark and nothing is sent; once re-anchored the request stays spent' {
    $e = New-Env 'n06a'; Install-New $e; $c = New-EnvCoordCtx $e; $null = Open $c
    $c.Prov.onVmRead = { param($vm) Remove-Anchor $e }.GetNewClosure()
    $r = Act $c @{ Action = 'emergency.acpi-shutdown'; Vm = 'P9S-D12' }
    $q = Read-P9SStopRequestState $c
    Expect ($r.outcome -ceq 'FAILED' -and (Has $r 'EMERGENCY_UNRESOLVED') -and (Has $r 'STATE_NOT_READY:ANCHOR_MISSING') -and $c.Calls.Count -eq 0 -and $q.ok -and $q.doc.acpi.Contains('P9S-D12')) "no validation: $(Why $r); calls $($c.Calls.Count); recorded $($q.ok -and $q.doc.acpi.Contains('P9S-D12'))"
    $e = New-Env 'n06b'; Install-New $e; $c = New-EnvCoordCtx $e; $null = Open $c
    $lg = Read-P9SAcpiLedger $c
    $lg.doc.entries = @([ordered]@{ authorization_id = 'acpiauth-20261127-0000aaaa'; target_vm = 'P9S-D12'; phase = 'RUNNING'; startups_used = 1; acpi_used = 0; acpi_source = $null; events = @() })
    Expect (Save-P9SAcpiLedger $c $lg.doc) 'fixture: the validation entry was not saved'
    $c.Prov.onVmRead = { param($vm) Remove-Anchor $e }.GetNewClosure()
    $r = Act $c @{ Action = 'emergency.acpi-shutdown'; Vm = 'P9S-D12' }
    $en = Get-P9SAcpiEntry (Read-P9SAcpiLedger $c).doc 'acpiauth-20261127-0000aaaa'
    Expect ($r.outcome -ceq 'FAILED' -and (Has $r 'ACPI_REQUEST_NOT_SENT') -and $c.Calls.Count -eq 0 -and [int]$en.acpi_used -eq 1) "with a validation: $(Why $r); calls $($c.Calls.Count); acpi_used on disk $($en.acpi_used)"
    $null = Test-Status $e 'ANCHOR_MISSING'
    $ri = New-P9SStateInitContext -State (New-EnvCtx $e) -VmNames @('P9S-D12', 'P9S-U24') -ReadVmStateIn { param($vm, $dir) 'off' } -CountVmProcesses { 0 }
    $ra = Invoke-P9SStateReanchor -I $ri
    Expect ($ra.outcome -ceq 'DONE') "reanchor: $(Why $ra)"
    $r = Act $c @{ Action = 'emergency.acpi-shutdown'; Vm = 'P9S-D12' }
    Expect ($r.outcome -ceq 'REFUSED' -and (Has $r 'ACPI_ALREADY_REQUESTED') -and $c.Calls.Count -eq 0) "once re-anchored: $(Why $r)"
}

# ================================================================ state.reanchor
function New-ReanchorCtx($E, [hashtable]$Vms = @{}, [int]$Procs = 0) {
    $t = @{ 'P9S-D12' = 'off'; 'P9S-U24' = 'off' }; foreach ($k in $Vms.Keys) { $t[$k] = $Vms[$k] }
    New-P9SStateInitContext -State (New-EnvCtx $E) -VmNames @('P9S-D12', 'P9S-U24') -ReadVmStateIn { param($vm, $dir) $t[$vm] }.GetNewClosure() -CountVmProcesses { $Procs }.GetNewClosure()
}
function Get-AuditPath($E) { Join-Path $E.state (Get-P9SStateMigrateNames).audit }
Case 'N07' 'state.reanchor from ANCHOR_MISSING: a dry run writes nothing; the run writes the anchor of the folder''s installation with a new initialized_utc and the ledger''s mark, READY again, one audit line; an authorization issued before it is void' {
    $e = New-Env 'n07'; Install-New $e; $c = New-EnvCoordCtx $e; $null = Save-Again $c
    $a0 = Get-Anchor $e
    # an authorization approved and valid after the installation time: usable now, void once re-anchored
    $t1 = (ConvertFrom-P9SStateUtcText $a0.initialized_utc).AddMilliseconds(1).ToString('o')
    $auth = [pscustomobject]@{ approved_utc = $t1; valid_from_utc = $t1; expires_utc = [DateTime]::UtcNow.AddHours(1).ToString('o') }
    $w0 = Test-P9SAcpiAuthorizationTime $auth ([DateTime]::UtcNow) $c
    Expect ($null -eq $w0) "the authorization before the reanchor: '$w0'"
    Start-Sleep -Milliseconds 20; Remove-Anchor $e
    $null = Test-Status $e 'ANCHOR_MISSING'
    $before = Get-Files $e.base
    $d = Invoke-P9SStateReanchor -I (New-ReanchorCtx $e) -DryRun
    Expect ($d.outcome -ceq 'DRYRUN' -and (Get-Files $e.base) -ceq $before -and (Get-KeyText $e) -ceq 'absent') "dry run: $(Why $d)"
    $r = Invoke-P9SStateReanchor -I (New-ReanchorCtx $e)
    $a = Get-Anchor $e; $m = Get-Mark (Get-LedgerPath $e)
    Expect ($r.outcome -ceq 'DONE' -and $a.installation_id -ceq $e.id -and $a.phase -ceq 'READY' -and [long]$a.ledger.revision -eq $m.revision -and $a.ledger.sha256 -ceq $m.sha256) "reanchor: $(Why $r)"
    Expect ((ConvertFrom-P9SStateUtcText $a.initialized_utc) -gt (ConvertFrom-P9SStateUtcText $a0.initialized_utc) -and $a.created_utc -ceq [string](ConvertFrom-P9SStateJson ([IO.File]::ReadAllBytes((Join-Path $e.state $names.install)))).created_utc) "initialized $($a0.initialized_utc) -> $($a.initialized_utc); created $($a.created_utc), not the install record's"
    $null = Test-Status $e 'READY'
    $lines = @(Get-Content -LiteralPath (Get-AuditPath $e) | ForEach-Object { $_ | ConvertFrom-Json } | Where-Object { $_.action -ceq 'state.reanchor' })
    Expect ($lines.Count -eq 1 -and $lines[0].outcome -ceq 'DONE' -and $lines[0].dispatched -eq 0) "audit lines $($lines.Count)"
    $w = Test-P9SAcpiAuthorizationTime $auth ([DateTime]::UtcNow) $c
    Expect ($w -ceq 'AUTHORIZATION_BEFORE_INSTALLATION') "the same authorization after the reanchor: '$w'"
    $o = Open $c; Expect ($o.outcome -ceq 'DONE') "a session after the reanchor: $(Why $o)"
}
Case 'N08' 'state.reanchor refuses: -Replace without an unreadable anchor, an unreadable anchor without -Replace, STATE_UNCERTAIN, READY, a VM not off, a VM process, a ledger or install record missing, a migrated folder whose earlier folder changed, a busy setup; nothing written; with -Replace the unreadable text is kept in the audit first' {
    $e = New-Env 'n08a'; Install-New $e; Remove-Anchor $e
    Test-Refused (Invoke-P9SStateReanchor -I (New-ReanchorCtx $e) -Replace) 'NOTHING_TO_REPLACE:ANCHOR_MISSING' '-Replace from ANCHOR_MISSING'
    $before = Get-Files $e.base
    Test-Refused (Invoke-P9SStateReanchor -I (New-ReanchorCtx $e @{ 'P9S-U24' = 'running' })) '*P9S-U24*' 'a VM running'
    Test-Refused (Invoke-P9SStateReanchor -I (New-ReanchorCtx $e -Procs 1)) 'VM_PROCESS_RUNNING:1' 'a VM process'
    $lk = [IO.FileStream]::new((Join-Path $e.root $names.setup_lock), 'OpenOrCreate', 'ReadWrite', 'None')
    try { Test-Refused (Invoke-P9SStateReanchor -I (New-ReanchorCtx $e)) 'SETUP_BUSY' 'a setup holding the lock' } finally { $lk.Dispose() }
    Expect ((Get-KeyText $e) -ceq 'absent') 'a refused reanchor wrote the anchor'
    $e = New-Env 'n08b'; Install-New $e; [IO.File]::Delete((Get-LedgerPath $e)); [IO.File]::Delete((Get-RevisionPath $e)); Remove-Anchor $e
    Test-Refused (Invoke-P9SStateReanchor -I (New-ReanchorCtx $e)) 'LEDGER:*' 'ledger missing'
    $e = New-Env 'n08c'; Install-New $e; [IO.File]::Delete((Join-Path $e.state $names.install)); Remove-Anchor $e
    $r = Invoke-P9SStateReanchor -I (New-ReanchorCtx $e); Expect ($r.outcome -ceq 'REFUSED' -and (Get-KeyText $e) -ceq 'absent') "install record missing: $(Why $r)"
    $e = New-Env 'n08d'; Install-New $e; $raw = '{"schema":"p9s-state-anchor/1","phase":'; Write-Anchor $e $null -Raw $raw
    $null = Test-Status $e 'ANCHOR_UNREADABLE'
    Test-Refused (Invoke-P9SStateReanchor -I (New-ReanchorCtx $e)) 'ANCHOR_UNREADABLE_NEEDS_REPLACE' 'an unreadable anchor without -Replace'
    Expect ((Get-KeyText $e) -ceq $raw) 'the unreadable anchor changed without -Replace'
    $r = Invoke-P9SStateReanchor -I (New-ReanchorCtx $e) -Replace
    Expect ($r.outcome -ceq 'DONE') "-Replace: $(Why $r)"
    $kept = @(Get-Content -LiteralPath (Get-AuditPath $e) | ForEach-Object { $_ | ConvertFrom-Json } | Where-Object { $_.outcome -ceq 'ANCHOR_KEPT' })
    Expect ($kept.Count -eq 1 -and [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String($kept[0].replaced_anchor_utf8_base64)) -ceq $raw) "the replaced text is not in the audit ($($kept.Count) lines)"
    $null = Test-Status $e 'READY' -Label 'replaced'
    $e = New-Env 'n08e'; Install-New $e; [IO.File]::Delete((Get-LedgerPath $e)); $before = Get-KeyText $e
    $null = Test-Status $e 'STATE_UNCERTAIN'
    $r = Invoke-P9SStateReanchor -I (New-ReanchorCtx $e)
    Test-Refused $r 'NOT_REANCHORABLE:STATE_UNCERTAIN' 'STATE_UNCERTAIN'
    Expect (($r.lines -join ' ') -match 'never re-marked' -and (Get-KeyText $e) -ceq $before) "STATE_UNCERTAIN: $($r.lines -join ' | ')"
    $e = New-Env 'n08f'; Install-New $e
    Test-Refused (Invoke-P9SStateReanchor -I (New-ReanchorCtx $e)) 'NOT_REANCHORABLE:READY' 'READY'
    $e = New-Env 'n08g'; Install-Migrated $e; Remove-Anchor $e
    [IO.File]::AppendAllText((Join-Path $e.legacy 'coordinator-session.json'), ' ')
    $r = Invoke-P9SStateReanchor -I (New-ReanchorCtx $e)
    Expect ($r.outcome -ceq 'REFUSED' -and (Get-KeyText $e) -ceq 'absent') "a migrated folder whose earlier folder changed: $(Why $r)"
    $e = New-Env 'n08h'; Install-Migrated $e; Remove-Anchor $e
    $r = Invoke-P9SStateReanchor -I (New-ReanchorCtx $e)
    Expect ($r.outcome -ceq 'DONE') "a migrated folder with its earlier folder intact: $(Why $r)"
    $null = Test-Status $e 'READY' -Label 'migrated, re-anchored'
}

# ================================================================ the earlier folder, STOP, ABORT, restart
Case 'N09' 'after a migration: an earlier version appending to a history of the earlier folder leaves READY; one changing a record there, or putting a real quarantine in place of the tombstone, makes the state STATE_DIVERGED and every action is refused; the earlier folder gone, READY' {
    $e = New-Env 'n09'; Install-Migrated $e; $c = New-EnvCoordCtx $e
    $o = Open $c; Expect ($o.outcome -ceq 'DONE') "open: $(Why $o)"
    Add-Line (Join-Path $e.legacy 'coordinator-audit.jsonl') @{ action = 'emergency.status'; outcome = 'DONE' }
    $r = Act $c @{ Action = 'guest.key'; Vm = 'P9S-D12'; Key = 'Enter' }; Expect ($r.outcome -ceq 'DISPATCHED') "history appended: $(Why $r)"
    $tp = Join-Path $e.legacy 'vbox-quarantine.json'; $tomb = [IO.File]::ReadAllBytes($tp)
    Write-Doc $tp ([ordered]@{ schema = 'p9s-vbox-quarantine/1'; reason = 'VBOX_COMMAND_TIMEOUT'; operation_id = 'vbox-20261127T120000000Z-1-0000abcd' })
    foreach ($rq in @(@{ Action = 'guest.key'; Vm = 'P9S-D12'; Key = 'Enter' }, @{ Action = 'emergency.acpi-shutdown'; Vm = 'P9S-D12' })) { $r = Act $c $rq; Expect ($r.outcome -ceq 'REFUSED' -and (Has $r 'STATE_NOT_READY:STATE_DIVERGED')) "a quarantine in place of the tombstone, $($rq.Action): $(Why $r)" }
    [IO.File]::WriteAllBytes($tp, $tomb)
    Write-Text (Join-Path $e.legacy 'coordinator-stop.request') '{"acpi":{},"unresolved":{}}'
    $r = Act $c @{ Action = 'vm.start'; Vm = 'P9S-U24' }; Expect ($r.outcome -ceq 'REFUSED' -and (Has $r 'STATE_NOT_READY:STATE_DIVERGED') -and $c.Calls.Count -eq 1) "a record added: $(Why $r)"
    Remove-Tree $e.legacy
    $r = Act $c @{ Action = 'guest.key'; Vm = 'P9S-D12'; Key = 'Enter' }; Expect ($r.outcome -ceq 'DISPATCHED' -and $c.Calls.Count -eq 2) "the earlier folder gone: $(Why $r)"
}
Case 'N10' 'STOP and ABORT while the state is not READY are recorded once it is READY again, in a new context as after a restart: the session halts, a new session needs a new monitor run, an ABORT its acknowledgement; the emergency is available again' {
    foreach ($mon in 'STOP', 'ABORT') {
        $e = New-Env "n10-$mon"; Install-New $e; $c = New-EnvCoordCtx $e; $null = Open $c
        $lp = Get-LedgerPath $e; $keep = [IO.File]::ReadAllBytes($lp); [IO.File]::Delete($lp)
        $c2 = New-EnvCoordCtx $e @($(if ($mon -ceq 'STOP') { $STOP_A } else { $ABORT_A }))
        $sess = Get-Content -Raw -LiteralPath $c2.SessionPath
        foreach ($rq in @(@{ Action = 'guest.key'; Vm = 'P9S-D12'; Key = 'Enter' }, @{ Action = 'emergency.acpi-shutdown'; Vm = 'P9S-D12' }, @{ Action = 'session.close' })) {
            $r = Act $c2 $rq; Expect ($r.outcome -ceq 'REFUSED' -and (Has $r 'STATE_NOT_READY:STATE_UNCERTAIN')) "${mon}, not READY, $($rq.Action): $(Why $r)"
        }
        Expect ((Get-Content -Raw -LiteralPath $c2.SessionPath) -ceq $sess -and -not (Test-Path -LiteralPath $c2.StopRequestPath)) "${mon}: written while not READY"
        [IO.File]::WriteAllBytes($lp, $keep)
        $c3 = New-EnvCoordCtx $e @($(if ($mon -ceq 'STOP') { $STOP_A } else { $ABORT_A }))
        $r = Act $c3 @{ Action = 'guest.key'; Vm = 'P9S-D12'; Key = 'Enter' }
        Expect ($r.outcome -ceq 'REFUSED' -and (Sess $c3).lifecycle -ceq "HALTED_$mon" -and $c3.Calls.Count -eq 0) "${mon}, READY again: $(Why $r); lifecycle $((Sess $c3).lifecycle)"
        $em = Act $c3 @{ Action = 'emergency.acpi-shutdown'; Vm = 'P9S-D12' }
        Expect ($em.outcome -ceq 'DONE' -and $c3.Calls.Count -eq 1) "${mon}: the emergency: $(Why $em)"
        $c4 = New-EnvCoordCtx $e @($READY_A) @{ 'P9S-D12' = 'off' }
        $o = Open $c4; Expect ($o.outcome -ceq 'REFUSED') "${mon}: a session on the same monitor run: $(Why $o)"
    }
}

# ================================================================ other processes
# A child process acting on a synthetic state folder: it waits at its first
# VM read (after its recheck under the lock) for the "go" event; its result
# goes to <out>.json, its dispatches to <out>.dispatch.log.
$actWorker = Join-Path $OutDir 'act-worker.ps1'
[IO.File]::WriteAllText($actWorker, @'
param([string]$Kit, [string]$Lad, [string]$Tmp, [string]$Key, [string]$Out, [string]$Events, [string]$Action, [string]$Monitor, [long]$Now, [long]$Freq)
$ErrorActionPreference = 'Stop'
. (Join-Path $Kit 'hostmon-lib.ps1'); . (Join-Path $Kit 'coordinator-lib.ps1'); . (Join-Path $Kit 'p9s-state.ps1'); . (Join-Path $Kit 'p9s-state-init.ps1')
$script:sc = New-P9SStateContext -LocalAppData $Lad -TempPath $Tmp -AnchorKey $Key
$script:mon = [IO.File]::ReadAllText($Monitor); $script:now = $Now; $script:log = "$Out.dispatch.log"; $script:held = $false; $script:off = $false; $script:ev = $Events
$dir = $script:sc.StateDir; $me = Get-Process -Id $PID
$ctx = [pscustomobject]@{
    StatePath = Join-Path $dir 'state.json'; SessionPath = Join-Path $dir 'coordinator-session.json'; LockPath = Join-Path $dir 'coordinator.lock'
    StopRequestPath = Join-Path $dir 'coordinator-stop.request'; AuditPath = Join-Path $dir 'coordinator-audit.jsonl'; ThresholdsPath = Join-Path $Kit 'paging-thresholds.json'
    ReadState = { param($p) [pscustomobject]@{ ok = $true; text = $script:mon; error = $null } }
    NowQpc = { $script:now }; QpcFrequency = $Freq; IsMonitorAlive = { param($a, $b) $true }
    ReadVmState = { param($vm)
        if (-not $script:held) {
            $script:held = $true
            $r = [Threading.EventWaitHandle]::OpenExisting("$($script:ev)-reached"); $g = [Threading.EventWaitHandle]::OpenExisting("$($script:ev)-go")
            $null = $r.Set(); if (-not $g.WaitOne(120000)) { throw 'not released' }
        }
        if ($vm -ceq 'P9S-D12' -and -not $script:off) { 'running' } else { 'off' } }
    ReadVmConfig = { param($vm) @() }
    Dispatch = @{ 'vm.start' = { param($vm) Add-Content -LiteralPath $script:log -Value "vm.start $vm" }; 'guest.key' = { param($vm, $codes) Add-Content -LiteralPath $script:log -Value "guest.key $vm" }
        'guest.text' = { param($vm, $t) Add-Content -LiteralPath $script:log -Value "guest.text $vm" }; 'guest.unlock' = { param($vm) Add-Content -LiteralPath $script:log -Value "guest.unlock $vm" }
        'vm.acpi' = { param($vm) Add-Content -LiteralPath $script:log -Value "vm.acpi $vm"; $script:off = $true } }
    Sleep = { param($ms) }; Pid = $PID; ProcessStartTicks = $me.StartTime.ToUniversalTime().Ticks
    EmergencyLockWaitMs = 10000; CalibrationHostChecks = { @() }
    ObservationPath = Join-Path $dir 'vm-observations.jsonl'; LedgerPath = Join-Path $dir 'acpi-validation-ledger.json'; UtcNow = { [DateTime]::UtcNow }; AcpiValidationLive = $true
    CountVmProcesses = { 0 }
    StateGate = { Get-P9SStateGateOf $script:sc }
    UpdateLedgerMark = { param($rev, $sha) Update-P9SStateLedgerMarkOf $script:sc $rev $sha }
}
$req = if ($Action -ceq 'emergency') { @{ Action = 'emergency.acpi-shutdown'; Vm = 'P9S-D12' } } else { @{ Action = 'guest.key'; Vm = 'P9S-D12'; Key = 'Enter' } }
$r = Invoke-P9SAction -Ctx $ctx -Request $req
[IO.File]::WriteAllText("$Out.json", ([ordered]@{ outcome = $r.outcome; reasons = @($r.reasons); dispatched = $r.dispatched; lines = @($r.lines) } | ConvertTo-Json -Compress))
'@, [Text.UTF8Encoding]::new($false))
$monFile = Join-Path $OutDir 'monitor-ready.json'; Write-Text $monFile $READY_A
function New-Events([string]$Name) {
    $n = "Local\p9s-enforce-$run-$Name"
    [pscustomobject]@{ name = $n; reached = [Threading.EventWaitHandle]::new($false, [Threading.EventResetMode]::ManualReset, "$n-reached"); go = [Threading.EventWaitHandle]::new($false, [Threading.EventResetMode]::ManualReset, "$n-go") }
}
function Start-ActWorker($E, $Ev, [string]$Action) {
    $o = Join-Path $OutDir "w-$([guid]::NewGuid().ToString('N').Substring(0, 10))"
    $q = { param($s) '"' + $s + '"' }
    $al = @('-NoProfile', '-NonInteractive', '-File', (& $q $actWorker), '-Kit', (& $q $kit), '-Lad', (& $q $E.lad), '-Tmp', (& $q $E.tmp), '-Key', (& $q $E.key), '-Out', (& $q $o),
        '-Events', (& $q $Ev.name), '-Action', $Action, '-Monitor', (& $q $monFile), '-Now', [string]$QNOW, '-Freq', [string]$QF)
    $p = Start-Process -FilePath $pw -PassThru -WindowStyle Hidden -ArgumentList $al -RedirectStandardOutput "$o.stdout" -RedirectStandardError "$o.stderr"
    $null = $p.Handle
    [pscustomobject]@{ p = $p; out = $o }
}
function Get-WorkerResult($W) { if (Test-Path -LiteralPath "$($W.out).json") { Get-Content -Raw -LiteralPath "$($W.out).json" | ConvertFrom-Json } else { $null } }
function Get-WorkerErr($W) { $t = if (Test-Path -LiteralPath "$($W.out).stderr") { Get-Content -Raw -LiteralPath "$($W.out).stderr" } else { '' }; if ($t) { $t.Trim() } else { '' } }
function Get-Dispatched($W) { if (Test-Path -LiteralPath "$($W.out).dispatch.log") { @(Get-Content -LiteralPath "$($W.out).dispatch.log") } else { @() } }
function Wait-Held($Ev, $W, [int]$Ms = 120000) {
    $sw = [Diagnostics.Stopwatch]::StartNew()
    while ($sw.ElapsedMilliseconds -lt $Ms) { if ($Ev.reached.WaitOne(200)) { return $true }; if ($W.p.HasExited) { return $Ev.reached.WaitOne(0) } }
    $false
}
Case 'N11' 'another process changes the state folder while an action of this one waits between its recheck under the lock and its dispatch: nothing is dispatched; the same interleaving without a change dispatches once' {
    $plan = [ordered]@{ 'none' = $null; 'anchor deleted' = 'anchor deleted'; 'ledger rolled back' = 'ledger rolled back'; 'another installation in READY' = 'another installation in READY' }
    $i = 0; $runs = @()
    foreach ($pk in $plan.Keys) {
        foreach ($what in 'action', 'emergency') {
            if ($what -ceq 'emergency' -and $pk -cnotin 'none', 'anchor deleted') { continue }
            $i++; $e = New-Env "n11-$i"; Install-New $e; $c = New-EnvCoordCtx $e; $null = Open $c
            $old = Get-OlderPair $e $c; $ev = New-Events "n11-$i"
            $runs += [pscustomobject]@{ change = $pk; what = $what; e = $e; old = $old; ev = $ev; w = (Start-ActWorker $e $ev $what) }
        }
    }
    foreach ($x in $runs) {
        if (-not (Wait-Held $x.ev $x.w)) { Expect $false "$($x.change) $($x.what): not held: $(Get-WorkerErr $x.w)"; continue }
        if ($plan[$x.change]) { & $changes[$plan[$x.change]].act $x.e $x.old }
        $null = $x.ev.go.Set()
    }
    foreach ($x in $runs) {
        $null = $x.w.p.WaitForExit(120000)
        $j = Get-WorkerResult $x.w; $sent = @(Get-Dispatched $x.w)
        $tag = "$($x.change), $($x.what)"
        if (-not $j) { Expect $false "${tag}: no result: $(Get-WorkerErr $x.w)"; continue }
        if ($x.change -ceq 'none') { Expect ($sent.Count -eq 1 -and $j.outcome -cin 'DISPATCHED', 'DONE') "${tag}: $($j.outcome) [$(@($j.reasons) -join ',')]; dispatched $($sent.Count)" }
        else {
            Expect ($sent.Count -eq 0 -and $j.outcome -cin 'REFUSED', 'FAILED' -and (@($j.reasons) -ccontains $changes[$plan[$x.change]].want)) "${tag}: $($j.outcome) [$(@($j.reasons) -join ',')]; dispatched $($sent.Count)"
        }
        Expect (-not (Get-WorkerErr $x.w)) "${tag}: errors: $(Get-WorkerErr $x.w)"
    }
}
# A child process saving the ledger once through the anchor mark, ended by
# TerminateProcess at one point of the save (it writes <out>.reached first).
$saveWorker = Join-Path $OutDir 'save-worker.ps1'
[IO.File]::WriteAllText($saveWorker, @'
param([string]$Kit, [string]$Lad, [string]$Tmp, [string]$Key, [string]$Out, [string]$Point)
$ErrorActionPreference = 'Stop'
. (Join-Path $Kit 'hostmon-lib.ps1'); . (Join-Path $Kit 'coordinator-lib.ps1'); . (Join-Path $Kit 'p9s-state.ps1'); . (Join-Path $Kit 'p9s-state-init.ps1')
$script:sc = New-P9SStateContext -LocalAppData $Lad -TempPath $Tmp -AnchorKey $Key
$script:point = $Point; $script:out = $Out
function Stop-Here([string]$P) { if ($P -ceq $script:point) { [IO.File]::WriteAllText("$($script:out).reached", $P); [Diagnostics.Process]::GetCurrentProcess().Kill() } }
$dir = $script:sc.StateDir
$ctx = [pscustomobject]@{ LedgerPath = Join-Path $dir 'acpi-validation-ledger.json'; SessionPath = Join-Path $dir 'coordinator-session.json'; StopRequestPath = Join-Path $dir 'coordinator-stop.request'
    WriteLedger = { param($p, $t) Stop-Here 'before-ledger'; Write-StateAtomic -Path $p -Text $t; Stop-Here 'ledger-written' }
    WriteLedgerRevision = { param($p, $t) Write-StateAtomic -Path $p -Text $t; Stop-Here 'revision-written' }
    UpdateLedgerMark = { param($rev, $sha) $ok = Update-P9SStateLedgerMarkOf $script:sc $rev $sha; Stop-Here 'mark-written'; $ok } }
if ($Point -ceq 'anchor-set') {
    Set-Item -LiteralPath function:script:Write-P9SStateRegistryValue -Value { param($C, $Name, $Text) $k = [Microsoft.Win32.Registry]::CurrentUser.CreateSubKey($C.AnchorKey, $true); $k.SetValue($Name, $Text, [Microsoft.Win32.RegistryValueKind]::String); Stop-Here 'anchor-set' }
}
$lg = Read-P9SAcpiLedger $ctx
$ok = Save-P9SAcpiLedger $ctx $lg.doc
[IO.File]::WriteAllText("$Out.json", ([ordered]@{ saved = $ok } | ConvertTo-Json -Compress))
'@, [Text.UTF8Encoding]::new($false))
Case 'N12' 'a ledger save ended by TerminateProcess before the ledger, after it, after its revision record, after the registry write before its flush, or after the mark: READY, the anchor at or behind the ledger, never ahead; the next save from another process puts the mark on the ledger' {
    $points = 'before-ledger', 'ledger-written', 'revision-written', 'anchor-set', 'mark-written'
    $runs = @(foreach ($pt in $points) {
        $e = New-Env "n12-$pt"; Install-New $e; $c = New-EnvCoordCtx $e; $null = Save-Again $c
        $o = Join-Path $OutDir "s-$pt"; $q = { param($s) '"' + $s + '"' }
        $al = @('-NoProfile', '-NonInteractive', '-File', (& $q $saveWorker), '-Kit', (& $q $kit), '-Lad', (& $q $e.lad), '-Tmp', (& $q $e.tmp), '-Key', (& $q $e.key), '-Out', (& $q $o), '-Point', $pt)
        $p = Start-Process -FilePath $pw -PassThru -WindowStyle Hidden -ArgumentList $al -RedirectStandardOutput "$o.stdout" -RedirectStandardError "$o.stderr"; $null = $p.Handle
        [pscustomobject]@{ pt = $pt; e = $e; c = $c; p = $p; out = $o; rev = (Get-Mark (Get-LedgerPath $e)).revision }
    })
    foreach ($x in $runs) {
        $null = $x.p.WaitForExit(120000)
        Expect ((Test-Path -LiteralPath "$($x.out).reached") -and -not (Test-Path -LiteralPath "$($x.out).json")) "$($x.pt): the process was not ended at its point"
        $pos = Get-MarkPosition $x.e; $now = (Get-Mark (Get-LedgerPath $x.e)).revision
        $want = if ($x.pt -ceq 'before-ledger') { $x.rev } else { $x.rev + 1 }
        Expect ($pos -cin 'equal', 'behind' -and $now -eq $want) "$($x.pt): anchor $pos the ledger; ledger revision $now, expected $want"
        $null = Test-Status $x.e 'READY' -Label $x.pt
        Expect (Save-Again $x.c) "$($x.pt): the next save failed"
        Expect ((Get-MarkPosition $x.e) -ceq 'equal') "$($x.pt): after the next save the anchor is $(Get-MarkPosition $x.e) the ledger"
    }
}

# ================================================================ the entry points on the real locations
$n13 = 'the entry points without -StatePath on a machine without a state folder: every action that can spend budget or reach a VM is refused with NOT_INITIALIZED and nothing is created; session.status runs quietly; the emergency says to shut down outside the toolkit; the shims refuse; state.reanchor refuses'
if (-not $env:P9S_TEST_REAL_LOCATIONS) { NotRun 'N13' $n13 'runs only in the main run of run-all.ps1 -CleanRunner (a clean runner)' }
else {
$realLegacy = @((Join-Path ([IO.Path]::GetTempPath()) $names.legacy), (Join-Path (Join-Path $realLad 'Temp') $names.legacy))
$realBusy = @(@($realLegacy | Where-Object { Test-Path -LiteralPath $_ }) + @(if ($realAtStart -ne 'real root False; real anchor key False') { 'state root or anchor' }))
if ($realBusy.Count) { NotRun 'N13' $n13 "the real state root, the anchor key or an earlier-version folder exists here ($($realBusy.Count) paths)" }
else {
Case 'N13' $n13 {
    $cli = Join-Path $kit 'p9s-action.ps1'
    $run1 = { param([string[]]$A) $o = @(& $pw -NoProfile -NonInteractive -File $cli @A 2>&1 | ForEach-Object { "$_" }); [pscustomobject]@{ code = $LASTEXITCODE; out = $o; text = ($o -join ' / ') } }
    foreach ($a in @(@('-Action', 'session.open', '-Kind', 'Test'), @('-Action', 'vm.start', '-Vm', 'P9S-D12'), @('-Action', 'vm.start', '-Vm', 'P9S-D12', '-DryRun'), @('-Action', 'acpi.start'), @('-Action', 'vbox.recover', '-Operation', 'vbox-20261127T120000000Z-1-0000abcd'))) {
        $x = & $run1 $a
        Expect ($x.code -eq 1 -and $x.text -match 'outcome: REFUSED; STATE_NOT_READY:NOT_INITIALIZED') "$($a -join ' '): exit $($x.code); $($x.text)"
    }
    $x = & $run1 @('-Action', 'emergency.acpi-shutdown', '-Vm', 'P9S-D12')
    Expect ($x.code -eq 1 -and $x.text -match 'Nothing was raised, written or sent' -and $x.text -match 'STATE_NOT_READY:NOT_INITIALIZED') "emergency: exit $($x.code); $($x.text)"
    $x = & $run1 @('-Action', 'session.status')
    Expect ($x.code -eq 0 -and $x.text -match 'nothing is written and VirtualBox is not called') "session.status: exit $($x.code); $($x.text)"
    $x = & $run1 @('-Action', 'state.reanchor')
    Expect ($x.code -eq 1 -and $x.text -match 'NOT_REANCHORABLE:NOT_INITIALIZED') "state.reanchor: exit $($x.code); $($x.text)"
    $m = ''; try { & (Join-Path $kit 'com-type.ps1') -Vm P9S-D12 -Text 'abc' } catch { $m = $_.Exception.Message }
    Expect ($m -match 'STATE_NOT_READY:NOT_INITIALIZED') "com-type: $m"
    $m = ''; try { & (Join-Path $kit 'vm-action.ps1') -Vm P9S-D12 -Action Start } catch { $m = $_.Exception.Message }
    Expect ($m -match 'STATE_NOT_READY:NOT_INITIALIZED') "vm-action: $m"
    Expect ((Get-RealSnap) -ceq 'real root False; real anchor key False') "after: $(Get-RealSnap)"
    foreach ($l in $realLegacy) { Expect (-not (Test-Path -LiteralPath $l)) "created $l" }
}
}
}

# ================================================================ end
Case 'N98' 'the real state root and anchor key are as they were at the start' {
    Expect ((Get-RealSnap) -ceq $realAtStart) "start: $realAtStart; now: $(Get-RealSnap)"
}
Case 'N99' 'the test keys and synthetic folders are removed' {
    try { [Microsoft.Win32.Registry]::CurrentUser.DeleteSubKeyTree($keyRoot, $false) } catch { Expect $false "test keys not removed: $($_.Exception.Message)" }
    $parent = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Software\Monolith-P9S-Test', $true)
    if ($parent) { try { if (-not $parent.SubKeyCount -and -not $parent.ValueCount) { $parent.Close(); [Microsoft.Win32.Registry]::CurrentUser.DeleteSubKey('Software\Monolith-P9S-Test', $false) } } catch { } finally { $parent.Dispose() } }
    $k = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey($keyRoot); Expect (-not $k) 'test key still present'; if ($k) { $k.Dispose() }
    Remove-Tree $base
    Expect (-not (Test-Path -LiteralPath $base)) "synthetic folders left in $base"
}

$pass = @($results | Where-Object result -eq 'PASS').Count
$fail = @($results | Where-Object result -eq 'FAIL').Count
$notRun = @($results | Where-Object result -ne 'PASS' | Where-Object result -ne 'FAIL').Count
"== $pass passed, $fail failed, $notRun not run"
$results | ConvertTo-Json -Depth 3 | Set-Content -LiteralPath (Join-Path $OutDir 'results.json') -Encoding utf8
$results | ForEach-Object { '{0} {1} {2} ({3} ms){4}' -f $_.result, $_.id, $_.name, $_.ms, $(if ($_.detail) { ' :: ' + $_.detail } else { '' }) } | Set-Content -LiteralPath (Join-Path $OutDir 'results.txt') -Encoding utf8
if ($fail) { exit 1 } elseif ($notRun) { exit 3 } else { exit 0 }
