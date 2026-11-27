#Requires -Version 7.0
# Tests of state.migrate (p9s-state-migrate.ps1) in process: the migration
# of an earlier coordinator folder with the laboratory's shape and with every
# shape it must refuse, the toolkit's readers and the audit witness on the
# earlier folder, the locks, the VM check, free space, resumes and restarts,
# and writers acting at a given step (the step hook of the context runs them
# in order, never a sleep). The state is synthetic, as in
# tests/persist-tests.ps1, whose helpers this suite repeats; the VM states,
# the VM process count and the free space are mocked (no VirtualBox); the
# anchor is written with the real registry API into test keys under
# HKCU\Software\Monolith-P9S-Test\<run>, removed at the end; the real state
# root and anchor key must be as they were at the start. Crash points and
# races between processes are in tests/migproc-tests.ps1.
#   pwsh -NoProfile -File tests\migrate-tests.ps1 -OutDir <new folder>
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
$nativeOk = Initialize-P9SStateNative
if (-not $nativeOk) { throw 'the native checks of p9s-state.ps1 could not be compiled' }
$names = Get-P9SStateNames; $classes = Get-P9SStateFileClasses

# Deletions made by the test itself retry briefly: another process (an
# antivirus scanner, for example) can hold or add an entry for a moment
# after a write. If a deletion still fails, the entries left are named.
function Remove-Tree([string[]]$Path) {
    foreach ($p in $Path) {
        $last = $null
        for ($i = 0; $i -lt 20 -and (Test-Path -LiteralPath $p); $i++) {
            try { Remove-Item -LiteralPath $p -Recurse -Force -ErrorAction Stop } catch { $last = $_; Start-Sleep -Milliseconds 100 }
        }
        if (Test-Path -LiteralPath $p) { throw "could not remove $p ($($last.Exception.Message)); left: $(@(Get-ChildItem -LiteralPath $p -Force -Recurse -ErrorAction SilentlyContinue | ForEach-Object Name) -join ', ')" }
    }
}
# ---- the synthetic base: a canonical path on a local NTFS volume ----------------------
function Test-UsableBase([string]$B) {
    New-Item -ItemType Directory -Force -Path $B | Out-Null
    $fin = (Get-P9SStateEntry $B).final
    $probe = Join-Path $fin 'probe'
    New-Item -ItemType Directory -Force -Path (Join-Path $probe 'LocalAppData'), (Join-Path $probe 'Temp') | Out-Null
    $c = New-P9SStateContext -LocalAppData (Join-Path $probe 'LocalAppData') -TempPath (Join-Path $probe 'Temp') -AnchorKey "$keyRoot\probe"
    $r = @(Get-P9SStateLocationReasons $c)
    Remove-Tree $probe
    [pscustomobject]@{ base = $fin; reasons = $r; temp = $false }
}
$pick = Test-UsableBase (Join-Path $OutDir 'synthetic')
if ($pick.reasons.Count) {
    $alt = Test-UsableBase (Join-Path ([IO.Path]::GetTempPath()) "p9s-persist-tests-$run")
    if (-not $alt.reasons.Count) { $alt.temp = $true; $pick = $alt } else { Remove-Tree $alt.base }
}
$base = $pick.base
"== synthetic base $base$(if ($pick.reasons.Count) { ' (location reasons: ' + ($pick.reasons -join ',') + ')' })"

# ---- snapshots --------------------------------------------------------------------------------
# Times, size, attributes and links come from a handle on each entry (the
# copy NTFS keeps in the parent folder's index is updated later, so a folder
# listing can show an older time than the entry has).
function Get-TreeSnap([string]$Root) {
    $out = [System.Collections.Generic.List[string]]::new()
    $todo = [System.Collections.Generic.Stack[string]]::new(); $todo.Push($Root)
    while ($todo.Count) {
        $d = $todo.Pop(); $items = $null
        try { $items = @([IO.DirectoryInfo]::new($d).EnumerateFileSystemInfos() | Sort-Object FullName) } catch { $out.Add("LIST-DENIED $($d.Substring($Root.Length))"); continue }
        foreach ($i in $items) {
            $rel = $i.FullName.Substring($Root.Length)
            $sddl = try { (Get-Acl -LiteralPath $i.FullName -ErrorAction Stop).Sddl } catch { 'ACL-DENIED' }
            $h = [P9SStateNative]::Inspect($i.FullName)
            $t = if ($h.Error) { "INSPECT-ERROR $($h.Error)" } else { "attr=$($h.Attributes) size=$($h.Size) links=$($h.Links) c=$($h.Created) w=$($h.Written) id=$($h.Volume):$($h.Index)" }
            if ($i -is [IO.DirectoryInfo]) {
                $out.Add("D $rel $t $sddl")
                if ($i.Attributes -band [IO.FileAttributes]::ReparsePoint) { $out.Add("  -> $($i.LinkTarget)") } else { $todo.Push($i.FullName) }
            } else {
                $x = try { (Get-FileHash -LiteralPath $i.FullName -Algorithm SHA256 -ErrorAction Stop).Hash } catch { 'READ-DENIED' }
                $out.Add("F $rel sha=$x $t $sddl")
            }
        }
    }
    $out -join "`n"
}
function Get-KeySnap([string]$Path) {
    $k = $null
    try { $k = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey($Path, $false) } catch { return "KEY-DENIED $Path" }
    if (-not $k) { return "KEY-ABSENT $Path" }
    $out = [System.Collections.Generic.List[string]]::new()
    try {
        $sd = try { $k.GetAccessControl([Security.AccessControl.AccessControlSections]'Access, Owner').GetSecurityDescriptorSddlForm([Security.AccessControl.AccessControlSections]'Access, Owner') } catch { 'ACL?' }
        $out.Add("K $Path $sd")
        foreach ($v in @($k.GetValueNames() | Sort-Object)) { $out.Add("V $v $($k.GetValueKind($v)) $($k.GetValue($v, $null, [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames))") }
        foreach ($s in @($k.GetSubKeyNames() | Sort-Object)) { $out.Add((Get-KeySnap "$Path\$s")) }
    } catch { $out.Add("KEY-READ-ERROR $Path") } finally { $k.Dispose() }
    $out -join "`n"
}
$realLad = [P9SStateNative]::LocalAppData()
function Get-RealSnap {
    $k = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey($names.anchor_key, $false)
    $kp = [bool]$k; if ($k) { $k.Dispose() }
    "real root $(Test-Path -LiteralPath (Join-Path $realLad $names.root)); real anchor key $kp"
}
$realAtStart = Get-RealSnap
function Get-EnvSnap($E) { (Get-TreeSnap $E.base) + "`n" + (Get-KeySnap $E.key) + "`n" + (Get-RealSnap) }
function Get-SnapDiff([string]$A, [string]$B) {
    $x = $A -split "`n"; $y = $B -split "`n"
    $only1 = @($x | Where-Object { $y -cnotcontains $_ }) | Select-Object -First 3; $only2 = @($y | Where-Object { $x -cnotcontains $_ }) | Select-Object -First 3
    "before only: $($only1 -join ' / '); after only: $($only2 -join ' / ')"
}

# ---- synthetic environments ---------------------------------------------------------------------
function New-Env([string]$Name) {
    $b = Join-Path $base $Name
    $lad = Join-Path $b 'LocalAppData'
    New-Item -ItemType Directory -Force -Path $lad, (Join-Path $b 'Temp') | Out-Null
    [pscustomobject]@{ base = $b; lad = $lad; tmp = Join-Path $b 'Temp'; key = "$keyRoot\$Name"; root = Join-Path $lad $names.root
        state = Join-Path (Join-Path $lad $names.root) $names.state; legacy = Join-Path (Join-Path $b 'Temp') $names.legacy
        legacy2 = Join-Path (Join-Path $lad 'Temp') $names.legacy; id = $null; mid = $null; manifest = $null }
}
function New-EnvCtx($E, [hashtable]$More = @{}) { $a = @{ LocalAppData = $E.lad; TempPath = $E.tmp; AnchorKey = $E.key }; foreach ($k in $More.Keys) { $a[$k] = $More[$k] }; New-P9SStateContext @a }
# Classifies once with snapshots around it; checks the status, the reasons
# (wildcards) and the notes.
function Test-Status($E, [string]$Want, [string[]]$Reasons = @(), [hashtable]$More = @{}, [string[]]$Notes = @(), [string]$Label = '') {
    $before = Get-EnvSnap $E
    $st = Get-P9SStateStatus (New-EnvCtx $E $More)
    $after = Get-EnvSnap $E
    $p = if ($Label) { "$($Label): " } else { '' }
    Expect ($st.status -ceq $Want) "${p}status $($st.status) (reasons $(@($st.reasons) -join ','); notes $(@($st.notes) -join ',')), expected $Want"
    foreach ($r in $Reasons) { Expect (@($st.reasons | Where-Object { $_ -clike $r }).Count -gt 0) "${p}reason $r missing (have $(@($st.reasons) -join ','))" }
    foreach ($n in $Notes) { Expect (@($st.notes | Where-Object { $_ -clike $n }).Count -gt 0) "${p}note $n missing (have $(@($st.notes) -join ','))" }
    Expect ($before -ceq $after) "${p}the classifier changed the synthetic state: $(Get-SnapDiff $before $after)"
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
function Get-Sec([string]$Path) { $i = Get-Item -LiteralPath $Path -Force; if ($i -is [IO.DirectoryInfo]) { [IO.FileSystemAclExtensions]::GetAccessControl($i, [Security.AccessControl.AccessControlSections]::Access) } else { [IO.FileSystemAclExtensions]::GetAccessControl([IO.FileInfo]$i, [Security.AccessControl.AccessControlSections]::Access) } }
function Set-Sec([string]$Path, $Sec) { $i = Get-Item -LiteralPath $Path -Force; if ($i -is [IO.DirectoryInfo]) { [IO.FileSystemAclExtensions]::SetAccessControl($i, $Sec) } else { [IO.FileSystemAclExtensions]::SetAccessControl([IO.FileInfo]$i, $Sec) } }
# an explicit rule for one object only; returns it so that the case can remove it
function Add-Rule([string]$Path, [string]$Sid, [string]$Rights, [string]$Type) {
    $s = Get-Sec $Path; $r = [Security.AccessControl.FileSystemAccessRule]::new([Security.Principal.SecurityIdentifier]::new($Sid), [Security.AccessControl.FileSystemRights]$Rights, 'None', 'None', $Type)
    $s.AddAccessRule($r); Set-Sec $Path $s; $r
}
function Remove-Rule([string]$Path, $Rule) { $s = Get-Sec $Path; $null = $s.RemoveAccessRule($Rule); Set-Sec $Path $s }
function Get-Utc { [DateTime]::UtcNow.ToString('yyyy-MM-ddTHH:mm:ss.fffZ', [Globalization.CultureInfo]::InvariantCulture) }
function New-Id([string]$P = 'inst') { "$P-" + [guid]::NewGuid().ToString('N').Substring(0, 16) }
function Get-Mark([string]$LedgerPath) {
    $d = [IO.File]::ReadAllText($LedgerPath) | ConvertFrom-Json -AsHashtable
    [ordered]@{ revision = [long]$(if ($d.Contains('revision')) { $d.revision } else { 0 }); sha256 = (Get-FileHash -LiteralPath $LedgerPath -Algorithm SHA256).Hash.ToLowerInvariant() }
}
function Write-Anchor($E, $Doc, [string]$Raw, $Kind = [Microsoft.Win32.RegistryValueKind]::String) {
    $k = [Microsoft.Win32.Registry]::CurrentUser.CreateSubKey($E.key)
    try { $v = if ($PSBoundParameters.ContainsKey('Raw')) { $Raw } else { $Doc | ConvertTo-Json -Depth 6 -Compress }; if ($Kind -eq [Microsoft.Win32.RegistryValueKind]::DWord) { $k.SetValue($names.anchor_value, 1, $Kind) } else { $k.SetValue($names.anchor_value, $v, $Kind) } } finally { $k.Dispose() }
}
# read back with the strict reader, which keeps times as strings
function Get-Anchor($E) { $k = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey($E.key, $false); try { ConvertFrom-P9SStateJson ([Text.UTF8Encoding]::new($false).GetBytes([string]$k.GetValue($names.anchor_value))) } finally { $k.Dispose() } }
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
function Save-Ledger([string]$Path) {
    $c = [pscustomobject]@{ LedgerPath = $Path; SessionPath = (Join-Path (Split-Path -Parent $Path) 'coordinator-session.json') }; $lg = Read-P9SAcpiLedger $c
    if (-not (Save-P9SAcpiLedger $c $lg.doc)) { throw 'ledger save failed' }
}
# A new installation in READY: protected root, install record, empty ledger
# at revision 1 with its revision record, anchor with the ledger's mark.
function Install-New($E) {
    New-Root $E; $E.id = New-Id
    Write-Doc (Join-Path $E.state $names.install) (New-InstallDoc $E $E.id)
    Save-Ledger (Get-LedgerPath $E)
    Write-Anchor $E (New-AnchorDoc $E $E.id -Mark (Get-Mark (Get-LedgerPath $E)))
}
# An earlier coordinator folder with the laboratory's shape (synthetic
# content): a version 1 ledger without revision, a closed validation session,
# histories, a finished VirtualBox operation, the lock files, the monitor.
function New-Legacy([string]$Dir) {
    New-Item -ItemType Directory -Force -Path $Dir | Out-Null
    Write-Doc (Join-Path $Dir 'acpi-validation-ledger.json') ([ordered]@{ schema = 'p9s-acpi-validation-ledger/1'; entries = @([ordered]@{ authorization_id = 'acpiauth-20260101-00000000'; target_vm = 'P9S-D12'; phase = 'OFF_CONFIRMED'; startups_used = 1; acpi_used = 1; events = @() }) })
    Write-Doc (Join-Path $Dir 'coordinator-session.json') ([ordered]@{ schema = 'p9s-coordinator-session/1'; session_id = 'ses-synthetic'; kind = 'AcpiValidation'; lifecycle = 'CLOSED'; expected_vm = @{} })
    Add-Line (Join-Path $Dir 'coordinator-audit.jsonl') ([ordered]@{ action = 'session.close'; outcome = 'DONE' })
    Add-Line (Join-Path $Dir 'vm-observations.jsonl') ([ordered]@{ vm = 'P9S-D12'; state = 'poweroff' })
    Add-Line (Join-Path $Dir 'vbox-audit.jsonl') ([ordered]@{ operation = 'vm.state'; outcome = 'DONE' })
    Write-Doc (Join-Path $Dir 'vbox-operation.json') ([ordered]@{ operation_id = 'vbox-synthetic'; finished = $true })
    Write-Text (Join-Path $Dir 'coordinator.lock') ''; Write-Text (Join-Path $Dir 'coordinator-emergency.hold') ''; Write-Text (Join-Path $Dir 'state.json') '{}'
}
# A migrated installation in READY: the earlier folder kept, its records and
# histories copied, the manifest and its digest, the marker, the anchor
# marking the copied version 1 ledger as revision 0. -Source records another
# source folder than the earlier folder of the environment.
function Install-Migrated($E, [string]$Source) {
    New-Legacy $E.legacy
    if (-not $Source) { $Source = $E.legacy }
    New-Root $E; $E.id = New-Id; $E.mid = New-Id 'op'
    $files = @(); $absent = @()
    foreach ($cl in 'record', 'history') {
        foreach ($n in $classes[$cl]) {
            $p = Join-Path $E.legacy $n
            if (Test-Path -LiteralPath $p) { Copy-Item -LiteralPath $p -Destination $E.state; $files += [ordered]@{ name = $n; class = $cl; length = [long](Get-Item -LiteralPath $p).Length; sha256 = (Get-FileHash -LiteralPath $p -Algorithm SHA256).Hash.ToLowerInvariant() } }
            elseif ($cl -eq 'record') { $absent += $n }
        }
    }
    $E.manifest = Get-P9SStateManifestDigest $files $absent $Source
    Write-Doc (Join-Path $E.state $names.migration) ([ordered]@{ schema = 'p9s-state-migration/1'; migration_id = $E.mid; installation_id = $E.id; source_dir = $Source; manifest_sha256 = $E.manifest
        files = $files; absent = $absent; ignored = @('state.json'); ledger_created = $false; vm_observations = @(); utc = (Get-Utc) })
    Write-Doc (Join-Path $E.state $names.install) (New-InstallDoc $E $E.id 'migrated' $E.mid)
    Write-Doc (Join-Path $E.legacy $names.marker) ([ordered]@{ schema = 'p9s-state-moved/1'; installation_id = $E.id; state_dir = $E.state; manifest_sha256 = $E.manifest; note = 'synthetic' })
    Write-Anchor $E (New-AnchorDoc $E $E.id 'READY' 'migrated' (Get-Mark (Get-LedgerPath $E)) $Source $E.manifest)
}
function ConvertTo-Encoded([string]$Code) { [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($Code)) }
function Invoke-Pwsh([string]$Code, [hashtable]$Env = @{}) {
    $saved = @{}
    foreach ($k in $Env.Keys) { $saved[$k] = [Environment]::GetEnvironmentVariable($k); [Environment]::SetEnvironmentVariable($k, [string]$Env[$k]) }
    try { $o = & $pw -NoProfile -NonInteractive -EncodedCommand (ConvertTo-Encoded $Code) 2>&1; [pscustomobject]@{ code = $LASTEXITCODE; out = @($o | ForEach-Object { "$_" }) } }
    finally { foreach ($k in $Env.Keys) { [Environment]::SetEnvironmentVariable($k, $saved[$k]) } }
}
function Get-LastJson($R) { $l = @($R.out | Where-Object { $_ -like '{*' }) | Select-Object -Last 1; if ($l) { $l | ConvertFrom-Json -AsHashtable } else { $null } }
$libs = ". '$((Join-Path $kit 'hostmon-lib.ps1').Replace("'", "''"))'; . '$((Join-Path $kit 'coordinator-lib.ps1').Replace("'", "''"))'; . '$((Join-Path $kit 'p9s-state.ps1').Replace("'", "''"))'"
function Q([string]$S) { "'" + $S.Replace("'", "''") + "'" }

# ================================================================ state.init
# state.init (p9s-state-init.ps1) on the same synthetic environments: the
# known folder, the temporary folder and the anchor key injected, the VM
# states and the count of VirtualBox VM processes mocked (no VirtualBox),
# the anchor written with the real registry API into the test keys, the
# folders and ACLs real. Child processes run state.init where a step must
# end the process (TerminateProcess) or wait for another process; named
# events order them, never a sleep.
if (-not (Initialize-P9SSetupNative)) { throw 'the native calls of p9s-state-init.ps1 could not be compiled' }
$vmOff = @{ 'P9S-D12' = 'off'; 'P9S-U24' = 'off' }
function New-InitCtx($E, [hashtable]$O = @{}) {
    $vms = if ($O.Vms) { $O.Vms } else { $vmOff }
    $reads = [System.Collections.Generic.List[string]]::new()
    $E | Add-Member -NotePropertyName reads -NotePropertyValue $reads -Force
    $a = @{ State = (New-EnvCtx $E $(if ($O.More) { $O.More } else { @{} })); VmNames = @('P9S-D12', 'P9S-U24')
        ReadVmStateIn = { param($vm, $dir) $reads.Add("$vm|$dir"); $vms[$vm] }.GetNewClosure()
        CountVmProcesses = $(if ($O.Procs) { $O.Procs } else { { 0 } }) }
    if ($O.OnStep) { $a.OnStep = $O.OnStep }
    if ($O.UtcNow) { $a.UtcNow = $O.UtcNow }
    New-P9SStateInitContext @a
}
function Invoke-Init($E, [string]$Supersede, [switch]$DryRun, [hashtable]$O = @{}) {
    $i = New-InitCtx $E $O
    $a = @{ I = $i; DryRun = [bool]$DryRun }
    if ($PSBoundParameters.ContainsKey('Supersede')) { $a.Supersede = $Supersede }
    $r = Invoke-P9SStateInit @a
    $r | Add-Member -NotePropertyName trace -NotePropertyValue @($i.Trace) -Force
    $r
}
function Why($R) { "$($R.outcome) [$(@($R.reasons) -join ',')]" }
function Test-Refused($R, [string]$Reason, [string]$Label) { Expect ($R.outcome -ceq 'REFUSED' -and @($R.reasons | Where-Object { $_ -clike $Reason }).Count -gt 0) "${Label}: $(Why $R), expected $Reason" }
# every file under a folder with its bytes; every value of an anchor key
function Get-Files([string]$Root) {
    $h = @{}
    if (Test-Path -LiteralPath $Root) {
        foreach ($f in @(Get-ChildItem -LiteralPath $Root -Recurse -Force -File)) {
            $v = $null
            try { $v = [IO.File]::ReadAllBytes($f.FullName) } catch { $v = [Text.Encoding]::ASCII.GetBytes("<not readable: $($_.Exception.GetType().Name)>") }
            $h[$f.FullName.Substring($Root.Length)] = $v
        }
    }
    $h
}
function Get-KeyValues($E) {
    $h = @{}; $k = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey($E.key, $false)
    if ($k) { try { foreach ($v in $k.GetValueNames()) { $h[$v] = [string]$k.GetValue($v) } } finally { $k.Dispose() } }
    $h
}
function Get-Hex($B) { if ($null -eq $B -or $B.Length -eq 0) { '' } else { [Convert]::ToHexString([byte[]]$B) } }
# nothing that existed is gone or changed; a history (.jsonl) may only grow
function Test-Kept($Before, $After, [string]$Label, [string]$Except) {
    foreach ($p in $Before.Keys) {
        if ($Except -and $p.StartsWith($Except, [StringComparison]::OrdinalIgnoreCase)) { continue }
        if (-not $After.ContainsKey($p)) { Expect $false "${Label}: $p is gone"; continue }
        $a = Get-Hex $Before[$p]; $b = Get-Hex $After[$p]
        Expect ($a -ceq $b -or ($p -like '*.jsonl' -and $b.StartsWith($a, [StringComparison]::Ordinal))) "${Label}: $p changed"
    }
}
function Test-SameFiles($A, $B) {
    if (@($A.Keys).Count -ne @($B.Keys).Count) { return $false }
    foreach ($p in $A.Keys) { if (-not $B.ContainsKey($p) -or (Get-Hex $A[$p]) -cne (Get-Hex $B[$p])) { return $false } }
    $true
}
function Get-Stagings($E) { @(if (Test-Path -LiteralPath $E.root) { Get-ChildItem -LiteralPath $E.root -Directory -Force | Where-Object Name -like 'state.staging-*' }) }

# ================================================================ state.migrate
$cli = Join-Path $kit 'p9s-action.ps1'
# The earlier coordinator folder as the laboratory left it (synthetic
# content): a version 1 ledger without revision holding a finished Debian
# validation, the closed validation session bound to it, the audit of that
# validation, the VM observations and VirtualBox audit, the lock files and
# the monitor's files; no VirtualBox operation record (the layer removes it
# when an operation ends). Every environment of this suite has a protected
# ACL (the account, SYSTEM, Administrators), as a profile's Temp folder has.
function New-Env([string]$Name) {
    $b = Join-Path $base $Name
    $lad = Join-Path $b 'LocalAppData'
    New-Item -ItemType Directory -Force -Path $lad, (Join-Path $b 'Temp') | Out-Null
    Set-ProtectedAcl $b
    [pscustomobject]@{ base = $b; lad = $lad; tmp = Join-Path $b 'Temp'; key = "$keyRoot\$Name"; root = Join-Path $lad $names.root
        state = Join-Path (Join-Path $lad $names.root) $names.state; legacy = Join-Path (Join-Path $b 'Temp') $names.legacy
        legacy2 = Join-Path (Join-Path $lad 'Temp') $names.legacy; id = $null; mid = $null; manifest = $null; reads = $null }
}
$labSession = 'ses-20261121T132730301Z-29300'; $labAuth = 'acpiauth-20261121-caa2f910'; $labAuthSha = '5' * 64
function New-AuditLine([string]$Action, [string]$Outcome, [string]$Vm, [int]$Dispatched = 0, [switch]$Dry) {
    [ordered]@{ utc = '2026-11-21T13:30:00.000Z'; pid = 4242; session = $labSession; action = $Action; vm = $(if ($Vm) { $Vm } else { $null }); outcome = $Outcome; reasons = @()
        monitor_status = 'UNCALIBRATED'; monitor_run = $null; dry_run = [bool]$Dry; dispatched = $Dispatched; params = [ordered]@{}; note = $null }
}
function New-LabSource([string]$Dir, [switch]$NoLocks) {
    New-Item -ItemType Directory -Force -Path $Dir | Out-Null
    Write-Doc (Join-Path $Dir 'acpi-validation-ledger.json') ([ordered]@{ schema = 'p9s-acpi-validation-ledger/1'; entries = @([ordered]@{ authorization_id = $labAuth; authorization_sha256 = $labAuthSha; target_vm = 'P9S-D12'
        session_id = $labSession; phase = 'OFF_CONFIRMED'; startups_used = 1; acpi_used = 1; acpi_source = 'validation'; events = @([ordered]@{ utc = '2026-11-21T13:31:55.000Z'; what = 'off'; detail = 'observed off' }) }) })
    Write-Doc (Join-Path $Dir 'coordinator-session.json') ([ordered]@{ schema = 'p9s-coordinator-session/1'; session_id = $labSession; kind = 'AcpiValidation'; opened_utc = '2026-11-21T13:27:30.301Z'; monitor_run_id = 'run-1'
        lifecycle = 'CLOSED'; halt = $null; previous_halt = $null; expected_vm = [ordered]@{}; inflight = $null; last_marker = $null
        validation = [ordered]@{ authorization_id = $labAuth; target_vm = 'P9S-D12'; authorization_sha256 = $labAuthSha } })
    $a = Join-Path $Dir 'coordinator-audit.jsonl'
    foreach ($x in @(@('session.open', 'DONE', '', 0), @('acpi.open', 'DONE', 'P9S-D12', 0), @('acpi.start', 'DISPATCHED', 'P9S-D12', 1), @('acpi.observe', 'DONE', 'P9S-D12', 0),
            @('acpi.shutdown', 'DISPATCHED', 'P9S-D12', 1), @('acpi.confirm-off', 'DONE', 'P9S-D12', 0), @('acpi.finalize', 'DONE', 'P9S-D12', 0), @('session.close', 'DONE', '', 0), @('emergency.status', 'DONE', '', 0))) {
        Add-Line $a (New-AuditLine $x[0] $x[1] $x[2] $x[3])
    }
    Add-Line (Join-Path $Dir 'vm-observations.jsonl') ([ordered]@{ schema = 'p9s-vm-observation/1'; utc = '2026-11-21T13:31:55.000Z'; pid = 4242; vm = 'P9S-D12'; state = 'poweroff'; observed = $true; purpose = 'acpi.confirm-off' })
    Add-Line (Join-Path $Dir 'vbox-audit.jsonl') ([ordered]@{ utc = '2026-11-21T13:31:55.000Z'; pid = 4242; operation = 'vm.state'; vm = 'P9S-D12'; outcome = 'DONE' })
    if (-not $NoLocks) { Write-Text (Join-Path $Dir 'coordinator.lock') ''; Write-Text (Join-Path $Dir 'coordinator-emergency.hold') '' }
    Write-Text (Join-Path $Dir 'state.json') '{}'; Write-Text (Join-Path $Dir 'state.json.lock') ''
}
$labFiles = 'acpi-validation-ledger.json', 'coordinator-audit.jsonl', 'coordinator-session.json', 'vbox-audit.jsonl', 'vm-observations.jsonl'
function Get-MigSteps([string[]]$Files, [switch]$LedgerCreated) {
    @('P0', 'M1', 'M2', 'M3', 'M4') + @($Files | ForEach-Object { "M5:$_-tmp"; "M5:$_" }) + @('V:P9S-D12', 'V:P9S-U24', 'V') +
        @(if ($LedgerCreated) { 'M6-ledger-tmp', 'M6-ledger', 'M6-revision-tmp', 'M6-revision', 'M6' }) + @('M7-migration', 'M7', 'M8', 'M9', 'M10', 'M11', 'M12')
}
function New-MigCtx($E, [hashtable]$O = @{}) {
    $vms = if ($O.Vms) { $O.Vms } else { $vmOff }
    $reads = [System.Collections.Generic.List[string]]::new()
    $E.reads = $reads
    $a = @{ State = (New-EnvCtx $E $(if ($O.More) { $O.More } else { @{} })); VmNames = @('P9S-D12', 'P9S-U24')
        ReadVmStateIn = { param($vm, $dir) $reads.Add("$vm|$dir"); $vms[$vm] }.GetNewClosure()
        CountVmProcesses = $(if ($O.Procs) { $O.Procs } else { { 0 } }); FreeBytes = $(if ($O.Free) { $O.Free } else { { param($d) 107374182400L } }) }
    if ($O.OnStep) { $a.OnStep = $O.OnStep }
    if ($O.UtcNow) { $a.UtcNow = $O.UtcNow }
    New-P9SStateMigrateContext @a
}
# one state.migrate; a step hook that throws ends it there (an interruption
# in process: the files stay as that step left them, the handles close)
function Invoke-Mig($E, [string]$Restart, [switch]$DryRun, [hashtable]$O = @{}) {
    $i = New-MigCtx $E $O
    $a = @{ I = $i; DryRun = [bool]$DryRun }
    if ($PSBoundParameters.ContainsKey('Restart')) { $a.Restart = $Restart }
    try { $r = Invoke-P9SStateMigrate @a }
    catch { $r = [pscustomobject]@{ outcome = 'THROWN'; reasons = @([string]$_.Exception.Message); lines = @(); notes = @(); installation_id = $null; op_id = $null; mode = $null } }
    $r | Add-Member -NotePropertyName trace -NotePropertyValue @($i.Trace) -Force
    $r
}
function Stop-At([string]$Step) { { param($s) if ($s -ceq $Step) { throw "interrupted at $s" } }.GetNewClosure() }
# the earlier folder after a run: every file it had is there with the same
# bytes (a history may only grow); only the names given may have been added
function Test-SourceKept($Before, [string]$Dir, [string]$Label, [string[]]$Added = @()) {
    $after = Get-Files $Dir
    Test-Kept $Before $after $Label
    foreach ($p in $after.Keys) { if (-not $Before.ContainsKey($p) -and [IO.Path]::GetFileName($p) -cnotin $Added) { Expect $false "${Label}: $p added to the earlier folder" } }
}
function Get-Marker($E) { $p = Join-Path $e.legacy $names.marker; if (Test-Path -LiteralPath $p) { ConvertFrom-P9SStateJson ([IO.File]::ReadAllBytes($p)) } else { $null } }
$script:sources = [System.Collections.Generic.List[object]]::new()
function Register-Source($E) { $script:sources.Add([pscustomobject]@{ dir = $E.legacy; files = (Get-Files $E.legacy) }) }

Case 'Q01' 'state.migrate on the laboratory shape: READY, origin migrated, every record and history copied byte for byte, the manifest, the marker, the version 1 ledger marked as revision 0, the earlier folder unchanged but for the marker' {
    $e = New-Env 'q01'; New-LabSource $e.legacy; Register-Source $e
    $null = Test-Status $e 'MIGRATION_REQUIRED'
    $src0 = Get-Files $e.legacy
    $r = Invoke-Mig $e
    Expect ($r.outcome -ceq 'DONE' -and $r.mode -ceq 'new' -and $r.status_before -ceq 'MIGRATION_REQUIRED' -and $r.status_after -ceq 'READY' -and $r.dispatched -eq 0 -and -not $r.ledger_created) (Why $r)
    Expect ((@($r.trace) -join ',') -ceq ((Get-MigSteps $labFiles) -join ',')) "steps $(@($r.trace) -join ',')"
    $st = Test-Status $e 'READY'
    Expect ($st.installation_id -ceq $r.installation_id -and $st.origin -ceq 'migrated') "status $($st.installation_id) $($st.origin)"
    $a = Get-Anchor $e
    $lsha = (Get-FileHash -LiteralPath (Join-Path $e.legacy 'acpi-validation-ledger.json') -Algorithm SHA256).Hash.ToLowerInvariant()
    Expect ($a.phase -ceq 'READY' -and $a.origin -ceq 'migrated' -and $a.op_id -ceq $r.op_id -and (Test-P9SStateSamePath $a.source_dir $e.legacy) -and $a.manifest_sha256 -ceq $r.manifest_sha256 -and $a.ledger.revision -eq 0 -and $a.ledger.sha256 -ceq $lsha -and $null -eq $a.supersedes) "anchor $($a | ConvertTo-Json -Compress)"
    $ct = ConvertFrom-P9SStateUtcText $a.created_utc; $it = ConvertFrom-P9SStateUtcText $a.initialized_utc
    Expect ($ct -and $it -and $it -ge $ct -and $a.initialized_utc -cmatch '\.\d{7}Z$') "times $($a.created_utc) $($a.initialized_utc)"
    $ins = ConvertFrom-P9SStateJson ([IO.File]::ReadAllBytes((Join-Path $e.state $names.install)))
    Expect ($ins.origin -ceq 'migrated' -and $ins.migration_id -ceq $r.op_id -and $ins.installation_id -ceq $r.installation_id -and $ins.created_utc -ceq $a.created_utc) "install $($ins | ConvertTo-Json -Compress)"
    $m = ConvertFrom-P9SStateJson ([IO.File]::ReadAllBytes((Join-Path $e.state $names.migration)))
    Expect ((@($m.files | ForEach-Object { $_.name }) -join ',') -ceq ($labFiles -join ',') -and (@($m.absent) -join ',') -ceq 'acpi-validation-ledger.revision.json,coordinator-stop.request,vbox-quarantine.json,vbox-operation.json' -and (@($m.ignored) -join ',') -ceq 'state.json,state.json.lock' -and $m.ledger_created -eq $false) "migration record $($m | ConvertTo-Json -Compress -Depth 4)"
    Expect (@($m.vm_observations).Count -eq 2 -and @($m.vm_observations | Where-Object { $_.state -ceq 'off' -and $_.observed }).Count -eq 2) "observations $($m.vm_observations | ConvertTo-Json -Compress)"
    Expect ((Get-P9SStateManifestDigest $m.files $m.absent $m.source_dir) -ceq $a.manifest_sha256) 'manifest digest'
    foreach ($n in $labFiles) {
        $sb = [IO.File]::ReadAllBytes((Join-Path $e.legacy $n)); $nb = [IO.File]::ReadAllBytes((Join-Path $e.state $n))
        if ($n -like '*.jsonl') { Expect ((Get-Hex $nb).StartsWith((Get-Hex $sb), [StringComparison]::Ordinal)) "$n does not begin with the copied bytes" } else { Expect ((Get-Hex $nb) -ceq (Get-Hex $sb)) "$n differs" }
    }
    $obs = @(Get-Content -LiteralPath (Join-Path $e.state 'vm-observations.jsonl') | ForEach-Object { $_ | ConvertFrom-Json } | Where-Object purpose -ceq 'state.migrate')
    Expect ($obs.Count -eq 2) "new observations $($obs.Count)"
    $aud = @(Get-Content -LiteralPath (Join-Path $e.state 'coordinator-audit.jsonl') | ForEach-Object { $_ | ConvertFrom-Json })
    Expect ($aud.Count -eq 10 -and $aud[-1].action -ceq 'state.migrate' -and $aud[-1].outcome -ceq 'DONE' -and $aud[-1].installation_id -ceq $r.installation_id -and $aud[-1].dispatched -eq 0) "audit $($aud[-1] | ConvertTo-Json -Compress)"
    Test-SourceKept $src0 $e.legacy 'the earlier folder' @($names.marker)
    $mk = Get-Marker $e
    Expect ($mk -and $mk.schema -ceq 'p9s-state-moved/1' -and $mk.installation_id -ceq $r.installation_id -and $mk.manifest_sha256 -ceq $a.manifest_sha256 -and (Test-P9SStateSamePath $mk.state_dir $e.state)) "marker $($mk | ConvertTo-Json -Compress)"
    Expect ((@(Get-ChildItem -LiteralPath $e.root -Force | ForEach-Object Name | Sort-Object) -join ',') -ceq 'setup.lock,state') "root holds $(@(Get-ChildItem -LiteralPath $e.root -Force | ForEach-Object Name) -join ',')"
    Expect ((@(Get-ChildItem -LiteralPath $e.state -Force | ForEach-Object Name | Sort-Object) -join ',') -ceq 'acpi-validation-ledger.json,coordinator-audit.jsonl,coordinator-session.json,install.json,migration.json,vbox-audit.jsonl,vm-observations.jsonl') "state folder holds $(@(Get-ChildItem -LiteralPath $e.state -Force | ForEach-Object Name) -join ',')"
    $dirs = @($e.reads | ForEach-Object { ($_ -split '\|', 2)[1] } | Select-Object -Unique)
    Expect (@($e.reads).Count -eq 2 -and $dirs.Count -eq 1 -and [IO.Path]::GetFileName($dirs[0]) -ceq "state.staging-$($r.op_id)-1") "VM reads $(@($e.reads) -join '; ')"
    Expect (@(Get-P9SStateAclReasons $e.root $me -RequireProtected).Count -eq 0 -and @(Get-P9SStateAclReasons $e.state $me -RequireProtected).Count -eq 0) 'ACL of the root or the state folder'
    $inst = Get-P9SStateInstallation (New-EnvCtx $e)
    Expect ($inst.known -and $inst.installed -and $inst.installation_id -ceq $r.installation_id -and $inst.initialized_utc -eq $it) "installation $($inst | ConvertTo-Json -Compress)"
    $hs = Get-P9SAcpiRequestHistory ([pscustomobject]@{ LedgerPath = (Join-Path $e.legacy 'acpi-validation-ledger.json'); SessionPath = (Join-Path $e.legacy 'coordinator-session.json'); StopRequestPath = (Join-Path $e.legacy 'coordinator-stop.request') }) 'P9S-D12'
    $hn = Get-P9SAcpiRequestHistory ([pscustomobject]@{ LedgerPath = (Get-LedgerPath $e); SessionPath = (Join-Path $e.state 'coordinator-session.json'); StopRequestPath = (Join-Path $e.state 'coordinator-stop.request') }) 'P9S-D12'
    Expect ($hs.ok -eq $hn.ok -and [string]$hs.spent -ceq [string]$hn.spent -and -not $hn.uncertain) "request history earlier $($hs.ok)/$($hs.spent), new $($hn.ok)/$($hn.spent)"
}
Case 'Q02' 'state.migrate -DryRun: the decision, the readers and the manifest only; nothing written anywhere, no lock file created, no VM state read' {
    foreach ($locks in $true, $false) {
        $e = New-Env "q02$(if ($locks) { 'a' } else { 'b' })"; New-LabSource $e.legacy -NoLocks:(-not $locks); Register-Source $e
        $before = Get-EnvSnap $e
        $r = Invoke-Mig $e -DryRun
        Expect ($r.outcome -ceq 'DRYRUN' -and $r.mode -ceq 'new' -and (@($r.trace) -join ',') -ceq 'P0') "$(Why $r) steps $(@($r.trace) -join ',')"
        foreach ($n in $labFiles) { Expect (@($r.lines | Where-Object { $_ -clike "*copy*$n *" }).Count -eq 1) "dry run does not list $n" }
        Expect ($before -ceq (Get-EnvSnap $e)) "changed: $(Get-SnapDiff $before (Get-EnvSnap $e))"
        Expect (@($e.reads).Count -eq 0 -and -not (Test-Path -LiteralPath $e.root)) 'a VM state read or the root created'
    }
}
Case 'Q03' 'nothing migrates implicitly: only the state.migrate branch of p9s-action.ps1 calls it, the coordinator does not know the action, -Restart is refused elsewhere, state.init still refuses an earlier folder and names state.migrate' {
    $calls = @(Get-ChildItem -LiteralPath $kit -Filter '*.ps1' | ForEach-Object { $fl = $_; Select-String -LiteralPath $fl.FullName -Pattern 'Invoke-P9SStateMigrate\b' -CaseSensitive | Where-Object { $_.Line -notmatch '^\s*#' -and $_.Line -notmatch '^\s*function ' } | ForEach-Object { "$($fl.Name):$($_.LineNumber)" } })
    Expect ($calls.Count -eq 1 -and $calls[0] -like 'p9s-action.ps1:*') "calls of Invoke-P9SStateMigrate: $($calls -join ', ')"
    $t = [IO.File]::ReadAllText((Join-Path $kit 'p9s-action.ps1')).Replace("`r`n", "`n")
    $b = $t.IndexOf("if (`$Action -ceq 'state.migrate') {"); $c = $t.IndexOf('Invoke-P9SStateMigrate @ma'); $end = if ($b -ge 0) { $t.IndexOf("`n}", $b) } else { -1 }
    Expect ($b -gt 0 -and $c -gt $b -and $end -gt $c) 'the call is not inside the state.migrate branch'
    $v = Test-P9SRequest -Request @{ Action = 'state.migrate' }
    Expect (-not $v.ok -and @($v.reasons) -ccontains 'UNKNOWN_ACTION') "the coordinator on state.migrate: $(@($v.reasons) -join ',')"
    $v = Test-P9SRequest -Request @{ Action = 'session.open'; Kind = 'Test'; Restart = 'migrate-0000' }
    Expect (-not $v.ok -and @($v.reasons) -ccontains 'PARAMETER_NOT_ALLOWED:Restart') "-Restart on session.open: $(@($v.reasons) -join ',')"
    $e = New-Env 'q03'; New-LabSource $e.legacy; Register-Source $e; $before = Get-EnvSnap $e
    $r = Invoke-Init $e; Test-Refused $r 'STATE_NOT_NEW:MIGRATION_REQUIRED' 'state.init'
    Expect (@($r.lines | Where-Object { $_ -clike 'way out: state.migrate copies*' }).Count -eq 1) "state.init way out: $(@($r.lines) -join ' / ')"
    Expect ($before -ceq (Get-EnvSnap $e)) 'state.init changed something'
}
Case 'Q04' 'state.migrate refuses every status but MIGRATION_REQUIRED and its own interruption, before anything is written or a VM is read; an empty or new-looking folder is never migrated' {
    $mk = [ordered]@{
        'NOT_INITIALIZED' = { param($e) }
        'NOT_INITIALIZED ' = { param($e) New-Item -ItemType Directory -Path $e.legacy | Out-Null; Write-Text (Join-Path $e.legacy 'state.json') '{}'; Write-Text (Join-Path $e.legacy 'state.json.lock') '' }
        'READY' = { param($e) Install-New $e }
        'READY  ' = { param($e) Install-Migrated $e }
        'MIGRATION_SOURCES_AMBIGUOUS' = { param($e) New-LabSource $e.legacy; New-LabSource $e.legacy2 }
        'INSTALLATION_LOST' = { param($e) Install-Migrated $e; Remove-Tree $e.root; [Microsoft.Win32.Registry]::CurrentUser.DeleteSubKeyTree($e.key) }
        'SETUP_LEFTOVER' = { param($e) New-LabSource $e.legacy; New-Root $e; [IO.Directory]::Delete($e.state); New-Item -ItemType Directory -Path (Join-Path $e.root 'state.staging-op-1-1') | Out-Null }
        'ANCHOR_MISSING' = { param($e) New-LabSource $e.legacy; Install-New $e; [Microsoft.Win32.Registry]::CurrentUser.DeleteSubKeyTree($e.key) }
        'ANCHOR_UNREADABLE' = { param($e) New-LabSource $e.legacy; Install-New $e; Write-Anchor $e $null -Raw '{"schema":' }
        'ACCOUNT_MISMATCH' = { param($e) Install-New $e; $a = Get-Anchor $e; $a.account = 'S-1-5-21-0-0-0-1001'; Write-Anchor $e $a }
        'STATE_MISSING' = { param($e) Install-New $e; Remove-Tree $e.state }
        'STATE_INCOMPLETE' = { param($e) Install-New $e; [IO.File]::Delete((Join-Path $e.state $names.install)) }
        'STATE_MISMATCH' = { param($e) Install-New $e; $a = Get-Anchor $e; $a.installation_id = New-Id; Write-Anchor $e $a }
        'STATE_UNCERTAIN' = { param($e) Install-New $e; [IO.File]::Delete((Get-LedgerPath $e)) }
        'STATE_DIVERGED' = { param($e) Install-Migrated $e; Write-Text (Join-Path $e.legacy 'coordinator-stop.request') '{}' }
        'INTERRUPTED' = { param($e) New-LabSource $e.legacy; Write-Anchor $e (New-AnchorDoc $e (New-Id) 'INITIALIZING' 'new') }
        'LOCATION_UNSAFE' = { param($e) New-LabSource $e.legacy; $real = Join-Path $e.base 'real-lad'; [IO.Directory]::Move($e.lad, $real); New-Item -ItemType Junction -Path $e.lad -Target $real | Out-Null }
    }
    $i = 0
    foreach ($k0 in $mk.Keys) {
        $s = $k0.Trim(); $i++; $e = New-Env "q04-$i"; & $mk[$k0] $e
        $null = Test-Status $e $s -Label "fixture $k0"
        $before = Get-EnvSnap $e
        $r = Invoke-Mig $e
        Test-Refused $r "NOT_MIGRATABLE:$s" $k0
        Expect ($r.status_before -ceq $s -and (@($r.trace) -join ',') -ceq '') "${k0}: decided on $($r.status_before), steps $(@($r.trace) -join ',')"
        Expect ($before -ceq (Get-EnvSnap $e)) "${k0}: changed $(Get-SnapDiff $before (Get-EnvSnap $e))"
        Expect (@($e.reads).Count -eq 0) "${k0}: a VM state was read"
    }
}
Case 'Q05' 'the earlier folder as it is: an unknown file, a folder, a junction, the folder a junction, a hard-linked record, a record that cannot be read, a folder that cannot be listed: refused, nothing written; write entries of other accounts: reported, the copies protected; a temporary file of an atomic write: ignored' {
    $vars = [ordered]@{
        'unknown' = @({ param($e) Write-Text (Join-Path $e.legacy 'ACPI-VALIDATION-LEDGER.JSON.tmp') 'x' }, 'MIGRATION_SOURCE_UNKNOWN_ENTRY:ACPI-VALIDATION-LEDGER.JSON.tmp')
        'folder' = @({ param($e) New-Item -ItemType Directory -Path (Join-Path $e.legacy 'old') | Out-Null }, 'MIGRATION_SOURCE_ENTRY_NOT_A_FILE:old')
        'junction' = @({ param($e) $t = Join-Path $e.base 'elsewhere'; New-Item -ItemType Directory -Path $t | Out-Null; New-Item -ItemType Junction -Path (Join-Path $e.legacy 'j') -Target $t | Out-Null }, 'MIGRATION_SOURCE_ENTRY_REPARSE_POINT:j')
        'source-junction' = @({ param($e) $t = Join-Path $e.base 'real-legacy'; [IO.Directory]::Move($e.legacy, $t); New-Item -ItemType Junction -Path $e.legacy -Target $t | Out-Null }, 'MIGRATION_SOURCE_REPARSE_POINT')
        'hard-link' = @({ param($e) New-Item -ItemType HardLink -Path (Join-Path $e.base 'session-link.json') -Target (Join-Path $e.legacy 'coordinator-session.json') | Out-Null }, 'MIGRATION_SOURCE_ENTRY_HARD_LINKED:coordinator-session.json')
        'data-denied' = @({ param($e) $p = Join-Path $e.legacy 'coordinator-stop.request'; Write-Text $p '{"acpi":{},"unresolved":{}}'; $script:rule = @($p, (Add-Rule $p $me 'ReadData' 'Deny')) }, 'MIGRATION_SOURCE_NOT_READABLE:coordinator-stop.request:denied')
        'entry-denied' = @({ param($e) $p = Join-Path $e.legacy 'coordinator-session.json'; $script:rule = @($p, (Add-Rule $p $me 'ReadAttributes, ReadData' 'Deny')) }, 'MIGRATION_SOURCE_NOT_READABLE:coordinator-session.json:denied')
        'not-listable' = @({ param($e) $script:rule = @($e.legacy, (Add-Rule $e.legacy $me 'ReadData' 'Deny')) }, 'MIGRATION_SOURCE_NOT_LISTABLE')
    }
    $i = 0
    foreach ($k0 in $vars.Keys) {
        $i++; $e = New-Env "q05-$i"; New-LabSource $e.legacy; $script:rule = $null
        & $vars[$k0][0] $e
        try {
            $before = Get-EnvSnap $e
            $r = Invoke-Mig $e
            Test-Refused $r $vars[$k0][1] $k0
            Expect ($before -ceq (Get-EnvSnap $e)) "${k0}: changed $(Get-SnapDiff $before (Get-EnvSnap $e))"
            Expect (@($e.reads).Count -eq 0 -and -not (Test-Path -LiteralPath $e.root)) "${k0}: a VM state read or the root created"
        } finally { if ($script:rule) { Remove-Rule $script:rule[0] $script:rule[1] } }
    }
    # write entries of other accounts on the earlier folder or a record: reported, not a refusal; the copies are protected
    $e = New-Env 'q05-acl'; New-LabSource $e.legacy; Register-Source $e
    $null = Add-Rule $e.legacy 'S-1-5-32-545' 'Write' 'Allow'; $null = Add-Rule (Join-Path $e.legacy 'acpi-validation-ledger.json') 'S-1-5-32-545' 'Write' 'Allow'
    $r = Invoke-Mig $e -DryRun
    Expect ($r.outcome -ceq 'DRYRUN' -and @($r.lines | Where-Object { $_ -clike '*notes:*SOURCE_ACL:ACL_FOREIGN_WRITE*SOURCE_ACL:acpi-validation-ledger.json:ACL_FOREIGN_WRITE*' }).Count -eq 1) "dry run with foreign write entries: $(Why $r) $(@($r.lines) -join ' / ')"
    $r = Invoke-Mig $e
    Expect ($r.outcome -ceq 'DONE' -and @($r.notes) -ccontains 'SOURCE_ACL:ACL_FOREIGN_WRITE' -and @($r.notes) -ccontains 'SOURCE_ACL:acpi-validation-ledger.json:ACL_FOREIGN_WRITE') "foreign write entries: $(Why $r) notes $(@($r.notes) -join ',')"
    Expect (@(Get-P9SStateAclReasons (Get-LedgerPath $e) $me).Count -eq 0 -and @(Get-P9SStateAclReasons $e.state $me -RequireProtected).Count -eq 0) 'the copy kept a foreign write entry'
    $aud = @(Get-Content -LiteralPath (Join-Path $e.state 'coordinator-audit.jsonl') | ForEach-Object { $_ | ConvertFrom-Json })[-1]
    Expect (@($aud.notes) -ccontains 'SOURCE_ACL:ACL_FOREIGN_WRITE') "audit notes $(@($aud.notes) -join ',')"
    $e = New-Env 'q05-tmp'; New-LabSource $e.legacy; Register-Source $e
    $tn = '.acpi-validation-ledger.json.' + ('0123456789abcdef' * 2) + '.tmp'; Write-Text (Join-Path $e.legacy $tn) '{"partial'
    $r = Invoke-Mig $e
    Expect ($r.outcome -ceq 'DONE' -and @($r.ignored) -ccontains $tn -and -not (Test-Path -LiteralPath (Join-Path $e.state $tn))) "temporary file: $(Why $r), ignored $(@($r.ignored) -join ',')"
}
Case 'Q06' 'the toolkit''s readers decide on the earlier folder: a ledger damaged, stale, inconsistent or missing, a stop request or session record that cannot be read, a validation session that no longer matches its ledger entry, a VirtualBox operation or quarantine record: refused, nothing written' {
    $lp = 'acpi-validation-ledger.json'; $rp = 'acpi-validation-ledger.revision.json'
    $rev = { param($e, [long]$n, [string]$sha) Write-Doc (Join-Path $e.legacy $rp) ([ordered]@{ schema = 'p9s-acpi-validation-ledger-revision/1'; revision = $n; sha256 = $sha; utc = '2026-11-21T13:40:00.0000000Z' }) }
    $vars = [ordered]@{
        'truncated' = @({ param($e) Write-Text (Join-Path $e.legacy $lp) '{"schema":"p9s-acpi-validation-ledger/1","entr' }, 'MIGRATION_SOURCE_UNCERTAIN:ACPI_LEDGER_UNREADABLE')
        'not-json' = @({ param($e) Write-Text (Join-Path $e.legacy $lp) 'not json' }, 'MIGRATION_SOURCE_UNCERTAIN:ACPI_LEDGER_UNREADABLE')
        'duplicated-name' = @({ param($e) Write-Text (Join-Path $e.legacy $lp) '{"schema":"p9s-acpi-validation-ledger/1","entries":[],"entries":[]}' }, 'MIGRATION_SOURCE_UNCERTAIN:ACPI_LEDGER_UNREADABLE')
        'stale' = @({ param($e) Write-Text (Join-Path $e.legacy $lp) '{"schema":"p9s-acpi-validation-ledger/1","revision":2,"entries":[]}'; & $rev $e 3 ('0' * 64) }, 'MIGRATION_SOURCE_UNCERTAIN:ACPI_LEDGER_STALE')
        'inconsistent' = @({ param($e) Write-Text (Join-Path $e.legacy $lp) '{"schema":"p9s-acpi-validation-ledger/1","revision":2,"entries":[]}'; & $rev $e 2 ('0' * 64) }, 'MIGRATION_SOURCE_UNCERTAIN:ACPI_LEDGER_INCONSISTENT')
        'record-without-ledger' = @({ param($e) & $rev $e 1 ('0' * 64); [IO.File]::Delete((Join-Path $e.legacy $lp)) }, 'MIGRATION_SOURCE_UNCERTAIN:ACPI_LEDGER_MISSING')
        'session-without-ledger' = @({ param($e) [IO.File]::Delete((Join-Path $e.legacy $lp)) }, 'MIGRATION_SOURCE_UNCERTAIN:ACPI_LEDGER_MISSING')
        'stop-request' = @({ param($e) Write-Text (Join-Path $e.legacy 'coordinator-stop.request') '{"acpi":' }, 'MIGRATION_SOURCE_UNCERTAIN:STOP_REQUEST_UNREADABLE')
        'session' = @({ param($e) Write-Text (Join-Path $e.legacy 'coordinator-session.json') '[1]' }, 'MIGRATION_SOURCE_UNCERTAIN:SESSION_UNREADABLE')
        'binding' = @({ param($e) $p = Join-Path $e.legacy 'coordinator-session.json'; $d = ConvertFrom-P9SStateJson ([IO.File]::ReadAllBytes($p)); $d.session_id = 'ses-other'; Write-Doc $p $d }, 'MIGRATION_SOURCE_UNCERTAIN:REQUEST_HISTORY:P9S-D12')
        'vbox-operation' = @({ param($e) Write-Doc (Join-Path $e.legacy 'vbox-operation.json') ([ordered]@{ schema = 'p9s-vbox-operation/1'; operation_id = 'vbox-x'; phase = 'RUNNING' }) }, 'SETUP_VBOX_RECORD:*vbox-operation.json')
        'vbox-quarantine' = @({ param($e) Write-Doc (Join-Path $e.legacy 'vbox-quarantine.json') ([ordered]@{ schema = 'p9s-vbox-quarantine/1'; reason = 'X' }) }, 'SETUP_VBOX_RECORD:*vbox-quarantine.json')
    }
    $i = 0
    foreach ($k0 in $vars.Keys) {
        $i++; $e = New-Env "q06-$i"; New-LabSource $e.legacy; & $vars[$k0][0] $e
        $null = Test-Status $e 'MIGRATION_REQUIRED' -Label $k0
        $before = Get-EnvSnap $e
        $r = Invoke-Mig $e
        Test-Refused $r $vars[$k0][1] $k0
        Expect ($before -ceq (Get-EnvSnap $e)) "${k0}: changed $(Get-SnapDiff $before (Get-EnvSnap $e))"
        Expect (@($e.reads).Count -eq 0) "${k0}: a VM state was read"
    }
    # the same files read the same way in the dry run
    $e = New-Env 'q06-dry'; New-LabSource $e.legacy; Write-Text (Join-Path $e.legacy 'coordinator-stop.request') '{"acpi":'
    $r = Invoke-Mig $e -DryRun; Test-Refused $r 'MIGRATION_SOURCE_UNCERTAIN:STOP_REQUEST_UNREADABLE' 'dry run'
}
Case 'Q07' 'the audit as a second witness: a validation with no ledger, an emergency after the last close with no stop request or one that does not hold its VM, an unreadable line: refused; an emergency before a close, a refused one, a cut last line: accepted' {
    $au = 'coordinator-audit.jsonl'; $ap = { param($e) Join-Path $e.legacy $au }
    $noLedger = { param($e) [IO.File]::Delete((Join-Path $e.legacy 'acpi-validation-ledger.json')); [IO.File]::Delete((Join-Path $e.legacy 'coordinator-session.json')) }
    $vars = [ordered]@{
        'ledger-used' = @({ param($e) & $noLedger $e }, 'MIGRATION_SOURCE_UNCERTAIN:LEDGER_USED_BUT_MISSING')
        'stop-request-gone' = @({ param($e) Add-Line (& $ap $e) (New-AuditLine 'emergency.acpi-shutdown' 'DONE' 'P9S-D12' 1) }, 'MIGRATION_SOURCE_UNCERTAIN:STOP_REQUEST_USED_BUT_MISSING')
        'stop-request-raised' = @({ param($e) Add-Line (& $ap $e) (New-AuditLine 'emergency.acpi-shutdown' 'OFF_ABNORMAL' 'P9S-D12' 0) }, 'MIGRATION_SOURCE_UNCERTAIN:STOP_REQUEST_USED_BUT_MISSING')
        'vm-not-held' = @({ param($e) Add-Line (& $ap $e) (New-AuditLine 'emergency.acpi-shutdown' 'DONE' 'P9S-D12' 1); Write-Doc (Join-Path $e.legacy 'coordinator-stop.request') ([ordered]@{ reason = 'EMERGENCY_ACPI_SHUTDOWN'; acpi = [ordered]@{ 'P9S-U24' = 'x' }; unresolved = [ordered]@{} }) }, 'MIGRATION_SOURCE_UNCERTAIN:STOP_REQUEST_USED_BUT_MISSING:P9S-D12')
        'line-unreadable' = @({ param($e) $t = [IO.File]::ReadAllLines((& $ap $e)); $t[3] = '{"utc":"2026-11-21T13:30:00.000Z","action":'; [IO.File]::WriteAllText((& $ap $e), (($t -join "`n") + "`n"), [Text.UTF8Encoding]::new($false)) }, 'MIGRATION_SOURCE_UNCERTAIN:AUDIT_LINE_UNREADABLE:4')
        'not-utf8' = @({ param($e) [IO.File]::AppendAllText((& $ap $e), ''); $fs = [IO.File]::Open((& $ap $e), 'Append'); $fs.Write([byte[]](0xC3, 0x28, 10), 0, 3); $fs.Dispose() }, 'MIGRATION_SOURCE_UNCERTAIN:AUDIT_NOT_UTF8')
    }
    $i = 0
    foreach ($k0 in $vars.Keys) {
        $i++; $e = New-Env "q07-$i"; New-LabSource $e.legacy; & $vars[$k0][0] $e
        $before = Get-EnvSnap $e
        $r = Invoke-Mig $e
        Test-Refused $r $vars[$k0][1] $k0
        Expect ($before -ceq (Get-EnvSnap $e)) "${k0}: changed"
    }
    $ok = [ordered]@{
        'emergency-before-close' = { param($e) $t = [IO.File]::ReadAllLines((& $ap $e)); $l = ($t[0..6] + @((New-AuditLine 'emergency.acpi-shutdown' 'DONE' 'P9S-D12' 1) | ConvertTo-Json -Compress) + $t[7..8]); [IO.File]::WriteAllText((& $ap $e), (($l -join "`n") + "`n"), [Text.UTF8Encoding]::new($false)) }
        'emergency-refused' = { param($e) Add-Line (& $ap $e) (New-AuditLine 'emergency.acpi-shutdown' 'REFUSED' 'P9S-D12' 0) }
        'emergency-dry-run' = { param($e) Add-Line (& $ap $e) (New-AuditLine 'emergency.acpi-shutdown' 'DRYRUN' 'P9S-D12' 0 -Dry) }
        'held' = { param($e) Add-Line (& $ap $e) (New-AuditLine 'emergency.acpi-shutdown' 'DONE' 'P9S-D12' 1); Write-Doc (Join-Path $e.legacy 'coordinator-stop.request') ([ordered]@{ reason = 'EMERGENCY_ACPI_SHUTDOWN'; acpi = [ordered]@{ 'P9S-D12' = '2026-11-21T14:00:00Z' }; unresolved = [ordered]@{} }) }
        'last-line-cut' = { param($e) [IO.File]::AppendAllText((& $ap $e), '{"utc":"2026-11-21T14:00:00.000Z","pid":1,"act', [Text.UTF8Encoding]::new($false)) }
    }
    foreach ($k0 in $ok.Keys) {
        $i++; $e = New-Env "q07-$i"; New-LabSource $e.legacy; & $ok[$k0] $e; Register-Source $e
        $sb = [IO.File]::ReadAllBytes((& $ap $e))
        $r = Invoke-Mig $e
        Expect ($r.outcome -ceq 'DONE') "${k0}: $(Why $r)"
        if ($k0 -ceq 'last-line-cut') {
            Expect (@($r.notes) -ccontains 'AUDIT_LAST_LINE_CUT') "${k0}: notes $(@($r.notes) -join ',')"
            $nb = [IO.File]::ReadAllBytes((Join-Path $e.state $au)); $tail = [Text.Encoding]::UTF8.GetString($nb, $sb.Length, $nb.Length - $sb.Length)
            Expect ((Get-Hex $nb).StartsWith((Get-Hex $sb), [StringComparison]::Ordinal) -and $tail.StartsWith("`n{") -and ($tail.Trim() | ConvertFrom-Json).action -ceq 'state.migrate') "${k0}: the new line does not start on its own line ($tail)"
        }
    }
}
Case 'Q08' 'an earlier folder that never had a ledger: with an audit that shows no ACPI step, the empty ledger at revision 1 is created and marked; with no audit, an empty one or only a cut line, the ACPI history is not verifiable: refused' {
    $e = New-Env 'q08a'; New-LabSource $e.legacy; [IO.File]::Delete((Join-Path $e.legacy 'acpi-validation-ledger.json')); [IO.File]::Delete((Join-Path $e.legacy 'coordinator-session.json'))
    $ap = Join-Path $e.legacy 'coordinator-audit.jsonl'; [IO.File]::Delete($ap); Add-Line $ap (New-AuditLine 'session.open' 'DONE' ''); Add-Line $ap (New-AuditLine 'session.close' 'DONE' ''); Register-Source $e
    $r = Invoke-Mig $e
    Expect ($r.outcome -ceq 'DONE' -and $r.ledger_created) (Why $r)
    Expect ((@($r.trace) -join ',') -ceq ((Get-MigSteps @('coordinator-audit.jsonl', 'vbox-audit.jsonl', 'vm-observations.jsonl') -LedgerCreated) -join ',')) "steps $(@($r.trace) -join ',')"
    $lg = Read-P9SAcpiLedger ([pscustomobject]@{ LedgerPath = (Get-LedgerPath $e); SessionPath = (Join-Path $e.state 'coordinator-session.json') })
    $a = Get-Anchor $e; $mk = Get-Mark (Get-LedgerPath $e)
    Expect ($lg.ok -and $lg.detail -ceq 'revision 1' -and @($lg.doc.entries).Count -eq 0 -and $a.ledger.revision -eq 1 -and $a.ledger.sha256 -ceq $mk.sha256) "ledger $($lg.detail), mark $($a.ledger | ConvertTo-Json -Compress)"
    $m = ConvertFrom-P9SStateJson ([IO.File]::ReadAllBytes((Join-Path $e.state $names.migration)))
    Expect ($m.ledger_created -eq $true -and @($m.absent) -ccontains 'acpi-validation-ledger.json') "migration record $($m | ConvertTo-Json -Compress -Depth 4)"
    $null = Test-Status $e 'READY'
    foreach ($how in 'no-audit', 'empty-audit', 'cut-only') {
        $e = New-Env "q08-$how"; New-LabSource $e.legacy; foreach ($x in 'acpi-validation-ledger.json', 'coordinator-session.json', 'coordinator-audit.jsonl') { [IO.File]::Delete((Join-Path $e.legacy $x)) }
        if ($how -ceq 'empty-audit') { Write-Text (Join-Path $e.legacy 'coordinator-audit.jsonl') '' }
        if ($how -ceq 'cut-only') { Write-Text (Join-Path $e.legacy 'coordinator-audit.jsonl') '{"utc":"2026' }
        $null = Test-Status $e 'MIGRATION_REQUIRED' -Label $how
        $before = Get-EnvSnap $e
        $r = Invoke-Mig $e; Test-Refused $r 'MIGRATION_SOURCE_UNCERTAIN:ACPI_HISTORY_UNVERIFIABLE' $how
        Expect ($before -ceq (Get-EnvSnap $e)) "${how}: changed"
    }
    # only the lock files of an earlier coordinator: evidence, no history
    $e = New-Env 'q08-locks'; New-Item -ItemType Directory -Path $e.legacy | Out-Null; Write-Text (Join-Path $e.legacy 'coordinator.lock') ''
    $null = Test-Status $e 'MIGRATION_REQUIRED' -Label 'lock only'
    $r = Invoke-Mig $e; Test-Refused $r 'MIGRATION_SOURCE_UNCERTAIN:ACPI_HISTORY_UNVERIFIABLE' 'lock only'
    $r = Invoke-Init $e; Test-Refused $r 'STATE_NOT_NEW:MIGRATION_REQUIRED' 'state.init on the lock only'
}
Case 'Q09' 'the authorization rule after a migration: permits approved or valid before initialized_utc refused, later ones pass; during an interrupted migration every permit is refused; the version 1 ledger reads as revision 0 in the new folder' {
    $e = New-Env 'q09'; New-LabSource $e.legacy; Register-Source $e
    $r = Invoke-Mig $e -O @{ OnStep = (Stop-At 'M7') }
    Expect ($r.outcome -ceq 'THROWN') (Why $r)
    $null = Test-Status $e 'INTERRUPTED'
    $inst = Get-P9SStateInstallation (New-EnvCtx $e)
    $w = Get-P9SAcpiInstallationReason ([pscustomobject]@{ approved_utc = '2099-01-01T00:00:00Z' }) ([datetime]::new(2099, 1, 2, 0, 0, 0, [DateTimeKind]::Utc)) ([pscustomobject]@{ Installation = { $inst }.GetNewClosure() })
    Expect (-not $inst.known -and $w -ceq 'AUTHORIZATION_INSTALLATION_UNKNOWN') "interrupted: $w"
    $r = Invoke-Mig $e; Expect ($r.outcome -ceq 'DONE') "resume $(Why $r)"
    $inst = Get-P9SStateInstallation (New-EnvCtx $e); $t0 = $inst.initialized_utc
    $ctx = [pscustomobject]@{ Installation = { $inst }.GetNewClosure() }
    $txt = { param([datetime]$d) $d.ToString('yyyy-MM-ddTHH:mm:ss.fffffffZ', [Globalization.CultureInfo]::InvariantCulture) }
    Expect ((Get-P9SAcpiInstallationReason ([pscustomobject]@{ approved_utc = (& $txt $t0) }) $t0.AddTicks(1) $ctx) -ceq 'AUTHORIZATION_BEFORE_INSTALLATION') 'approved at initialized_utc accepted'
    Expect ((Get-P9SAcpiInstallationReason ([pscustomobject]@{ approved_utc = (& $txt $t0.AddTicks(1)) }) $t0 $ctx) -ceq 'AUTHORIZATION_BEFORE_INSTALLATION') 'valid from initialized_utc accepted'
    Expect ($null -eq (Get-P9SAcpiInstallationReason ([pscustomobject]@{ approved_utc = (& $txt $t0.AddTicks(1)) }) $t0.AddTicks(1) $ctx)) 'a later permit refused'
    $lg = Read-P9SAcpiLedger ([pscustomobject]@{ LedgerPath = (Get-LedgerPath $e); SessionPath = (Join-Path $e.state 'coordinator-session.json') })
    Expect ($lg.ok -and $lg.detail -ceq 'revision 0' -and @($lg.doc.entries).Count -eq 1) "ledger in the new folder: $($lg.detail) $($lg.reason)"
}
Case 'Q10' 'a consumed stop request, a ledger with its revision record and an open session are carried over byte for byte; the request history in the new folder is that of the earlier folder (no budget comes back)' {
    $e = New-Env 'q10'; New-LabSource $e.legacy
    Write-Doc (Join-Path $e.legacy 'coordinator-stop.request') ([ordered]@{ reason = 'EMERGENCY_ACPI_SHUTDOWN'; utc = '2026-11-21T14:00:00Z'; pid = 1; acpi = [ordered]@{ 'P9S-D12' = '2026-11-21T14:00:01Z' }; unresolved = [ordered]@{} })
    Add-Line (Join-Path $e.legacy 'coordinator-audit.jsonl') (New-AuditLine 'emergency.acpi-shutdown' 'DONE' 'P9S-D12' 1)
    Save-Ledger (Join-Path $e.legacy 'acpi-validation-ledger.json'); Save-Ledger (Join-Path $e.legacy 'acpi-validation-ledger.json')
    $p = Join-Path $e.legacy 'coordinator-session.json'; $d = ConvertFrom-P9SStateJson ([IO.File]::ReadAllBytes($p)); $d.lifecycle = 'OPEN'; Write-Doc $p $d
    Register-Source $e
    $ctxOf = { param($dir) [pscustomobject]@{ LedgerPath = (Join-Path $dir 'acpi-validation-ledger.json'); SessionPath = (Join-Path $dir 'coordinator-session.json'); StopRequestPath = (Join-Path $dir 'coordinator-stop.request') } }
    $hs = Get-P9SAcpiRequestHistory (& $ctxOf $e.legacy) 'P9S-D12'
    $r = Invoke-Mig $e
    Expect ($r.outcome -ceq 'DONE') (Why $r)
    foreach ($n in 'coordinator-stop.request', 'acpi-validation-ledger.json', 'acpi-validation-ledger.revision.json', 'coordinator-session.json') {
        Expect ((Get-Hex ([IO.File]::ReadAllBytes((Join-Path $e.state $n)))) -ceq (Get-Hex ([IO.File]::ReadAllBytes((Join-Path $e.legacy $n))))) "$n differs"
    }
    $hn = Get-P9SAcpiRequestHistory (& $ctxOf $e.state) 'P9S-D12'
    Expect ($hs.spent -and $hn.spent -and -not $hn.ok -and ($hn.reasons -ccontains 'ACPI_ALREADY_REQUESTED')) "request history earlier $($hs.spent), new $($hn.spent) $(@($hn.reasons) -join ',')"
    Expect ((Get-Anchor $e).ledger.revision -eq 2) "mark $((Get-Anchor $e).ledger | ConvertTo-Json -Compress)"
}
Case 'Q11' 'the locks: the earlier folder''s coordinator lock held by an old coordinator, its emergency hold held by an old emergency: refused, nothing put in place; released, the same command completes; absent lock files are created empty, nothing else is added' {
    $e = New-Env 'q11a'; New-LabSource $e.legacy; Register-Source $e; $src0 = Get-Files $e.legacy
    $h = [IO.FileStream]::new((Join-Path $e.legacy 'coordinator.lock'), 'OpenOrCreate', 'ReadWrite', 'None')
    try { $r = Invoke-Mig $e; Test-Refused $r 'MIGRATION_SOURCE_BUSY' 'coordinator lock held'; Expect ((@($r.trace) -join ',') -ceq 'P0,M1') "steps $(@($r.trace) -join ',')" } finally { $h.Dispose() }
    $null = Test-Status $e 'MIGRATION_REQUIRED' -Label 'after the busy lock'
    $h = [IO.FileStream]::new((Join-Path $e.legacy 'coordinator-emergency.hold'), 'OpenOrCreate', 'Read', 'Read')
    try { $r = Invoke-Mig $e; Test-Refused $r 'EMERGENCY_IN_PROGRESS' 'hold held'; Expect (-not (Get-KeyValues $e).Count -and -not (Test-Path -LiteralPath $e.state) -and @(Get-Stagings $e).Count -eq 0) 'anchor, state or staging folder written' } finally { $h.Dispose() }
    $r = Invoke-Mig $e; Expect ($r.outcome -ceq 'DONE') "released: $(Why $r)"
    Test-SourceKept $src0 $e.legacy 'q11a' @($names.marker)
    $e = New-Env 'q11b'; New-LabSource $e.legacy -NoLocks; Register-Source $e; $src0 = Get-Files $e.legacy
    $r = Invoke-Mig $e; Expect ($r.outcome -ceq 'DONE') (Why $r)
    Test-SourceKept $src0 $e.legacy 'q11b' @($names.marker, 'coordinator.lock', 'coordinator-emergency.hold')
    foreach ($n in 'coordinator.lock', 'coordinator-emergency.hold') { Expect ((Get-Item -LiteralPath (Join-Path $e.legacy $n)).Length -eq 0) "$n not empty" }
}
Case 'Q12' 'two migrations at once, interleaved at the setup lock, the earlier folder''s lock and a copy: the second is refused SETUP_BUSY and writes nothing; a migration that decided before another completed sees READY under the lock and refuses; one installation' {
    foreach ($at in 'M1', 'M2', 'M5:coordinator-audit.jsonl', 'M8', 'M10') {
        $e = New-Env "q12-$($at.Replace(':', '').Replace('.', ''))"; New-LabSource $e.legacy; Register-Source $e
        $inner = [System.Collections.Generic.List[object]]::new(); $snap = [System.Collections.Generic.List[string]]::new()
        $hook = { param($s) if ($s -ceq $at -and -not $inner.Count) { $snap.Add((Get-EnvSnap $e)); $inner.Add((Invoke-Mig $e)); $snap.Add((Get-EnvSnap $e)) } }.GetNewClosure()
        $r = Invoke-Mig $e -O @{ OnStep = $hook }
        Expect ($inner.Count -eq 1 -and $inner[0].outcome -ceq 'REFUSED' -and @($inner[0].reasons) -ccontains 'SETUP_BUSY') "at ${at}: the second $(if ($inner.Count) { Why $inner[0] })"
        Expect ($snap.Count -eq 2 -and $snap[0] -ceq $snap[1]) "at ${at}: the second wrote something"
        Expect ($r.outcome -ceq 'DONE') "at ${at}: the first $(Why $r)"
        $x = Invoke-Mig $e; Test-Refused $x 'NOT_MIGRATABLE:READY' "at ${at}: again"
        $st = Test-Status $e 'READY'; Expect ($st.installation_id -ceq $r.installation_id) "at ${at}: another installation"
        Expect (@(Get-Stagings $e).Count -eq 0) "at ${at}: a staging folder left"
    }
    $e = New-Env 'q12-p0'; New-LabSource $e.legacy; Register-Source $e
    $inner = [System.Collections.Generic.List[object]]::new()
    $hook = { param($s) if ($s -ceq 'P0' -and -not $inner.Count) { $inner.Add((Invoke-Mig $e)) } }.GetNewClosure()
    $r = Invoke-Mig $e -O @{ OnStep = $hook }
    Expect ($inner.Count -eq 1 -and $inner[0].outcome -ceq 'DONE') "the other: $(if ($inner.Count) { Why $inner[0] })"
    Test-Refused $r 'SETUP_STATE_CHANGED:READY' 'the one that decided first'
    $st = Test-Status $e 'READY'; Expect ($inner.Count -and $st.installation_id -ceq $inner[0].installation_id) 'another installation'
}
Case 'Q13' 'the VMs: a VM process refuses before anything is written; a VM not observed off refuses after the copies, the staging folder kept; once every VM is off the same command completes the same installation; a VM process during the queries refuses' {
    $e = New-Env 'q13a'; New-LabSource $e.legacy; $before = Get-EnvSnap $e
    $r = Invoke-Mig $e -O @{ Procs = { 1 } }; Test-Refused $r 'VM_PROCESS_RUNNING:1' 'a VM process'
    $r = Invoke-Mig $e -O @{ Procs = { throw 'no process list' } }; Test-Refused $r 'VM_PROCESSES_UNKNOWN' 'count failed'
    $r = Invoke-Mig $e -O @{ Procs = { 'two' } }; Test-Refused $r 'VM_PROCESSES_UNKNOWN' 'count not a number'
    Expect ($before -ceq (Get-EnvSnap $e) -and @($e.reads).Count -eq 0) 'written or read before the VM process check'
    foreach ($s in 'running', 'saved', 'paused', 'aborted', 'unreadable') {
        $e = New-Env "q13$($s.Substring(0, 3))"; New-LabSource $e.legacy; Register-Source $e; $src0 = Get-Files $e.legacy
        $r = Invoke-Mig $e -O @{ Vms = @{ 'P9S-D12' = 'off'; 'P9S-U24' = $s } }
        Test-Refused $r "VM_NOT_OFF:P9S-U24:$s" $s
        $null = Test-Status $e 'INTERRUPTED' -Label $s
        $sg = @(Get-Stagings $e)
        Expect ($sg.Count -eq 1 -and -not (Test-Path -LiteralPath (Join-Path $sg[0].FullName $names.install)) -and -not (Test-Path -LiteralPath $e.state) -and @(Get-Content -LiteralPath (Join-Path $sg[0].FullName 'vm-observations.jsonl')).Count -eq 3) "${s}: staging $(@($sg | ForEach-Object Name) -join ',')"
        Expect (-not (Get-Marker $e)) "${s}: marker written"
        $r2 = Invoke-Mig $e
        Expect ($r2.outcome -ceq 'DONE' -and $r2.installation_id -ceq $r.installation_id -and $r2.mode -ceq 'resume') "${s}: then off: $(Why $r2)"
        Expect (@(Get-Stagings $e).Count -eq 1 -and (Test-Path -LiteralPath $sg[0].FullName)) "${s}: the first staging folder is not kept"
        Test-SourceKept $src0 $e.legacy $s @($names.marker)
    }
    $e = New-Env 'q13b'; New-LabSource $e.legacy; $n = [System.Collections.Generic.List[int]]::new()
    $r = Invoke-Mig $e -O @{ Procs = { $n.Add(1); if ($n.Count -ge 4) { 1 } else { 0 } }.GetNewClosure() }
    Test-Refused $r 'VM_PROCESS_RUNNING:1' 'a VM process started during the VM queries'
    Expect (-not (Test-Path -LiteralPath $e.state) -and -not (Get-Marker $e)) 'state folder or marker written'
}
Case 'Q14' 'free space on the state volume: less than twice the bytes to copy plus 64 MiB, or an unknown amount, refuses before anything is written; the staging folder is on the volume of the state folder' {
    $e = New-Env 'q14'; New-LabSource $e.legacy; $before = Get-EnvSnap $e
    $total = [long](@(Get-ChildItem -LiteralPath $e.legacy -File | Where-Object { $_.Name -cin $labFiles } | Measure-Object -Property Length -Sum).Sum)
    $need = 2 * $total + 67108864
    $r = Invoke-Mig $e -O @{ Free = { param($d) [long]($need - 1) }.GetNewClosure() }; Test-Refused $r "STATE_VOLUME_FREE_SPACE:$($need - 1):$need" 'one byte short'
    $r = Invoke-Mig $e -O @{ Free = { param($d) throw 'no drive' } }; Test-Refused $r 'STATE_VOLUME_FREE_SPACE_UNKNOWN' 'unknown'
    $r = Invoke-Mig $e -O @{ Free = { param($d) 'lots' } }; Test-Refused $r 'STATE_VOLUME_FREE_SPACE_UNKNOWN' 'not a number'
    Expect ($before -ceq (Get-EnvSnap $e) -and @($e.reads).Count -eq 0) 'written or read before the space check'
    $seen = [System.Collections.Generic.List[string]]::new()
    $r = Invoke-Mig $e -O @{ Free = { param($d) $seen.Add($d); [long]$need }.GetNewClosure(); OnStep = { param($s) if ($s -ceq 'M4') { $seen.Add("vol $((Get-P9SSetupEntry (Get-Stagings $e)[0].FullName).volume) $((Get-P9SSetupEntry $e.root).volume)") } }.GetNewClosure() }
    Expect ($r.outcome -ceq 'DONE') "exactly enough: $(Why $r)"
    Expect ((Test-P9SStateSamePath $seen[0] $e.lad) -and @($seen | Where-Object { $_ -cmatch '^vol (\d+) \1$' }).Count -eq 1) "asked for $(@($seen) -join ' / ')"
}
Case 'Q15' 'the earlier folder changed during the migration: before or during the copies, refused and nothing put in place; the plain rerun refuses, -Restart takes a new manifest and completes, the earlier anchor value and staging folder kept; a record changed after the rename: never READY; a history that grew: noted' {
    $e = New-Env 'q15a'; New-LabSource $e.legacy; Register-Source $e
    $ap = Join-Path $e.legacy 'coordinator-audit.jsonl'
    $hook = { param($s) if ($s -ceq 'M5:coordinator-audit.jsonl') { Add-Line $ap (New-AuditLine 'emergency.status' 'DONE' '') } }.GetNewClosure()
    $r = Invoke-Mig $e -O @{ OnStep = $hook }
    Test-Refused $r 'MIGRATION_SOURCE_CHANGED' 'a history appended during the copies'
    $null = Test-Status $e 'INTERRUPTED'
    Expect (-not (Test-Path -LiteralPath $e.state) -and -not (Get-Marker $e)) 'state folder or marker written'
    $a0 = Get-Anchor $e; $raw0 = Read-P9SStateAnchorRaw (New-EnvCtx $e)
    $r2 = Invoke-Mig $e; Test-Refused $r2 'MIGRATION_SOURCE_CHANGED' 'the plain rerun'
    Expect (@($r2.lines | Where-Object { $_ -clike "way out:*-Restart $($a0.op_id)*" }).Count -eq 1) "way out: $(@($r2.lines) -join ' / ')"
    $r3 = Invoke-Mig $e -Restart $a0.op_id
    Expect ($r3.outcome -ceq 'DONE' -and $r3.mode -ceq 'restart' -and $r3.installation_id -ceq $a0.installation_id -and $r3.op_id -ceq $a0.op_id -and $r3.manifest_sha256 -cne $a0.manifest_sha256) "restart $(Why $r3)"
    Expect ((@($r3.trace) -join ',') -clike 'P0,M1,M2,M3a,M3,M4,*') "restart steps $(@($r3.trace) -join ',')"
    $kv = Get-KeyValues $e
    Expect ($kv["MigrationRestarted-$($a0.op_id)-1"] -ceq $raw0) "the replaced anchor not kept ($(@($kv.Keys) -join ','))"
    Expect (@(Get-Stagings $e).Count -eq 1) 'the first staging folder is not kept'
    Expect ((Get-Hex ([IO.File]::ReadAllBytes((Join-Path $e.state 'coordinator-audit.jsonl')))).StartsWith((Get-Hex ([IO.File]::ReadAllBytes($ap))), [StringComparison]::Ordinal)) 'the new manifest does not hold the appended line'
    $null = Test-Status $e 'READY'
    # a record changed before its copy
    $e = New-Env 'q15b'; New-LabSource $e.legacy
    $sp = Join-Path $e.legacy 'coordinator-session.json'
    $hook = { param($s) if ($s -ceq 'M5:acpi-validation-ledger.json') { $d = ConvertFrom-P9SStateJson ([IO.File]::ReadAllBytes($sp)); $d.monitor_run_id = 'run-2'; Write-Doc $sp $d } }.GetNewClosure()
    $r = Invoke-Mig $e -O @{ OnStep = $hook }; Test-Refused $r 'MIGRATION_SOURCE_CHANGED:coordinator-session.json' 'a record changed before its copy'
    Expect (-not (Test-Path -LiteralPath $e.state)) 'state folder put in place'
    # a record changed after the rename: READY is never written; put back, the rerun completes
    $e = New-Env 'q15c'; New-LabSource $e.legacy; Register-Source $e
    $sp = Join-Path $e.legacy 'coordinator-session.json'; $orig = [IO.File]::ReadAllBytes($sp)
    $hook = { param($s) if ($s -ceq 'M9') { [IO.File]::WriteAllBytes($sp, [Text.Encoding]::UTF8.GetBytes('{"schema":"p9s-coordinator-session/1","lifecycle":"OPEN"}')) } }.GetNewClosure()
    $r = Invoke-Mig $e -O @{ OnStep = $hook }; Test-Refused $r 'MIGRATION_SOURCE_CHANGED' 'a record changed after the rename'
    Expect ((Get-Anchor $e).phase -ceq 'MIGRATING') "anchor $((Get-Anchor $e).phase)"
    $null = Test-Status $e 'INTERRUPTED'
    $x = Invoke-Mig $e; Test-Refused $x 'MIGRATION_SOURCE_CHANGED:coordinator-session.json' 'the rerun while changed'
    $x = Invoke-Mig $e -Restart (Get-Anchor $e).op_id; Test-Refused $x 'RESTART_NOT_APPLICABLE:STATE_FOLDER_IN_PLACE' 'restart after the rename'
    [IO.File]::WriteAllBytes($sp, $orig)
    $x = Invoke-Mig $e; Expect ($x.outcome -ceq 'DONE' -and $x.installation_id -ceq $r.installation_id) "put back: $(Why $x)"
    # a history that grew after the marker: READY, noted
    $e = New-Env 'q15d'; New-LabSource $e.legacy; Register-Source $e; $ap = Join-Path $e.legacy 'coordinator-audit.jsonl'
    $hook = { param($s) if ($s -ceq 'M10') { Add-Line $ap (New-AuditLine 'emergency.status' 'DONE' '') } }.GetNewClosure()
    $r = Invoke-Mig $e -O @{ OnStep = $hook }
    Expect ($r.outcome -ceq 'DONE' -and @($r.notes) -ccontains 'OLD_FOLDER_HISTORY_CHANGED') "history grew: $(Why $r) notes $(@($r.notes) -join ',')"
}
Case 'Q16' 'interrupted at each kind of step (a step that throws): never READY before M11, the same command completes the same installation, every staging folder kept, nothing lost' {
    $qi = 0
    foreach ($at in 'M3', 'M4', 'M5:coordinator-session.json-tmp', 'M5:vm-observations.jsonl', 'V:P9S-U24', 'V', 'M7-migration', 'M7', 'M8', 'M9', 'M10', 'M11', 'M12') {
        $qi++; $e = New-Env "q16-$qi"; New-LabSource $e.legacy; Register-Source $e; $src0 = Get-Files $e.legacy
        $r = Invoke-Mig $e -O @{ OnStep = (Stop-At $at) }
        $want = if ($at -cin 'M11', 'M12') { 'READY' } else { 'INTERRUPTED' }
        $st = Test-Status $e $want -Label "after $at"
        $fb = Get-Files $e.root
        $r2 = Invoke-Mig $e
        if ($want -ceq 'READY') { Test-Refused $r2 'NOT_MIGRATABLE:READY' "$at, rerun" }
        else { Expect ($r2.outcome -ceq 'DONE' -and $r2.installation_id -ceq $st.installation_id) "${at}: rerun $(Why $r2)" }
        $null = Test-Status $e 'READY' -Label "$at, after the rerun"
        Test-Kept $fb (Get-Files $e.root) $at '\state\'
        Test-SourceKept $src0 $e.legacy $at @($names.marker)
    }
}
Case 'Q17' '-Restart only for its own interrupted migration before anything is published: an invalid id, another id, no interruption, an interrupted initialization, a state folder in place: refused, nothing written; on an unchanged folder it completes' {
    $e = New-Env 'q17a'; New-LabSource $e.legacy; $before = Get-EnvSnap $e
    $r = Invoke-Mig $e -Restart 'not an id!'; Expect ($r.outcome -ceq 'REJECTED' -and @($r.reasons) -ccontains 'RESTART_ID_INVALID') (Why $r)
    $r = Invoke-Mig $e -Restart 'migrate-20261127T000000Z-00000000'; Test-Refused $r 'NOTHING_TO_RESTART:MIGRATION_REQUIRED' 'nothing interrupted'
    Expect ($before -ceq (Get-EnvSnap $e)) 'changed'
    $r = Invoke-Mig $e -O @{ OnStep = (Stop-At 'M5:coordinator-audit.jsonl') }; $op = (Get-Anchor $e).op_id; $m0 = (Get-Anchor $e).manifest_sha256; $before = Get-EnvSnap $e
    $x = Invoke-Mig $e -Restart 'migrate-20261127T000000Z-00000000'; Test-Refused $x 'RESTART_ID_MISMATCH' 'another id'
    Expect ($before -ceq (Get-EnvSnap $e)) 'changed by a refused restart'
    $x = Invoke-Mig $e -Restart $op; Expect ($x.outcome -ceq 'DONE' -and $x.mode -ceq 'restart' -and $x.manifest_sha256 -ceq $m0) "unchanged folder: $(Why $x)"
    $e = New-Env 'q17b'; New-LabSource $e.legacy; Write-Anchor $e (New-AnchorDoc $e (New-Id) 'INITIALIZING' 'new'); $before = Get-EnvSnap $e
    $x = Invoke-Mig $e -Restart (Get-Anchor $e).op_id; Test-Refused $x 'NOTHING_TO_RESTART:INTERRUPTED' 'an interrupted initialization'
    Expect ($before -ceq (Get-EnvSnap $e)) 'changed'
    $e = New-Env 'q17c'; New-LabSource $e.legacy; $r = Invoke-Mig $e -O @{ OnStep = (Stop-At 'M9') }; $before = Get-EnvSnap $e
    $x = Invoke-Mig $e -Restart (Get-Anchor $e).op_id; Test-Refused $x 'RESTART_NOT_APPLICABLE:STATE_FOLDER_IN_PLACE' 'after the rename'
    Expect ($before -ceq (Get-EnvSnap $e)) 'changed'
}
Case 'Q18' 'a state folder in the way: one already there (no anchor) refuses before anything is written; one appearing before the rename refuses the rename, both kept, never READY' {
    $e = New-Env 'q18a'; New-LabSource $e.legacy; New-Root $e
    $null = Test-Status $e 'MIGRATION_REQUIRED' -Label 'an empty state folder'
    $before = Get-EnvSnap $e
    $r = Invoke-Mig $e; Test-Refused $r 'STATE_FOLDER_EXISTS' 'an empty state folder'
    Expect ($before -ceq (Get-EnvSnap $e)) "changed: $(Get-SnapDiff $before (Get-EnvSnap $e))"
    $e = New-Env 'q18b'; New-LabSource $e.legacy
    $hook = { param($s) if ($s -ceq 'M8') { New-Item -ItemType Directory -Path $e.state | Out-Null; Write-Text (Join-Path $e.state 'other.txt') 'x' } }.GetNewClosure()
    $r = Invoke-Mig $e -O @{ OnStep = $hook }; Test-Refused $r 'STATE_FOLDER_EXISTS*' 'a folder appearing'
    Expect ((Get-Content -Raw -LiteralPath (Join-Path $e.state 'other.txt')) -ceq 'x' -and @(Get-ChildItem -LiteralPath $e.state -Force).Count -eq 1 -and @(Get-Stagings $e).Count -eq 1) 'the folder that appeared or the staging folder was changed'
    $null = Test-Status $e 'INTERRUPTED'
    Expect (-not (Get-Marker $e)) 'marker written'
}
Case 'Q19' 'the anchor rewritten by another writer before READY: ANCHOR_CHANGED, never READY' {
    $e = New-Env 'q19'; New-LabSource $e.legacy
    $hook = { param($s) if ($s -ceq 'M10') { $a = Get-Anchor $e; $a.manifest_sha256 = '0' * 64; Write-Anchor $e $a } }.GetNewClosure()
    $r = Invoke-Mig $e -O @{ OnStep = $hook }; Test-Refused $r 'ANCHOR_CHANGED' 'anchor rewritten'
    Expect ((Get-Anchor $e).phase -ceq 'MIGRATING') 'phase'
    $null = Test-Status $e 'INTERRUPTED'
}
Case 'Q20' 'the staging folder altered before its check: a copied record changed, a record listed as absent added, an unknown file, a write entry for another account, a hard link: STAGING_NOT_COMPLETE, never renamed' {
    $vars = [ordered]@{
        'record-changed' = @({ param($d) [IO.File]::AppendAllText((Join-Path $d 'coordinator-session.json'), ' ') }, 'COPY_DIFFERS:coordinator-session.json')
        'record-added' = @({ param($d) Write-Text (Join-Path $d 'coordinator-stop.request') '{}' }, 'UNEXPECTED_ENTRY:coordinator-stop.request')
        'unknown' = @({ param($d) Write-Text (Join-Path $d 'notes.txt') 'x' }, 'UNEXPECTED_ENTRY:notes.txt')
        'foreign-write' = @({ param($d) $null = Add-Rule (Join-Path $d 'acpi-validation-ledger.json') 'S-1-5-32-545' 'Write' 'Allow' }, 'ACL:acpi-validation-ledger.json:ACL_FOREIGN_WRITE')
        'hard-link' = @({ param($d) New-Item -ItemType HardLink -Path (Join-Path (Split-Path $d -Parent) 'link.json') -Target (Join-Path $d 'install.json') | Out-Null }, 'ENTRY:install.json')
        'history-cut' = @({ param($d) $p = Join-Path $d 'vbox-audit.jsonl'; $b = [IO.File]::ReadAllBytes($p); [IO.File]::WriteAllBytes($p, $b[0..10]) }, 'COPY_DIFFERS:vbox-audit.jsonl')
    }
    $i = 0
    foreach ($k0 in $vars.Keys) {
        $i++; $e = New-Env "q20-$i"; New-LabSource $e.legacy; $act = $vars[$k0][0]
        $hook = { param($s) if ($s -ceq 'M7') { & $act (Get-Stagings $e)[0].FullName } }.GetNewClosure()
        $r = Invoke-Mig $e -O @{ OnStep = $hook }
        Expect ($r.outcome -ceq 'REFUSED' -and @($r.reasons) -ccontains 'STAGING_NOT_COMPLETE' -and @($r.reasons) -ccontains $vars[$k0][1]) "${k0}: $(Why $r)"
        Expect (-not (Test-Path -LiteralPath $e.state) -and -not (Get-Marker $e)) "${k0}: renamed or marker written"
        $null = Test-Status $e 'INTERRUPTED' -Label $k0
    }
}
Case 'Q21' 'the clock: earlier than the last write of the earlier folder, or than a resumed anchor''s created_utc: refused, nothing written; initialized_utc at 100 ns, never before created_utc' {
    $e = New-Env 'q21a'; New-LabSource $e.legacy; $before = Get-EnvSnap $e
    $r = Invoke-Mig $e -O @{ UtcNow = { [DateTime]::UtcNow.AddHours(-1) } }; Test-Refused $r 'CLOCK_BEHIND_EVIDENCE' 'clock behind the folder'
    Expect ($before -ceq (Get-EnvSnap $e)) 'changed'
    $r = Invoke-Mig $e -O @{ OnStep = (Stop-At 'M4') }; $a = Get-Anchor $e; $c0 = ConvertFrom-P9SStateUtcText $a.created_utc; $before = Get-EnvSnap $e
    $r = Invoke-Mig $e -O @{ UtcNow = { $c0.AddTicks(-1) }.GetNewClosure() }; Test-Refused $r 'CLOCK_BEHIND_EVIDENCE' 'clock behind the anchor'
    Expect ($before -ceq (Get-EnvSnap $e)) 'changed'
    $r = Invoke-Mig $e; Expect ($r.outcome -ceq 'DONE') (Why $r)
    $a = Get-Anchor $e; Expect ($a.initialized_utc -cmatch '^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{7}Z$' -and (ConvertFrom-P9SStateUtcText $a.initialized_utc) -ge $c0) "initialized $($a.initialized_utc) created $($a.created_utc)"
}
Case 'Q22' 'after a migration: state.init and state.migrate refuse READY; a record changed in the earlier folder is STATE_DIVERGED and nothing migrates it again; a history appended or the folder deleted keeps READY with a note' {
    $e = New-Env 'q22'; New-LabSource $e.legacy; $r = Invoke-Mig $e; Expect ($r.outcome -ceq 'DONE') (Why $r)
    $x = Invoke-Init $e; Test-Refused $x 'STATE_NOT_NEW:READY' 'state.init'
    $x = Invoke-Mig $e; Test-Refused $x 'NOT_MIGRATABLE:READY' 'state.migrate'
    Add-Line (Join-Path $e.legacy 'coordinator-audit.jsonl') (New-AuditLine 'emergency.status' 'DONE' '')
    $null = Test-Status $e 'READY' -Notes @('OLD_FOLDER_HISTORY_CHANGED')
    Write-Text (Join-Path $e.legacy 'coordinator-stop.request') '{"acpi":{},"unresolved":{}}'
    $null = Test-Status $e 'STATE_DIVERGED' -Reasons @('OLD_FOLDER_CHANGED:coordinator-stop.request')
    $x = Invoke-Mig $e; Test-Refused $x 'NOT_MIGRATABLE:STATE_DIVERGED' 'state.migrate on a divergence'
    Remove-Tree $e.legacy
    $null = Test-Status $e 'READY' -Notes @('OLD_FOLDER_GONE')
}
Case 'Q23' 'the earlier folder is a computed location: the one under the known folder''s Temp migrates when TMP points elsewhere; an anchor naming another folder is refused' {
    $e = New-Env 'q23a'; New-LabSource $e.legacy2; Register-Source ([pscustomobject]@{ legacy = $e.legacy2 })
    $r = Invoke-Mig $e; Expect ($r.outcome -ceq 'DONE' -and (Test-P9SStateSamePath $r.source $e.legacy2)) "$(Why $r) from $($r.source)"
    $a = Get-Anchor $e; Expect ((Test-P9SStateSamePath $a.source_dir $e.legacy2) -and (Test-P9SStateSamePath $e.state (New-EnvCtx $e).StateDir)) "source $($a.source_dir)"
    $e = New-Env 'q23b'; New-LabSource $e.legacy; $r = Invoke-Mig $e -O @{ OnStep = (Stop-At 'M4') }
    $a = Get-Anchor $e; $a.source_dir = Join-Path $e.base 'elsewhere\p9s-hostmon'; Write-Anchor $e $a; $before = Get-EnvSnap $e
    $x = Invoke-Mig $e; Test-Refused $x 'MIGRATION_SOURCE_NOT_A_KNOWN_LOCATION' 'anchor source elsewhere'
    Expect ($before -ceq (Get-EnvSnap $e)) 'changed'
}
Case 'Q24' 'state.migrate with another parameter (-StatePath, -Supersede, -Vm) on the command line: refused, exit 1, nothing read' {
    $before = Get-RealSnap
    foreach ($x in @(@('-StatePath', (Join-Path $base 'q24\state.json')), @('-Supersede', 'inst-00000000'), @('-Vm', 'P9S-D12'))) {
        $o = @(& $pw -NoProfile -NonInteractive -File $cli -Action state.migrate $x[0] $x[1] 2>&1 | ForEach-Object { "$_" }); $code = $LASTEXITCODE
        Expect ($code -eq 1 -and ($o -ccontains 'outcome: REFUSED; STATE_MIGRATE_PARAMETER')) "$($x[0]): exit $code; $($o -join ' / ')"
    }
    Expect ($before -ceq (Get-RealSnap) -and -not (Test-Path -LiteralPath (Join-Path $base 'q24'))) 'something was created'
}
Case 'Q25' 'the state folder changed between the rename and READY: a record removed, a copied record changed, a record listed as absent added: STATE_FOLDER_NOT_COMPLETE, never READY' {
    $vars = [ordered]@{
        'record-removed' = @({ param($d) [IO.File]::Delete((Join-Path $d 'coordinator-session.json')) }, 'COPY_NOT_READ:coordinator-session.json:absent')
        'record-changed' = @({ param($d) [IO.File]::AppendAllText((Join-Path $d 'acpi-validation-ledger.json'), ' ') }, 'COPY_DIFFERS:acpi-validation-ledger.json')
        'record-added' = @({ param($d) Write-Text (Join-Path $d 'coordinator-stop.request') '{"acpi":{},"unresolved":{}}' }, 'UNEXPECTED_ENTRY:coordinator-stop.request')
    }
    $i = 0
    foreach ($k0 in $vars.Keys) {
        $i++; $e = New-Env "q25-$i"; New-LabSource $e.legacy; $act = $vars[$k0][0]
        $hook = { param($s) if ($s -ceq 'M10') { & $act $e.state } }.GetNewClosure()
        $r = Invoke-Mig $e -O @{ OnStep = $hook }
        Expect ($r.outcome -ceq 'REFUSED' -and @($r.reasons) -ccontains 'STATE_FOLDER_NOT_COMPLETE' -and @($r.reasons) -ccontains $vars[$k0][1]) "${k0}: $(Why $r)"
        Expect ((Get-Anchor $e).phase -ceq 'MIGRATING') "${k0}: anchor $((Get-Anchor $e).phase)"
        $null = Test-Status $e 'INTERRUPTED' -Label $k0
    }
}
Case 'Q26' 'a marker in the earlier folder that this migration did not write: one naming another installation after the rename, or one without a state folder: refused, never READY' {
    $other = { param($e) Write-Doc (Join-Path $e.legacy $names.marker) ([ordered]@{ schema = 'p9s-state-moved/1'; installation_id = (New-Id); state_dir = $e.state; manifest_sha256 = ('0' * 64); note = 'another' }) }
    $e = New-Env 'q26a'; New-LabSource $e.legacy; $null = Invoke-Mig $e -O @{ OnStep = (Stop-At 'M9') }; & $other $e
    $r = Invoke-Mig $e; Test-Refused $r 'MIGRATION_MARKER_CONFLICT' 'another installation''s marker'
    Expect ((Get-Anchor $e).phase -ceq 'MIGRATING') "anchor $((Get-Anchor $e).phase)"
    $e = New-Env 'q26b'; New-LabSource $e.legacy; $null = Invoke-Mig $e -O @{ OnStep = (Stop-At 'M8') }; & $other $e
    $r = Invoke-Mig $e; Test-Refused $r 'MIGRATION_MARKER_WITHOUT_STATE_FOLDER' 'a marker without a state folder'
    $x = Invoke-Mig $e -Restart (Get-Anchor $e).op_id; Test-Refused $x 'RESTART_NOT_APPLICABLE:MARKER_WRITTEN' 'restart with a marker'
    Expect (-not (Test-Path -LiteralPath $e.state) -and (Get-Anchor $e).phase -ceq 'MIGRATING') 'put in place or READY'
}
$q63 = "state.migrate -DryRun on this machine's real locations: NOT_INITIALIZED, refused, nothing created"
if (-not $env:P9S_TEST_REAL_LOCATIONS) { NotRun 'Q63' $q63 'runs only in the main run of run-all.ps1 -CleanRunner (a clean runner)' }
else {
    $realLegacy = @((Join-Path ([IO.Path]::GetTempPath()) $names.legacy), (Join-Path (Join-Path $realLad 'Temp') $names.legacy))
    $realBusy = @(@($realLegacy | Where-Object { Test-Path -LiteralPath $_ }) + @(if ($realAtStart -ne 'real root False; real anchor key False') { 'state root or anchor' }))
    if ($realBusy.Count) { NotRun 'Q63' $q63 "the real state root, the anchor key or an earlier-version folder exists here ($($realBusy.Count) paths)" }
    else {
        Case 'Q63' $q63 {
            $o = @(& $pw -NoProfile -NonInteractive -File $cli -Action state.migrate -DryRun 2>&1 | ForEach-Object { "$_" }); $code = $LASTEXITCODE
            Expect ($code -eq 1 -and ($o -ccontains 'status: NOT_INITIALIZED') -and ($o -ccontains 'outcome: REFUSED; NOT_MIGRATABLE:NOT_INITIALIZED')) "exit $code; $($o -join ' / ')"
            Expect ((Get-RealSnap) -ceq 'real root False; real anchor key False') "after: $(Get-RealSnap)"
            foreach ($l in $realLegacy) { Expect (-not (Test-Path -LiteralPath $l)) "created $l" }
        }
    }
}
# ================================================================ end
Case 'Q97' 'nothing was deleted from any earlier folder of this suite: every file each had is there with its bytes (histories only grew)' {
    foreach ($s in $script:sources) { if (Test-Path -LiteralPath $s.dir) { Test-Kept $s.files (Get-Files $s.dir) $s.dir } }
    Expect ($script:sources.Count -ge 20) "only $($script:sources.Count) earlier folders registered"
}
Case 'Q98' 'the real state root and anchor key are as they were at the start' {
    Expect ((Get-RealSnap) -ceq $realAtStart) "start: $realAtStart; now: $(Get-RealSnap)"
}
Case 'Q99' 'the test keys and synthetic folders are removed' {
    try { [Microsoft.Win32.Registry]::CurrentUser.DeleteSubKeyTree($keyRoot, $false) } catch { Expect $false "test keys not removed: $($_.Exception.Message)" }
    $parent = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Software\Monolith-P9S-Test', $true)
    if ($parent) { try { if (-not $parent.SubKeyCount -and -not $parent.ValueCount) { $parent.Close(); [Microsoft.Win32.Registry]::CurrentUser.DeleteSubKey('Software\Monolith-P9S-Test', $false) } } catch { } finally { $parent.Dispose() } }
    $k = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey($keyRoot); Expect (-not $k) 'test key still present'; if ($k) { $k.Dispose() }
    Get-ChildItem -LiteralPath $base -Recurse -Force -Attributes ReparsePoint -ErrorAction SilentlyContinue | ForEach-Object { [IO.Directory]::Delete($_.FullName) }
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
