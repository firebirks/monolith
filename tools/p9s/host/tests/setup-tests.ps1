#Requires -Version 7.0
# Tests of state.init (p9s-state-init.ps1) that need separate processes:
# an initialization or a supersede ended by TerminateProcess at each of its
# steps, and the races (two initializations, two resumes, writers acting
# while an initialization waits at a step). The state is synthetic, as in
# tests/persist-tests.ps1, whose helpers this suite repeats; the VM states
# and the VM process count are mocked (no VirtualBox); child processes end
# at a step or wait at one on named events, never on a sleep. The rest of
# state.init is tested in process by tests/persist-tests.ps1. Folders
# under -OutDir (or under the system temporary folder when -OutDir is not
# on a local NTFS volume) and test registry keys under
# HKCU\Software\Monolith-P9S-Test\<run>, removed at the end; the real
# state root and anchor key must be as they were at the start.
#   pwsh -NoProfile -File tests\setup-tests.ps1 -OutDir <new folder>
param([Parameter(Mandatory)][string]$OutDir)
$ErrorActionPreference = 'Stop'
$kit = Split-Path $PSScriptRoot -Parent
. (Join-Path $kit 'hostmon-lib.ps1')
. (Join-Path $kit 'coordinator-lib.ps1')
. (Join-Path $kit 'p9s-state.ps1')
. (Join-Path $kit 'p9s-state-init.ps1')
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
# a child process running state.init on an environment: -CrashAt ends it at
# that step (it writes <out>.reached first), -HoldAt makes it wait there
# for the "go" event after setting "reached"; its result goes to <out>.json
$initWorker = Join-Path $OutDir 'init-worker.ps1'
[IO.File]::WriteAllText($initWorker, @'
param([string]$Kit, [string]$Lad, [string]$Tmp, [string]$Key, [string]$Out, [string]$CrashAt, [string]$HoldAt, [string]$Events, [string]$Supersede, [string]$Vms = 'P9S-D12=off,P9S-U24=off')
$ErrorActionPreference = 'Stop'
. (Join-Path $Kit 'hostmon-lib.ps1'); . (Join-Path $Kit 'coordinator-lib.ps1'); . (Join-Path $Kit 'p9s-state.ps1'); . (Join-Path $Kit 'p9s-state-init.ps1')
$st = @{}; foreach ($x in ($Vms -split ',')) { $kv = $x -split '=', 2; $st[$kv[0]] = $kv[1] }
$wk = [pscustomobject]@{ crash = $CrashAt; hold = $HoldAt; events = $Events; out = $Out }
$step = {
    param($s)
    if ($wk.crash -and $s -ceq $wk.crash) { [IO.File]::WriteAllText("$($wk.out).reached", $s); [Diagnostics.Process]::GetCurrentProcess().Kill() }
    if ($wk.hold -and $s -ceq $wk.hold) {
        $r = [Threading.EventWaitHandle]::OpenExisting("$($wk.events)-reached"); $g = [Threading.EventWaitHandle]::OpenExisting("$($wk.events)-go")
        $null = $r.Set(); if (-not $g.WaitOne(180000)) { throw 'not released' }
    }
}.GetNewClosure()
$sc = New-P9SStateContext -LocalAppData $Lad -TempPath $Tmp -AnchorKey $Key
$ic = New-P9SStateInitContext -State $sc -VmNames @('P9S-D12', 'P9S-U24') -ReadVmStateIn { param($vm, $dir) $st[$vm] }.GetNewClosure() -CountVmProcesses { 0 } -OnStep $step
$a = @{ I = $ic }; if ($Supersede) { $a.Supersede = $Supersede }
$r = Invoke-P9SStateInit @a
[IO.File]::WriteAllText("$Out.json", ([ordered]@{ outcome = $r.outcome; reasons = @($r.reasons); installation_id = $r.installation_id; trace = @($ic.Trace) } | ConvertTo-Json -Compress))
'@, [Text.UTF8Encoding]::new($false))
function Start-InitWorker($E, [hashtable]$A = @{}) {
    $o = Join-Path $OutDir "w-$([guid]::NewGuid().ToString('N').Substring(0, 10))"
    $q = { param($s) '"' + $s + '"' }
    $al = @('-NoProfile', '-NonInteractive', '-File', (& $q $initWorker), '-Kit', (& $q $kit), '-Lad', (& $q $E.lad), '-Tmp', (& $q $E.tmp), '-Key', (& $q $E.key), '-Out', (& $q $o))
    foreach ($k in $A.Keys) { $al += "-$k"; $al += (& $q ([string]$A[$k])) }
    $p = Start-Process -FilePath $pw -PassThru -WindowStyle Hidden -ArgumentList $al -RedirectStandardOutput "$o.stdout" -RedirectStandardError "$o.stderr"
    $null = $p.Handle
    [pscustomobject]@{ p = $p; out = $o }
}
function Get-WorkerResult($W) { if (Test-Path -LiteralPath "$($W.out).json") { Get-Content -Raw -LiteralPath "$($W.out).json" | ConvertFrom-Json } else { $null } }
function Get-WorkerErr($W) { $t = if (Test-Path -LiteralPath "$($W.out).stderr") { Get-Content -Raw -LiteralPath "$($W.out).stderr" } else { '' }; if ($t) { $t.Trim() } else { '' } }
# $true once the worker is at its hold point; $false when it ended first or
# the time ran out (its result and errors then explain why)
function Wait-Held($Ev, $W, [int]$Ms = 180000) {
    $sw = [Diagnostics.Stopwatch]::StartNew()
    while ($sw.ElapsedMilliseconds -lt $Ms) { if ($Ev.reached.WaitOne(200)) { return $true }; if ($W.p.HasExited) { return $Ev.reached.WaitOne(0) } }
    $false
}
function Get-WorkerWhy($W) { "$((Get-WorkerResult $W) | ConvertTo-Json -Compress) $(Get-WorkerErr $W)" }
function New-Events([string]$Name) {
    $n = "Local\p9s-persist-$run-$Name"
    [pscustomobject]@{ name = $n; reached = [Threading.EventWaitHandle]::new($false, [Threading.EventResetMode]::ManualReset, "$n-reached"); go = [Threading.EventWaitHandle]::new($false, [Threading.EventResetMode]::ManualReset, "$n-go") }
}
$newSteps = 'P0', 'I1', 'I2', 'I4', 'V:P9S-D12', 'V:P9S-U24', 'V', 'I5', 'I6-ledger-tmp', 'I6-ledger', 'I6-revision-tmp', 'I6-revision', 'I6', 'I7', 'I8', 'I9', 'I10'
$supSteps = 'P0', 'I1', 'I2a', 'I2', 'I4', 'V:P9S-D12', 'V:P9S-U24', 'V', 'I3', 'I5', 'I6-ledger-tmp', 'I6-ledger', 'I6-revision-tmp', 'I6-revision', 'I6', 'I7', 'I8', 'I9', 'I10'
# an installation whose ledger is gone: STATE_UNCERTAIN, the status a supersede starts from here
function New-Uncertain($E) { $r = Invoke-Init $E; if ($r.outcome -cne 'DONE') { throw "fixture: $(Why $r)" }; [IO.File]::Delete((Get-LedgerPath $E)); $E.id = $r.installation_id }

Case 'J01' 'state.init ended by TerminateProcess at each step: never READY before the anchor is, the installation unknown to the authorization rule meanwhile, the same command completes the same installation, nothing lost' {
    $runs = @(foreach ($s in $newSteps) { $e = New-Env "j01-$([array]::IndexOf($newSteps, $s))"; [pscustomobject]@{ step = $s; e = $e; w = (Start-InitWorker $e @{ CrashAt = $s }) } })
    foreach ($x in $runs) { $null = $x.w.p.WaitForExit(240000) }
    foreach ($x in $runs) {
        $s = $x.step; $e = $x.e
        if (-not (Test-Path -LiteralPath "$($x.w.out).reached")) { Expect $false "${s}: not reached (exit $($x.w.p.ExitCode); $(Get-WorkerErr $x.w))"; continue }
        $want = if ($s -cin 'P0', 'I1') { 'NOT_INITIALIZED' } elseif ($s -cin 'I9', 'I10') { 'READY' } else { 'INTERRUPTED' }
        $st = Test-Status $e $want -Label "$s, after the crash"
        $inst = Get-P9SStateInstallation (New-EnvCtx $e)
        if ($want -ceq 'INTERRUPTED') {
            Expect (-not $inst.known) "${s}: installation known while interrupted"
            $w = Get-P9SAcpiInstallationReason ([pscustomobject]@{ approved_utc = '2099-01-01T00:00:00Z' }) ([datetime]::new(2099, 1, 2, 0, 0, 0, [DateTimeKind]::Utc)) ([pscustomobject]@{ Installation = { $inst }.GetNewClosure() })
            Expect ($w -ceq 'AUTHORIZATION_INSTALLATION_UNKNOWN') "${s}: the authorization rule says $w"
        }
        $fb = Get-Files $e.root; $kb = Get-KeyValues $e
        $r = Invoke-Init $e
        if ($want -ceq 'READY') { Test-Refused $r 'STATE_NOT_NEW:READY' "$s, rerun" }
        else {
            Expect ($r.outcome -ceq 'DONE') "${s}: rerun $(Why $r)"
            if ($want -ceq 'INTERRUPTED') { Expect ($r.installation_id -ceq $st.installation_id) "${s}: the rerun made $($r.installation_id), the anchor named $($st.installation_id)" }
        }
        $null = Test-Status $e 'READY' -Label "$s, after the rerun"
        Test-Kept $fb (Get-Files $e.root) $s
        $ka = Get-KeyValues $e
        foreach ($v in $kb.Keys) { if ($v -cne $names.anchor_value) { Expect ($ka[$v] -ceq $kb[$v]) "${s}: registry value $v changed" } }
    }
}
Case 'J02' 'state.init -Supersede ended by TerminateProcess at each step: the superseded folder kept byte for byte, the replaced anchor kept beside it, the next run completes, one new installation that names the old one' {
    $runs = @(foreach ($s in $supSteps) {
        $e = New-Env "j02-$([array]::IndexOf($supSteps, $s))"; New-Uncertain $e
        $e | Add-Member -NotePropertyName oldFiles -NotePropertyValue (Get-Files $e.state); $e | Add-Member -NotePropertyName oldRaw -NotePropertyValue (Read-P9SStateAnchorRaw (New-EnvCtx $e))
        [pscustomobject]@{ step = $s; e = $e; w = (Start-InitWorker $e @{ CrashAt = $s; Supersede = $e.id }) }
    })
    foreach ($x in $runs) { $null = $x.w.p.WaitForExit(240000) }
    foreach ($x in $runs) {
        $s = $x.step; $e = $x.e
        if (-not (Test-Path -LiteralPath "$($x.w.out).reached")) { Expect $false "${s}: not reached (exit $($x.w.p.ExitCode); $(Get-WorkerErr $x.w))"; continue }
        $want = if ($s -cin 'P0', 'I1', 'I2a') { 'STATE_UNCERTAIN' } elseif ($s -cin 'I9', 'I10') { 'READY' } else { 'INTERRUPTED' }
        $null = Test-Status $e $want -Label "$s, after the crash"
        $fb = Get-Files $e.root
        # the old folder still in place: the rerun renames it (checked below), so its paths are not compared
        $oldInPlace = Test-SameFiles $e.oldFiles (Get-Files $e.state)
        $r = if ($want -ceq 'STATE_UNCERTAIN') { Invoke-Init $e -Supersede $e.id } else { Invoke-Init $e }
        if ($want -ceq 'READY') { Test-Refused $r 'STATE_NOT_NEW:READY' "$s, rerun" } else { Expect ($r.outcome -ceq 'DONE') "${s}: rerun $(Why $r)" }
        $st = Test-Status $e 'READY' -Label "$s, after the rerun"
        $a = Get-Anchor $e
        Expect ($a.supersedes -ceq $e.id -and $a.installation_id -cne $e.id) "${s}: anchor supersedes $($a.supersedes)"
        $kept = @(Get-ChildItem -LiteralPath $e.root -Directory -Force | Where-Object Name -like 'state.superseded-*' | Where-Object { Test-SameFiles $e.oldFiles (Get-Files $_.FullName) })
        Expect ($kept.Count -eq 1) "${s}: the old state folder kept byte for byte in $($kept.Count) folders"
        $kv = Get-KeyValues $e
        Expect (@($kv.Keys | Where-Object { $_ -clike 'Superseded-*' -and $kv[$_] -ceq $e.oldRaw }).Count -ge 1) "${s}: the replaced anchor is not kept ($(@($kv.Keys) -join ','))"
        Test-Kept $fb (Get-Files $e.root) $s $(if ($oldInPlace) { '\state\' } else { '' })
    }
}
Case 'J03' 'two state.init at once: the one holding the setup lock completes, the other is refused SETUP_BUSY and writes nothing; one installation' {
    $e = New-Env 'j03'; $ev = New-Events 'j03'
    $w = Start-InitWorker $e @{ HoldAt = 'I1'; Events = $ev.name }
    if (-not (Wait-Held $ev $w)) { Expect $false "the first did not reach I1: $(Get-WorkerWhy $w)"; return }
    $fb = Get-Files $e.root; $kb = Get-KeyValues $e
    $r = Invoke-Init $e; Test-Refused $r 'SETUP_BUSY' 'the second'
    Expect ((Test-SameFiles $fb (Get-Files $e.root)) -and ((Get-KeyValues $e).Count -eq $kb.Count)) 'the second wrote something'
    $null = $ev.go.Set(); $null = $w.p.WaitForExit(180000)
    $j = Get-WorkerResult $w; Expect ($j -and $j.outcome -ceq 'DONE') "the first: $($j | ConvertTo-Json -Compress) $(Get-WorkerErr $w)"
    $r = Invoke-Init $e; Test-Refused $r 'STATE_NOT_NEW:READY' 'the second, again'
    $st = Test-Status $e 'READY'; Expect ($j -and $st.installation_id -ceq $j.installation_id) 'another installation'
    Expect (@(Get-Stagings $e).Count -eq 0) 'a second staging folder'
}
Case 'J04' 'a state.init that decided before another one completed: under the setup lock it sees READY and refuses, writing nothing' {
    $e = New-Env 'j04'; $ev = New-Events 'j04'
    $w = Start-InitWorker $e @{ HoldAt = 'P0'; Events = $ev.name }
    if (-not (Wait-Held $ev $w)) { Expect $false "the first did not reach P0: $(Get-WorkerWhy $w)"; return }
    $r = Invoke-Init $e; Expect ($r.outcome -ceq 'DONE') "the other: $(Why $r)"
    $fb = Get-Files $e.root; $kb = Get-KeyValues $e
    $null = $ev.go.Set(); $null = $w.p.WaitForExit(180000)
    $j = Get-WorkerResult $w
    Expect ($j -and $j.outcome -ceq 'REFUSED' -and @($j.reasons) -ccontains 'SETUP_STATE_CHANGED:READY') "the held one: $($j | ConvertTo-Json -Compress) $(Get-WorkerErr $w)"
    Expect ((Test-SameFiles $fb (Get-Files $e.root)) -and (Get-KeyValues $e)[$names.anchor_value] -ceq $kb[$names.anchor_value]) 'the held one wrote something'
    $st = Test-Status $e 'READY'; Expect ($st.installation_id -ceq $r.installation_id) 'another installation'
}
Case 'J05' 'an initialization ended at I5, then two resumes at once (after a restart): one completes the same installation, the other is refused SETUP_BUSY' {
    $e = New-Env 'j05'; $w0 = Start-InitWorker $e @{ CrashAt = 'I5' }; $null = $w0.p.WaitForExit(180000)
    $st = Test-Status $e 'INTERRUPTED'; $id = $st.installation_id
    $ev = New-Events 'j05'; $w = Start-InitWorker $e @{ HoldAt = 'I1'; Events = $ev.name }
    if (-not (Wait-Held $ev $w)) { Expect $false "the resume did not reach I1: $(Get-WorkerWhy $w)"; return }
    $r = Invoke-Init $e; Test-Refused $r 'SETUP_BUSY' 'the second resume'
    $null = $ev.go.Set(); $null = $w.p.WaitForExit(180000)
    $j = Get-WorkerResult $w; Expect ($j -and $j.outcome -ceq 'DONE' -and $j.installation_id -ceq $id) "the first resume: $($j | ConvertTo-Json -Compress) (expected $id) $(Get-WorkerErr $w)"
    $r = Invoke-Init $e; Test-Refused $r 'STATE_NOT_NEW:READY' 'the second, again'
    $null = Test-Status $e 'READY'
}
Case 'J06' 'the anchor rewritten by another writer while an initialization waits after I8: READY is never written (ANCHOR_CHANGED)' {
    $e = New-Env 'j06'; $ev = New-Events 'j06'; $w = Start-InitWorker $e @{ HoldAt = 'I8'; Events = $ev.name }
    if (-not (Wait-Held $ev $w)) { Expect $false "not at I8: $(Get-WorkerWhy $w)"; return }
    $a = Get-Anchor $e; $a.op_id = New-Id 'op'; Write-Anchor $e $a
    $null = $ev.go.Set(); $null = $w.p.WaitForExit(180000)
    $j = Get-WorkerResult $w; Expect ($j -and $j.outcome -ceq 'REFUSED' -and @($j.reasons) -ccontains 'ANCHOR_CHANGED') "$($j | ConvertTo-Json -Compress) $(Get-WorkerErr $w)"
    $null = Test-Status $e 'INTERRUPTED'
    $b = Get-Anchor $e; Expect ($b.phase -ceq 'INITIALIZING' -and $b.op_id -ceq $a.op_id) "anchor $($b.phase) $($b.op_id)"
}
Case 'J07' 'a state folder appearing while an initialization waits after I7: the rename is refused, the folder and the staging folder both kept' {
    $e = New-Env 'j07'; $ev = New-Events 'j07'; $w = Start-InitWorker $e @{ HoldAt = 'I7'; Events = $ev.name }
    if (-not (Wait-Held $ev $w)) { Expect $false "not at I7: $(Get-WorkerWhy $w)"; return }
    New-Item -ItemType Directory -Path $e.state | Out-Null; Write-Text (Join-Path $e.state 'other.txt') 'x'
    $sg = @(Get-Stagings $e); $sf = if ($sg.Count) { Get-Files $sg[0].FullName } else { @{} }
    $null = $ev.go.Set(); $null = $w.p.WaitForExit(180000)
    $j = Get-WorkerResult $w; Expect ($j -and $j.outcome -ceq 'REFUSED' -and @($j.reasons | Where-Object { $_ -clike 'STATE_FOLDER_EXISTS*' }).Count) "$($j | ConvertTo-Json -Compress) $(Get-WorkerErr $w)"
    Expect ((Get-Content -Raw -LiteralPath (Join-Path $e.state 'other.txt')) -ceq 'x' -and @(Get-ChildItem -LiteralPath $e.state -Force).Count -eq 1) 'the folder that appeared was changed'
    Expect ($sg.Count -eq 1 -and (Test-SameFiles $sf (Get-Files $sg[0].FullName))) 'the staging folder was changed'
    $null = Test-Status $e 'INTERRUPTED'
}
Case 'J08' 'the ACL of the staging folder changed while an initialization waits after I6: refused at I7; the next run uses a new staging folder and completes' {
    $e = New-Env 'j08'; $ev = New-Events 'j08'; $w = Start-InitWorker $e @{ HoldAt = 'I6'; Events = $ev.name }
    if (-not (Wait-Held $ev $w)) { Expect $false "not at I6: $(Get-WorkerWhy $w)"; return }
    $sg = @(Get-Stagings $e); $null = Add-Rule $sg[0].FullName 'S-1-5-32-545' 'Write' 'Allow'
    $null = $ev.go.Set(); $null = $w.p.WaitForExit(180000)
    $j = Get-WorkerResult $w; Expect ($j -and $j.outcome -ceq 'REFUSED' -and @($j.reasons | Where-Object { $_ -clike 'STAGING_ACL:ACL_FOREIGN_WRITE' }).Count) "$($j | ConvertTo-Json -Compress) $(Get-WorkerErr $w)"
    $null = Test-Status $e 'INTERRUPTED'
    $r = Invoke-Init $e; Expect ($r.outcome -ceq 'DONE' -and $j -and $r.installation_id -ceq $j.installation_id) "the next run: $(Why $r)"
    Expect ((Test-Path -LiteralPath $sg[0].FullName) -and @(Get-Stagings $e).Count -eq 1) 'the altered staging folder is not kept'
}
Case 'J09' 'the state folder changed between I8 and READY: STATE_FOLDER_NOT_COMPLETE, never READY' {
    foreach ($how in 'revision-record-gone', 'stop-request-added', 'install-record-changed') {
        $e = New-Env "j09$($how.Substring(0, 3))"; $ev = New-Events "j09-$how"; $w = Start-InitWorker $e @{ HoldAt = 'I8'; Events = $ev.name }
        if (-not (Wait-Held $ev $w)) { Expect $false "${how}: not at I8: $(Get-WorkerWhy $w)"; continue }
        switch ($how) {
            'revision-record-gone' { [IO.File]::Delete((Join-Path $e.state 'acpi-validation-ledger.revision.json')) }
            'stop-request-added' { Write-Text (Join-Path $e.state 'coordinator-stop.request') '{}' }
            'install-record-changed' { $p = Join-Path $e.state $names.install; $d = ConvertFrom-P9SStateJson ([IO.File]::ReadAllBytes($p)); $d.created_utc = '2020-01-01T00:00:00.0000000Z'; Write-Doc $p $d }
        }
        $null = $ev.go.Set(); $null = $w.p.WaitForExit(180000)
        $j = Get-WorkerResult $w; Expect ($j -and $j.outcome -ceq 'REFUSED' -and @($j.reasons) -ccontains 'STATE_FOLDER_NOT_COMPLETE') "${how}: $($j | ConvertTo-Json -Compress) $(Get-WorkerErr $w)"
        $null = Test-Status $e 'INTERRUPTED' -Label $how
        Expect ((Get-Anchor $e).phase -ceq 'INITIALIZING') "${how}: anchor phase"
    }
}
# ================================================================ end
Case 'J98' 'the real state root and anchor key are as they were at the start' {
    Expect ((Get-RealSnap) -ceq $realAtStart) "start: $realAtStart; now: $(Get-RealSnap)"
}
Case 'J99' 'the test keys and synthetic folders are removed' {
    try { [Microsoft.Win32.Registry]::CurrentUser.DeleteSubKeyTree($keyRoot, $false) } catch { Expect $false "test keys not removed: $($_.Exception.Message)" }
    $parent = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Software\Monolith-P9S-Test', $true)
    # the shared parent goes too when no other run of the suite uses it
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
