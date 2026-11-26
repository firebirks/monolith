#Requires -Version 7.0
# Tests of the read-only status of the persistent state folder
# (p9s-state.ps1). The state is synthetic: folders under -OutDir (or under
# the system temporary folder when -OutDir is not on a local NTFS volume)
# and test registry keys under HKCU\Software\Monolith-P9S-Test\<run>,
# removed at the end. Every classification is bracketed by a snapshot of
# the synthetic tree and of the test keys (names, kinds, sizes, SHA-256,
# attributes, creation and write times, ACLs, registry values) and of the
# presence of the real state root and anchor key; the snapshots must be
# equal. The last access time is not compared: Windows may update it when a
# file is read. Real Windows facts are used where they exist: the known
# folder, the package identity and token of this process, junctions, hard
# links, ACLs and registry keys created by the test. P62 runs the entry
# point against this machine's real locations, and only where none of them
# exists (a clean runner); elsewhere it is NOT_RUN.
#   pwsh -NoProfile -File tests\persist-tests.ps1 -OutDir <new folder>
param([Parameter(Mandatory)][string]$OutDir)
$ErrorActionPreference = 'Stop'
$kit = Split-Path $PSScriptRoot -Parent
. (Join-Path $kit 'hostmon-lib.ps1')
. (Join-Path $kit 'coordinator-lib.ps1')
. (Join-Path $kit 'p9s-state.ps1')
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

# ================================================================ real Windows facts
Case 'P01' 'LocalApplicationData comes from the known folder API, not from the environment' {
    $net = [Environment]::GetFolderPath([Environment+SpecialFolder]::LocalApplicationData)
    Expect ($realLad -and (Test-P9SStateSamePath $realLad $net)) "SHGetKnownFolderPath $realLad, .NET $net"
    $prod = New-P9SStateContext
    Expect ((Test-P9SStateSamePath $prod.LocalAppData $realLad) -and @($prod.KnownFolderReasons).Count -eq 0) "production context $($prod.LocalAppData) $(@($prod.KnownFolderReasons) -join ',')"
    $alt = Join-Path $base 'p01-elsewhere'; New-Item -ItemType Directory -Force -Path $alt | Out-Null
    $r = Invoke-Pwsh "$libs; `$c = New-P9SStateContext; [ordered]@{ lad = `$c.LocalAppData; kf = @(`$c.KnownFolderReasons); legacy = @(`$c.LegacyDirs) } | ConvertTo-Json -Compress" @{ LOCALAPPDATA = $alt; TMP = $alt; TEMP = $alt }
    $j = Get-LastJson $r
    Expect ($j -and (Test-P9SStateSamePath $j.lad $realLad) -and @($j.kf).Count -eq 0) "child with LOCALAPPDATA, TMP and TEMP elsewhere: $($r.out -join ' / ')"
    Expect ($j -and @($j.legacy | Where-Object { Test-P9SStateSamePath $_ (Join-Path $alt $names.legacy) }).Count -eq 1) "the earlier-version folder through TMP is a candidate: $(@($j.legacy) -join ', ')"
    Expect ($j -and @($j.legacy | Where-Object { Test-P9SStateSamePath $_ (Join-Path (Join-Path $realLad 'Temp') $names.legacy) }).Count -eq 1) 'the earlier-version folder under the known folder is a candidate too'
}
Case 'P02' 'this process: no package identity, no app container, no UAC virtualization (Windows APIs)' {
    $f = Get-P9SStateProcessFacts $true
    Expect ($f.package_code -eq 15700) "GetCurrentPackageFullName returned $($f.package_code) (15700 = no package)"
    Expect ($f.app_container -eq 0) "TokenIsAppContainer $($f.app_container)"
    Expect ($f.virtualization -eq 0) "TokenVirtualizationEnabled $($f.virtualization)"
    Expect ([string]$f.path -notlike '*\WindowsApps\*') "process path $($f.path)"
    Expect ($f.account -ceq $me -and $f.account -notin 'S-1-5-18', 'S-1-5-19', 'S-1-5-20') 'account'
}
Case 'P03' 'the native checks and a classification write nothing to the temporary folder' {
    $e = New-Env 'p03'; New-Legacy $e.legacy; $t = Join-Path $base 'p03-tmp'; New-Item -ItemType Directory -Force -Path $t | Out-Null
    $code = "$libs; `$b = @(Get-ChildItem -LiteralPath `$env:TMP -Force -Recurse | ForEach-Object FullName); `$c = New-P9SStateContext -LocalAppData $(Q $e.lad) -TempPath $(Q $e.tmp) -AnchorKey $(Q $e.key); `$s = Get-P9SStateStatus `$c; `$a = @(Get-ChildItem -LiteralPath `$env:TMP -Force -Recurse | ForEach-Object FullName); [ordered]@{ status = `$s.status; before = `$b.Count; after = `$a.Count; same = ((`$b -join '|') -ceq (`$a -join '|')); native = ('P9SStateNative' -as [type]) -ne `$null } | ConvertTo-Json -Compress"
    $before = Get-EnvSnap $e
    $j = Get-LastJson (Invoke-Pwsh $code @{ TMP = $t; TEMP = $t })
    Expect ($j -and $j.native -and $j.status -eq 'MIGRATION_REQUIRED') "child: $($j | ConvertTo-Json -Compress)"
    Expect ($j -and $j.same) "files in the temporary folder before $($j.before), after $($j.after)"
    Expect ($before -ceq (Get-EnvSnap $e)) 'synthetic state changed'
}

# ================================================================ the 17 statuses
Case 'P10' 'SANDBOX: a -StatePath folder other than the state folder' {
    $e = New-Env 'p10'; Install-New $e
    $st = Test-Status $e 'SANDBOX' @('ASKED_FOLDER_IS_NOT_THE_STATE_FOLDER') @{ AskedStatePath = (Join-Path $e.base 'elsewhere\state.json') }
    $st = Test-Status $e 'READY' @() @{ AskedStatePath = (Join-Path $e.state 'state.json') } -Label 'the state folder itself'
}
Case 'P11' 'LOCATION_UNSAFE: the state folder, the root or LocalAppData is a junction' {
    $e = New-Env 'p11a'; Install-New $e; $copy = Join-Path $e.base 'copy'; Copy-Item -LiteralPath $e.state -Destination $copy -Recurse
    Remove-Tree $e.state; New-Item -ItemType Junction -Path $e.state -Target $copy | Out-Null
    $null = Test-Status $e 'LOCATION_UNSAFE' @('STATE_PATH_REPARSE_POINT') -Label 'state'
    $e = New-Env 'p11b'; Install-New $e; $mv = Join-Path $e.base 'moved'; [IO.Directory]::Move($e.root, $mv); New-Item -ItemType Junction -Path $e.root -Target $mv | Out-Null
    $null = Test-Status $e 'LOCATION_UNSAFE' @('STATE_PATH_REPARSE_POINT') -Label 'root'
    $e = New-Env 'p11c'; $real = Join-Path $e.base 'real-lad'; [IO.Directory]::Move($e.lad, $real); New-Item -ItemType Junction -Path $e.lad -Target $real | Out-Null
    $null = Test-Status $e 'LOCATION_UNSAFE' @('STATE_PATH_REPARSE_POINT') -Label 'LocalAppData'
    $e = New-Env 'p11d'; Install-New $e; New-Item -ItemType Junction -Path (Join-Path $e.state 'inner') -Target $e.base | Out-Null
    $null = Test-Status $e 'LOCATION_UNSAFE' @('STATE_ENTRY_REPARSE_POINT') -Label 'junction inside the state folder'
}
Case 'P12' 'LOCATION_UNSAFE: a write entry for another account on the state folder or a record' {
    $e = New-Env 'p12a'; Install-New $e; $null = Add-Rule $e.state 'S-1-1-0' 'Write' 'Allow'
    $null = Test-Status $e 'LOCATION_UNSAFE' @('ACL_FOREIGN_WRITE') -Label 'folder'
    $e = New-Env 'p12b'; Install-New $e; $null = Add-Rule (Get-LedgerPath $e) 'S-1-5-32-545' 'Modify' 'Allow'
    $null = Test-Status $e 'LOCATION_UNSAFE' @('ACL_FOREIGN_WRITE') -Label 'ledger file'
    $e = New-Env 'p12c'; Install-New $e; $null = Add-Rule $e.state 'S-1-1-0' 'ReadAndExecute' 'Allow'
    $null = Test-Status $e 'READY' -Label 'a read-only entry for another account'
}
Case 'P13' 'LOCATION_UNSAFE: a root whose ACL is inherited' {
    $e = New-Env 'p13'; [IO.Directory]::CreateDirectory($e.state) | Out-Null
    $null = Test-Status $e 'LOCATION_UNSAFE' @('ACL_NOT_PROTECTED')
}
Case 'P14' 'LOCATION_UNSAFE: a hard-linked file in the state folder' {
    $e = New-Env 'p14'; Install-New $e
    New-Item -ItemType HardLink -Path (Join-Path $e.base 'outside-link.json') -Target (Join-Path $e.state $names.install) | Out-Null
    $null = Test-Status $e 'LOCATION_UNSAFE' @('STATE_ENTRY_HARD_LINKED')
}
Case 'P15' 'LOCATION_UNSAFE, not absence: a state folder that cannot be listed' {
    $e = New-Env 'p15'; Install-New $e; $rule = Add-Rule $e.state $me 'ReadData' 'Deny'
    try { $null = Test-Status $e 'LOCATION_UNSAFE' @('STATE_FOLDER_NOT_LISTABLE') } finally { Remove-Rule $e.state $rule }
    $null = Test-Status $e 'READY' -Label 'after the entry is removed'
}
Case 'P16' 'LOCATION_UNSAFE: packaged process, app container, UAC virtualization, WindowsApps path (simulated facts)' {
    $e = New-Env 'p16'; Install-New $e
    $ok = [pscustomobject]@{ account = $me; package_code = 15700; app_container = 0; virtualization = 0; path = 'X:\pwsh\pwsh.exe' }
    $null = Test-Status $e 'READY' @() @{ ProcessFacts = $ok } -Label 'baseline'
    foreach ($t in @(@{ package_code = 122; want = 'PACKAGED_PROCESS' }, @{ package_code = 0; want = 'PACKAGED_PROCESS' }, @{ package_code = 87; want = 'PACKAGE_STATE_UNVERIFIABLE' },
                     @{ app_container = 1; want = 'APP_CONTAINER' }, @{ app_container = -1; want = 'APP_CONTAINER_UNVERIFIABLE' }, @{ virtualization = 1; want = 'UAC_VIRTUALIZATION' },
                     @{ path = 'X:\Program Files\WindowsApps\Microsoft.PowerShell_7\pwsh.exe'; want = 'PROCESS_IN_WINDOWSAPPS' })) {
        $f = $ok.PSObject.Copy(); foreach ($k in @($t.Keys | Where-Object { $_ -ne 'want' })) { $f.$k = $t[$k] }
        $null = Test-Status $e 'LOCATION_UNSAFE' @($t.want) @{ ProcessFacts = $f } -Label $t.want
    }
}
Case 'P17' 'LOCATION_UNSAFE: service account, volume not local NTFS (simulated facts)' {
    $e = New-Env 'p17'; Install-New $e
    $null = Test-Status $e 'LOCATION_UNSAFE' @('SERVICE_ACCOUNT') @{ ProcessFacts = [pscustomobject]@{ account = 'S-1-5-18'; package_code = 15700; app_container = 0; virtualization = 0; path = 'X:\p.exe' } }
    $null = Test-Status $e 'LOCATION_UNSAFE' @('STATE_VOLUME_NOT_LOCAL_NTFS') @{ VolumeFacts = [pscustomobject]@{ type = 'Removable'; format = 'exFAT' } }
    $null = Test-Status $e 'LOCATION_UNSAFE' @('STATE_VOLUME_NOT_LOCAL_NTFS') @{ VolumeFacts = [pscustomobject]@{ type = 'Network'; format = 'NTFS' } }
}
Case 'P20' 'ANCHOR_UNREADABLE: not JSON, unknown or missing field, wrong kind, no value, duplicated name' {
    $e = New-Env 'p20'; Install-New $e; $good = Get-Anchor $e
    Write-Anchor $e $null -Raw '{"schema":"p9s-state-anchor/1"'; $null = Test-Status $e 'ANCHOR_UNREADABLE' @('ANCHOR_NOT_PARSEABLE') -Label 'cut'
    $x = [ordered]@{}; foreach ($k in $good.Keys) { $x[$k] = $good[$k] }; $x.extra = 1
    Write-Anchor $e ($x); $null = Test-Status $e 'ANCHOR_UNREADABLE' @('ANCHOR_UNKNOWN_FIELD:extra') -Label 'extra field'
    $x.Remove('extra'); $x.Remove('ledger'); Write-Anchor $e $x; $null = Test-Status $e 'ANCHOR_UNREADABLE' @('ANCHOR_MISSING_FIELD:ledger') -Label 'no mark'
    $x.ledger = [ordered]@{ revision = 'one'; sha256 = 'x' }; Write-Anchor $e $x; $null = Test-Status $e 'ANCHOR_UNREADABLE' @('ANCHOR_FIELD:ledger') -Label 'bad mark'
    Write-Anchor $e $null -Kind DWord; $null = Test-Status $e 'ANCHOR_UNREADABLE' @('ANCHOR_VALUE_KIND') -Label 'DWORD'
    Write-Anchor $e $null -Raw ('{"phase":"READY",' + ($good | ConvertTo-Json -Compress -Depth 5).Substring(1)); $null = Test-Status $e 'ANCHOR_UNREADABLE' @('ANCHOR_NOT_PARSEABLE') -Label 'duplicated name'
    $k = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey($e.key, $true); $k.DeleteValue($names.anchor_value); $k.Dispose()
    $null = Test-Status $e 'ANCHOR_UNREADABLE' @('ANCHOR_VALUE_ABSENT') -Label 'key without value'
}
Case 'P21' 'ANCHOR_UNREADABLE, not absence: an anchor key that cannot be read' {
    $e = New-Env 'p21'; Install-New $e
    $k = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey($e.key, [Microsoft.Win32.RegistryKeyPermissionCheck]::ReadWriteSubTree, [Security.AccessControl.RegistryRights]'ChangePermissions, ReadPermissions')
    $rule = [Security.AccessControl.RegistryAccessRule]::new([Security.Principal.SecurityIdentifier]::new($me), [Security.AccessControl.RegistryRights]'QueryValues, EnumerateSubKeys', 'Deny')
    $sec = $k.GetAccessControl(); $sec.AddAccessRule($rule); $k.SetAccessControl($sec)
    try { $null = Test-Status $e 'ANCHOR_UNREADABLE' @('ANCHOR_KEY_NOT_READABLE') }
    finally { $sec = $k.GetAccessControl(); $null = $sec.RemoveAccessRule($rule); $k.SetAccessControl($sec); $k.Dispose() }
    $null = Test-Status $e 'READY' -Label 'after the entry is removed'
}
Case 'P22' 'ANCHOR_MISSING: a state folder with content and no anchor' {
    $e = New-Env 'p22'; Install-New $e; [Microsoft.Win32.Registry]::CurrentUser.DeleteSubKeyTree($e.key)
    $st = Test-Status $e 'ANCHOR_MISSING' @('STATE_FOLDER_HOLDS:*install.json*')
    Expect ($st.installation_id -ceq $e.id) "installation id $($st.installation_id)"
    $e = New-Env 'p22b'; New-Root $e; Write-Text (Join-Path $e.state 'state.json') '{}'
    $null = Test-Status $e 'NOT_INITIALIZED' -Label 'only the monitor file'
}
Case 'P23' 'SETUP_LEFTOVER: a staging or superseded folder under the root, no anchor' {
    $e = New-Env 'p23'; New-Root $e; [IO.Directory]::Delete($e.state); New-Item -ItemType Directory -Path (Join-Path $e.root 'state.staging-op-1-1') | Out-Null
    $null = Test-Status $e 'SETUP_LEFTOVER' @('ROOT_HOLDS:state.staging-op-1-1')
    $e = New-Env 'p23b'; New-Root $e; [IO.Directory]::Delete($e.state); New-Item -ItemType Directory -Path (Join-Path $e.root 'state.superseded-op-2') | Out-Null
    $null = Test-Status $e 'SETUP_LEFTOVER' @('ROOT_HOLDS:state.superseded-op-2') -Label 'superseded'
    $e = New-Env 'p23c'; New-Root $e; [IO.Directory]::Delete($e.state); Write-Text (Join-Path $e.root $names.setup_lock) ''
    $null = Test-Status $e 'NOT_INITIALIZED' -Label 'setup lock only'
}
Case 'P24' 'INSTALLATION_LOST: the marker of a migration, no anchor, no state folder' {
    $e = New-Env 'p24'; Install-Migrated $e; $id = $e.id
    Remove-Tree $e.root; [Microsoft.Win32.Registry]::CurrentUser.DeleteSubKeyTree($e.key)
    $st = Test-Status $e 'INSTALLATION_LOST' @('MARKER_IN:*')
    Expect ($st.installation_id -ceq $id) "installation id $($st.installation_id), expected $id"
}
Case 'P25' 'MIGRATION_SOURCES_AMBIGUOUS: two earlier-version folders with evidence' {
    $e = New-Env 'p25'; New-Legacy $e.legacy; New-Legacy $e.legacy2
    $null = Test-Status $e 'MIGRATION_SOURCES_AMBIGUOUS' @('EVIDENCE_IN:*')
    $e = New-Env 'p25b'; New-Legacy $e.legacy2
    $null = Test-Status $e 'MIGRATION_REQUIRED' @() @{ TempPath = (Join-Path $e.lad 'Temp') } -Label 'TMP and the known folder name one folder'
}
Case 'P26' 'MIGRATION_REQUIRED: the laboratory shape, a lock only, a junction, an unlistable folder' {
    $e = New-Env 'p26a'; New-Legacy $e.legacy; $null = Test-Status $e 'MIGRATION_REQUIRED' @('EVIDENCE_IN:*') -Label 'laboratory shape'
    $e = New-Env 'p26b'; New-Item -ItemType Directory -Path $e.legacy | Out-Null; Write-Text (Join-Path $e.legacy 'coordinator.lock') ''
    $null = Test-Status $e 'MIGRATION_REQUIRED' -Label 'lock file only'
    $e = New-Env 'p26c'; New-Item -ItemType Directory -Path $e.legacy | Out-Null; Write-Text (Join-Path $e.legacy 'ACPI-VALIDATION-LEDGER.JSON.tmp') 'x'
    $null = Test-Status $e 'MIGRATION_REQUIRED' -Label 'unknown file'
    $e = New-Env 'p26d'; $tgt = Join-Path $e.base 'real-legacy'; New-Legacy $tgt; New-Item -ItemType Junction -Path $e.legacy -Target $tgt | Out-Null
    $null = Test-Status $e 'MIGRATION_REQUIRED' @('LEGACY_FOLDER_REPARSE_POINT') -Label 'junction'
    $e = New-Env 'p26e'; New-Legacy $e.legacy; $rule = Add-Rule $e.legacy $me 'ReadData' 'Deny'
    try { $null = Test-Status $e 'MIGRATION_REQUIRED' @('LEGACY_FOLDER_NOT_LISTABLE') -Label 'cannot be listed' } finally { Remove-Rule $e.legacy $rule }
}
Case 'P27' 'MIGRATION_REQUIRED, not absence: an earlier folder whose attributes cannot be read' {
    $e = New-Env 'p27'; New-Legacy $e.legacy
    $r1 = Add-Rule $e.legacy $me 'ReadAttributes, ReadData' 'Deny'; $r2 = Add-Rule $e.tmp $me 'ReadData' 'Deny'
    try {
        Expect ((Get-P9SStateEntry $e.legacy).kind -eq 'denied') "the probe says $((Get-P9SStateEntry $e.legacy).kind), not denied"
        $null = Test-Status $e 'MIGRATION_REQUIRED' @('LEGACY_FOLDER_NOT_ACCESSIBLE')
    } finally { Remove-Rule $e.tmp $r2; Remove-Rule $e.legacy $r1 }
}
Case 'P28' 'NOT_INITIALIZED: nothing; monitor files only; setup lock only' {
    $e = New-Env 'p28'; $null = Test-Status $e 'NOT_INITIALIZED' -Label 'nothing'
    New-Item -ItemType Directory -Path $e.legacy | Out-Null; Write-Text (Join-Path $e.legacy 'state.json') '{}'; Write-Text (Join-Path $e.legacy 'state.json.lock') ''
    Write-Text (Join-Path $e.legacy ".state.json.$([guid]::NewGuid().ToString('N')).tmp") '{'
    $null = Test-Status $e 'NOT_INITIALIZED' -Label 'monitor files'
}
Case 'P29' 'INTERRUPTED: anchor in phase INITIALIZING or MIGRATING' {
    $e = New-Env 'p29'; Write-Anchor $e (New-AnchorDoc $e (New-Id) 'INITIALIZING'); $null = Test-Status $e 'INTERRUPTED' @('ANCHOR_PHASE:INITIALIZING')
    New-Legacy $e.legacy; Write-Anchor $e (New-AnchorDoc $e (New-Id) 'MIGRATING' 'migrated' $null $e.legacy ('0' * 64)); $null = Test-Status $e 'INTERRUPTED' @('ANCHOR_PHASE:MIGRATING')
}
Case 'P30' 'ACCOUNT_MISMATCH: an anchor of another account' {
    $e = New-Env 'p30'; Install-New $e; $a = Get-Anchor $e; $a.account = 'S-1-5-21-0-0-0-1001'; Write-Anchor $e $a
    $null = Test-Status $e 'ACCOUNT_MISMATCH' @('ANCHOR_ACCOUNT')
}
Case 'P31' 'STATE_MISMATCH: another state folder, another installation, another origin' {
    $e = New-Env 'p31'; Install-New $e; $a = Get-Anchor $e; $a.state_dir = Join-Path $e.base 'other\state'; Write-Anchor $e $a
    $null = Test-Status $e 'STATE_MISMATCH' @('STATE_DIR') -Label 'state_dir'
    $e = New-Env 'p31b'; Install-New $e; Write-Doc (Join-Path $e.state $names.install) (New-InstallDoc $e (New-Id))
    $null = Test-Status $e 'STATE_MISMATCH' @('INSTALLATION_ID') -Label 'installation id'
    $e = New-Env 'p31c'; Install-New $e; Write-Doc (Join-Path $e.state $names.install) (New-InstallDoc $e $e.id 'migrated' (New-Id 'op'))
    $null = Test-Status $e 'STATE_MISMATCH' @('INSTALL_RECORD_DIFFERS:origin') -Label 'origin'
}
Case 'P32' 'STATE_MISSING: anchor READY, no state folder' {
    $e = New-Env 'p32'; Install-New $e; Remove-Tree $e.state
    $null = Test-Status $e 'STATE_MISSING' @('STATE_FOLDER_ABSENT')
}
Case 'P33' 'STATE_INCOMPLETE: install record absent, damaged, unreadable; migration record absent, inconsistent, unexpected' {
    $e = New-Env 'p33a'; Install-New $e; Remove-Tree (Join-Path $e.state $names.install)
    $null = Test-Status $e 'STATE_INCOMPLETE' @('INSTALL_RECORD_NOT_READ:absent') -Label 'absent'
    $e = New-Env 'p33b'; Install-New $e; Write-Text (Join-Path $e.state $names.install) '{"schema":'
    $null = Test-Status $e 'STATE_INCOMPLETE' @('INSTALL_RECORD_NOT_PARSEABLE') -Label 'cut'
    $e = New-Env 'p33c'; Install-New $e; $p = Join-Path $e.state $names.install; $rule = Add-Rule $p $me 'ReadData' 'Deny'
    try { $null = Test-Status $e 'STATE_INCOMPLETE' @('INSTALL_RECORD_NOT_READ:denied') -Label 'denied' } finally { Remove-Rule $p $rule }
    $e = New-Env 'p33d'; Install-Migrated $e; Remove-Tree (Join-Path $e.state $names.migration)
    $null = Test-Status $e 'STATE_INCOMPLETE' @('MIGRATION_RECORD_NOT_READ:absent') -Label 'migration record absent'
    $e = New-Env 'p33e'; Install-Migrated $e; $p = Join-Path $e.state $names.migration; $m = ConvertFrom-P9SStateJson ([IO.File]::ReadAllBytes($p))
    $m.files[0].length = [long]$m.files[0].length + 1; Write-Doc $p $m
    $null = Test-Status $e 'STATE_INCOMPLETE' @('MIGRATION_RECORD_INCONSISTENT') -Label 'a file entry no longer matches the digest'
    $e = New-Env 'p33f'; Install-New $e; Write-Text (Join-Path $e.state $names.migration) '{}'
    $null = Test-Status $e 'STATE_INCOMPLETE' @('MIGRATION_RECORD_UNEXPECTED') -Label 'new installation with a migration record'
}
Case 'P34' 'STATE_UNCERTAIN: ledger absent, older than the mark, other content at the mark, damaged, unreadable, older than its record' {
    $e = New-Env 'p34a'; Install-New $e; Remove-Tree (Get-LedgerPath $e), ([IO.Path]::ChangeExtension((Get-LedgerPath $e), '.revision.json'))
    $null = Test-Status $e 'STATE_UNCERTAIN' @('LEDGER:ACPI_LEDGER_MISSING') -Label 'ledger and revision record absent (the toolkit reader alone: never used)'
    $e = New-Env 'p34b'; Install-New $e; $a = Get-Anchor $e; $a.ledger.revision = 3; Write-Anchor $e $a
    $null = Test-Status $e 'STATE_UNCERTAIN' @('LEDGER:STATE_STALE') -Label 'older than the mark'
    $e = New-Env 'p34c'; Install-New $e; $a = Get-Anchor $e; $a.ledger.sha256 = 'f' * 64; Write-Anchor $e $a
    $null = Test-Status $e 'STATE_UNCERTAIN' @('LEDGER:ACPI_LEDGER_INCONSISTENT') -Label 'other content at the mark'
    $e = New-Env 'p34d'; Install-New $e; Write-Text (Get-LedgerPath $e) '{"schema":"p9s-acpi'
    $null = Test-Status $e 'STATE_UNCERTAIN' @('LEDGER:ACPI_LEDGER_*') -Label 'cut'
    $e = New-Env 'p34e'; Install-New $e; $p = Get-LedgerPath $e; $rule = Add-Rule $p $me 'ReadData' 'Deny'
    try { $null = Test-Status $e 'STATE_UNCERTAIN' @('LEDGER:ACPI_LEDGER_UNREADABLE') -Label 'denied' } finally { Remove-Rule $p $rule }
    $e = New-Env 'p34f'; Install-New $e; $p = Get-LedgerPath $e; $old = [IO.File]::ReadAllBytes($p); Save-Ledger $p; [IO.File]::WriteAllBytes($p, $old)
    $null = Test-Status $e 'STATE_UNCERTAIN' @('LEDGER:ACPI_LEDGER_STALE') -Label 'older than its revision record'
}
Case 'P35' 'STATE_DIVERGED: the earlier folder changed, unreadable, or not at a computed location' {
    $e = New-Env 'p35a'; Install-Migrated $e; Write-Text (Join-Path $e.legacy 'acpi-validation-ledger.json') ([IO.File]::ReadAllText((Join-Path $e.legacy 'acpi-validation-ledger.json')) + ' ')
    $null = Test-Status $e 'STATE_DIVERGED' @('OLD_FOLDER_CHANGED:acpi-validation-ledger.json') -Label 'ledger rewritten'
    $e = New-Env 'p35b'; Install-Migrated $e; Write-Text (Join-Path $e.legacy 'coordinator-stop.request') '{"acpi":{"P9S-D12":"x"},"unresolved":{}}'
    $null = Test-Status $e 'STATE_DIVERGED' @('OLD_FOLDER_CHANGED:coordinator-stop.request') -Label 'new stop request'
    $e = New-Env 'p35c'; Install-Migrated $e; $rule = Add-Rule $e.legacy $me 'ReadData' 'Deny'
    try { $null = Test-Status $e 'STATE_DIVERGED' @('OLD_FOLDER_NOT_VERIFIABLE') -Label 'cannot be listed' } finally { Remove-Rule $e.legacy $rule }
    $e = New-Env 'p35d'; Install-Migrated $e (Join-Path $e.base 'elsewhere\p9s-hostmon')
    $null = Test-Status $e 'STATE_DIVERGED' @('OLD_FOLDER_NOT_A_KNOWN_LOCATION') -Label 'recorded source not computed'
}
Case 'P36' 'READY: new; migrated with the earlier folder intact, gone or with a longer history; anchor behind the ledger' {
    $e = New-Env 'p36a'; Install-New $e; Write-Text (Join-Path $e.state 'state.json') '{}'; Write-Text (Join-Path $e.state ".x.$([guid]::NewGuid().ToString('N')).tmp") '{'
    $st = Test-Status $e 'READY' -Label 'new'
    Expect ($st.installation_id -ceq $e.id -and $st.origin -ceq 'new') "installation $($st.installation_id) origin $($st.origin)"
    $e = New-Env 'p36b'; Install-Migrated $e; $st = Test-Status $e 'READY' -Label 'migrated'
    Expect (-not @($st.notes).Count) "notes $(@($st.notes) -join ',')"
    $e = New-Env 'p36c'; Install-Migrated $e; Remove-Tree $e.legacy; $null = Test-Status $e 'READY' -Notes @('OLD_FOLDER_GONE') -Label 'earlier folder gone'
    $e = New-Env 'p36d'; Install-Migrated $e; Add-Line (Join-Path $e.legacy 'coordinator-audit.jsonl') @{ action = 'emergency.status' }
    $null = Test-Status $e 'READY' -Notes @('OLD_FOLDER_HISTORY_CHANGED') -Label 'history appended in the earlier folder'
    $e = New-Env 'p36e'; Install-New $e; Save-Ledger (Get-LedgerPath $e); $null = Test-Status $e 'READY' -Label 'anchor mark behind the ledger'
    $e = New-Env 'p36f'; Install-New $e; New-Item -ItemType Directory -Path (Join-Path $e.root 'state.staging-op-9-1') | Out-Null
    $null = Test-Status $e 'READY' -Notes @('ROOT_HOLDS:state.staging-op-9-1') -Label 'staging folder left'
}
Case 'P37' 'the 17 statuses are the ones of the design, and each case above produced one of them' {
    $want = 'SANDBOX', 'LOCATION_UNSAFE', 'ANCHOR_UNREADABLE', 'ANCHOR_MISSING', 'SETUP_LEFTOVER', 'INSTALLATION_LOST', 'MIGRATION_SOURCES_AMBIGUOUS', 'MIGRATION_REQUIRED', 'NOT_INITIALIZED',
        'INTERRUPTED', 'ACCOUNT_MISMATCH', 'STATE_MISMATCH', 'STATE_MISSING', 'STATE_INCOMPLETE', 'STATE_UNCERTAIN', 'STATE_DIVERGED', 'READY'
    $have = @(Get-P9SStateStatuses)
    Expect ($have.Count -eq 17 -and (($have -join ',') -ceq ($want -join ','))) "statuses: $($have -join ',')"
}

# ================================================================ precedence
Case 'P40' 'precedence: SANDBOX before LOCATION_UNSAFE' {
    $e = New-Env 'p40'; Install-New $e; $null = Add-Rule $e.state 'S-1-1-0' 'Write' 'Allow'
    $null = Test-Status $e 'SANDBOX' @() @{ AskedStatePath = (Join-Path $e.base 'x\state.json') }
}
Case 'P41' 'precedence: LOCATION_UNSAFE before ANCHOR_UNREADABLE' {
    $e = New-Env 'p41'; Install-New $e; Write-Anchor $e $null -Raw 'garbage'; $null = Add-Rule $e.state 'S-1-1-0' 'Write' 'Allow'
    $null = Test-Status $e 'LOCATION_UNSAFE' @('ACL_FOREIGN_WRITE')
}
Case 'P42' 'precedence: ANCHOR_UNREADABLE before everything that needs no anchor' {
    $e = New-Env 'p42'; Install-Migrated $e; New-Legacy $e.legacy2; Write-Anchor $e $null -Raw 'garbage'
    $null = Test-Status $e 'ANCHOR_UNREADABLE'
}
Case 'P43' 'precedence without anchor: ANCHOR_MISSING, SETUP_LEFTOVER, INSTALLATION_LOST, MIGRATION_SOURCES_AMBIGUOUS, MIGRATION_REQUIRED, NOT_INITIALIZED' {
    $e = New-Env 'p43'; Install-Migrated $e; New-Legacy $e.legacy2; [Microsoft.Win32.Registry]::CurrentUser.DeleteSubKeyTree($e.key)
    New-Item -ItemType Directory -Path (Join-Path $e.root 'state.staging-op-3-1') | Out-Null
    $null = Test-Status $e 'ANCHOR_MISSING' -Label 'all present'
    Remove-Tree $e.state; $null = Test-Status $e 'SETUP_LEFTOVER' -Label 'no state folder'
    Remove-Tree $e.root; $null = Test-Status $e 'INSTALLATION_LOST' -Label 'no root'
    Remove-Tree (Join-Path $e.legacy $names.marker); $null = Test-Status $e 'MIGRATION_SOURCES_AMBIGUOUS' -Label 'no marker'
    Remove-Tree $e.legacy2; $null = Test-Status $e 'MIGRATION_REQUIRED' -Label 'one earlier folder'
    Remove-Tree $e.legacy; $null = Test-Status $e 'NOT_INITIALIZED' -Label 'nothing left'
}
Case 'P44' 'precedence with an anchor: INTERRUPTED, ACCOUNT_MISMATCH, STATE_MISMATCH, STATE_MISSING' {
    $e = New-Env 'p44'; Install-New $e; $a = Get-Anchor $e
    $b = [ordered]@{}; foreach ($k in $a.Keys) { $b[$k] = $a[$k] }; $b.phase = 'INITIALIZING'; $b.ledger = $null; $b.initialized_utc = $null; $b.account = 'S-1-5-21-0-0-0-1001'; $b.state_dir = Join-Path $e.base 'other'
    Write-Anchor $e $b; Remove-Tree $e.state
    $null = Test-Status $e 'INTERRUPTED' -Label 'interrupted, other account, other folder, no folder'
    $a.account = 'S-1-5-21-0-0-0-1001'; $a.state_dir = Join-Path $e.base 'other'; Write-Anchor $e $a
    $null = Test-Status $e 'ACCOUNT_MISMATCH' -Label 'other account, other folder, no folder'
    $a.account = $me; Write-Anchor $e $a; $null = Test-Status $e 'STATE_MISMATCH' @('STATE_DIR') -Label 'other folder, no folder'
    $a.state_dir = $e.state; Write-Anchor $e $a; $null = Test-Status $e 'STATE_MISSING' -Label 'no folder'
}
Case 'P45' 'precedence inside the folder: STATE_INCOMPLETE (install), STATE_MISMATCH, STATE_UNCERTAIN, STATE_INCOMPLETE (migration), STATE_DIVERGED' {
    $e = New-Env 'p45'; Install-Migrated $e
    Write-Text (Join-Path $e.legacy 'coordinator-stop.request') '{}'
    Remove-Tree (Join-Path $e.state $names.migration)
    $lp = Get-LedgerPath $e; $lbytes = [IO.File]::ReadAllBytes($lp); Remove-Tree $lp
    $ip = Join-Path $e.state $names.install; $ibytes = [IO.File]::ReadAllBytes($ip); Write-Doc $ip (New-InstallDoc $e (New-Id) 'migrated' $e.mid)
    $null = Test-Status $e 'STATE_MISMATCH' @('INSTALLATION_ID') -Label 'other installation id, no ledger, no migration record, old folder changed'
    Remove-Tree $ip; $null = Test-Status $e 'STATE_INCOMPLETE' @('INSTALL_RECORD_*') -Label 'no install record'
    [IO.File]::WriteAllBytes($ip, $ibytes); $null = Test-Status $e 'STATE_UNCERTAIN' -Label 'no ledger'
    [IO.File]::WriteAllBytes($lp, $lbytes); $null = Test-Status $e 'STATE_INCOMPLETE' @('MIGRATION_RECORD_*') -Label 'no migration record'
}

# ================================================================ side effects
Case 'P50' 'concurrent classifications by eight processes change nothing' {
    $e1 = New-Env 'p50a'; Install-Migrated $e1; $e2 = New-Env 'p50b'; New-Legacy $e2.legacy
    $s1 = Get-EnvSnap $e1; $s2 = Get-EnvSnap $e2
    $go = "Local\p9s-persist-go-$run"; $ev = [Threading.EventWaitHandle]::new($false, [Threading.EventResetMode]::ManualReset, $go)
    $wd = Join-Path $OutDir 'p50'; New-Item -ItemType Directory -Path $wd | Out-Null
    $procs = foreach ($i in 1..8) {
        $e = if ($i -le 6) { $e1 } else { $e2 }
        $code = "$libs; `$c = New-P9SStateContext -LocalAppData $(Q $e.lad) -TempPath $(Q $e.tmp) -AnchorKey $(Q $e.key); [IO.File]::WriteAllText($(Q (Join-Path $wd "ready-$i")), 'x'); `$null = [Threading.EventWaitHandle]::OpenExisting($(Q $go)).WaitOne(60000); `$o = foreach (`$n in 1..15) { (Get-P9SStateStatus `$c).status }; `$o -join ','"
        Start-Process -FilePath $pw -PassThru -WindowStyle Hidden -RedirectStandardOutput (Join-Path $wd "out-$i.txt") -RedirectStandardError (Join-Path $wd "err-$i.txt") -ArgumentList @('-NoProfile', '-NonInteractive', '-EncodedCommand', (ConvertTo-Encoded $code))
    }
    $sw = [Diagnostics.Stopwatch]::StartNew(); while (@(1..8 | Where-Object { -not (Test-Path -LiteralPath (Join-Path $wd "ready-$_")) }).Count -and $sw.Elapsed.TotalSeconds -lt 90) { Start-Sleep -Milliseconds 50 }
    $null = $ev.Set(); foreach ($p in $procs) { $null = $p.WaitForExit(120000) }; $ev.Dispose()
    foreach ($i in 1..8) {
        $o = (Get-Content -Raw -LiteralPath (Join-Path $wd "out-$i.txt")).Trim(); $want = if ($i -le 6) { 'READY' } else { 'MIGRATION_REQUIRED' }
        Expect ($o -ceq ((@($want) * 15) -join ',')) "process $i returned $o"
        $err = Get-Content -Raw -LiteralPath (Join-Path $wd "err-$i.txt"); Expect (-not ($err -and $err.Trim())) "process $i wrote errors: $err"
    }
    Expect ($s1 -ceq (Get-EnvSnap $e1)) "migrated state changed: $(Get-SnapDiff $s1 (Get-EnvSnap $e1))"
    Expect ($s2 -ceq (Get-EnvSnap $e2)) "earlier folder changed: $(Get-SnapDiff $s2 (Get-EnvSnap $e2))"
}

# ================================================================ the entry point
$cli = Join-Path $kit 'p9s-action.ps1'
Case 'P60' 'state.status with a -StatePath elsewhere: SANDBOX, exit 0, nothing created' {
    $e = New-Env 'p60'; $before = Get-EnvSnap $e
    $o = @(& $pw -NoProfile -NonInteractive -File $cli -Action state.status -StatePath (Join-Path $e.base 'x\state.json') 2>&1 | ForEach-Object { "$_" }); $code = $LASTEXITCODE
    Expect ($code -eq 0 -and ($o -ccontains 'status: SANDBOX') -and ($o -ccontains 'outcome: DONE')) "exit $code; $($o -join ' / ')"
    Expect ($before -ceq (Get-EnvSnap $e)) 'something changed'
    Expect (-not (Test-Path -LiteralPath (Join-Path $e.base 'x'))) 'the -StatePath folder was created'
}
Case 'P61' 'state.status with another parameter: refused, exit 1, nothing read' {
    $before = Get-RealSnap
    $o = @(& $pw -NoProfile -NonInteractive -File $cli -Action state.status -Vm P9S-D12 2>&1 | ForEach-Object { "$_" }); $code = $LASTEXITCODE
    Expect ($code -eq 1 -and ($o -ccontains 'outcome: REFUSED; STATE_STATUS_PARAMETER')) "exit $code; $($o -join ' / ')"
    Expect ($before -ceq (Get-RealSnap)) 'real locations changed'
}
$p62 = "state.status on this machine's real locations: NOT_INITIALIZED, nothing created"
# only on a clean runner, in the main run of run-all.ps1 -CleanRunner: the
# real locations are not even looked at elsewhere
if (-not $env:P9S_TEST_REAL_LOCATIONS) { NotRun 'P62' $p62 'runs only in the main run of run-all.ps1 -CleanRunner (a clean runner)' }
else {
$realLegacy = @((Join-Path ([IO.Path]::GetTempPath()) $names.legacy), (Join-Path (Join-Path $realLad 'Temp') $names.legacy))
$realBusy = @(@($realLegacy | Where-Object { Test-Path -LiteralPath $_ }) + @(if ($realAtStart -ne 'real root False; real anchor key False') { 'state root or anchor' }))
if ($realBusy.Count) { NotRun 'P62' $p62 "the real state root, the anchor key or an earlier-version folder exists here ($($realBusy.Count) paths)" }
else {
Case 'P62' $p62 {
    $o = @(& $pw -NoProfile -NonInteractive -File $cli -Action state.status 2>&1 | ForEach-Object { "$_" }); $code = $LASTEXITCODE
    Expect ($code -eq 0 -and ($o -ccontains 'status: NOT_INITIALIZED') -and ($o -ccontains 'outcome: DONE')) "exit $code; $($o -join ' / ')"
    Expect ((Get-RealSnap) -ceq 'real root False; real anchor key False') "after: $(Get-RealSnap)"
    foreach ($l in $realLegacy) { Expect (-not (Test-Path -LiteralPath $l)) "created $l" }
}
}
}

# ================================================================ end
Case 'P98' 'the real state root and anchor key are as they were at the start' {
    Expect ((Get-RealSnap) -ceq $realAtStart) "start: $realAtStart; now: $(Get-RealSnap)"
}
Case 'P99' 'the test keys and synthetic folders are removed' {
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
