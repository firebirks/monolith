#Requires -Version 7.0
# P9-S persistent state, read only. Computes the status of the state
# folder that later versions will create and use
# (<LocalApplicationData>\Monolith-P9S\state), from what exists now: that
# folder and its install, migration and ledger records, the anchor value
# under HKCU\Software\Monolith-P9S, and the coordinator folder of this and
# earlier versions (%TEMP%\p9s-hostmon). It never creates, changes or
# deletes a file, folder or registry key, takes no lock and writes no audit
# record. No action uses the result: the coordinator keeps its files where
# it always did, and nothing is gated on the status yet. A missing file is
# never taken as proof that nothing existed, and an access error is never
# taken for a missing file. Dot-source after hostmon-lib.ps1 and
# coordinator-lib.ps1.

function Get-P9SStateNames {
    [ordered]@{
        root = 'Monolith-P9S'; state = 'state'; setup_lock = 'setup.lock'; anchor_key = 'Software\Monolith-P9S'; anchor_value = 'StateAnchor'
        install = 'install.json'; migration = 'migration.json'; marker = 'MIGRATED-TO.json'; legacy = 'p9s-hostmon'
    }
}
# Files of the coordinator folder by name. records and histories are what a
# migration would copy; locks show that a coordinator ran; the monitor's
# files and the temporary files of an unfinished atomic write show nothing.
function Get-P9SStateFileClasses {
    [ordered]@{
        record = @('acpi-validation-ledger.json', 'acpi-validation-ledger.revision.json', 'coordinator-stop.request', 'coordinator-session.json', 'vbox-quarantine.json', 'vbox-operation.json')
        history = @('coordinator-audit.jsonl', 'vm-observations.jsonl', 'vbox-audit.jsonl')
        lock = @('coordinator.lock', 'coordinator-emergency.hold')
        monitor = @('state.json', 'state.json.lock')
    }
}
function Get-P9SStateFileClass([string]$Name) {
    $c = Get-P9SStateFileClasses
    foreach ($k in $c.Keys) { if ($Name -cin $c[$k]) { return $k } }
    if ($Name -cmatch '^\..+\.[0-9a-f]{32}\.tmp$') { return 'temporary' }
    if ($Name -ceq (Get-P9SStateNames).marker) { return 'marker' }
    'unknown'
}
function Get-P9SStateStatuses {
    'SANDBOX', 'LOCATION_UNSAFE', 'ANCHOR_UNREADABLE', 'ANCHOR_MISSING', 'SETUP_LEFTOVER', 'INSTALLATION_LOST', 'MIGRATION_SOURCES_AMBIGUOUS', 'MIGRATION_REQUIRED',
    'NOT_INITIALIZED', 'INTERRUPTED', 'ACCOUNT_MISMATCH', 'STATE_MISMATCH', 'STATE_MISSING', 'STATE_INCOMPLETE', 'STATE_UNCERTAIN', 'STATE_DIVERGED', 'READY'
}

# ------------------------------------------------------------ native checks
# Windows calls with no managed equivalent: the known folder, the package
# identity and token flags of this process, and the identity of a file or
# folder opened for its attributes only (OPEN_EXISTING: it never creates;
# the last component is not followed when it is a reparse point).
$script:P9SStateNativeSource = @'
using System;
using System.Runtime.InteropServices;
using System.Text;
using Microsoft.Win32.SafeHandles;
public static class P9SStateNative {
    [DllImport("shell32.dll")]
    private static extern int SHGetKnownFolderPath([MarshalAs(UnmanagedType.LPStruct)] Guid id, uint flags, IntPtr token, out IntPtr path);
    public static string LocalAppData() {
        IntPtr p = IntPtr.Zero;
        int hr = SHGetKnownFolderPath(new Guid("F1B32785-6FBA-4FCF-9D55-7B8E7F157091"), 0, IntPtr.Zero, out p);
        try { return hr == 0 ? Marshal.PtrToStringUni(p) : null; } finally { if (p != IntPtr.Zero) Marshal.FreeCoTaskMem(p); }
    }
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode)]
    private static extern int GetCurrentPackageFullName(ref int length, StringBuilder name);
    public static int PackageCode() { int n = 0; return GetCurrentPackageFullName(ref n, null); }
    [DllImport("advapi32.dll", SetLastError = true)]
    private static extern bool GetTokenInformation(IntPtr token, int infoClass, out int info, int length, out int returned);
    public static int TokenFlag(IntPtr token, int infoClass) { int v, r; return GetTokenInformation(token, infoClass, out v, 4, out r) ? v : -1; }
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern SafeFileHandle CreateFileW(string name, uint access, uint share, IntPtr security, uint disposition, uint flags, IntPtr template);
    [StructLayout(LayoutKind.Sequential)]
    private struct HandleInfo { public uint Attributes, C1, C2, A1, A2, W1, W2, Volume, SizeHigh, SizeLow, Links, IndexHigh, IndexLow; }
    [DllImport("kernel32.dll", SetLastError = true)]
    private static extern bool GetFileInformationByHandle(SafeFileHandle handle, out HandleInfo info);
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern uint GetFinalPathNameByHandleW(SafeFileHandle handle, StringBuilder path, uint length, uint flags);
    public sealed class Entry { public int Error; public uint Attributes; public uint Volume; public ulong Index; public uint Links; public ulong Size; public ulong Created; public ulong Written; public string FinalPath; }
    public static Entry Inspect(string path) {
        Entry e = new Entry();
        using (SafeFileHandle h = CreateFileW(path, 0x80, 7, IntPtr.Zero, 3, 0x02200000, IntPtr.Zero)) {
            if (h.IsInvalid) { e.Error = Marshal.GetLastWin32Error(); return e; }
            HandleInfo i;
            if (!GetFileInformationByHandle(h, out i)) { e.Error = Marshal.GetLastWin32Error(); return e; }
            e.Attributes = i.Attributes; e.Volume = i.Volume; e.Index = ((ulong)i.IndexHigh << 32) | i.IndexLow; e.Links = i.Links;
            e.Size = ((ulong)i.SizeHigh << 32) | i.SizeLow; e.Created = ((ulong)i.C2 << 32) | i.C1; e.Written = ((ulong)i.W2 << 32) | i.W1;
            StringBuilder b = new StringBuilder(2048);
            uint n = GetFinalPathNameByHandleW(h, b, 2048, 0);
            if (n == 0 || n >= 2048) { e.Error = n == 0 ? Marshal.GetLastWin32Error() : 206; return e; }
            e.FinalPath = b.ToString();
        }
        return e;
    }
}
'@
function Initialize-P9SStateNative {
    if ('P9SStateNative' -as [type]) { return $true }
    try { Add-Type -TypeDefinition $script:P9SStateNativeSource -ErrorAction Stop; return $true } catch { return $false }
}
function ConvertFrom-P9SStateFinalPath([string]$P) {
    if (-not $P) { return $null }
    if ($P.StartsWith('\\?\UNC\')) { return '\\' + $P.Substring(8) }
    if ($P.StartsWith('\\?\')) { return $P.Substring(4) }
    $P
}
function ConvertTo-P9SStateNormalPath([string]$P) {
    if (-not $P) { return $null }
    try { $f = [IO.Path]::GetFullPath($P) } catch { return $null }
    if ($f.Length -gt 3) { $f = $f.TrimEnd('\') }
    $f
}
function Test-P9SStateSamePath([string]$A, [string]$B) {
    $x = ConvertTo-P9SStateNormalPath $A; $y = ConvertTo-P9SStateNormalPath $B
    [bool]($x -and $y -and [string]::Equals($x, $y, [StringComparison]::OrdinalIgnoreCase))
}
# kind: absent, file, directory, denied or error. Only "file not found" and
# "path not found" are absent; access denied is denied, never absent.
function Get-P9SStateEntry([string]$Path) {
    $e = [P9SStateNative]::Inspect($Path)
    $kind = switch ($e.Error) {
        0 { if ($e.Attributes -band 0x10) { 'directory' } else { 'file' } }
        2 { 'absent' }
        3 { 'absent' }
        5 { 'denied' }
        default { 'error' }
    }
    [pscustomobject]@{ path = $Path; kind = $kind; error = $e.Error; reparse = [bool]($e.Error -eq 0 -and ($e.Attributes -band 0x400)); links = [int]$e.Links
        volume = $e.Volume; index = $e.Index; final = (ConvertFrom-P9SStateFinalPath $e.FinalPath) }
}
function Get-P9SStateListing([string]$Dir) {
    $o = [IO.EnumerationOptions]::new(); $o.AttributesToSkip = [IO.FileAttributes]0; $o.IgnoreInaccessible = $false; $o.RecurseSubdirectories = $false
    try { [pscustomobject]@{ ok = $true; items = @([IO.DirectoryInfo]::new($Dir).EnumerateFileSystemInfos('*', $o)); error = $null } }
    catch [UnauthorizedAccessException] { [pscustomobject]@{ ok = $false; items = @(); error = 'denied' } }
    catch { [pscustomobject]@{ ok = $false; items = @(); error = $_.Exception.GetType().Name } }
}
# The bytes of a file (open for read, share all, never create): ok, absent,
# denied or busy.
function Read-P9SStateBytes([string]$Path) {
    for ($i = 0; $i -lt 5; $i++) {
        try {
            $fs = [IO.FileStream]::new($Path, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::ReadWrite -bor [IO.FileShare]::Delete)
            try { $ms = [IO.MemoryStream]::new(); $fs.CopyTo($ms); return [pscustomobject]@{ kind = 'ok'; bytes = $ms.ToArray() } } finally { $fs.Dispose() }
        }
        catch [IO.FileNotFoundException] { return [pscustomobject]@{ kind = 'absent'; bytes = $null } }
        catch [IO.DirectoryNotFoundException] { return [pscustomobject]@{ kind = 'absent'; bytes = $null } }
        catch [UnauthorizedAccessException] { return [pscustomobject]@{ kind = 'denied'; bytes = $null } }
        catch [IO.IOException] { Start-Sleep -Milliseconds 20 }
    }
    [pscustomobject]@{ kind = 'busy'; bytes = $null }
}
function Get-P9SStateSha([byte[]]$Bytes) { [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData($Bytes)).ToLowerInvariant() }

# ------------------------------------------------------------ strict JSON
# One JSON object, UTF-8 without byte order mark, no duplicated name; strings
# stay strings (no date conversion), numbers must be integers. $null when
# any of that fails.
function ConvertFrom-P9SStateJsonElement($El) {
    switch ([string]$El.ValueKind) {
        'Object' { $h = [ordered]@{}; foreach ($p in $El.EnumerateObject()) { $h[$p.Name] = ConvertFrom-P9SStateJsonElement $p.Value }; return $h }
        'Array' { return , @($El.EnumerateArray() | ForEach-Object { ConvertFrom-P9SStateJsonElement $_ }) }
        'String' { return $El.GetString() }
        'Number' { $v = 0L; if ($El.TryGetInt64([ref]$v)) { return $v }; throw 'not an integer' }
        'True' { return $true }
        'False' { return $false }
        'Null' { return $null }
    }
    throw 'unexpected JSON value'
}
function ConvertFrom-P9SStateJson([byte[]]$Bytes) {
    if (-not $Bytes -or ($Bytes.Length -ge 3 -and $Bytes[0] -eq 0xEF -and $Bytes[1] -eq 0xBB -and $Bytes[2] -eq 0xBF)) { return $null }
    try { $t = [Text.UTF8Encoding]::new($false, $true).GetString($Bytes) } catch { return $null }
    if (@(Get-P9SJsonDuplicateNames $t).Count) { return $null }
    $doc = $null
    try {
        $doc = [System.Text.Json.JsonDocument]::Parse($t)
        if ($doc.RootElement.ValueKind -ne [System.Text.Json.JsonValueKind]::Object) { return $null }
        return (ConvertFrom-P9SStateJsonElement $doc.RootElement)
    } catch { return $null } finally { if ($doc) { $doc.Dispose() } }
}
function Get-P9SStateLedgerRevision([byte[]]$Bytes) {
    $doc = $null
    try {
        $doc = [System.Text.Json.JsonDocument]::Parse([Text.UTF8Encoding]::new($false, $true).GetString($Bytes))
        $p = @($doc.RootElement.EnumerateObject() | Where-Object { $_.Name -ceq 'revision' })
        if (-not $p.Count) { return 0L }
        $v = 0L; if ($p[0].Value.ValueKind -eq [System.Text.Json.JsonValueKind]::Number -and $p[0].Value.TryGetInt64([ref]$v) -and $v -ge 0) { return $v }
        return $null
    } catch { return $null } finally { if ($doc) { $doc.Dispose() } }
}

# ------------------------------------------------------------ record schemas
function Test-P9SStateUtc($V) { $d = [DateTime]::MinValue; ($V -is [string]) -and $V.EndsWith('Z') -and [DateTime]::TryParse($V, [Globalization.CultureInfo]::InvariantCulture, [Globalization.DateTimeStyles]::RoundtripKind, [ref]$d) }
function Test-P9SStateId($V) { ($V -is [string]) -and $V -cmatch '^[A-Za-z0-9][A-Za-z0-9-]{3,63}$' }
function Test-P9SStateSha256($V) { ($V -is [string]) -and $V -cmatch '^[0-9a-f]{64}$' }
function Test-P9SStateSid($V) { ($V -is [string]) -and $V -cmatch '^S-1-[0-9]+(-[0-9]+)+$' }
function Test-P9SStateAbsolute($V) { ($V -is [string]) -and $V -match '^[A-Za-z]:\\' }
# reasons why a document is not of the expected kind (empty: it is)
function Get-P9SStateDocReasons([string]$Kind, $D) {
    $out = [System.Collections.Generic.List[string]]::new()
    if ($D -isnot [System.Collections.IDictionary]) { $out.Add('NOT_A_JSON_OBJECT'); return @($out) }
    $fields = switch ($Kind) {
        'anchor' { 'schema', 'phase', 'op_id', 'installation_id', 'account', 'state_dir', 'origin', 'source_dir', 'manifest_sha256', 'supersedes', 'created_utc', 'initialized_utc', 'ledger' }
        'install' { 'schema', 'installation_id', 'account', 'state_dir', 'origin', 'migration_id', 'supersedes', 'created_utc' }
        'migration' { 'schema', 'migration_id', 'installation_id', 'source_dir', 'manifest_sha256', 'files', 'absent', 'ignored', 'ledger_created', 'vm_observations', 'utc' }
        'marker' { 'schema', 'installation_id', 'state_dir', 'manifest_sha256', 'note' }
    }
    foreach ($k in $D.Keys) { if ($k -cnotin $fields) { $out.Add("UNKNOWN_FIELD:$k") } }
    foreach ($k in $fields) { if (-not $D.Contains($k)) { $out.Add("MISSING_FIELD:$k") } }
    if ($out.Count) { return @($out) }
    $schema = @{ anchor = 'p9s-state-anchor/1'; install = 'p9s-state-install/1'; migration = 'p9s-state-migration/1'; marker = 'p9s-state-moved/1' }[$Kind]
    if ($D.schema -cne $schema) { $out.Add('SCHEMA'); return @($out) }
    $bad = { param($Field) $out.Add("FIELD:$Field") }
    switch ($Kind) {
        'anchor' {
            if ($D.phase -cnotin 'INITIALIZING', 'MIGRATING', 'READY') { & $bad 'phase' }
            if (-not (Test-P9SStateId $D.op_id)) { & $bad 'op_id' }
            if (-not (Test-P9SStateId $D.installation_id)) { & $bad 'installation_id' }
            if (-not (Test-P9SStateSid $D.account)) { & $bad 'account' }
            if (-not (Test-P9SStateAbsolute $D.state_dir)) { & $bad 'state_dir' }
            if ($D.origin -cnotin 'new', 'migrated') { & $bad 'origin' }
            if ($D.origin -ceq 'migrated') { if (-not (Test-P9SStateAbsolute $D.source_dir)) { & $bad 'source_dir' }; if (-not (Test-P9SStateSha256 $D.manifest_sha256)) { & $bad 'manifest_sha256' } }
            elseif ($null -ne $D.source_dir -or $null -ne $D.manifest_sha256) { & $bad 'source_dir' }
            if ($null -ne $D.supersedes -and -not (Test-P9SStateId $D.supersedes)) { & $bad 'supersedes' }
            if (-not (Test-P9SStateUtc $D.created_utc)) { & $bad 'created_utc' }
            if ($D.phase -ceq 'READY') {
                if (-not (Test-P9SStateUtc $D.initialized_utc)) { & $bad 'initialized_utc' }
                $l = $D.ledger
                if ($l -isnot [System.Collections.IDictionary] -or @($l.Keys).Count -ne 2 -or -not $l.Contains('revision') -or -not $l.Contains('sha256') -or $l.revision -isnot [long] -or $l.revision -lt 0 -or -not (Test-P9SStateSha256 $l.sha256)) { & $bad 'ledger' }
            } elseif ($null -ne $D.initialized_utc -or $null -ne $D.ledger) { & $bad 'ledger' }
        }
        'install' {
            if (-not (Test-P9SStateId $D.installation_id)) { & $bad 'installation_id' }
            if (-not (Test-P9SStateSid $D.account)) { & $bad 'account' }
            if (-not (Test-P9SStateAbsolute $D.state_dir)) { & $bad 'state_dir' }
            if ($D.origin -cnotin 'new', 'migrated') { & $bad 'origin' }
            if ($D.origin -ceq 'migrated') { if (-not (Test-P9SStateId $D.migration_id)) { & $bad 'migration_id' } } elseif ($null -ne $D.migration_id) { & $bad 'migration_id' }
            if ($null -ne $D.supersedes -and -not (Test-P9SStateId $D.supersedes)) { & $bad 'supersedes' }
            if (-not (Test-P9SStateUtc $D.created_utc)) { & $bad 'created_utc' }
        }
        'migration' {
            foreach ($k in 'migration_id', 'installation_id') { if (-not (Test-P9SStateId $D[$k])) { & $bad $k } }
            if (-not (Test-P9SStateAbsolute $D.source_dir)) { & $bad 'source_dir' }
            if (-not (Test-P9SStateSha256 $D.manifest_sha256)) { & $bad 'manifest_sha256' }
            if ($D.ledger_created -isnot [bool]) { & $bad 'ledger_created' }
            if (-not (Test-P9SStateUtc $D.utc)) { & $bad 'utc' }
            foreach ($k in 'files', 'absent', 'ignored', 'vm_observations') { if ($D[$k] -isnot [array]) { & $bad $k } }
            if (-not $out.Count) {
                $names = [System.Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
                foreach ($x in @($D.files)) {
                    $cls = if ($x -is [System.Collections.IDictionary]) { Get-P9SStateFileClass ([string]$x.name) } else { 'unknown' }
                    if ($x -isnot [System.Collections.IDictionary] -or @($x.Keys).Count -ne 4 -or $cls -cnotin 'record', 'history' -or $x.class -cne $cls -or $x.length -isnot [long] -or $x.length -lt 0 -or -not (Test-P9SStateSha256 $x.sha256) -or -not $names.Add([string]$x.name)) { & $bad 'files'; break }
                }
                foreach ($x in @($D.absent)) { if ($x -isnot [string] -or (Get-P9SStateFileClass $x) -cne 'record' -or -not $names.Add($x)) { & $bad 'absent'; break } }
            }
        }
        'marker' { if (-not (Test-P9SStateId $D.installation_id)) { & $bad 'installation_id' } }
    }
    @($out)
}
# A record file: ok with its document, absent, or not readable with the
# reason (denied, busy, not parseable, the schema reasons).
function Read-P9SStateRecord([string]$Path, [string]$Kind) {
    $b = Read-P9SStateBytes $Path
    if ($b.kind -ne 'ok') { return [pscustomobject]@{ kind = $b.kind; ok = $false; doc = $null; reasons = @("NOT_READ:$($b.kind)") } }
    $d = ConvertFrom-P9SStateJson $b.bytes
    if ($null -eq $d) { return [pscustomobject]@{ kind = 'invalid'; ok = $false; doc = $null; reasons = @('NOT_PARSEABLE') } }
    $r = @(Get-P9SStateDocReasons $Kind $d)
    [pscustomobject]@{ kind = $(if ($r.Count) { 'invalid' } else { 'ok' }); ok = (-not $r.Count); doc = $d; reasons = $r }
}
# The digest a migration manifest records (a later migration step writes it
# the same way): the SHA-256 of the UTF-8 text made of a header line, the
# source folder, then one line per copied file and one per absent record,
# these lines sorted by ordinal comparison, each line ended by a line feed.
function Get-P9SStateManifestDigest($Files, $Absent, [string]$Source) {
    $body = [string[]]@(@(@($Files) | ForEach-Object { "file:$($_.name)|$($_.class)|$($_.length)|$($_.sha256)" }) + @(@($Absent) | Where-Object { $null -ne $_ } | ForEach-Object { "absent:$_" }))
    [Array]::Sort($body, [StringComparer]::Ordinal)
    $lines = @('p9s-state-manifest/1', "source:$Source") + $body
    Get-P9SStateSha ([Text.UTF8Encoding]::new($false).GetBytes(($lines -join "`n") + "`n"))
}

# ------------------------------------------------------------ context
# Production: no argument but -AskedStatePath (the coordinator's -StatePath).
# Tests inject the folders, the anchor key and the facts of the process;
# the command line offers none of them.
function New-P9SStateContext {
    param([string]$AskedStatePath, [string]$LocalAppData, [string]$TempPath, [string]$AnchorKey, $ProcessFacts, $VolumeFacts)
    $n = Get-P9SStateNames
    $native = Initialize-P9SStateNative
    $kf = [System.Collections.Generic.List[string]]::new()
    if (-not $LocalAppData) {
        if ($native) {
            $api = [P9SStateNative]::LocalAppData(); $net = [Environment]::GetFolderPath([Environment+SpecialFolder]::LocalApplicationData)
            if (-not $api) { $kf.Add('KNOWN_FOLDER_UNAVAILABLE') } elseif (-not (Test-P9SStateSamePath $api $net)) { $kf.Add('KNOWN_FOLDER_DISAGREES') }
            $LocalAppData = $api
        } else { $kf.Add('KNOWN_FOLDER_UNAVAILABLE') }
    }
    $lad = ConvertTo-P9SStateNormalPath $LocalAppData
    if ($lad -and -not (Test-P9SStateAbsolute $lad)) { $kf.Add('KNOWN_FOLDER_NOT_LOCAL'); $lad = $null }
    if (-not $lad -and -not $kf.Count) { $kf.Add('KNOWN_FOLDER_UNAVAILABLE') }
    $tmp = ConvertTo-P9SStateNormalPath $(if ($TempPath) { $TempPath } else { [IO.Path]::GetTempPath() })
    $root = if ($lad) { Join-Path $lad $n.root } else { $null }
    $legacy = [System.Collections.Generic.List[string]]::new()
    foreach ($p in @($(if ($tmp) { Join-Path $tmp $n.legacy }), $(if ($lad) { Join-Path (Join-Path $lad 'Temp') $n.legacy }))) {
        if ($p -and -not @($legacy | Where-Object { Test-P9SStateSamePath $_ $p }).Count) { $legacy.Add($p) }
    }
    $asked = $null
    if ($AskedStatePath) { $asked = ConvertTo-P9SStateNormalPath ([IO.Path]::GetDirectoryName([IO.Path]::GetFullPath($AskedStatePath))) }
    [pscustomobject]@{
        Native = $native; KnownFolderReasons = @($kf); LocalAppData = $lad; Root = $root; StateDir = $(if ($root) { Join-Path $root $n.state } else { $null })
        TempPath = $tmp; LegacyDirs = @($legacy); AnchorKey = $(if ($AnchorKey) { $AnchorKey } else { $n.anchor_key }); AnchorValue = $n.anchor_value
        AskedFolder = $asked; Process = $(if ($ProcessFacts) { $ProcessFacts } else { Get-P9SStateProcessFacts $native }); Volume = $VolumeFacts
    }
}
# What this process is: its account, whether it has a package identity, runs
# in an app container or with UAC file and registry virtualization (each
# turns some writes of the account into writes to a private copy), and where
# its executable is.
function Get-P9SStateProcessFacts([bool]$Native) {
    $id = [Security.Principal.WindowsIdentity]::GetCurrent()
    try {
        $f = [ordered]@{ account = $id.User.Value; package_code = $null; app_container = $null; virtualization = $null; path = (Get-Process -Id $PID).Path }
        if ($Native) {
            $f.package_code = [P9SStateNative]::PackageCode()
            $f.app_container = [P9SStateNative]::TokenFlag($id.Token, 29)
            $f.virtualization = [P9SStateNative]::TokenFlag($id.Token, 24)
        }
        [pscustomobject]$f
    } finally { $id.Dispose() }
}

# ------------------------------------------------------------ location
function Get-P9SStateAclReasons([string]$Path, [string]$Account, [switch]$RequireProtected) {
    $out = [System.Collections.Generic.List[string]]::new()
    $acl = $null
    try { $acl = Get-Acl -LiteralPath $Path -ErrorAction Stop } catch { $out.Add('ACL_UNREADABLE'); return @($out) }
    $allowed = @($Account, 'S-1-5-18', 'S-1-5-32-544')
    $own = try { ([Security.Principal.NTAccount]$acl.Owner).Translate([Security.Principal.SecurityIdentifier]).Value } catch { [string]$acl.Owner }
    if ($own -notin $allowed) { $out.Add('ACL_FOREIGN_OWNER') }
    if ($RequireProtected -and -not $acl.AreAccessRulesProtected) { $out.Add('ACL_NOT_PROTECTED') }
    $write = 0x2 -bor 0x4 -bor 0x10 -bor 0x40 -bor 0x100 -bor 0x10000 -bor 0x40000 -bor 0x80000 -bor 0x40000000 -bor 0x10000000
    foreach ($r in @($acl.Access)) {
        if ([string]$r.AccessControlType -ne 'Allow' -or ($r.PropagationFlags -band [Security.AccessControl.PropagationFlags]::InheritOnly)) { continue }
        $sid = try { $r.IdentityReference.Translate([Security.Principal.SecurityIdentifier]).Value } catch { 'UNRESOLVED' }
        if ($sid -in $allowed) { continue }
        if ([long]([Security.AccessControl.FileSystemRights]$r.FileSystemRights) -band $write) { $out.Add('ACL_FOREIGN_WRITE') }
    }
    @($out | Select-Object -Unique)
}
function Get-P9SStateLocationReasons($C) {
    $out = [System.Collections.Generic.List[string]]::new()
    if (-not $C.Native) { $out.Add('NATIVE_CHECKS_UNAVAILABLE'); return @($out) }
    $pf = $C.Process
    switch ([int]$(if ($null -eq $pf.package_code) { -1 } else { $pf.package_code })) { 15700 { } { $_ -in 0, 122 } { $out.Add('PACKAGED_PROCESS') } default { $out.Add('PACKAGE_STATE_UNVERIFIABLE') } }
    if ($pf.app_container -ne 0) { $out.Add($(if ($pf.app_container -eq 1) { 'APP_CONTAINER' } else { 'APP_CONTAINER_UNVERIFIABLE' })) }
    if ($pf.virtualization -ne 0) { $out.Add($(if ($pf.virtualization -eq 1) { 'UAC_VIRTUALIZATION' } else { 'VIRTUALIZATION_UNVERIFIABLE' })) }
    if ([string]$pf.path -like '*\WindowsApps\*') { $out.Add('PROCESS_IN_WINDOWSAPPS') }
    if (-not (Test-P9SStateSid $pf.account)) { $out.Add('ACCOUNT_UNKNOWN') } elseif ($pf.account -in 'S-1-5-18', 'S-1-5-19', 'S-1-5-20') { $out.Add('SERVICE_ACCOUNT') }
    foreach ($x in $C.KnownFolderReasons) { $out.Add($x) }
    if (-not $C.LocalAppData) { return @($out | Select-Object -Unique) }
    $v = $C.Volume
    if (-not $v) { try { $d = [IO.DriveInfo]::new([IO.Path]::GetPathRoot($C.LocalAppData)); $v = [pscustomobject]@{ type = [string]$d.DriveType; format = [string]$d.DriveFormat } } catch { $v = $null } }
    if (-not $v) { $out.Add('STATE_VOLUME_UNVERIFIABLE') } elseif ($v.type -ne 'Fixed' -or $v.format -ne 'NTFS') { $out.Add('STATE_VOLUME_NOT_LOCAL_NTFS') }
    # every existing component from the volume root to the state folder:
    # no reparse point, accessible, and the deepest one where it says it is
    $parts = $C.StateDir.Split('\'); $p = $null; $deep = $null
    foreach ($s in $parts) {
        $p = if ($null -eq $p) { $s + '\' } elseif ($p.EndsWith('\')) { $p + $s } else { $p + '\' + $s }
        $e = Get-P9SStateEntry $p
        if ($e.kind -eq 'absent') { break }
        if ($e.kind -in 'denied', 'error') { $out.Add('STATE_PATH_NOT_ACCESSIBLE'); break }
        if ($e.reparse) { $out.Add('STATE_PATH_REPARSE_POINT'); break }
        if ($e.kind -ne 'directory') { $out.Add('STATE_PATH_NOT_A_FOLDER'); break }
        $deep = $e
    }
    if ($deep -and -not (Test-P9SStateSamePath $deep.final $deep.path)) { $out.Add('STATE_PATH_NOT_CANONICAL') }
    if ($out.Count) { return @($out | Select-Object -Unique) }
    $acct = [string]$pf.account
    foreach ($d in @($C.Root, $C.StateDir)) {
        if ((Get-P9SStateEntry $d).kind -ne 'directory') { continue }
        if (-not (Get-P9SStateListing $d).ok) { $out.Add('STATE_FOLDER_NOT_LISTABLE') }
        foreach ($x in @(Get-P9SStateAclReasons $d $acct -RequireProtected:($d -eq $C.Root))) { $out.Add($x) }
    }
    # the files of the state folder: none a reparse point or a hard link
    # shared with another path, the records writable by no other account
    if ((Get-P9SStateEntry $C.StateDir).kind -eq 'directory') {
        $n = Get-P9SStateNames; $cls = Get-P9SStateFileClasses
        foreach ($i in @((Get-P9SStateListing $C.StateDir).items)) {
            $e = Get-P9SStateEntry $i.FullName
            if ($e.kind -in 'denied', 'error') { $out.Add('STATE_ENTRY_NOT_ACCESSIBLE'); continue }
            if ($e.reparse) { $out.Add('STATE_ENTRY_REPARSE_POINT') }
            if ($i -is [IO.DirectoryInfo]) { continue }
            if ($e.links -gt 1) { $out.Add('STATE_ENTRY_HARD_LINKED') }
            if ($i.Name -cin @($cls.record) + @($n.install, $n.migration)) { foreach ($x in @(Get-P9SStateAclReasons $i.FullName $acct)) { $out.Add($x) } }
        }
    }
    @($out | Select-Object -Unique)
}

# ------------------------------------------------------------ anchor
# present: the key exists or could not be opened; ok: one REG_SZ value
# holding a valid anchor document. Opened read only: nothing is created.
function Read-P9SStateAnchor($C) {
    $r = [pscustomobject]@{ present = $false; ok = $false; doc = $null; reasons = [System.Collections.Generic.List[string]]::new() }
    $k = $null
    try { $k = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey($C.AnchorKey, $false) }
    catch { $r.present = $true; $r.reasons.Add('ANCHOR_KEY_NOT_READABLE'); return $r }
    if ($null -eq $k) { return $r }
    $r.present = $true
    try {
        if (@($k.GetValueNames()) -cnotcontains $C.AnchorValue) { $r.reasons.Add('ANCHOR_VALUE_ABSENT'); return $r }
        if ($k.GetValueKind($C.AnchorValue) -ne [Microsoft.Win32.RegistryValueKind]::String) { $r.reasons.Add('ANCHOR_VALUE_KIND'); return $r }
        $v = [string]$k.GetValue($C.AnchorValue, $null, [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
        $d = ConvertFrom-P9SStateJson ([Text.UTF8Encoding]::new($false).GetBytes($v))
        if ($null -eq $d) { $r.reasons.Add('ANCHOR_NOT_PARSEABLE'); return $r }
        foreach ($x in @(Get-P9SStateDocReasons 'anchor' $d)) { $r.reasons.Add("ANCHOR_$x") }
        if (-not $r.reasons.Count) { $r.ok = $true; $r.doc = $d }
    } catch { $r.reasons.Add('ANCHOR_VALUE_NOT_READABLE') } finally { $k.Dispose() }
    $r
}

# ------------------------------------------------------------ coordinator folders of earlier versions
# evidence: anything that shows a coordinator used the folder (any entry but
# the monitor's files and temporary files), or a folder that cannot be read
# (its content is unknown). -Hashes also reads the records and histories.
function Get-P9SStateLegacyScan([string]$Dir, [switch]$Hashes) {
    $s = [pscustomobject]@{ dir = $Dir; kind = $null; evidence = $false; listed = $false; reasons = [System.Collections.Generic.List[string]]::new(); entries = @(); marker = $null; identity = $null }
    $e = Get-P9SStateEntry $Dir
    $s.kind = $e.kind; $s.identity = "$($e.volume):$($e.index)"
    if ($e.kind -eq 'absent') { return $s }
    $s.evidence = $true
    if ($e.kind -in 'denied', 'error') { $s.reasons.Add('LEGACY_FOLDER_NOT_ACCESSIBLE'); return $s }
    if ($e.kind -ne 'directory') { $s.reasons.Add('LEGACY_PATH_NOT_A_FOLDER'); return $s }
    if ($e.reparse) { $s.reasons.Add('LEGACY_FOLDER_REPARSE_POINT'); return $s }
    $l = Get-P9SStateListing $Dir
    if (-not $l.ok) { $s.reasons.Add('LEGACY_FOLDER_NOT_LISTABLE'); return $s }
    $s.listed = $true
    $list = [System.Collections.Generic.List[object]]::new()
    foreach ($i in $l.items) {
        $cls = Get-P9SStateFileClass $i.Name
        $plain = ($i -is [IO.FileInfo]) -and -not ($i.Attributes -band [IO.FileAttributes]::ReparsePoint)
        if (-not $plain) { $cls = 'unknown' }
        $o = [pscustomobject]@{ name = $i.Name; class = $cls; length = $(if ($plain) { [long]$i.Length } else { $null }); sha256 = $null; read = $null }
        if ($cls -eq 'marker') {
            $m = Read-P9SStateRecord $i.FullName 'marker'
            $s.marker = [pscustomobject]@{ ok = $m.ok; installation_id = $(if ($m.ok) { [string]$m.doc.installation_id } else { $null }); reasons = $m.reasons }
        }
        if ($Hashes -and $cls -in 'record', 'history') { $b = Read-P9SStateBytes $i.FullName; $o.read = $b.kind; if ($b.kind -eq 'ok') { $o.sha256 = Get-P9SStateSha $b.bytes; $o.length = [long]$b.bytes.Length } }
        $list.Add($o)
    }
    $s.entries = @($list)
    $s.evidence = [bool]@($list | Where-Object { $_.class -notin 'monitor', 'temporary' }).Count
    $s
}

# ------------------------------------------------------------ ledger of the state folder
function Get-P9SStateLedgerReasons($C, $A) {
    $nx = [pscustomobject]@{ LedgerPath = Join-Path $C.StateDir 'acpi-validation-ledger.json'; SessionPath = Join-Path $C.StateDir 'coordinator-session.json'; StopRequestPath = Join-Path $C.StateDir 'coordinator-stop.request' }
    foreach ($p in @($nx.LedgerPath, [IO.Path]::ChangeExtension($nx.LedgerPath, '.revision.json'), $nx.SessionPath)) {
        $e = Get-P9SStateEntry $p
        if ($e.kind -in 'denied', 'error') { return @("LEDGER_FILE_NOT_ACCESSIBLE:$([IO.Path]::GetFileName($p))") }
        if ($e.kind -eq 'directory') { return @("LEDGER_FILE_NOT_A_FILE:$([IO.Path]::GetFileName($p))") }
    }
    # the toolkit's own reader first: what it refuses, refuses here too
    $lg = Read-P9SAcpiLedger $nx
    if (-not $lg.ok) { return @("LEDGER:$($lg.reason)") }
    if ($lg.detail -ceq 'absent, never used') { return @('LEDGER:ACPI_LEDGER_MISSING') }
    $b = Read-P9SStateBytes $nx.LedgerPath
    if ($b.kind -ne 'ok') { return @("LEDGER_NOT_READ:$($b.kind)") }
    $rev = Get-P9SStateLedgerRevision $b.bytes
    if ($null -eq $rev) { return @('LEDGER:REVISION_UNREADABLE') }
    $mark = $A.ledger
    if ($rev -lt [long]$mark.revision) { return @('LEDGER:STATE_STALE') }
    if ($rev -eq [long]$mark.revision -and (Get-P9SStateSha $b.bytes) -cne [string]$mark.sha256) { return @('LEDGER:ACPI_LEDGER_INCONSISTENT') }
    @()
}

# ------------------------------------------------------------ the status
# The first match wins, in the order of the design (R14/X5 states.txt);
# SANDBOX comes first: it concerns a folder other than the state folder.
function Get-P9SStateStatus($C) {
    $st = [ordered]@{ status = $null; reasons = [System.Collections.Generic.List[string]]::new(); notes = [System.Collections.Generic.List[string]]::new()
        installation_id = $null; phase = $null; origin = $null; state_dir = $C.StateDir; anchor = 'absent'; legacy = @() }
    $done = { param([string]$S, $Why) $st.status = $S; foreach ($x in @($Why)) { if ($x) { $st.reasons.Add([string]$x) } }; [pscustomobject]$st }
    $n = Get-P9SStateNames
    if ($C.AskedFolder -and -not (Test-P9SStateSamePath $C.AskedFolder $C.StateDir)) { return (& $done 'SANDBOX' 'ASKED_FOLDER_IS_NOT_THE_STATE_FOLDER') }
    $loc = @(Get-P9SStateLocationReasons $C)
    $anc = if ($C.Native) { Read-P9SStateAnchor $C } else { [pscustomobject]@{ present = $true; ok = $false; doc = $null; reasons = @('ANCHOR_NOT_READ') } }
    $st.anchor = if (-not $anc.present) { 'absent' } elseif ($anc.ok) { 'present' } else { 'unreadable' }
    if ($loc.Count) { return (& $done 'LOCATION_UNSAFE' $loc) }
    if ($anc.present -and -not $anc.ok) { return (& $done 'ANCHOR_UNREADABLE' $anc.reasons) }
    if (-not $anc.present) {
        $nItems = if ((Get-P9SStateEntry $C.StateDir).kind -eq 'directory') { @((Get-P9SStateListing $C.StateDir).items | Where-Object { (Get-P9SStateFileClass $_.Name) -notin 'monitor', 'temporary' }) } else { @() }
        $rItems = if ((Get-P9SStateEntry $C.Root).kind -eq 'directory') { @((Get-P9SStateListing $C.Root).items | Where-Object { $_.Name -cne $n.setup_lock -and $_.Name -cne $n.state }) } else { @() }
        $scans = @($C.LegacyDirs | ForEach-Object { Get-P9SStateLegacyScan $_ })
        $ev = [System.Collections.Generic.List[object]]::new()
        foreach ($x in @($scans | Where-Object evidence)) { if (-not @($ev | Where-Object { $_.kind -eq 'directory' -and $x.kind -eq 'directory' -and $_.identity -eq $x.identity }).Count) { $ev.Add($x) } }
        $st.legacy = @($scans | ForEach-Object { [pscustomobject]@{ dir = $_.dir; kind = $_.kind; evidence = $_.evidence; entries = @($_.entries).Count; marker = [bool]$_.marker; reasons = @($_.reasons) } })
        $lost = @($ev | Where-Object { $_.marker })
        if ($nItems.Count) { $ir = Read-P9SStateRecord (Join-Path $C.StateDir $n.install) 'install'; if ($ir.ok) { $st.installation_id = [string]$ir.doc.installation_id }; return (& $done 'ANCHOR_MISSING' "STATE_FOLDER_HOLDS:$(@($nItems | ForEach-Object Name | Sort-Object) -join ',')") }
        if ($rItems.Count) { return (& $done 'SETUP_LEFTOVER' "ROOT_HOLDS:$(@($rItems | ForEach-Object Name | Sort-Object) -join ',')") }
        if ($lost.Count) { $st.installation_id = $lost[0].marker.installation_id; return (& $done 'INSTALLATION_LOST' "MARKER_IN:$($lost[0].dir)") }
        if ($ev.Count -gt 1) { return (& $done 'MIGRATION_SOURCES_AMBIGUOUS' @($ev | ForEach-Object { "EVIDENCE_IN:$($_.dir)" })) }
        if ($ev.Count -eq 1) { return (& $done 'MIGRATION_REQUIRED' (@("EVIDENCE_IN:$($ev[0].dir)") + @($ev[0].reasons))) }
        return (& $done 'NOT_INITIALIZED' $null)
    }
    $A = $anc.doc
    $st.installation_id = [string]$A.installation_id; $st.phase = [string]$A.phase; $st.origin = [string]$A.origin
    if ($A.phase -cne 'READY') { return (& $done 'INTERRUPTED' "ANCHOR_PHASE:$($A.phase)") }
    if ([string]$A.account -cne [string]$C.Process.account) { return (& $done 'ACCOUNT_MISMATCH' 'ANCHOR_ACCOUNT') }
    if (-not (Test-P9SStateSamePath $A.state_dir $C.StateDir)) { return (& $done 'STATE_MISMATCH' 'STATE_DIR') }
    if ((Get-P9SStateEntry $C.StateDir).kind -eq 'absent') { return (& $done 'STATE_MISSING' 'STATE_FOLDER_ABSENT') }
    $ir = Read-P9SStateRecord (Join-Path $C.StateDir $n.install) 'install'
    if (-not $ir.ok) { return (& $done 'STATE_INCOMPLETE' @($ir.reasons | ForEach-Object { "INSTALL_RECORD_$_" })) }
    $I = $ir.doc
    if ([string]$I.installation_id -cne [string]$A.installation_id) { return (& $done 'STATE_MISMATCH' 'INSTALLATION_ID') }
    foreach ($k in 'account', 'origin') { if ([string]$I[$k] -cne [string]$A[$k]) { return (& $done 'STATE_MISMATCH' "INSTALL_RECORD_DIFFERS:$k") } }
    if (-not (Test-P9SStateSamePath $I.state_dir $C.StateDir)) { return (& $done 'STATE_MISMATCH' 'INSTALL_RECORD_DIFFERS:state_dir') }
    $lr = @(Get-P9SStateLedgerReasons $C $A)
    if ($lr.Count) { return (& $done 'STATE_UNCERTAIN' $lr) }
    $mr = Read-P9SStateRecord (Join-Path $C.StateDir $n.migration) 'migration'
    if ([string]$A.origin -ceq 'new') {
        if ($mr.kind -ne 'absent') { return (& $done 'STATE_INCOMPLETE' 'MIGRATION_RECORD_UNEXPECTED') }
    } else {
        if (-not $mr.ok) { return (& $done 'STATE_INCOMPLETE' @($mr.reasons | ForEach-Object { "MIGRATION_RECORD_$_" })) }
        $M = $mr.doc
        $dg = Get-P9SStateManifestDigest $M.files $M.absent ([string]$M.source_dir)
        if ([string]$M.installation_id -cne [string]$A.installation_id -or [string]$M.migration_id -cne [string]$I.migration_id -or $dg -cne [string]$M.manifest_sha256 -or $dg -cne [string]$A.manifest_sha256 -or -not (Test-P9SStateSamePath $M.source_dir $A.source_dir)) {
            return (& $done 'STATE_INCOMPLETE' 'MIGRATION_RECORD_INCONSISTENT')
        }
        # the old folder is compared with the manifest, at a location the
        # toolkit computes; the recorded path is compared, never followed
        $src = @($C.LegacyDirs | Where-Object { Test-P9SStateSamePath $_ $M.source_dir })
        if (-not $src.Count) { return (& $done 'STATE_DIVERGED' 'OLD_FOLDER_NOT_A_KNOWN_LOCATION') }
        $sc = Get-P9SStateLegacyScan $src[0] -Hashes
        if ($sc.kind -eq 'absent') { $st.notes.Add('OLD_FOLDER_GONE') }
        elseif (-not $sc.listed) { return (& $done 'STATE_DIVERGED' (@('OLD_FOLDER_NOT_VERIFIABLE') + @($sc.reasons))) }
        else {
            $known = @{}; foreach ($x in @($M.files)) { $known[[string]$x.name] = [string]$x.sha256 }
            foreach ($x in @($sc.entries | Where-Object class -eq 'record')) {
                if ($x.read -ne 'ok') { return (& $done 'STATE_DIVERGED' "OLD_FOLDER_NOT_VERIFIABLE:$($x.name)") }
                if (-not $known.ContainsKey($x.name) -or $known[$x.name] -cne $x.sha256) { return (& $done 'STATE_DIVERGED' "OLD_FOLDER_CHANGED:$($x.name)") }
            }
            if (@($sc.entries | Where-Object { $_.class -eq 'history' -and (-not $known.ContainsKey($_.name) -or $known[$_.name] -cne $_.sha256) }).Count) { $st.notes.Add('OLD_FOLDER_HISTORY_CHANGED') }
            if (@($sc.entries | Where-Object class -eq 'unknown').Count) { $st.notes.Add('OLD_FOLDER_UNKNOWN_ENTRY') }
            foreach ($x in @($M.files | Where-Object { $_.class -ceq 'record' })) { if (-not @($sc.entries | Where-Object name -ceq $x.name).Count) { $st.notes.Add("OLD_FOLDER_RECORD_GONE:$($x.name)") } }
        }
    }
    foreach ($i in @((Get-P9SStateListing $C.Root).items | Where-Object { $_.Name -cne $n.setup_lock -and $_.Name -cne $n.state })) { $st.notes.Add("ROOT_HOLDS:$($i.Name)") }
    & $done 'READY' $null
}

# ------------------------------------------------------------ report
function Format-P9SStateStatus($St, $C) {
    "state folder: $(if ($C.StateDir) { $C.StateDir } else { '(not resolved)' }) (known folder LocalApplicationData; no action of this version uses it)"
    "status: $($St.status)"
    if (@($St.reasons).Count) { "reasons: $(@($St.reasons) -join ', ')" }
    if (@($St.notes).Count) { "notes: $(@($St.notes) -join ', ')" }
    if ($St.status -eq 'SANDBOX') { "the folder of -StatePath ($($C.AskedFolder)) is not the state folder; nothing else was read" ; return }
    "anchor: HKCU\$($C.AnchorKey) value $($C.AnchorValue): $($St.anchor)"
    if ($St.installation_id) { "installation: $($St.installation_id)$(if ($St.phase) { ", anchor phase $($St.phase)" })$(if ($St.origin) { ", origin $($St.origin)" })" }
    foreach ($l in @($St.legacy)) { "earlier coordinator folder $($l.dir): $($l.kind)$(if ($l.kind -eq 'directory') { ", $($l.entries) entries, evidence $($l.evidence)$(if ($l.marker) { ', migration marker' })" })$(if (@($l.reasons).Count) { ' (' + (@($l.reasons) -join ', ') + ')' })" }
    "process: package code $($C.Process.package_code), app container $($C.Process.app_container), virtualization $($C.Process.virtualization)"
    "this version's actions keep using $(Split-Path -Parent (Get-DefaultStatePath)) (unchanged); the status gates nothing yet"
    'read only: no file, folder or registry key was created or changed'
}
