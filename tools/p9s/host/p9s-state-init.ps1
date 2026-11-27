#Requires -Version 7.0
# P9-S persistent state: the explicit initialization of a new installation
# (state.init, state.init -Supersede <installation_id>) and the rule that
# voids every ACPI validation authorization issued before it.
#
# Only the action state.init calls Invoke-P9SStateInit: no other action
# initializes, repairs or recreates state, and an absent or empty folder is
# never taken as permission to initialize. The decision is the status of
# p9s-state.ps1 (read only, unchanged), computed before anything is written
# and again under the setup lock:
#   NOT_INITIALIZED          a new installation;
#   INTERRUPTED (anchor in phase INITIALIZING)
#                            the same initialization, resumed;
#   INSTALLATION_LOST, STATE_MISSING, STATE_INCOMPLETE, STATE_MISMATCH,
#   STATE_UNCERTAIN, STATE_DIVERGED
#                            only with -Supersede naming the installation
#                            the evidence names;
#   anything else            refused (an earlier installation, an earlier
#                            coordinator folder, an unreadable anchor, an
#                            unsafe location, leftovers of a setup).
# Every configured VM must be observed off and no VirtualBox VM process may
# run: the process count is checked before anything is written (no
# VirtualBox call), the VM states through the VirtualBox layer once the
# staging folder exists, with the layer's records and the observations in
# it, so that they become the first history of the new installation. A
# VirtualBox operation or quarantine record left in any folder under the
# root, or in an earlier coordinator folder, refuses every later run until
# the operator resolves it.
# Steps (each a crash point; the anchor is flushed after each write):
#   P0  the decision and the checks; nothing is written before this point
#   I1  root (protected ACL at creation) and the setup lock, no wait
#   I2a (supersede) the anchor value being replaced copied to Superseded-<op>
#   I2  anchor: phase INITIALIZING, a new installation id
#   I4  staging folder state.staging-<op>-<n> (protected ACL at creation)
#   V   VM states, observed into the staging folder
#   I3  (supersede) the state folder renamed to state.superseded-<op>, once
#       the VMs are confirmed off
#   I5  install.json (exclusive create, write-through)
#   I6  the empty ledger at revision 1 and its revision record (the
#       toolkit's save; each file written beside its name and renamed with
#       write-through)
#   I7  the ACL of the staging folder and its files checked
#   I8  the staging folder renamed to state (refused if state exists)
#   I9  the folder checked, the anchor checked unchanged, then: phase READY,
#       initialized_utc, the ledger mark
#   I10 one line in the new folder's coordinator-audit.jsonl
# Nothing is ever deleted: a replaced state folder is renamed aside, a
# staging folder left by an interruption stays where it is, a replaced
# anchor value is kept beside it. The coordinator keeps its files where it
# always did (%TEMP%\p9s-hostmon): no action of this version uses the new
# folder. This file writes nothing there; it only looks for the VirtualBox
# layer's records in it.
# Dot-source after hostmon-lib.ps1, coordinator-lib.ps1 and p9s-state.ps1.

# ------------------------------------------------------------ native calls
# CreateDirectoryW fails when the path exists (a folder, a file or a
# junction put there in advance) and applies the protected ACL at creation;
# MoveFileExW without MOVEFILE_REPLACE_EXISTING fails when the target
# exists, and MOVEFILE_WRITE_THROUGH returns once the rename is on disk.
$script:P9SSetupNativeSource = @'
using System;
using System.Runtime.InteropServices;
using Microsoft.Win32.SafeHandles;
public static class P9SSetupNative {
    [StructLayout(LayoutKind.Sequential)]
    private struct SecurityAttributes { public int Length; public IntPtr Descriptor; public int Inherit; }
    [DllImport("advapi32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern bool ConvertStringSecurityDescriptorToSecurityDescriptorW(string sddl, uint revision, out IntPtr descriptor, out uint size);
    [DllImport("kernel32.dll")]
    private static extern IntPtr LocalFree(IntPtr p);
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern bool CreateDirectoryW(string path, ref SecurityAttributes sa);
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern bool MoveFileExW(string from, string to, uint flags);
    public static int CreateDirectory(string path, string sddl) {
        IntPtr sd; uint n;
        if (!ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl, 1, out sd, out n)) { return Marshal.GetLastWin32Error(); }
        try {
            SecurityAttributes sa = new SecurityAttributes();
            sa.Length = Marshal.SizeOf(typeof(SecurityAttributes)); sa.Descriptor = sd; sa.Inherit = 0;
            return CreateDirectoryW(Long(path), ref sa) ? 0 : Marshal.GetLastWin32Error();
        } finally { LocalFree(sd); }
    }
    public static int Move(string from, string to) { return MoveFileExW(Long(from), Long(to), 8) ? 0 : Marshal.GetLastWin32Error(); }
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern SafeFileHandle CreateFileW(string name, uint access, uint share, IntPtr security, uint disposition, uint flags, IntPtr template);
    [StructLayout(LayoutKind.Sequential)]
    private struct HandleInfo { public uint Attributes, C1, C2, A1, A2, W1, W2, Volume, SizeHigh, SizeLow, Links, IndexHigh, IndexLow; }
    [DllImport("kernel32.dll", SetLastError = true)]
    private static extern bool GetFileInformationByHandle(SafeFileHandle handle, out HandleInfo info);
    public sealed class Entry { public int Error; public uint Attributes; public uint Volume; public ulong Index; public uint Links; }
    // opened for its attributes only, never created, the last component not followed
    public static Entry Inspect(string path) {
        Entry e = new Entry();
        using (SafeFileHandle h = CreateFileW(Long(path), 0x80, 7, IntPtr.Zero, 3, 0x02200000, IntPtr.Zero)) {
            if (h.IsInvalid) { e.Error = Marshal.GetLastWin32Error(); return e; }
            HandleInfo i;
            if (!GetFileInformationByHandle(h, out i)) { e.Error = Marshal.GetLastWin32Error(); return e; }
            e.Attributes = i.Attributes; e.Volume = i.Volume; e.Index = ((ulong)i.IndexHigh << 32) | i.IndexLow; e.Links = i.Links;
        }
        return e;
    }
    // full paths only; the prefix lifts the limit of 260 characters
    private static string Long(string p) {
        if (p.StartsWith(@"\\?\")) { return p; }
        if (p.StartsWith(@"\\")) { return @"\\?\UNC\" + p.Substring(2); }
        return @"\\?\" + p;
    }
}
'@
function Initialize-P9SSetupNative {
    if ('P9SSetupNative' -as [type]) { return $true }
    try { Add-Type -TypeDefinition $script:P9SSetupNativeSource -ErrorAction Stop; return $true } catch { return $false }
}
# What is at a path, as Get-P9SStateEntry of p9s-state.ps1 says it
# (absent, file, directory, denied, error; only "not found" is absent), for
# paths of any length.
function Get-P9SSetupEntry([string]$Path) {
    $e = [P9SSetupNative]::Inspect($Path)
    $kind = switch ($e.Error) { 0 { if ($e.Attributes -band 0x10) { 'directory' } else { 'file' } } 2 { 'absent' } 3 { 'absent' } 5 { 'denied' } default { 'error' } }
    [pscustomobject]@{ path = $Path; kind = $kind; error = $e.Error; reparse = [bool]($e.Error -eq 0 -and ($e.Attributes -band 0x400)); links = [int]$e.Links; volume = $e.Volume; index = $e.Index }
}
# Full control for the account, SYSTEM and Administrators, inherited by
# everything below, nothing inherited from above.
function Get-P9SStateProtectedSddl([string]$Account) { "D:PAI(A;OICI;FA;;;$Account)(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)" }

# ------------------------------------------------------------ names
function Get-P9SStateInitNames {
    [ordered]@{ staging = 'state.staging-'; superseded = 'state.superseded-'; kept_anchor = 'Superseded-'; purpose = 'state.init' }
}
function Get-P9SStateSupersedable { 'INSTALLATION_LOST', 'STATE_MISSING', 'STATE_INCOMPLETE', 'STATE_MISMATCH', 'STATE_UNCERTAIN', 'STATE_DIVERGED' }
function New-P9SStateOpId([datetime]$Utc) { 'init-' + $Utc.ToString("yyyyMMdd'T'HHmmss'Z'", [Globalization.CultureInfo]::InvariantCulture) + '-' + [guid]::NewGuid().ToString('N').Substring(0, 8) }
function New-P9SStateInstallationId { 'inst-' + [guid]::NewGuid().ToString('N') }
function ConvertTo-P9SStateUtcText([datetime]$Utc) { $Utc.ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ss.fffffffZ', [Globalization.CultureInfo]::InvariantCulture) }
# a time of a state record as an instant (UTC), or $null
function ConvertFrom-P9SStateUtcText($V) {
    if ($V -isnot [string] -or $V -cnotmatch '^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(\.\d{1,7})?Z$') { return $null }
    $d = [datetime]::MinValue
    if ([datetime]::TryParse($V, [Globalization.CultureInfo]::InvariantCulture, [Globalization.DateTimeStyles]::AdjustToUniversal -bor [Globalization.DateTimeStyles]::AssumeUniversal, [ref]$d)) { return $d }
    $null
}

# ------------------------------------------------------------ context
# The production context reads VM states through the VirtualBox layer with
# its records in the folder given (the staging folder, or the state folder
# when a resumed initialization finds it in place) and counts VirtualBox VM
# processes from the Windows process list. Tests inject every fact; the
# command line offers none of them.
function New-P9SStateInitContext {
    param([Parameter(Mandatory)]$State, [Parameter(Mandatory)][scriptblock]$ReadVmStateIn, [Parameter(Mandatory)][scriptblock]$CountVmProcesses,
          [string[]]$VmNames, [scriptblock]$UtcNow, [scriptblock]$NowQpc, [long]$QpcFrequency = 0, [scriptblock]$OnStep)
    [pscustomobject]@{
        State = $State; ReadVmStateIn = $ReadVmStateIn; CountVmProcesses = $CountVmProcesses
        VmNames = @($(if ($VmNames) { $VmNames } else { Get-P9SVmNames }))
        UtcNow = $(if ($UtcNow) { $UtcNow } else { { [DateTime]::UtcNow } }); NowQpc = $(if ($NowQpc) { $NowQpc } else { { [Diagnostics.Stopwatch]::GetTimestamp() } })
        QpcFrequency = $(if ($QpcFrequency -gt 0) { $QpcFrequency } else { [Diagnostics.Stopwatch]::Frequency })
        OnStep = $OnStep; Trace = [System.Collections.Generic.List[string]]::new()
    }
}
function New-P9SStateInitRealContext($State) {
    New-P9SStateInitContext -State $State -VmNames @(Get-P9SVmNames) `
        -ReadVmStateIn { param($vm, $dir) Get-P9SVmStateReal $vm (New-P9SVBoxContext -Dir $dir -VmNames @(Get-P9SVmNames)) } `
        -CountVmProcesses { @(Get-Process -Name 'VirtualBoxVM', 'VBoxHeadless' -ErrorAction SilentlyContinue).Count }
}
function Invoke-P9SStateInitStep($I, [string]$Step) { $I.Trace.Add($Step); if ($I.OnStep) { & $I.OnStep $Step } }
function Get-P9SStateInitNow($I) { [datetime](& $I.UtcNow) }

# ------------------------------------------------------------ facts read before a decision
# The anchor value as stored (for the copy kept before it is replaced).
function Read-P9SStateAnchorRaw($C) {
    $k = $null
    try { $k = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey($C.AnchorKey, $false) } catch { return $null }
    if (-not $k) { return $null }
    try { if ($k.GetValueKind($C.AnchorValue) -ne [Microsoft.Win32.RegistryValueKind]::String) { return $null }; [string]$k.GetValue($C.AnchorValue, $null, [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames) }
    catch { $null } finally { $k.Dispose() }
}
# A VirtualBox operation or quarantine record in any folder directly under
# the root (the state folder, staging and superseded folders) or in an
# earlier coordinator folder (the one this version's coordinator uses): the
# layer's records of another run, which a new folder would not see.
# Anything but "absent" counts, an access error included; nothing is read.
function Get-P9SStateInitVBoxReasons($C) {
    $out = [System.Collections.Generic.List[string]]::new()
    $dirs = [System.Collections.Generic.List[object]]::new()
    foreach ($d in @($C.LegacyDirs)) { $dirs.Add([pscustomobject]@{ path = $d; label = $d }) }
    $e = Get-P9SSetupEntry $C.Root
    if ($e.kind -eq 'directory') {
        $l = Get-P9SStateListing $C.Root
        if (-not $l.ok) { $out.Add('ROOT_NOT_LISTABLE') }
        foreach ($i in @($l.items | Where-Object { $_ -is [IO.DirectoryInfo] })) { $dirs.Add([pscustomobject]@{ path = $i.FullName; label = $i.Name }) }
    } elseif ($e.kind -ne 'absent') { $out.Add('ROOT_NOT_VERIFIABLE') }
    foreach ($d in $dirs) {
        foreach ($n in 'vbox-operation.json', 'vbox-quarantine.json') {
            # the tombstone a migration left in an earlier coordinator folder
            # is no VirtualBox operation (p9s-state.ps1)
            if ($n -ceq 'vbox-quarantine.json' -and @($C.LegacyDirs) -ccontains $d.path -and (Read-P9SStateTombstone (Join-Path $d.path $n)).kind -ceq 'tombstone') { continue }
            $x = Get-P9SSetupEntry (Join-Path $d.path $n)
            if ($x.kind -ne 'absent') { $out.Add("SETUP_VBOX_RECORD:$($d.label)\$n") }
        }
    }
    @($out)
}
# The process count: an unknown count refuses, like a running VM.
function Get-P9SStateInitProcessReasons($I) {
    $n = $null
    try { $n = & $I.CountVmProcesses } catch { $n = $null }
    if ($n -isnot [int]) { return @('VM_PROCESSES_UNKNOWN') }
    if ($n -gt 0) { return @("VM_PROCESS_RUNNING:$n") }
    @()
}
# The times the evidence holds; the clock must be later than each of them,
# or a voided permit could look newer than the new installation.
function Get-P9SStateInitEvidenceTimes($C, $AnchorDoc) {
    $t = [System.Collections.Generic.List[datetime]]::new()
    if ($AnchorDoc) { foreach ($k in 'created_utc', 'initialized_utc') { $x = ConvertFrom-P9SStateUtcText $AnchorDoc[$k]; if ($x) { $t.Add($x) } } }
    $ir = Read-P9SStateRecord (Join-Path $C.StateDir (Get-P9SStateNames).install) 'install'
    if ($ir.ok) { $x = ConvertFrom-P9SStateUtcText $ir.doc.created_utc; if ($x) { $t.Add($x) } }
    @($t)
}

# ------------------------------------------------------------ the decision
# What state.init may do with what exists, from the status alone; nothing is
# written. mode: new, resume, supersede, or $null (refused, with reasons).
function Get-P9SStateInitDecision($I, [string]$Supersede) {
    $C = $I.State
    $st = Get-P9SStateStatus $C
    $a = if ($C.Native) { Read-P9SStateAnchor $C } else { $null }
    $d = [pscustomobject]@{ status = $st.status; status_reasons = @($st.reasons); installation_id = $st.installation_id; mode = $null; reasons = [System.Collections.Generic.List[string]]::new()
        anchor = $(if ($a -and $a.ok) { $a.doc } else { $null }); exit = $null }
    $s = [string]$st.status
    if ($Supersede) {
        if ($s -cin (Get-P9SStateSupersedable)) {
            if (-not $st.installation_id -or $Supersede -cne [string]$st.installation_id) { $d.reasons.Add('SUPERSEDE_ID_MISMATCH') } else { $d.mode = 'supersede' }
        }
        elseif ($s -ceq 'READY' -or $s -ceq 'NOT_INITIALIZED') { $d.reasons.Add("NOTHING_TO_SUPERSEDE:$s") }
        elseif ($s -ceq 'INTERRUPTED') { $d.reasons.Add('SUPERSEDE_NOT_APPLICABLE:INTERRUPTED'); $d.exit = if ($d.anchor -and [string]$d.anchor.phase -ceq 'MIGRATING') { 'state.migrate completes the interrupted migration' } else { 'state.init without -Supersede completes the interrupted initialization' } }
        else { $d.reasons.Add("NOT_SUPERSEDABLE:$s") }
    } else {
        if ($s -ceq 'NOT_INITIALIZED') { $d.mode = 'new' }
        elseif ($s -ceq 'INTERRUPTED' -and $d.anchor -and [string]$d.anchor.phase -ceq 'INITIALIZING' -and [string]$d.anchor.origin -ceq 'new') { $d.mode = 'resume' }
        else {
            $d.reasons.Add("STATE_NOT_NEW:$s")
            $d.exit = switch -CaseSensitive ($s) {
                'MIGRATION_REQUIRED' { 'state.migrate copies the earlier coordinator folder into a new installation; state.init never starts over it' }
                'INTERRUPTED' { 'state.migrate completes the interrupted migration' }
                { $_ -cin (Get-P9SStateSupersedable) } { "state.init -Supersede $($st.installation_id) starts a new installation and keeps this one aside" }
                'READY' { 'the installation is in place; nothing to do' }
                default { 'see state.status; nothing is chosen automatically' }
            }
        }
    }
    $d
}

# ------------------------------------------------------------ writers
function ConvertTo-P9SStateAnchorText($Doc) { $Doc | ConvertTo-Json -Compress -Depth 5 }
function Write-P9SStateRegistryValue($C, [string]$Name, [string]$Text) {
    $k = [Microsoft.Win32.Registry]::CurrentUser.CreateSubKey($C.AnchorKey, $true)
    if (-not $k) { throw 'anchor key not opened' }
    try { $k.SetValue($Name, $Text, [Microsoft.Win32.RegistryValueKind]::String); $k.Flush() } finally { $k.Dispose() }
}
# One write of the whole anchor, flushed, then read back with the strict
# reader of p9s-state.ps1.
function Write-P9SStateAnchor($C, $Doc) {
    $text = ConvertTo-P9SStateAnchorText $Doc
    Write-P9SStateRegistryValue $C $C.AnchorValue $text
    $back = Read-P9SStateAnchor $C
    if (-not $back.ok -or (ConvertTo-P9SStateAnchorText $back.doc) -cne $text) { throw 'anchor not read back as written' }
}
# A new file, never an existing one, written through to the disk.
function Write-P9SStateNewFile([string]$Path, [byte[]]$Bytes) {
    $fs = [IO.FileStream]::new($Path, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None, 4096, [IO.FileOptions]::WriteThrough)
    try { $fs.Write($Bytes, 0, $Bytes.Length); $fs.Flush($true) } finally { $fs.Dispose() }
}
# The atomic write of the staging folder: a temporary file beside the name
# (the toolkit's pattern, ignored by every reader), then a rename with
# write-through that never replaces.
function Write-P9SStateInitAtomic($I, [string]$Path, [string]$Text, [string]$Step) {
    $full = [IO.Path]::GetFullPath($Path)
    $tmp = [IO.Path]::Combine([IO.Path]::GetDirectoryName($full), '.' + [IO.Path]::GetFileName($full) + '.' + [guid]::NewGuid().ToString('N') + '.tmp')
    Write-P9SStateNewFile $tmp ([Text.UTF8Encoding]::new($false).GetBytes($Text))
    Invoke-P9SStateInitStep $I "$Step-tmp"
    $e = Move-P9SStateEntry $tmp $full
    if ($e) { throw "rename of $([IO.Path]::GetFileName($full)) failed ($e)" }
    Invoke-P9SStateInitStep $I $Step
}
function Add-P9SStateInitAudit([string]$Dir, $Entry) {
    $bytes = [Text.UTF8Encoding]::new($false).GetBytes(($Entry | ConvertTo-Json -Compress -Depth 5) + "`n")
    $fs = [IO.FileStream]::new((Join-Path $Dir 'coordinator-audit.jsonl'), [IO.FileMode]::Append, [IO.FileAccess]::Write, [IO.FileShare]::ReadWrite, 4096, [IO.FileOptions]::WriteThrough)
    try { $fs.Write($bytes, 0, $bytes.Length); $fs.Flush($true) } finally { $fs.Dispose() }
}
# A folder created here, never one that existed: CreateDirectoryW with the
# protected ACL; then it must be a plain folder.
function New-P9SStateFolder([string]$Path, [string]$Account) {
    $e = [P9SSetupNative]::CreateDirectory($Path, (Get-P9SStateProtectedSddl $Account))
    if ($e -eq 183) { return 'EXISTS' }
    if ($e) { return "ERROR_$e" }
    $x = Get-P9SSetupEntry $Path
    if ($x.kind -ne 'directory' -or $x.reparse) { return 'NOT_A_PLAIN_FOLDER' }
    $null
}
# A rename that never replaces. Access denied and sharing violations (a
# scanner holding a file just written) are tried again, 25 times 10 ms
# apart, as Write-StateAtomic does; "exists" never is. 0, or the last error.
function Move-P9SStateEntry([string]$From, [string]$To) {
    $e = 0
    for ($i = 0; $i -lt 25; $i++) {
        $e = [P9SSetupNative]::Move($From, $To)
        if ($e -notin 5, 32) { return $e }
        Start-Sleep -Milliseconds 10
    }
    $e
}
function Test-P9SStateExistsError([int]$E) { $E -in 80, 183 }
function Get-P9SStateIdentity([string]$Path) { $x = Get-P9SSetupEntry $Path; if ($x.kind -eq 'directory') { "$($x.volume):$($x.index)" } else { $null } }

# ------------------------------------------------------------ the VM check
# Every configured VM observed off through the reader of the context, with
# the observations appended to the folder's vm-observations.jsonl, and the
# process count before and after the queries. Purpose: the action that
# observes (state.init, or state.migrate in p9s-state-migrate.ps1).
function Test-P9SStateInitVms($I, [string]$Dir, [string]$Purpose = (Get-P9SStateInitNames).purpose) {
    $out = [System.Collections.Generic.List[string]]::new(); $seen = [ordered]@{}
    foreach ($x in @(Get-P9SStateInitProcessReasons $I)) { $out.Add($x) }
    if ($out.Count) { return [pscustomobject]@{ reasons = @($out); states = $seen } }
    $read = $I.ReadVmStateIn
    $oc = [pscustomobject]@{ ReadVmState = { param($vm) & $read $vm $Dir }.GetNewClosure(); NowQpc = $I.NowQpc; QpcFrequency = $I.QpcFrequency; UtcNow = $I.UtcNow
        ObservationPath = Join-Path $Dir 'vm-observations.jsonl' }
    foreach ($vm in $I.VmNames) {
        $o = Get-P9SVmObservation $oc $vm $Purpose
        $seen[$vm] = [string]$o.state
        Invoke-P9SStateInitStep $I "V:$vm"
        if (-not $o.observed -or [string]$o.state -cne 'off') { $out.Add("VM_NOT_OFF:$($vm):$($o.state)") }
    }
    foreach ($x in @(Get-P9SStateInitProcessReasons $I)) { $out.Add($x) }
    # each observation is in the folder: the new history starts with them
    $lines = @()
    $r = Read-StateFile -Path $oc.ObservationPath
    if ($r.ok) { $lines = @($r.text -split "`n" | Where-Object { $_ } | ForEach-Object { try { $_ | ConvertFrom-Json } catch {} } | Where-Object { $_.pid -eq $PID -and $_.purpose -ceq $Purpose }) }
    foreach ($vm in $I.VmNames) { if (-not @($lines | Where-Object { $_.vm -ceq $vm }).Count) { $out.Add("VM_OBSERVATION_NOT_RECORDED:$vm") } }
    [pscustomobject]@{ reasons = @($out | Select-Object -Unique); states = $seen }
}

# ------------------------------------------------------------ the folder check before READY
# The state folder in place must be exactly what this initialization wrote:
# its install record, the empty ledger at revision 1 with its revision
# record, no other record, and a location the status accepts.
function Get-P9SStateInitFolderCheck($I, $A) {
    $C = $I.State; $n = Get-P9SStateNames
    $out = [System.Collections.Generic.List[string]]::new()
    $r = [pscustomobject]@{ reasons = $out; mark = $null }
    $e = Get-P9SSetupEntry $C.StateDir
    if ($e.kind -ne 'directory' -or $e.reparse) { $out.Add("STATE_FOLDER_$($e.kind.ToUpperInvariant())"); return $r }
    foreach ($x in @(Get-P9SStateLocationReasons $C)) { $out.Add("LOCATION:$x") }
    $ir = Read-P9SStateRecord (Join-Path $C.StateDir $n.install) 'install'
    if (-not $ir.ok) { foreach ($x in $ir.reasons) { $out.Add("INSTALL_RECORD_$x") }; return $r }
    $D = $ir.doc
    foreach ($k in 'installation_id', 'account', 'origin', 'supersedes', 'created_utc') { if ([string]$D[$k] -cne [string]$A[$k]) { $out.Add("INSTALL_RECORD_DIFFERS:$k") } }
    if ($null -ne $D.migration_id) { $out.Add('INSTALL_RECORD_DIFFERS:migration_id') }
    if (-not (Test-P9SStateSamePath $D.state_dir $C.StateDir)) { $out.Add('INSTALL_RECORD_DIFFERS:state_dir') }
    $lc = [pscustomobject]@{ LedgerPath = Join-Path $C.StateDir 'acpi-validation-ledger.json'; SessionPath = Join-Path $C.StateDir 'coordinator-session.json'; StopRequestPath = Join-Path $C.StateDir 'coordinator-stop.request' }
    $rv = Read-P9SAcpiLedgerRevision $lc
    $lg = Read-P9SAcpiLedger $lc
    if (-not $rv.present -or -not $rv.ok -or [long]$rv.revision -ne 1) { $out.Add('LEDGER_REVISION_RECORD') }
    if (-not $lg.ok -or [string]$lg.detail -cne 'revision 1' -or @($lg.doc.entries).Count) { $out.Add("LEDGER:$(if ($lg.ok) { $lg.detail } else { $lg.reason })") }
    $b = Read-P9SStateBytes $lc.LedgerPath
    if ($b.kind -ne 'ok') { $out.Add("LEDGER_NOT_READ:$($b.kind)") } else { $r.mark = [ordered]@{ revision = 1L; sha256 = (Get-P9SStateSha $b.bytes) } }
    $cls = Get-P9SStateFileClasses
    foreach ($x in @($cls.record)) {
        if ($x -cin 'acpi-validation-ledger.json', 'acpi-validation-ledger.revision.json') { continue }
        if ((Get-P9SSetupEntry (Join-Path $C.StateDir $x)).kind -ne 'absent') { $out.Add("UNEXPECTED_RECORD:$x") }
    }
    if ($n.migration -and (Get-P9SSetupEntry (Join-Path $C.StateDir $n.migration)).kind -ne 'absent') { $out.Add("UNEXPECTED_RECORD:$($n.migration)") }
    $r
}

# ------------------------------------------------------------ state.init
function New-P9SStateInitResult {
    [pscustomobject]@{ action = 'state.init'; outcome = 'REFUSED'; reasons = [System.Collections.Generic.List[string]]::new(); lines = [System.Collections.Generic.List[string]]::new()
        mode = $null; status_before = $null; status_after = $null; installation_id = $null; op_id = $null; supersedes = $null; staging = $null; superseded_dir = $null; vm_states = $null; dispatched = 0 }
}
function Invoke-P9SStateInit {
    param([Parameter(Mandatory)]$I, [string]$Supersede, [switch]$DryRun)
    $res = New-P9SStateInitResult
    $C = $I.State; $n = Get-P9SStateNames; $nn = Get-P9SStateInitNames
    $refuse = { param([string[]]$Why) foreach ($x in $Why) { if ($x) { $res.reasons.Add($x) } }; $res.outcome = 'REFUSED'; $res }
    if ($PSBoundParameters.ContainsKey('Supersede') -and -not (Test-P9SStateId $Supersede)) { $res.outcome = 'REJECTED'; $res.reasons.Add('SUPERSEDE_ID_INVALID'); return $res }
    if (-not (Initialize-P9SSetupNative) -or -not $C.Native) { return (& $refuse 'NATIVE_CHECKS_UNAVAILABLE') }
    # ---- the decision, before anything is written
    $d = Get-P9SStateInitDecision $I $Supersede
    $res.status_before = $d.status; $res.mode = $d.mode
    $res.lines.Add("status: $($d.status)$(if ($d.status_reasons.Count) { ' (' + ($d.status_reasons -join ', ') + ')' })")
    if (-not $d.mode) { if ($d.exit) { $res.lines.Add("way out: $($d.exit)") }; return (& $refuse $d.reasons) }
    $acct = [string]$C.Process.account
    # a new installation never takes over a folder that is already there
    if ($d.mode -ceq 'new' -and (Get-P9SSetupEntry $C.StateDir).kind -ne 'absent') { return (& $refuse 'STATE_FOLDER_EXISTS') }
    $pre = @(@(Get-P9SStateInitProcessReasons $I) + @(Get-P9SStateInitVBoxReasons $C))
    if ($pre.Count) { return (& $refuse $pre) }
    $now = Get-P9SStateInitNow $I
    $times = @(Get-P9SStateInitEvidenceTimes $C $d.anchor)
    if (@($times | Where-Object { $_ -gt $now }).Count) { return (& $refuse 'CLOCK_BEHIND_EVIDENCE') }
    Invoke-P9SStateInitStep $I 'P0'
    if ($DryRun) {
        $res.outcome = 'DRYRUN'
        $res.lines.Add("would $(switch ($d.mode) { 'new' { 'start a new installation' } 'resume' { "complete the initialization of $($d.anchor.installation_id)" } 'supersede' { "start a new installation in place of $Supersede, keeping it aside" } }) in $($C.StateDir)")
        $res.lines.Add('the VM states are read once the staging folder exists; nothing was written')
        return $res
    }
    # ---- I1: the root and the setup lock
    $re = Get-P9SSetupEntry $C.Root
    if ($re.kind -eq 'absent') { $x = New-P9SStateFolder $C.Root $acct; if ($x -and $x -cne 'EXISTS') { return (& $refuse "ROOT_NOT_CREATED:$x") } }
    elseif ($re.kind -ne 'directory') { return (& $refuse "ROOT_$($re.kind.ToUpperInvariant())") }
    $lock = $null
    try { $lock = [IO.FileStream]::new((Join-Path $C.Root $n.setup_lock), [IO.FileMode]::OpenOrCreate, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None) } catch { $lock = $null }
    if (-not $lock) { return (& $refuse 'SETUP_BUSY') }
    try {
        Invoke-P9SStateInitStep $I 'I1'
        # the same decision under the lock: whatever ran meanwhile is seen here
        $d2 = Get-P9SStateInitDecision $I $Supersede
        if ($d2.mode -cne $d.mode -or $d2.status -cne $d.status -or [string]$d2.installation_id -cne [string]$d.installation_id -or [string]$(if ($d2.anchor) { $d2.anchor.op_id }) -cne [string]$(if ($d.anchor) { $d.anchor.op_id })) {
            $res.status_before = $d2.status; return (& $refuse (@("SETUP_STATE_CHANGED:$($d2.status)") + @($d2.reasons)))
        }
        $pre = @(@(Get-P9SStateInitProcessReasons $I) + @(Get-P9SStateInitVBoxReasons $C))
        if ($pre.Count) { return (& $refuse $pre) }
        # ---- I2: the anchor of this initialization
        if ($d.mode -ceq 'resume') { $A = $d.anchor }
        else {
            $op = New-P9SStateOpId $now
            if ($d.mode -ceq 'supersede' -and $d.anchor) {
                $raw = Read-P9SStateAnchorRaw $C
                if (-not $raw) { return (& $refuse 'ANCHOR_NOT_KEPT') }
                Write-P9SStateRegistryValue $C "$($nn.kept_anchor)$op" $raw
                Invoke-P9SStateInitStep $I 'I2a'
            }
            $A = [ordered]@{ schema = 'p9s-state-anchor/1'; phase = 'INITIALIZING'; op_id = $op; installation_id = (New-P9SStateInstallationId); account = $acct; state_dir = $C.StateDir; origin = 'new'
                source_dir = $null; manifest_sha256 = $null; supersedes = $(if ($d.mode -ceq 'supersede') { $Supersede } else { $null }); created_utc = (ConvertTo-P9SStateUtcText $now); initialized_utc = $null; ledger = $null }
            try { Write-P9SStateAnchor $C $A } catch { return (& $refuse 'ANCHOR_WRITE_FAILED') }
            Invoke-P9SStateInitStep $I 'I2'
        }
        $op = [string]$A.op_id
        $res.installation_id = [string]$A.installation_id; $res.op_id = $op; $res.supersedes = $A.supersedes
        # a state folder in place: this installation's (a resumed run after
        # I8), or the one being superseded; anything else refuses
        $ne = Get-P9SSetupEntry $C.StateDir
        $ours = $false; $other = $false
        if ($ne.kind -eq 'directory') {
            $ir = Read-P9SStateRecord (Join-Path $C.StateDir $n.install) 'install'
            $ours = [bool]($ir.ok -and [string]$ir.doc.installation_id -ceq [string]$A.installation_id)
            $other = -not $ours
            if ($other -and -not $A.supersedes) { return (& $refuse 'STATE_FOLDER_EXISTS') }
        } elseif ($ne.kind -ne 'absent') { return (& $refuse "STATE_FOLDER_$($ne.kind.ToUpperInvariant())") }
        if (-not $ours) {
            # ---- I4: the staging folder
            $k = 1; $stage = Join-Path $C.Root "$($nn.staging)$op-$k"
            while ((Get-P9SSetupEntry $stage).kind -ne 'absent') { $k++; $stage = Join-Path $C.Root "$($nn.staging)$op-$k" }
            $x = New-P9SStateFolder $stage $acct
            if ($x) { return (& $refuse "STAGING_NOT_CREATED:$x") }
            $res.staging = $stage
            Invoke-P9SStateInitStep $I 'I4'
            # ---- V: the VMs, observed into the staging folder, before the
            # folder being superseded is touched
            $v = Test-P9SStateInitVms $I $stage
            $res.vm_states = $v.states
            if ($v.reasons.Count) { return (& $refuse $v.reasons) }
            Invoke-P9SStateInitStep $I 'V'
            # ---- I3: the folder being superseded, renamed aside, never deleted
            if ($other) {
                $k = 1; $aside = Join-Path $C.Root "$($nn.superseded)$op"
                while ((Get-P9SSetupEntry $aside).kind -ne 'absent') { $k++; $aside = Join-Path $C.Root "$($nn.superseded)$op-$k" }
                $id0 = Get-P9SStateIdentity $C.StateDir
                $e = Move-P9SStateEntry $C.StateDir $aside
                if ($e) { return (& $refuse "STATE_FOLDER_NOT_MOVED_ASIDE:$e") }
                if ((Get-P9SStateIdentity $aside) -cne $id0 -or (Get-P9SSetupEntry $C.StateDir).kind -ne 'absent') { return (& $refuse 'STATE_FOLDER_MOVE_UNVERIFIED') }
                $res.superseded_dir = $aside
                Invoke-P9SStateInitStep $I 'I3'
            }
            # ---- I5: the install record
            $inst = [ordered]@{ schema = 'p9s-state-install/1'; installation_id = $A.installation_id; account = $A.account; state_dir = $A.state_dir; origin = 'new'; migration_id = $null; supersedes = $A.supersedes; created_utc = $A.created_utc }
            try { Write-P9SStateNewFile (Join-Path $stage $n.install) ([Text.UTF8Encoding]::new($false).GetBytes(($inst | ConvertTo-Json -Depth 4))) } catch { return (& $refuse 'INSTALL_RECORD_NOT_WRITTEN') }
            Invoke-P9SStateInitStep $I 'I5'
            # ---- I6: the empty ledger at revision 1, by the toolkit's save
            $lc = [pscustomobject]@{ LedgerPath = Join-Path $stage 'acpi-validation-ledger.json'; SessionPath = Join-Path $stage 'coordinator-session.json'
                WriteLedger = { param($p, $t) Write-P9SStateInitAtomic $I $p $t 'I6-ledger' }.GetNewClosure()
                WriteLedgerRevision = { param($p, $t) Write-P9SStateInitAtomic $I $p $t 'I6-revision' }.GetNewClosure() }
            if (-not (Save-P9SAcpiLedger $lc (New-P9SAcpiLedger))) { return (& $refuse 'LEDGER_NOT_WRITTEN') }
            Invoke-P9SStateInitStep $I 'I6'
            # ---- I7: the ACL of the staging folder and of its files
            $acl = [System.Collections.Generic.List[string]]::new()
            foreach ($x in @(Get-P9SStateAclReasons $stage $acct -RequireProtected)) { $acl.Add("STAGING_ACL:$x") }
            $sl = Get-P9SStateListing $stage
            if (-not $sl.ok) { $acl.Add('STAGING_NOT_LISTABLE') }
            foreach ($f in @($sl.items)) {
                $fe = Get-P9SSetupEntry $f.FullName
                if ($fe.kind -ne 'file' -or $fe.reparse -or $fe.links -gt 1) { $acl.Add("STAGING_ENTRY:$($f.Name)"); continue }
                foreach ($x in @(Get-P9SStateAclReasons $f.FullName $acct)) { $acl.Add("STAGING_ACL:$($f.Name):$x") }
            }
            if ($acl.Count) { return (& $refuse @($acl)) }
            Invoke-P9SStateInitStep $I 'I7'
            # ---- I8: into place (the rename refuses an existing target too)
            if ((Get-P9SSetupEntry $C.StateDir).kind -ne 'absent') { return (& $refuse 'STATE_FOLDER_EXISTS') }
            $id0 = Get-P9SStateIdentity $stage
            $e = Move-P9SStateEntry $stage $C.StateDir
            if (Test-P9SStateExistsError $e) { return (& $refuse "STATE_FOLDER_EXISTS:$e") }
            if ($e) { return (& $refuse "STAGING_NOT_RENAMED:$e") }
            if ((Get-P9SStateIdentity $C.StateDir) -cne $id0) { return (& $refuse 'STATE_FOLDER_RENAME_UNVERIFIED') }
            Invoke-P9SStateInitStep $I 'I8'
        } else {
            # a resumed initialization whose folder is in place: the VMs again
            $v = Test-P9SStateInitVms $I $C.StateDir
            $res.vm_states = $v.states
            if ($v.reasons.Count) { return (& $refuse $v.reasons) }
            Invoke-P9SStateInitStep $I 'V'
        }
        # ---- I9: the folder checked, the anchor unchanged, then READY
        $chk = Get-P9SStateInitFolderCheck $I $A
        if ($chk.reasons.Count) { return (& $refuse (@('STATE_FOLDER_NOT_COMPLETE') + @($chk.reasons))) }
        $cur = Read-P9SStateAnchor $C
        if (-not $cur.ok -or [string]$cur.doc.phase -cne 'INITIALIZING' -or [string]$cur.doc.op_id -cne $op -or [string]$cur.doc.installation_id -cne [string]$A.installation_id) { return (& $refuse 'ANCHOR_CHANGED') }
        $now = Get-P9SStateInitNow $I
        $created = ConvertFrom-P9SStateUtcText $A.created_utc
        if (-not $created -or $now -lt $created) { return (& $refuse 'CLOCK_BEHIND_EVIDENCE') }
        $R = [ordered]@{}; foreach ($k in $A.Keys) { $R[$k] = $A[$k] }
        $R.phase = 'READY'; $R.initialized_utc = ConvertTo-P9SStateUtcText $now; $R.ledger = $chk.mark
        try { Write-P9SStateAnchor $C $R } catch { return (& $refuse 'ANCHOR_WRITE_FAILED') }
        Invoke-P9SStateInitStep $I 'I9'
        # READY only as the status says it, never as this function assumes it
        $after = Get-P9SStateStatus $C
        $res.status_after = $after.status
        if ($after.status -cne 'READY' -or [string]$after.installation_id -cne [string]$A.installation_id) {
            $res.outcome = 'FAILED'; $res.reasons.Add("STATUS_AFTER_INIT:$($after.status)"); foreach ($x in @($after.reasons)) { $res.reasons.Add($x) }; return $res
        }
        # ---- I10: the first line of the new installation's audit
        try {
            Add-P9SStateInitAudit $C.StateDir ([ordered]@{ utc = (ConvertTo-P9SStateUtcText (Get-P9SStateInitNow $I)); pid = $PID; action = 'state.init'; outcome = 'DONE'; reasons = @()
                installation_id = $A.installation_id; op_id = $op; supersedes = $A.supersedes; superseded_dir = $(if ($res.superseded_dir) { [IO.Path]::GetFileName($res.superseded_dir) } else { $null })
                initialized_utc = $R.initialized_utc; vm_states = $res.vm_states; dispatched = 0 })
        } catch { $res.lines.Add('the audit line of the new installation was not written') }
        Invoke-P9SStateInitStep $I 'I10'
        $res.outcome = 'DONE'
        $res.lines.Add("installation $($A.installation_id) ready in $($C.StateDir); initialized $($R.initialized_utc)")
        if ($A.supersedes) { $res.lines.Add("supersedes $($A.supersedes)$(if ($res.superseded_dir) { "; its folder is now $($res.superseded_dir)" }); every ACPI validation authorization approved or valid from before $($R.initialized_utc) is void") }
        return $res
    } finally { $lock.Dispose() }
}

# ------------------------------------------------------------ the ledger mark
# Every save of the installed ledger ends here (Save-P9SAcpiLedger, under the
# coordinator lock): the anchor must be READY for this account and state
# folder, and its mark only moves forward. Order of one save: the ledger
# (written beside its name, renamed), read back; its revision record; then
# this anchor write, flushed and read back. Windows gives no atomicity
# across the file system and the registry: a crash between them leaves the
# anchor behind the ledger, which state.status accepts (a lower bound); the
# anchor is never written ahead of a ledger that is not on disk. The anchor
# is read and written under setup.lock, taken without waiting: a setup
# (state.init, state.migrate, state.reanchor) holds it for its whole run, so
# that neither can overwrite the other's anchor; while one runs the save
# fails and nothing is dispatched. $true only once the new mark is in place.
function Update-P9SStateLedgerMark($C, [long]$Revision, [string]$Sha) {
    if (-not $C -or -not $C.Native -or -not $C.Root -or -not (Test-P9SStateSha256 $Sha)) { return $false }
    $lk = $null
    try { $lk = [IO.FileStream]::new((Join-Path $C.Root (Get-P9SStateNames).setup_lock), [IO.FileMode]::OpenOrCreate, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None) } catch { return $false }
    try {
        $a = Read-P9SStateAnchor $C
        if (-not $a.ok) { return $false }
        $A = $a.doc
        if ([string]$A.phase -cne 'READY' -or [string]$A.account -cne [string]$C.Process.account -or -not (Test-P9SStateSamePath $A.state_dir $C.StateDir)) { return $false }
        $m = $A.ledger
        if ($Revision -lt [long]$m.revision) { return $false }
        if ($Revision -eq [long]$m.revision) { return ([string]$m.sha256 -ceq $Sha) }
        $R = [ordered]@{}; foreach ($k in $A.Keys) { $R[$k] = $A[$k] }
        $R.ledger = [ordered]@{ revision = $Revision; sha256 = $Sha }
        try { Write-P9SStateAnchor $C $R } catch { return $false }
        $true
    } finally { $lk.Dispose() }
}

# ------------------------------------------------------------ installation time (authorizations)
# What the authorization rule needs: whether an installation exists, and
# since when. known = $false whenever that cannot be established (an
# unreadable anchor, an initialization in progress, a folder without its
# anchor, an access error): the rule then refuses. No anchor and no root,
# or a root holding only the setup lock: no installation, nothing to void.
function Get-P9SStateInstallation($C) {
    $o = [pscustomobject]@{ known = $false; installed = $false; installation_id = $null; initialized_utc = $null; reason = $null }
    if (-not $C -or -not $C.Native -or -not $C.Root) { $o.reason = 'INSTALLATION_NOT_VERIFIABLE'; return $o }
    $a = Read-P9SStateAnchor $C
    if ($a.present) {
        if (-not $a.ok) { $o.reason = 'ANCHOR_UNREADABLE'; return $o }
        $A = $a.doc
        if ([string]$A.account -cne [string]$C.Process.account) { $o.reason = 'ANCHOR_ACCOUNT'; return $o }
        if ([string]$A.phase -cne 'READY') { $o.reason = "INSTALLATION_IN_PROGRESS:$($A.phase)"; return $o }
        $t = ConvertFrom-P9SStateUtcText $A.initialized_utc
        if (-not $t) { $o.reason = 'INITIALIZED_UTC_INVALID'; return $o }
        $o.known = $true; $o.installed = $true; $o.installation_id = [string]$A.installation_id; $o.initialized_utc = $t
        return $o
    }
    $e = Get-P9SSetupEntry $C.Root
    if ($e.kind -eq 'absent') { $o.known = $true; return $o }
    if ($e.kind -ne 'directory') { $o.reason = 'ROOT_NOT_VERIFIABLE'; return $o }
    $l = Get-P9SStateListing $C.Root
    if (-not $l.ok) { $o.reason = 'ROOT_NOT_LISTABLE'; return $o }
    if (@($l.items | Where-Object { $_.Name -cne (Get-P9SStateNames).setup_lock }).Count) { $o.reason = 'INSTALLATION_WITHOUT_ANCHOR'; return $o }
    $o.known = $true
    $o
}
