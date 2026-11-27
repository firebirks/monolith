#Requires -Version 7.0
# Tests of state.migrate (p9s-state-migrate.ps1) that need separate
# processes: a migration, a resumed one and a restarted one ended by
# TerminateProcess at each of their steps, two migrations at once, and the
# coordinator's own lock and emergency hold functions, run in other
# processes, against a migration that holds the earlier folder. The state is
# synthetic, as in tests/migrate-tests.ps1, whose helpers this suite
# repeats; the VM states, the VM process count and the free space are mocked
# (no VirtualBox); child processes end at a step or wait at one on named
# events, never on a sleep. Folders under -OutDir (or under the system
# temporary folder when -OutDir is not on a local NTFS volume) and test
# registry keys under HKCU\Software\Monolith-P9S-Test\<run>, removed at the
# end; the real state root and anchor key must be as they were at the start.
#   pwsh -NoProfile -File tests\migproc-tests.ps1 -OutDir <new folder>
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

# ================================================================ state.migrate (the helpers of tests/migrate-tests.ps1)
# The earlier coordinator folder as the laboratory left it (synthetic
# content); every environment has a protected ACL, as a profile's Temp has.
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


# ================================================================ workers
# a child process running state.migrate on an environment: -CrashAt ends it
# at that step (it writes <out>.reached first), -HoldAt makes it wait there
# for the "go" event after setting "reached"; its result goes to <out>.json
$migWorker = Join-Path $OutDir 'migrate-worker.ps1'
[IO.File]::WriteAllText($migWorker, @'
param([string]$Kit, [string]$Lad, [string]$Tmp, [string]$Key, [string]$Out, [string]$CrashAt, [string]$HoldAt, [string]$Events, [string]$Restart, [string]$Vms = 'P9S-D12=off,P9S-U24=off')
$ErrorActionPreference = 'Stop'
. (Join-Path $Kit 'hostmon-lib.ps1'); . (Join-Path $Kit 'coordinator-lib.ps1'); . (Join-Path $Kit 'p9s-state.ps1'); . (Join-Path $Kit 'p9s-state-init.ps1'); . (Join-Path $Kit 'p9s-state-migrate.ps1')
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
$ic = New-P9SStateMigrateContext -State $sc -VmNames @('P9S-D12', 'P9S-U24') -ReadVmStateIn { param($vm, $dir) $st[$vm] }.GetNewClosure() -CountVmProcesses { 0 } -FreeBytes { param($d) 107374182400L } -OnStep $step
$a = @{ I = $ic }; if ($Restart) { $a.Restart = $Restart }
$r = Invoke-P9SStateMigrate @a
[IO.File]::WriteAllText("$Out.json", ([ordered]@{ outcome = $r.outcome; reasons = @($r.reasons); installation_id = $r.installation_id; op_id = $r.op_id; trace = @($ic.Trace) } | ConvertTo-Json -Compress))
'@, [Text.UTF8Encoding]::new($false))
# a child process using the coordinator's own functions on the earlier
# folder: -Mode lock takes coordinator.lock (Enter-P9SLock), hold takes the
# emergency hold shared (Enter-P9SEmergencyHold), each with the wait given;
# with -Events it keeps what it took until "go" (or until it is ended)
$coordWorker = Join-Path $OutDir 'coordinator-worker.ps1'
[IO.File]::WriteAllText($coordWorker, @'
param([string]$Kit, [string]$Dir, [string]$Out, [string]$Mode, [int]$WaitMs = 0, [string]$Events)
$ErrorActionPreference = 'Stop'
. (Join-Path $Kit 'hostmon-lib.ps1'); . (Join-Path $Kit 'coordinator-lib.ps1')
$ctx = [pscustomobject]@{ LockPath = Join-Path $Dir 'coordinator.lock' }
$h = if ($Mode -ceq 'lock') { Enter-P9SLock $ctx $WaitMs } else { Enter-P9SEmergencyHold $ctx $WaitMs }
[IO.File]::WriteAllText("$Out.json", ([ordered]@{ mode = $Mode; taken = [bool]$h } | ConvertTo-Json -Compress))
if ($h -and $Events) {
    $r = [Threading.EventWaitHandle]::OpenExisting("$Events-reached"); $g = [Threading.EventWaitHandle]::OpenExisting("$Events-go")
    $null = $r.Set(); $null = $g.WaitOne(180000)
}
if ($h) { $h.Dispose() }
'@, [Text.UTF8Encoding]::new($false))
$q = { param($s) '"' + $s + '"' }
function Start-MigWorker($E, [hashtable]$A = @{}) {
    $o = Join-Path $OutDir "w-$([guid]::NewGuid().ToString('N').Substring(0, 10))"
    $al = @('-NoProfile', '-NonInteractive', '-File', (& $q $migWorker), '-Kit', (& $q $kit), '-Lad', (& $q $E.lad), '-Tmp', (& $q $E.tmp), '-Key', (& $q $E.key), '-Out', (& $q $o))
    foreach ($k in $A.Keys) { $al += "-$k"; $al += (& $q ([string]$A[$k])) }
    $p = Start-Process -FilePath $pw -PassThru -WindowStyle Hidden -ArgumentList $al -RedirectStandardOutput "$o.stdout" -RedirectStandardError "$o.stderr"
    $null = $p.Handle
    [pscustomobject]@{ p = $p; out = $o }
}
function Start-CoordWorker([string]$Dir, [string]$Mode, [int]$WaitMs = 0, [string]$Events) {
    $o = Join-Path $OutDir "c-$([guid]::NewGuid().ToString('N').Substring(0, 10))"
    $al = @('-NoProfile', '-NonInteractive', '-File', (& $q $coordWorker), '-Kit', (& $q $kit), '-Dir', (& $q $Dir), '-Out', (& $q $o), '-Mode', $Mode, '-WaitMs', "$WaitMs")
    if ($Events) { $al += '-Events'; $al += (& $q $Events) }
    $p = Start-Process -FilePath $pw -PassThru -WindowStyle Hidden -ArgumentList $al -RedirectStandardOutput "$o.stdout" -RedirectStandardError "$o.stderr"
    $null = $p.Handle
    [pscustomobject]@{ p = $p; out = $o }
}
# workers started a few at a time; each one's environment and step kept
function Invoke-CrashRuns($Runs, [int]$Pool = 6) {
    $live = [System.Collections.Generic.List[object]]::new()
    foreach ($x in $Runs) {
        while ($live.Count -ge $Pool) { $done = @($live | Where-Object { $_.w.p.HasExited }); if ($done.Count) { foreach ($d in $done) { $null = $live.Remove($d) } } else { $null = $live[0].w.p.WaitForExit(500) } }
        $x.w = Start-MigWorker $x.e $x.args
        $live.Add($x)
    }
    foreach ($x in $Runs) { if ($x.w) { $null = $x.w.p.WaitForExit(240000) } }
}
function Get-SourceRecords($E) { $h = @{}; foreach ($n in $classes.record) { $p = Join-Path $E.legacy $n; if (Test-Path -LiteralPath $p) { $h[$n] = Get-Hex ([IO.File]::ReadAllBytes($p)) } }; $h }
# after a migration: each record of the earlier folder byte for byte in the state folder
function Test-RecordsCopied($E, $Records, [string]$Label) {
    foreach ($n in $Records.Keys) { $p = Join-Path $E.state $n; Expect ((Test-Path -LiteralPath $p) -and (Get-Hex ([IO.File]::ReadAllBytes($p))) -ceq $Records[$n]) "${Label}: $n not copied byte for byte" }
}
function Test-Crashed($X, [string]$Label) {
    if (Test-Path -LiteralPath "$($X.w.out).reached") { return $true }
    Expect $false "${Label}: not reached (exit $($X.w.p.ExitCode); $(Get-WorkerWhy $X.w))"; $false
}
# the status after an interruption: never READY before M11; meanwhile the
# installation is unknown to the authorization rule
function Test-Interrupted($E, [string]$Want, [string]$Label) {
    $st = Test-Status $E $Want -Label "$Label, after the crash"
    if ($Want -ceq 'INTERRUPTED') {
        $inst = Get-P9SStateInstallation (New-EnvCtx $E)
        $w = Get-P9SAcpiInstallationReason ([pscustomobject]@{ approved_utc = '2099-01-01T00:00:00Z' }) ([datetime]::new(2099, 1, 2, 0, 0, 0, [DateTimeKind]::Utc)) ([pscustomobject]@{ Installation = { $inst }.GetNewClosure() })
        Expect (-not $inst.known -and $w -ceq 'AUTHORIZATION_INSTALLATION_UNKNOWN') "${Label}: the authorization rule says $w"
    }
    $st
}

$crashSteps = Get-MigSteps $labFiles
Case 'W01' 'state.migrate ended by TerminateProcess at each step: never READY before M11, the installation unknown to the authorization rule meanwhile, the same command completes the same installation, the earlier folder kept, the records copied byte for byte' {
    $runs = @(for ($i = 0; $i -lt $crashSteps.Count; $i++) { $e = New-Env "w01-$i"; New-LabSource $e.legacy; [pscustomobject]@{ step = $crashSteps[$i]; e = $e; src = (Get-Files $e.legacy); rec = (Get-SourceRecords $e); args = @{ CrashAt = $crashSteps[$i] }; w = $null } })
    Invoke-CrashRuns $runs
    foreach ($x in $runs) {
        $s = $x.step; $e = $x.e
        if (-not (Test-Crashed $x $s)) { continue }
        $want = if ($s -cin 'P0', 'M1', 'M2') { 'MIGRATION_REQUIRED' } elseif ($s -cin 'M11', 'M12') { 'READY' } else { 'INTERRUPTED' }
        $st = Test-Interrupted $e $want $s
        Test-SourceKept $x.src $e.legacy "$s, after the crash" @($names.marker)
        $fb = Get-Files $e.root
        $r = Invoke-Mig $e
        if ($want -ceq 'READY') { Test-Refused $r 'NOT_MIGRATABLE:READY' "$s, rerun" }
        else {
            Expect ($r.outcome -ceq 'DONE') "${s}: rerun $(Why $r)"
            if ($want -ceq 'INTERRUPTED') { Expect ($r.installation_id -ceq $st.installation_id -and $r.mode -ceq 'resume') "${s}: the rerun made $($r.installation_id) ($($r.mode)), the anchor named $($st.installation_id)" }
        }
        $null = Test-Status $e 'READY' -Label "$s, after the rerun"
        Test-Kept $fb (Get-Files $e.root) $s '\state\'
        Test-SourceKept $x.src $e.legacy "$s, after the rerun" @($names.marker)
        Test-RecordsCopied $e $x.rec $s
        Expect ((Get-Marker $e).installation_id -ceq (Get-Anchor $e).installation_id) "${s}: the marker names another installation"
    }
}
Case 'W02' 'a migration that creates the empty ledger ended by TerminateProcess around it: never READY early, the same command completes with one ledger at revision 1' {
    $steps = 'V', 'M6-ledger-tmp', 'M6-ledger', 'M6-revision-tmp', 'M6-revision', 'M6', 'M7-migration'
    $mk = { param($e) New-LabSource $e.legacy; foreach ($n in 'acpi-validation-ledger.json', 'coordinator-session.json', 'coordinator-audit.jsonl') { [IO.File]::Delete((Join-Path $e.legacy $n)) }; foreach ($x in 'session.open', 'session.close') { Add-Line (Join-Path $e.legacy 'coordinator-audit.jsonl') (New-AuditLine $x 'DONE' '') } }
    $runs = @(for ($i = 0; $i -lt $steps.Count; $i++) { $e = New-Env "w02-$i"; & $mk $e; [pscustomobject]@{ step = $steps[$i]; e = $e; src = (Get-Files $e.legacy); args = @{ CrashAt = $steps[$i] }; w = $null } })
    Invoke-CrashRuns $runs
    foreach ($x in $runs) {
        $s = $x.step; $e = $x.e
        if (-not (Test-Crashed $x $s)) { continue }
        $null = Test-Interrupted $e 'INTERRUPTED' $s
        $r = Invoke-Mig $e; Expect ($r.outcome -ceq 'DONE' -and $r.ledger_created) "${s}: rerun $(Why $r)"
        $lg = Read-P9SAcpiLedger ([pscustomobject]@{ LedgerPath = (Get-LedgerPath $e); SessionPath = (Join-Path $e.state 'coordinator-session.json') })
        Expect ($lg.ok -and $lg.detail -ceq 'revision 1' -and @($lg.doc.entries).Count -eq 0 -and (Get-Anchor $e).ledger.revision -eq 1) "${s}: ledger $($lg.detail)"
        Test-SourceKept $x.src $e.legacy $s @($names.marker)
    }
}
Case 'W03' 'a resumed migration (its state folder in place) ended by TerminateProcess at each of its steps: the next run completes the same installation' {
    $steps = 'P0', 'M1', 'M2', 'V:P9S-D12', 'V', 'M10', 'M11', 'M12'
    $runs = @(for ($i = 0; $i -lt $steps.Count; $i++) {
        $e = New-Env "w03-$i"; New-LabSource $e.legacy; $r0 = Invoke-Mig $e -O @{ OnStep = (Stop-At 'M9') }
        [pscustomobject]@{ step = $steps[$i]; e = $e; id = (Get-Anchor $e).installation_id; src = (Get-Files $e.legacy); args = @{ CrashAt = $steps[$i] }; w = $null } })
    Invoke-CrashRuns $runs
    foreach ($x in $runs) {
        $s = $x.step; $e = $x.e
        if (-not (Test-Crashed $x $s)) { continue }
        $want = if ($s -cin 'M11', 'M12') { 'READY' } else { 'INTERRUPTED' }
        $null = Test-Interrupted $e $want "resume $s"
        $r = Invoke-Mig $e
        if ($want -ceq 'READY') { Test-Refused $r 'NOT_MIGRATABLE:READY' "resume $s, rerun" } else { Expect ($r.outcome -ceq 'DONE' -and $r.installation_id -ceq $x.id) "resume ${s}: rerun $(Why $r)" }
        $st = Test-Status $e 'READY' -Label "resume $s, after the rerun"; Expect ($st.installation_id -ceq $x.id) "resume ${s}: another installation"
        Test-SourceKept $x.src $e.legacy "resume $s" @($names.marker)
    }
}
Case 'W04' 'state.migrate -Restart ended by TerminateProcess at its first steps: the earlier anchor value kept each time, the plain rerun still refuses the changed folder, -Restart again completes' {
    $steps = 'M2', 'M3a', 'M3', 'M4'
    $runs = @(for ($i = 0; $i -lt $steps.Count; $i++) {
        $e = New-Env "w04-$i"; New-LabSource $e.legacy; $null = Invoke-Mig $e -O @{ OnStep = (Stop-At 'M5:coordinator-audit.jsonl') }
        Add-Line (Join-Path $e.legacy 'coordinator-audit.jsonl') (New-AuditLine 'emergency.status' 'DONE' '')
        $a = Get-Anchor $e
        [pscustomobject]@{ step = $steps[$i]; e = $e; op = $a.op_id; m0 = $a.manifest_sha256; raw = (Read-P9SStateAnchorRaw (New-EnvCtx $e)); args = @{ CrashAt = $steps[$i]; Restart = $a.op_id }; w = $null } })
    Invoke-CrashRuns $runs
    foreach ($x in $runs) {
        $s = $x.step; $e = $x.e
        if (-not (Test-Crashed $x $s)) { continue }
        $null = Test-Interrupted $e 'INTERRUPTED' "restart $s"
        $kv = Get-KeyValues $e
        if ($s -cne 'M2') { Expect ($kv["MigrationRestarted-$($x.op)-1"] -ceq $x.raw) "restart ${s}: the replaced anchor not kept" }
        $a = Get-Anchor $e
        if ($s -cin 'M3', 'M4') { Expect ($a.manifest_sha256 -cne $x.m0) "restart ${s}: the anchor still has the old manifest" }
        else {
            Expect ($a.manifest_sha256 -ceq $x.m0) "restart ${s}: the anchor has a new manifest"
            $r = Invoke-Mig $e; Test-Refused $r 'MIGRATION_SOURCE_CHANGED' "restart ${s}: the plain rerun"
        }
        $r = if ($s -cin 'M3', 'M4') { Invoke-Mig $e } else { Invoke-Mig $e -Restart $x.op }
        Expect ($r.outcome -ceq 'DONE' -and $r.op_id -ceq $x.op) "restart ${s}: rerun $(Why $r)"
        $null = Test-Status $e 'READY' -Label "restart $s"
    }
}
Case 'W05' 'two migrations in two processes: the one holding the setup lock and the earlier folder completes, the other is refused SETUP_BUSY and writes nothing; one that decided before the other completed refuses SETUP_STATE_CHANGED' {
    $e = New-Env 'w05a'; New-LabSource $e.legacy; $ev = New-Events 'w05a'
    $w = Start-MigWorker $e @{ HoldAt = 'M2'; Events = $ev.name }
    if (-not (Wait-Held $ev $w)) { Expect $false "the first did not reach M2: $(Get-WorkerWhy $w)"; return }
    $fb = Get-Files $e.root; $kb = Get-KeyValues $e
    $r = Invoke-Mig $e; Test-Refused $r 'SETUP_BUSY' 'the second'
    Expect ((Test-SameFiles $fb (Get-Files $e.root)) -and (Get-KeyValues $e).Count -eq $kb.Count) 'the second wrote something'
    $null = $ev.go.Set(); $null = $w.p.WaitForExit(180000)
    $j = Get-WorkerResult $w; Expect ($j -and $j.outcome -ceq 'DONE') "the first: $(Get-WorkerWhy $w)"
    $st = Test-Status $e 'READY'; Expect ($j -and $st.installation_id -ceq $j.installation_id) 'another installation'
    $e = New-Env 'w05b'; New-LabSource $e.legacy; $ev = New-Events 'w05b'
    $w = Start-MigWorker $e @{ HoldAt = 'P0'; Events = $ev.name }
    if (-not (Wait-Held $ev $w)) { Expect $false "the first did not reach P0: $(Get-WorkerWhy $w)"; return }
    $r = Invoke-Mig $e; Expect ($r.outcome -ceq 'DONE') "the other: $(Why $r)"
    $fb = Get-Files $e.root; $kb = Get-KeyValues $e
    $null = $ev.go.Set(); $null = $w.p.WaitForExit(180000)
    $j = Get-WorkerResult $w
    Expect ($j -and $j.outcome -ceq 'REFUSED' -and @($j.reasons) -ccontains 'SETUP_STATE_CHANGED:READY') "the held one: $(Get-WorkerWhy $w)"
    Expect ((Test-SameFiles $fb (Get-Files $e.root)) -and (Get-KeyValues $e)[$names.anchor_value] -ceq $kb[$names.anchor_value]) 'the held one wrote something'
}
Case 'W06' 'the coordinator''s own functions in other processes: a coordinator holding coordinator.lock or an emergency holding the hold refuses the migration, which writes nothing; once that process ends (TerminateProcess) the migration completes; while a migration holds the earlier folder, Enter-P9SLock and Enter-P9SEmergencyHold get nothing' {
    foreach ($mode in 'lock', 'hold') {
        $e = New-Env "w06$mode"; New-LabSource $e.legacy; $ev = New-Events "w06-$mode"
        $c = Start-CoordWorker $e.legacy $mode 0 $ev.name
        if (-not (Wait-Held $ev $c)) { Expect $false "${mode}: the coordinator did not take it: $(Get-WorkerWhy $c)"; continue }
        $r = Invoke-Mig $e; Test-Refused $r $(if ($mode -ceq 'lock') { 'MIGRATION_SOURCE_BUSY' } else { 'EMERGENCY_IN_PROGRESS' }) $mode
        Expect (-not (Get-KeyValues $e).Count -and -not (Test-Path -LiteralPath $e.state) -and @(Get-Stagings $e).Count -eq 0) "${mode}: anchor, state or staging folder written"
        $c.p.Kill(); $null = $c.p.WaitForExit(30000)
        $r = Invoke-Mig $e; Expect ($r.outcome -ceq 'DONE') "${mode}: after the holder ended: $(Why $r)"
    }
    foreach ($at in 'M2', 'M5:coordinator-session.json', 'M9', 'M11') {
        $e = New-Env "w06-$($at.Replace(':', '').Replace('.', '').Replace('-', ''))"; New-LabSource $e.legacy; $ev = New-Events "w06m-$($at -replace '[^A-Za-z0-9]', '')"
        $w = Start-MigWorker $e @{ HoldAt = $at; Events = $ev.name }
        if (-not (Wait-Held $ev $w)) { Expect $false "at ${at}: not held: $(Get-WorkerWhy $w)"; continue }
        foreach ($mode in 'lock', 'hold') {
            $c = Start-CoordWorker $e.legacy $mode 300; $null = $c.p.WaitForExit(60000)
            $j = Get-WorkerResult $c; Expect ($j -and $j.taken -eq $false) "at ${at}: the coordinator's $mode was taken: $(Get-WorkerWhy $c)"
        }
        $null = $ev.go.Set(); $null = $w.p.WaitForExit(180000)
        $j = Get-WorkerResult $w; Expect ($j -and $j.outcome -ceq 'DONE') "at ${at}: the migration: $(Get-WorkerWhy $w)"
        $c = Start-CoordWorker $e.legacy 'lock' 0; $null = $c.p.WaitForExit(60000)
        $j = Get-WorkerResult $c; Expect ($j -and $j.taken) "at ${at}: the lock not free after the migration: $(Get-WorkerWhy $c)"
    }
}
Case 'W07' 'state.status in another process while a migration holds at each phase: never READY before M11, the migration''s installation only after it' {
    foreach ($at in 'M3', 'M7', 'M9', 'M10', 'M11') {
        $e = New-Env "w07-$at"; New-LabSource $e.legacy; $ev = New-Events "w07-$at"
        $w = Start-MigWorker $e @{ HoldAt = $at; Events = $ev.name }
        if (-not (Wait-Held $ev $w)) { Expect $false "at ${at}: not held: $(Get-WorkerWhy $w)"; continue }
        $code = "$libs; . $(Q (Join-Path $kit 'p9s-state-init.ps1')); `$c = New-P9SStateContext -LocalAppData $(Q $e.lad) -TempPath $(Q $e.tmp) -AnchorKey $(Q $e.key); `$s = Get-P9SStateStatus `$c; `$i = Get-P9SStateInstallation `$c; [ordered]@{ status = `$s.status; known = `$i.known } | ConvertTo-Json -Compress"
        $o = Get-LastJson (Invoke-Pwsh $code)
        $want = if ($at -ceq 'M11') { 'READY' } else { 'INTERRUPTED' }
        Expect ($o -and $o.status -ceq $want -and $o.known -eq ($want -ceq 'READY')) "at ${at}: another process saw $($o | ConvertTo-Json -Compress)"
        $null = $ev.go.Set(); $null = $w.p.WaitForExit(180000)
        $j = Get-WorkerResult $w; Expect ($j -and $j.outcome -ceq 'DONE') "at ${at}: $(Get-WorkerWhy $w)"
    }
}
# ================================================================ end
Case 'W98' 'the real state root and anchor key are as they were at the start' {
    Expect ((Get-RealSnap) -ceq $realAtStart) "start: $realAtStart; now: $(Get-RealSnap)"
}
Case 'W99' 'the test keys and synthetic folders are removed' {
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
