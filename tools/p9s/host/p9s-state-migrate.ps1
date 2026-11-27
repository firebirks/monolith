#Requires -Version 7.0
# P9-S persistent state: the explicit migration of the coordinator folder of
# earlier versions (%TEMP%\p9s-hostmon) into a new installation
# (state.migrate, state.migrate -Restart <operation id>).
#
# Only the action state.migrate calls Invoke-P9SStateMigrate: no other action
# migrates, and nothing migrates by itself. The decision is the status of
# p9s-state.ps1 (read only, unchanged), computed before anything is written
# and again under the setup lock:
#   MIGRATION_REQUIRED       a new migration of the one earlier folder that
#                            holds evidence;
#   INTERRUPTED (anchor in phase MIGRATING)
#                            the same migration, resumed, only while the
#                            earlier folder still matches its manifest;
#   with -Restart <operation id>, INTERRUPTED with that migration's anchor,
#   no state folder in place and no marker written: the same migration
#   started again with a new manifest (the earlier folder changed while it
#   was interrupted);
#   anything else            refused (an empty or new-looking folder is
#                            never a reason to migrate or to initialize).
# The earlier folder is read with the toolkit's own readers (the ledger and
# its revision record, the stop request, the session record, the request
# history of every configured VM) and with its audit as a second witness;
# an uncertain history, an unknown, linked, unreadable or foreign-writable
# entry, or a VirtualBox operation or quarantine record refuses before
# anything is written. Every configured VM must be observed off: the process
# count before anything is written, the VM states through the VirtualBox
# layer once the copies are in the staging folder (the observations continue
# the copied histories).
# Steps (each a crash point; the anchor is flushed after each write):
#   P0  the decision and the checks; nothing is written before this point
#   M1  root (protected ACL at creation) and the setup lock, no wait
#   M2  the earlier folder's coordinator.lock and coordinator-emergency.hold,
#       both exclusive, no wait, held to the end: no coordinator of any
#       version can act there meanwhile; the folder read again under them
#   M3  anchor: phase MIGRATING, the manifest digest (-Restart: the value it
#       replaces kept as MigrationRestarted-<op>-<n> first)
#   M4  staging folder state.staging-<op>-<n> (protected ACL, same volume)
#   M5  each record and history copied: read, checked against the manifest,
#       written beside its name, renamed without replacing, read back
#   V   VM states, observed into the staging folder
#   M6  only when the earlier folder never held a ledger and its audit shows
#       no ACPI step: the empty ledger at revision 1 and its revision record
#   M7  migration.json and install.json (exclusive create, write-through)
#   M8  the staging folder checked (records byte for byte, histories that
#       begin with the copied bytes, ledger, records, ACL, no other entry),
#       then the earlier folder against the manifest once more
#   M9  the staging folder renamed to state (refused if state exists)
#   M10 MIGRATED-TO.json in the earlier folder (exclusive create)
#   M11 the folder checked again, the anchor checked unchanged, then: phase
#       READY, initialized_utc, the ledger mark; state.status must say READY
#   M12 one line in the new folder's coordinator-audit.jsonl
# Nothing is ever deleted, moved or changed in the earlier folder: it gets
# MIGRATED-TO.json, and coordinator.lock and coordinator-emergency.hold
# (empty) if they were not there. A staging folder left by an interruption
# stays where it is. The coordinator of this version keeps using the earlier
# folder after a migration: nothing here switches it (a later version does).
# Dot-source after hostmon-lib.ps1, coordinator-lib.ps1, p9s-acpival.ps1,
# p9s-state.ps1 and p9s-state-init.ps1.

# ------------------------------------------------------------ names and limits
function Get-P9SStateMigrateNames {
    [ordered]@{ purpose = 'state.migrate'; restart_anchor = 'MigrationRestarted-'; lock = 'coordinator.lock'; hold = 'coordinator-emergency.hold'; audit = 'coordinator-audit.jsonl' }
}
# The staging folder needs room for the copies and for what the migration
# writes beside them: twice the bytes copied plus 64 MiB must be free.
function Get-P9SStateMigrateLimits { [ordered]@{ reserve_bytes = 67108864L; copy_factor = 2L } }
# The validation steps that write the ledger: one of them in the audit
# means the ledger was used.
function Get-P9SStateLedgerActions { 'acpi.open', 'acpi.start', 'acpi.observe', 'acpi.shutdown', 'acpi.confirm-off', 'acpi.integrity', 'acpi.finalize' }
function Get-P9SStateNotActed { 'REFUSED', 'REJECTED', 'DRYRUN' }
function New-P9SStateMigrateOpId([datetime]$Utc) { 'migrate-' + $Utc.ToString("yyyyMMdd'T'HHmmss'Z'", [Globalization.CultureInfo]::InvariantCulture) + '-' + [guid]::NewGuid().ToString('N').Substring(0, 8) }

# ------------------------------------------------------------ context
# The context of state.init plus the free space of the state volume. Tests
# inject every fact; the command line offers none of them.
function New-P9SStateMigrateContext {
    param([Parameter(Mandatory)]$State, [Parameter(Mandatory)][scriptblock]$ReadVmStateIn, [Parameter(Mandatory)][scriptblock]$CountVmProcesses,
          [string[]]$VmNames, [scriptblock]$UtcNow, [scriptblock]$NowQpc, [long]$QpcFrequency = 0, [scriptblock]$OnStep, [scriptblock]$FreeBytes)
    $a = @{ State = $State; ReadVmStateIn = $ReadVmStateIn; CountVmProcesses = $CountVmProcesses; QpcFrequency = $QpcFrequency }
    foreach ($k in 'VmNames', 'UtcNow', 'NowQpc', 'OnStep') { if ($PSBoundParameters.ContainsKey($k)) { $a[$k] = $PSBoundParameters[$k] } }
    $i = New-P9SStateInitContext @a
    $i | Add-Member -NotePropertyName FreeBytes -NotePropertyValue $(if ($FreeBytes) { $FreeBytes } else { { param($dir) [long][IO.DriveInfo]::new([IO.Path]::GetPathRoot($dir)).AvailableFreeSpace } })
    $i
}
function New-P9SStateMigrateRealContext($State) {
    New-P9SStateMigrateContext -State $State -VmNames @(Get-P9SVmNames) `
        -ReadVmStateIn { param($vm, $dir) Get-P9SVmStateReal $vm (New-P9SVBoxContext -Dir $dir -VmNames @(Get-P9SVmNames)) } `
        -CountVmProcesses { @(Get-Process -Name 'VirtualBoxVM', 'VBoxHeadless' -ErrorAction SilentlyContinue).Count }
}

# ------------------------------------------------------------ the earlier folder
# Every entry by name and kind, the bytes and digest of each record and
# history, the manifest digest. reasons: why the folder cannot be migrated
# as it is (nothing is skipped: an entry the migration does not know, cannot
# read, or that another path can change refuses it); notes: what is only
# reported (write entries of other accounts).
function Get-P9SStateSourceScan([string]$Dir, [string]$Account) {
    $s = [pscustomobject]@{ dir = $Dir; reasons = [System.Collections.Generic.List[string]]::new(); notes = [System.Collections.Generic.List[string]]::new(); files = [System.Collections.Generic.List[object]]::new()
        absent = @(); ignored = [System.Collections.Generic.List[string]]::new(); locks = [System.Collections.Generic.List[string]]::new(); marker = $null; written = [datetime]::MinValue; digest = $null }
    $e = Get-P9SSetupEntry $Dir
    if ($e.kind -eq 'absent') { $s.reasons.Add('MIGRATION_SOURCE_GONE'); return $s }
    if ($e.kind -ne 'directory') { $s.reasons.Add("MIGRATION_SOURCE_$($e.kind.ToUpperInvariant())"); return $s }
    if ($e.reparse) { $s.reasons.Add('MIGRATION_SOURCE_REPARSE_POINT'); return $s }
    # the earlier folder's ACL is reported, never changed and never a reason
    # to refuse what can be read: the copies get the protected ACL of the
    # staging folder, and a writer outside the toolkit is detected by the
    # second reading and, after the migration, by state.status
    foreach ($x in @(Get-P9SStateAclReasons $Dir $Account)) { $s.notes.Add("SOURCE_ACL:$x") }
    $l = Get-P9SStateListing $Dir
    if (-not $l.ok) { $s.reasons.Add('MIGRATION_SOURCE_NOT_LISTABLE'); return $s }
    $cls = Get-P9SStateFileClasses
    $items = [object[]]@($l.items); $keys = [string[]]@($items | ForEach-Object Name)
    [Array]::Sort($keys, $items, [StringComparer]::Ordinal)
    foreach ($i in $items) {
        $n = $i.Name
        $x = Get-P9SSetupEntry $i.FullName
        if ($x.kind -eq 'absent') { $s.reasons.Add("MIGRATION_SOURCE_ENTRY_VANISHED:$n"); continue }
        if ($x.kind -in 'denied', 'error') { $s.reasons.Add("MIGRATION_SOURCE_ENTRY_NOT_ACCESSIBLE:$n"); continue }
        if ($x.reparse) { $s.reasons.Add("MIGRATION_SOURCE_ENTRY_REPARSE_POINT:$n"); continue }
        if ($x.kind -ne 'file') { $s.reasons.Add("MIGRATION_SOURCE_ENTRY_NOT_A_FILE:$n"); continue }
        $c = Get-P9SStateFileClass $n
        if ($c -eq 'unknown') { $s.reasons.Add("MIGRATION_SOURCE_UNKNOWN_ENTRY:$n"); continue }
        if ($c -in 'monitor', 'temporary') { $s.ignored.Add($n); continue }
        if ($x.links -gt 1) { $s.reasons.Add("MIGRATION_SOURCE_ENTRY_HARD_LINKED:$n"); continue }
        if ($c -eq 'lock') { $s.locks.Add($n); continue }
        if ($c -eq 'marker') { $m = Read-P9SStateRecord $i.FullName 'marker'; $s.marker = [pscustomobject]@{ ok = $m.ok; doc = $m.doc; reasons = @($m.reasons) }; continue }
        foreach ($y in @(Get-P9SStateAclReasons $i.FullName $Account)) { $s.notes.Add("SOURCE_ACL:$($n):$y") }
        $b = Read-P9SStateBytes $i.FullName
        if ($b.kind -ne 'ok') { $s.reasons.Add("MIGRATION_SOURCE_NOT_READABLE:$($n):$($b.kind)"); continue }
        $s.files.Add([pscustomobject]@{ name = $n; class = $c; length = [long]$b.bytes.Length; sha256 = (Get-P9SStateSha $b.bytes); bytes = $b.bytes })
        try { $w = [IO.File]::GetLastWriteTimeUtc($i.FullName); if ($w -gt $s.written) { $s.written = $w } } catch { $s.reasons.Add("MIGRATION_SOURCE_NOT_READABLE:$($n):time") }
    }
    $s.absent = @($cls.record | Where-Object { $_ -cnotin @($s.files | ForEach-Object name) })
    $s.digest = Get-P9SStateManifestDigest @(Get-P9SStateManifestFiles $s) $s.absent $Dir
    $s
}
# the manifest entries: exactly name, class, length and digest, by name
function Get-P9SStateManifestFiles($Scan) { @($Scan.files | ForEach-Object { [ordered]@{ name = $_.name; class = $_.class; length = [long]$_.length; sha256 = $_.sha256 } }) }
# The SHA-256 of the first $Length bytes ($null when there are fewer).
function Get-P9SStatePrefixSha([byte[]]$Bytes, [long]$Length) {
    if ($Length -lt 0 -or $Bytes.Length -lt $Length) { return $null }
    $h = [Security.Cryptography.SHA256]::Create()
    try { [Convert]::ToHexString($h.ComputeHash($Bytes, 0, [int]$Length)).ToLowerInvariant() } finally { $h.Dispose() }
}
# The earlier folder against the manifest of a migration whose state folder
# is in place, by the rules state.status applies once it is READY: a record
# new, changed or unreadable there diverges; a history that grew, or a
# record that is gone, is only noted.
function Get-P9SStateSourceAgainst($Scan, $M) {
    $o = [pscustomobject]@{ reasons = [System.Collections.Generic.List[string]]::new(); notes = [System.Collections.Generic.List[string]]::new() }
    $known = @{}; foreach ($x in @($M.files)) { $known[[string]$x.name] = [string]$x.sha256 }
    foreach ($x in @($Scan.files)) {
        if ($x.class -ceq 'record') { if (-not $known.ContainsKey($x.name) -or $known[$x.name] -cne $x.sha256) { $o.reasons.Add("MIGRATION_SOURCE_CHANGED:$($x.name)") } }
        elseif (-not $known.ContainsKey($x.name) -or $known[$x.name] -cne $x.sha256) { $o.notes.Add("OLD_FOLDER_HISTORY_CHANGED:$($x.name)") }
    }
    foreach ($x in @($M.files | Where-Object { [string]$_.class -ceq 'record' })) { if (-not @($Scan.files | Where-Object { $_.name -ceq [string]$x.name }).Count) { $o.notes.Add("OLD_FOLDER_RECORD_GONE:$($x.name)") } }
    $o
}
# What a resumed migration may continue from. Before the rename the earlier
# folder must still be exactly its manifest; once the state folder is in
# place it is compared with the manifest kept there, by the rules above.
function Get-P9SStateResumeReasons($I, $D, $View) {
    $C = $I.State; $n = Get-P9SStateNames; $A = $D.anchor
    $o = [pscustomobject]@{ reasons = [System.Collections.Generic.List[string]]::new(); notes = [System.Collections.Generic.List[string]]::new(); way = $null }
    $ne = Get-P9SSetupEntry $C.StateDir
    if ($ne.kind -eq 'absent') {
        if ($View.digest -cne [string]$A.manifest_sha256) {
            $o.reasons.Add('MIGRATION_SOURCE_CHANGED')
            $o.way = if (-not $View.scan.marker) { "the earlier folder changed since the migration began; state.migrate -Restart $($A.op_id) starts it again with a new manifest" } else { 'a marker without a state folder: see state.status; nothing is chosen automatically' }
        }
        return $o
    }
    if ($ne.kind -ne 'directory') { $o.reasons.Add("STATE_FOLDER_$($ne.kind.ToUpperInvariant())"); return $o }
    $mr = Read-P9SStateRecord (Join-Path $C.StateDir $n.migration) 'migration'
    $ir = Read-P9SStateRecord (Join-Path $C.StateDir $n.install) 'install'
    if (-not $mr.ok -or -not $ir.ok -or [string]$mr.doc.migration_id -cne [string]$A.op_id -or [string]$mr.doc.manifest_sha256 -cne [string]$A.manifest_sha256 -or [string]$ir.doc.installation_id -cne [string]$A.installation_id) {
        $o.reasons.Add('STATE_FOLDER_NOT_THIS_MIGRATION'); return $o
    }
    $x = Get-P9SStateSourceAgainst $View.scan $mr.doc
    foreach ($y in $x.reasons) { $o.reasons.Add($y) }
    foreach ($y in $x.notes) { $o.notes.Add($y) }
    if ($o.reasons.Count) { $o.way = 'a record of the earlier folder changed after the copy: put it back as migration.json of the state folder lists it; nothing is chosen automatically' }
    $o
}

# The coordinator audit of the earlier folder as a second witness of its
# history: every complete line must be one JSON object (the last may be cut
# by an interrupted append: noted, copied as it is).
function Get-P9SStateAuditWitness($Scan, $Ledger, $Stop) {
    $w = [pscustomobject]@{ present = $false; entries = 0; reasons = [System.Collections.Generic.List[string]]::new(); notes = [System.Collections.Generic.List[string]]::new(); ledger_used = $false }
    $f = @($Scan.files | Where-Object { $_.name -ceq (Get-P9SStateMigrateNames).audit })
    if (-not $f.Count) { return $w }
    $w.present = $true
    $t = $null
    try { $t = [Text.UTF8Encoding]::new($false, $true).GetString($f[0].bytes) } catch { $w.reasons.Add('MIGRATION_SOURCE_UNCERTAIN:AUDIT_NOT_UTF8'); return $w }
    $parts = $t.Split("`n")
    $open = [System.Collections.Generic.List[object]]::new()
    for ($i = 0; $i -lt $parts.Count; $i++) {
        $line = $parts[$i].TrimEnd("`r")
        if (-not $line.Trim()) { continue }
        $last = ($i -eq $parts.Count - 1)
        $x = $null; try { $x = $line | ConvertFrom-Json -AsHashtable -ErrorAction Stop } catch { $x = $null }
        if ($x -isnot [System.Collections.IDictionary]) {
            if ($last) { $w.notes.Add('AUDIT_LAST_LINE_CUT') } else { $w.reasons.Add("MIGRATION_SOURCE_UNCERTAIN:AUDIT_LINE_UNREADABLE:$($i + 1)") }
            continue
        }
        $w.entries++
        $act = [string]$x.action; $out = [string]$x.outcome; $dry = ($x.dry_run -eq $true)
        if ($act -ceq 'session.close' -and $out -ceq 'DONE') { $open.Clear(); continue }
        if ($dry -or $out -cin (Get-P9SStateNotActed)) { continue }
        if ($act -cin (Get-P9SStateLedgerActions)) { $w.ledger_used = $true }
        if ($act -ceq 'emergency.acpi-shutdown') { $open.Add($x) }
    }
    # a validation step recorded, and no ledger that it wrote
    if ($w.ledger_used -and $Ledger.ok -and $Ledger.detail -ceq 'absent, never used') { $w.reasons.Add('MIGRATION_SOURCE_UNCERTAIN:LEDGER_USED_BUT_MISSING') }
    # an emergency since the last close raised the stop request, which only a
    # close removes; a request it dispatched is recorded in it per VM
    if ($open.Count) {
        if (-not $Stop.present) { $w.reasons.Add('MIGRATION_SOURCE_UNCERTAIN:STOP_REQUEST_USED_BUT_MISSING') }
        elseif ($Stop.ok) {
            foreach ($x in $open) {
                if (-not ($x.dispatched -is [ValueType] -and [long]$x.dispatched -gt 0)) { continue }
                $vm = [string]$x.vm
                $held = if ($vm) { $Stop.doc.acpi.Contains($vm) -or $Stop.doc.unresolved.Contains($vm) } else { (@($Stop.doc.acpi.Keys).Count + @($Stop.doc.unresolved.Keys).Count) -gt 0 }
                if (-not $held) { $w.reasons.Add("MIGRATION_SOURCE_UNCERTAIN:STOP_REQUEST_USED_BUT_MISSING$(if ($vm) { ":$vm" })") }
            }
        }
    }
    $w
}
# The earlier folder read as the toolkit reads it, then scanned again: the
# readers' verdict must be about the bytes of the manifest.
function Get-P9SStateSourceView($I, [string]$Dir) {
    $C = $I.State; $acct = [string]$C.Process.account
    $v = [pscustomobject]@{ dir = $Dir; reasons = [System.Collections.Generic.List[string]]::new(); notes = [System.Collections.Generic.List[string]]::new(); scan = $null; digest = $null
        never_used = $false; ledger_detail = $null; audit_entries = 0 }
    $s = Get-P9SStateSourceScan $Dir $acct
    $v.scan = $s
    foreach ($x in $s.reasons) { $v.reasons.Add($x) }
    foreach ($x in $s.notes) { $v.notes.Add($x) }
    if ($s.reasons.Count) { return $v }
    $sc = [pscustomobject]@{ LedgerPath = Join-Path $Dir 'acpi-validation-ledger.json'; SessionPath = Join-Path $Dir 'coordinator-session.json'; StopRequestPath = Join-Path $Dir 'coordinator-stop.request' }
    $lg = Read-P9SAcpiLedger $sc
    $v.ledger_detail = if ($lg.ok) { $lg.detail } else { "$($lg.reason) ($($lg.detail))" }
    if (-not $lg.ok) { $v.reasons.Add("MIGRATION_SOURCE_UNCERTAIN:$($lg.reason)") }
    $sq = Read-P9SStopRequestState $sc
    if ($sq.present -and -not $sq.ok) { $v.reasons.Add('MIGRATION_SOURCE_UNCERTAIN:STOP_REQUEST_UNREADABLE') }
    $sr = Read-P9SSession $sc
    if (-not $sr.ok) { $v.reasons.Add('MIGRATION_SOURCE_UNCERTAIN:SESSION_UNREADABLE') }
    foreach ($vm in @($I.VmNames)) {
        $h = Get-P9SAcpiRequestHistory $sc $vm
        if ($h.uncertain) { $v.reasons.Add("MIGRATION_SOURCE_UNCERTAIN:REQUEST_HISTORY:$vm") }
    }
    $w = Get-P9SStateAuditWitness $s $lg $sq
    foreach ($x in $w.reasons) { $v.reasons.Add($x) }
    foreach ($x in $w.notes) { $v.notes.Add($x) }
    $v.audit_entries = $w.entries
    $v.never_used = [bool]($lg.ok -and $lg.detail -ceq 'absent, never used')
    # a ledger that never existed is believed only with an audit that shows it
    if ($v.never_used -and (-not $w.present -or $w.entries -eq 0)) { $v.reasons.Add('MIGRATION_SOURCE_UNCERTAIN:ACPI_HISTORY_UNVERIFIABLE') }
    $s2 = Get-P9SStateSourceScan $Dir $acct
    if ($s2.reasons.Count -or $s2.digest -cne $s.digest) { $v.reasons.Add('MIGRATION_SOURCE_CHANGED') }
    $v.digest = $s.digest
    $v
}

# ------------------------------------------------------------ the decision
# What state.migrate may do with what exists, from the status alone; nothing
# is written. mode: new, resume, restart, or $null (refused, with reasons).
# The earlier folder is always one of the locations the toolkit computes; a
# path in a record is compared with them, never followed.
function Get-P9SStateMigrateDecision($I, [string]$Restart) {
    $C = $I.State
    $st = Get-P9SStateStatus $C
    $a = if ($C.Native) { Read-P9SStateAnchor $C } else { $null }
    $d = [pscustomobject]@{ status = $st.status; status_reasons = @($st.reasons); installation_id = $st.installation_id; mode = $null; reasons = [System.Collections.Generic.List[string]]::new()
        anchor = $(if ($a -and $a.ok) { $a.doc } else { $null }); source = $null; exit = $null }
    $s = [string]$st.status
    $mig = [bool]($s -ceq 'INTERRUPTED' -and $d.anchor -and [string]$d.anchor.phase -ceq 'MIGRATING' -and [string]$d.anchor.origin -ceq 'migrated')
    if ($Restart) {
        if (-not $mig) { $d.reasons.Add("NOTHING_TO_RESTART:$s"); $d.exit = 'state.migrate -Restart only starts an interrupted migration again' }
        elseif ([string]$d.anchor.op_id -cne $Restart) { $d.reasons.Add('RESTART_ID_MISMATCH'); $d.exit = "the interrupted migration is $($d.anchor.op_id)" }
        else { $d.mode = 'restart' }
    } else {
        if ($s -ceq 'MIGRATION_REQUIRED') { $d.mode = 'new' }
        elseif ($mig) { $d.mode = 'resume' }
        else {
            $d.reasons.Add("NOT_MIGRATABLE:$s")
            $d.exit = switch -CaseSensitive ($s) {
                'NOT_INITIALIZED' { 'no earlier coordinator folder holds evidence: there is nothing to migrate, and nothing is initialized by this command' }
                'READY' { 'the installation is in place; nothing to do' }
                'INTERRUPTED' { 'state.init completes the interrupted initialization' }
                'MIGRATION_SOURCES_AMBIGUOUS' { 'two earlier coordinator folders hold evidence: make one of them the only candidate by hand; nothing is chosen automatically' }
                { $_ -cin (Get-P9SStateSupersedable) } { "state.init -Supersede $($st.installation_id) starts a new installation and keeps this one aside" }
                default { 'see state.status; nothing is chosen automatically' }
            }
        }
    }
    if (-not $d.mode) { return $d }
    if ($d.mode -ceq 'new') {
        $ev = [System.Collections.Generic.List[object]]::new()
        foreach ($x in @($C.LegacyDirs | ForEach-Object { Get-P9SStateLegacyScan $_ } | Where-Object evidence)) { if (-not @($ev | Where-Object { $_.kind -eq 'directory' -and $x.kind -eq 'directory' -and $_.identity -eq $x.identity }).Count) { $ev.Add($x) } }
        if ($ev.Count -ne 1) { $d.mode = $null; $d.reasons.Add("MIGRATION_SOURCE_NOT_SINGLE:$($ev.Count)"); return $d }
        $d.source = [string]$ev[0].dir
    } else {
        $A = $d.anchor
        $src = @($C.LegacyDirs | Where-Object { Test-P9SStateSamePath $_ $A.source_dir })
        if (-not $src.Count) { $d.mode = $null; $d.reasons.Add('MIGRATION_SOURCE_NOT_A_KNOWN_LOCATION'); return $d }
        if ([string]$A.account -cne [string]$C.Process.account -or -not (Test-P9SStateSamePath $A.state_dir $C.StateDir) -or -not (Test-P9SStateSha256 $A.manifest_sha256)) { $d.mode = $null; $d.reasons.Add('ANCHOR_NOT_THIS_MIGRATION'); return $d }
        $d.source = [string]$src[0]
    }
    $d
}

# ------------------------------------------------------------ writers
# A copy: written beside its name (the toolkit's temporary pattern), renamed
# with write-through, never over an existing file.
function Write-P9SStateMigrateCopy($I, [string]$Path, [byte[]]$Bytes, [string]$Step) {
    $full = [IO.Path]::GetFullPath($Path)
    $tmp = [IO.Path]::Combine([IO.Path]::GetDirectoryName($full), '.' + [IO.Path]::GetFileName($full) + '.' + [guid]::NewGuid().ToString('N') + '.tmp')
    Write-P9SStateNewFile $tmp $Bytes
    Invoke-P9SStateInitStep $I "$Step-tmp"
    $e = Move-P9SStateEntry $tmp $full
    if ($e) { throw "rename of $([IO.Path]::GetFileName($full)) failed ($e)" }
    Invoke-P9SStateInitStep $I $Step
}
# Appends to a history, written through. A history whose last line was cut
# by an interrupted append gets a line feed first, so that what follows
# starts on its own line; the bytes before stay as they are.
function Add-P9SStateHistoryBytes([string]$Path, [byte[]]$Bytes) {
    $fs = [IO.FileStream]::new($Path, [IO.FileMode]::OpenOrCreate, [IO.FileAccess]::ReadWrite, [IO.FileShare]::ReadWrite, 4096, [IO.FileOptions]::WriteThrough)
    try {
        $pre = [byte[]]@()
        if ($fs.Length -gt 0) { $null = $fs.Seek(-1, [IO.SeekOrigin]::End); if ($fs.ReadByte() -ne 10) { $pre = [byte[]]@(10) } }
        $null = $fs.Seek(0, [IO.SeekOrigin]::End)
        $all = [byte[]]($pre + $Bytes)
        if ($all.Length) { $fs.Write($all, 0, $all.Length) }
        $fs.Flush($true)
    } finally { $fs.Dispose() }
}
function Close-P9SStateHistoryLine([string]$Path) { if ((Get-P9SSetupEntry $Path).kind -eq 'file') { Add-P9SStateHistoryBytes $Path ([byte[]]@()) } }

# ------------------------------------------------------------ the folder check
# A staging folder, or the state folder in place, must hold exactly what this
# migration put there: each record of the manifest byte for byte and no
# record it lists as absent, each history beginning with the copied bytes,
# a ledger the toolkit's reader accepts (the empty one when it was created),
# the install and migration records of this migration, the protected ACL,
# plain files only, and nothing else.
function Get-P9SStateMigrateFolderCheck($I, $A, [string]$Dir, $M) {
    $C = $I.State; $n = Get-P9SStateNames; $acct = [string]$C.Process.account
    $out = [System.Collections.Generic.List[string]]::new()
    $r = [pscustomobject]@{ reasons = $out; mark = $null }
    $e = Get-P9SSetupEntry $Dir
    if ($e.kind -ne 'directory' -or $e.reparse) { $out.Add("FOLDER_$($e.kind.ToUpperInvariant())"); return $r }
    foreach ($x in @(Get-P9SStateAclReasons $Dir $acct -RequireProtected)) { $out.Add("ACL:$x") }
    $l = Get-P9SStateListing $Dir
    if (-not $l.ok) { $out.Add('NOT_LISTABLE'); return $r }
    $created = [bool]$M.ledger_created
    $allowed = [System.Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
    foreach ($x in @($M.files)) { $null = $allowed.Add([string]$x.name) }
    foreach ($x in 'vm-observations.jsonl', 'vbox-audit.jsonl', $n.install, $n.migration) { $null = $allowed.Add($x) }
    if ($created) { $null = $allowed.Add('acpi-validation-ledger.json'); $null = $allowed.Add('acpi-validation-ledger.revision.json') }
    foreach ($it in @($l.items)) {
        $x = Get-P9SSetupEntry $it.FullName
        if ($x.kind -ne 'file' -or $x.reparse -or $x.links -gt 1) { $out.Add("ENTRY:$($it.Name)"); continue }
        if (-not $allowed.Contains($it.Name)) { $out.Add("UNEXPECTED_ENTRY:$($it.Name)"); continue }
        foreach ($y in @(Get-P9SStateAclReasons $it.FullName $acct)) { $out.Add("ACL:$($it.Name):$y") }
    }
    # each record exactly as the manifest lists it; each history beginning
    # with the bytes that were copied (the VM observations follow them)
    foreach ($x in @($M.files)) {
        $b = Read-P9SStateBytes (Join-Path $Dir ([string]$x.name))
        if ($b.kind -ne 'ok') { $out.Add("COPY_NOT_READ:$($x.name):$($b.kind)"); continue }
        $got = if ([string]$x.class -ceq 'record') { if ($b.bytes.Length -eq [long]$x.length) { Get-P9SStateSha $b.bytes } } else { Get-P9SStatePrefixSha $b.bytes ([long]$x.length) }
        if ($got -cne [string]$x.sha256) { $out.Add("COPY_DIFFERS:$($x.name)") }
    }
    foreach ($x in @($M.absent)) {
        if ($created -and $x -cin 'acpi-validation-ledger.json', 'acpi-validation-ledger.revision.json') { continue }
        if ((Get-P9SSetupEntry (Join-Path $Dir $x)).kind -ne 'absent') { $out.Add("UNEXPECTED_RECORD:$x") }
    }
    # the ledger as the toolkit reads it
    $lc = [pscustomobject]@{ LedgerPath = Join-Path $Dir 'acpi-validation-ledger.json'; SessionPath = Join-Path $Dir 'coordinator-session.json'; StopRequestPath = Join-Path $Dir 'coordinator-stop.request' }
    $lg = Read-P9SAcpiLedger $lc
    if (-not $lg.ok -or $lg.detail -ceq 'absent, never used') { $out.Add("LEDGER:$(if ($lg.ok) { 'ACPI_LEDGER_MISSING' } else { $lg.reason })") }
    elseif ($created -and ([string]$lg.detail -cne 'revision 1' -or @($lg.doc.entries).Count)) { $out.Add("LEDGER_NOT_EMPTY:$($lg.detail)") }
    $lb = Read-P9SStateBytes $lc.LedgerPath
    if ($lb.kind -ne 'ok') { $out.Add("LEDGER_NOT_READ:$($lb.kind)") }
    else {
        $rev = Get-P9SStateLedgerRevision $lb.bytes
        if ($null -eq $rev) { $out.Add('LEDGER_REVISION_UNREADABLE') } else { $r.mark = [ordered]@{ revision = [long]$rev; sha256 = (Get-P9SStateSha $lb.bytes) } }
    }
    # the install and migration records of this migration
    $ir = Read-P9SStateRecord (Join-Path $Dir $n.install) 'install'
    if (-not $ir.ok) { foreach ($x in $ir.reasons) { $out.Add("INSTALL_RECORD_$x") } }
    else {
        $D = $ir.doc
        foreach ($k in 'installation_id', 'account', 'created_utc') { if ([string]$D[$k] -cne [string]$A[$k]) { $out.Add("INSTALL_RECORD_DIFFERS:$k") } }
        if ([string]$D.origin -cne 'migrated' -or [string]$D.migration_id -cne [string]$A.op_id -or $null -ne $D.supersedes) { $out.Add('INSTALL_RECORD_DIFFERS:origin') }
        if (-not (Test-P9SStateSamePath $D.state_dir $C.StateDir)) { $out.Add('INSTALL_RECORD_DIFFERS:state_dir') }
    }
    $mr = Read-P9SStateRecord (Join-Path $Dir $n.migration) 'migration'
    if (-not $mr.ok) { foreach ($x in $mr.reasons) { $out.Add("MIGRATION_RECORD_$x") } }
    else {
        $D = $mr.doc
        $dg = Get-P9SStateManifestDigest $D.files $D.absent ([string]$D.source_dir)
        if ([string]$D.installation_id -cne [string]$A.installation_id -or [string]$D.migration_id -cne [string]$A.op_id -or $dg -cne [string]$D.manifest_sha256 -or $dg -cne [string]$A.manifest_sha256 -or [bool]$D.ledger_created -ne $created -or -not (Test-P9SStateSamePath $D.source_dir $A.source_dir)) { $out.Add('MIGRATION_RECORD_DIFFERS') }
    }
    $r
}

# ------------------------------------------------------------ state.migrate
function New-P9SStateMigrateResult {
    [pscustomobject]@{ action = 'state.migrate'; outcome = 'REFUSED'; reasons = [System.Collections.Generic.List[string]]::new(); lines = [System.Collections.Generic.List[string]]::new(); notes = [System.Collections.Generic.List[string]]::new()
        mode = $null; status_before = $null; status_after = $null; installation_id = $null; op_id = $null; source = $null; manifest_sha256 = $null; files = @(); absent = @(); ignored = @()
        staging = $null; ledger_created = $false; vm_states = $null; dispatched = 0 }
}
function Invoke-P9SStateMigrate {
    param([Parameter(Mandatory)]$I, [string]$Restart, [switch]$DryRun)
    $res = New-P9SStateMigrateResult
    $C = $I.State; $n = Get-P9SStateNames; $nn = Get-P9SStateInitNames; $mn = Get-P9SStateMigrateNames
    $refuse = { param([string[]]$Why) foreach ($x in $Why) { if ($x) { $res.reasons.Add($x) } }; $res.outcome = 'REFUSED'; $res }
    if ($PSBoundParameters.ContainsKey('Restart') -and -not (Test-P9SStateId $Restart)) { $res.outcome = 'REJECTED'; $res.reasons.Add('RESTART_ID_INVALID'); return $res }
    if (-not (Initialize-P9SSetupNative) -or -not $C.Native) { return (& $refuse 'NATIVE_CHECKS_UNAVAILABLE') }
    # ---- the decision and the checks, before anything is written
    $d = Get-P9SStateMigrateDecision $I $Restart
    $res.status_before = $d.status; $res.mode = $d.mode
    $res.lines.Add("status: $($d.status)$(if ($d.status_reasons.Count) { ' (' + ($d.status_reasons -join ', ') + ')' })")
    if (-not $d.mode) { if ($d.exit) { $res.lines.Add("way out: $($d.exit)") }; return (& $refuse $d.reasons) }
    $acct = [string]$C.Process.account; $src = $d.source; $res.source = $src
    $ne = Get-P9SSetupEntry $C.StateDir
    if ($d.mode -ceq 'new' -and $ne.kind -ne 'absent') { return (& $refuse 'STATE_FOLDER_EXISTS') }
    if ($d.mode -ceq 'restart' -and $ne.kind -ne 'absent') { $res.lines.Add('way out: the state folder is in place; state.migrate completes this migration'); return (& $refuse 'RESTART_NOT_APPLICABLE:STATE_FOLDER_IN_PLACE') }
    $pre = @(@(Get-P9SStateInitProcessReasons $I) + @(Get-P9SStateInitVBoxReasons $C))
    if ($pre.Count) { return (& $refuse $pre) }
    $v = Get-P9SStateSourceView $I $src
    foreach ($x in $v.notes) { $res.notes.Add($x) }
    if ($v.reasons.Count) { return (& $refuse $v.reasons) }
    $sc = $v.scan
    if ($sc.marker) {
        if ($d.mode -ceq 'new') { return (& $refuse 'MIGRATION_SOURCE_ALREADY_MIGRATED') }
        if ($d.mode -ceq 'restart') { return (& $refuse 'RESTART_NOT_APPLICABLE:MARKER_WRITTEN') }
    }
    if ($d.mode -ceq 'resume') {
        $rr = Get-P9SStateResumeReasons $I $d $v
        foreach ($x in $rr.notes) { $res.notes.Add($x) }
        if ($rr.reasons.Count) { if ($rr.way) { $res.lines.Add("way out: $($rr.way)") }; return (& $refuse $rr.reasons) }
    }
    $now = Get-P9SStateInitNow $I
    $times = [System.Collections.Generic.List[datetime]]::new()
    if ($sc.written -gt [datetime]::MinValue) { $times.Add($sc.written) }
    if ($d.anchor) { $x = ConvertFrom-P9SStateUtcText $d.anchor.created_utc; if ($x) { $times.Add($x) } }
    if (@($times | Where-Object { $_ -gt $now }).Count) { return (& $refuse 'CLOCK_BEHIND_EVIDENCE') }
    $lim = Get-P9SStateMigrateLimits
    $need = [long](@($sc.files | Measure-Object -Property length -Sum).Sum) * $lim.copy_factor + $lim.reserve_bytes
    $free = $null; try { $free = & $I.FreeBytes $C.LocalAppData } catch { $free = $null }
    if ($free -isnot [long] -and $free -isnot [int]) { return (& $refuse 'STATE_VOLUME_FREE_SPACE_UNKNOWN') }
    if ([long]$free -lt $need) { return (& $refuse "STATE_VOLUME_FREE_SPACE:$([long]$free):$need") }
    $res.files = @(Get-P9SStateManifestFiles $sc); $res.absent = @($sc.absent); $res.ignored = @($sc.ignored); $res.manifest_sha256 = $v.digest
    Invoke-P9SStateInitStep $I 'P0'
    if ($DryRun) {
        $res.outcome = 'DRYRUN'
        $res.lines.Add("would $(switch ($d.mode) { 'new' { 'migrate' } 'resume' { "complete the migration $($d.anchor.op_id) of" } 'restart' { "start the migration $($d.anchor.op_id) again with a new manifest of" } }) $src into $($C.StateDir)")
        foreach ($x in $sc.files) { $res.lines.Add("  copy  $($x.class) $($x.name) $($x.length) bytes sha256 $($x.sha256)") }
        foreach ($x in $sc.absent) { $res.lines.Add("  absent record $x") }
        foreach ($x in $sc.ignored) { $res.lines.Add("  ignored $x") }
        foreach ($x in $sc.locks) { $res.lines.Add("  lock $x (taken during the migration, not copied)") }
        $res.lines.Add("  ledger: $($v.ledger_detail)$(if ($v.never_used) { '; an empty ledger would be created' })")
        $res.lines.Add("  manifest sha256 $($v.digest); free $free bytes, needed $need")
        if ($res.notes.Count) { $res.lines.Add("  notes: $(@($res.notes | Select-Object -Unique) -join ', ')") }
        $res.lines.Add('the VM states are read once the copies exist; no lock was taken and nothing was written')
        return $res
    }
    # ---- M1: the root and the setup lock
    $re = Get-P9SSetupEntry $C.Root
    if ($re.kind -eq 'absent') { $x = New-P9SStateFolder $C.Root $acct; if ($x -and $x -cne 'EXISTS') { return (& $refuse "ROOT_NOT_CREATED:$x") } }
    elseif ($re.kind -ne 'directory') { return (& $refuse "ROOT_$($re.kind.ToUpperInvariant())") }
    $lock = $null; $olock = $null; $ohold = $null
    try { $lock = [IO.FileStream]::new((Join-Path $C.Root $n.setup_lock), [IO.FileMode]::OpenOrCreate, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None) } catch { $lock = $null }
    if (-not $lock) { return (& $refuse 'SETUP_BUSY') }
    try {
        Invoke-P9SStateInitStep $I 'M1'
        $d2 = Get-P9SStateMigrateDecision $I $Restart
        if ($d2.mode -cne $d.mode -or $d2.status -cne $d.status -or $d2.source -cne $d.source -or [string]$(if ($d2.anchor) { $d2.anchor.op_id }) -cne [string]$(if ($d.anchor) { $d.anchor.op_id }) -or [string]$(if ($d2.anchor) { $d2.anchor.manifest_sha256 }) -cne [string]$(if ($d.anchor) { $d.anchor.manifest_sha256 })) {
            $res.status_before = $d2.status; return (& $refuse (@("SETUP_STATE_CHANGED:$($d2.status)") + @($d2.reasons)))
        }
        $pre = @(@(Get-P9SStateInitProcessReasons $I) + @(Get-P9SStateInitVBoxReasons $C))
        if ($pre.Count) { return (& $refuse $pre) }
        # ---- M2: the earlier folder's lock and hold, then the folder read again
        try { $olock = [IO.FileStream]::new((Join-Path $src $mn.lock), [IO.FileMode]::OpenOrCreate, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None) } catch { $olock = $null }
        if (-not $olock) { return (& $refuse 'MIGRATION_SOURCE_BUSY') }
        try { $ohold = [IO.FileStream]::new((Join-Path $src $mn.hold), [IO.FileMode]::OpenOrCreate, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None) } catch { $ohold = $null }
        if (-not $ohold) { return (& $refuse 'EMERGENCY_IN_PROGRESS') }
        $v = Get-P9SStateSourceView $I $src
        if ($v.reasons.Count) { return (& $refuse $v.reasons) }
        $sc = $v.scan
        $res.files = @(Get-P9SStateManifestFiles $sc); $res.absent = @($sc.absent); $res.ignored = @($sc.ignored); $res.manifest_sha256 = $v.digest
        Invoke-P9SStateInitStep $I 'M2'
        if ($d.mode -ceq 'resume') { $rr = Get-P9SStateResumeReasons $I $d $v; if ($rr.reasons.Count) { return (& $refuse $rr.reasons) } }
        if ($sc.marker -and $d.mode -cne 'resume') { return (& $refuse 'MIGRATION_SOURCE_ALREADY_MIGRATED') }
        # ---- M3: the anchor of this migration
        if ($d.mode -ceq 'new') {
            $A = [ordered]@{ schema = 'p9s-state-anchor/1'; phase = 'MIGRATING'; op_id = (New-P9SStateMigrateOpId $now); installation_id = (New-P9SStateInstallationId); account = $acct; state_dir = $C.StateDir; origin = 'migrated'
                source_dir = $src; manifest_sha256 = $v.digest; supersedes = $null; created_utc = (ConvertTo-P9SStateUtcText $now); initialized_utc = $null; ledger = $null }
            try { Write-P9SStateAnchor $C $A } catch { return (& $refuse 'ANCHOR_WRITE_FAILED') }
            Invoke-P9SStateInitStep $I 'M3'
        } elseif ($d.mode -ceq 'restart') {
            $raw = Read-P9SStateAnchorRaw $C
            if (-not $raw) { return (& $refuse 'ANCHOR_NOT_KEPT') }
            $k = 1; $vals = @(Get-P9SStateAnchorValueNames $C)
            while ($vals -ccontains "$($mn.restart_anchor)$($d.anchor.op_id)-$k") { $k++ }
            try { Write-P9SStateRegistryValue $C "$($mn.restart_anchor)$($d.anchor.op_id)-$k" $raw } catch { return (& $refuse 'ANCHOR_NOT_KEPT') }
            Invoke-P9SStateInitStep $I 'M3a'
            $A = [ordered]@{}; foreach ($x in $d.anchor.Keys) { $A[$x] = $d.anchor[$x] }
            $A.manifest_sha256 = $v.digest
            try { Write-P9SStateAnchor $C $A } catch { return (& $refuse 'ANCHOR_WRITE_FAILED') }
            Invoke-P9SStateInitStep $I 'M3'
        } else { $A = $d.anchor }
        $op = [string]$A.op_id
        $res.installation_id = [string]$A.installation_id; $res.op_id = $op
        # ---- the state folder: absent (copied now), or this migration's (a
        # resumed run after M9); anything else refuses
        $ne = Get-P9SSetupEntry $C.StateDir
        $ours = $false
        if ($ne.kind -eq 'directory') {
            $ir = Read-P9SStateRecord (Join-Path $C.StateDir $n.install) 'install'
            $ours = [bool]($d.mode -ceq 'resume' -and $ir.ok -and [string]$ir.doc.installation_id -ceq [string]$A.installation_id -and [string]$ir.doc.migration_id -ceq $op)
            if (-not $ours) { return (& $refuse 'STATE_FOLDER_EXISTS') }
        } elseif ($ne.kind -ne 'absent') { return (& $refuse "STATE_FOLDER_$($ne.kind.ToUpperInvariant())") }
        if (-not $ours) {
            if ($sc.marker) { return (& $refuse 'MIGRATION_MARKER_WITHOUT_STATE_FOLDER') }
            # ---- M4: the staging folder, on the volume of the state folder
            $k = 1; $stage = Join-Path $C.Root "$($nn.staging)$op-$k"
            while ((Get-P9SSetupEntry $stage).kind -ne 'absent') { $k++; $stage = Join-Path $C.Root "$($nn.staging)$op-$k" }
            $x = New-P9SStateFolder $stage $acct
            if ($x) { return (& $refuse "STAGING_NOT_CREATED:$x") }
            $res.staging = $stage
            Invoke-P9SStateInitStep $I 'M4'
            if ((Get-P9SSetupEntry $stage).volume -ne (Get-P9SSetupEntry $C.Root).volume) { return (& $refuse 'STAGING_NOT_ON_STATE_VOLUME') }
            # ---- M5: each record and history, copied and read back
            foreach ($fi in @($sc.files)) {
                $b = Read-P9SStateBytes (Join-Path $src $fi.name)
                if ($b.kind -ne 'ok' -or (Get-P9SStateSha $b.bytes) -cne $fi.sha256) { return (& $refuse "MIGRATION_SOURCE_CHANGED:$($fi.name)") }
                try { Write-P9SStateMigrateCopy $I (Join-Path $stage $fi.name) $b.bytes "M5:$($fi.name)" } catch { return (& $refuse "COPY_NOT_WRITTEN:$($fi.name)") }
                $cb = Read-P9SStateBytes (Join-Path $stage $fi.name)
                if ($cb.kind -ne 'ok' -or (Get-P9SStateSha $cb.bytes) -cne $fi.sha256) { return (& $refuse "MIGRATION_COPY_MISMATCH:$($fi.name)") }
            }
            $dir = $stage
        } else { $dir = $C.StateDir }
        # ---- V: the VMs, observed into the copied histories
        foreach ($x in 'vm-observations.jsonl', 'vbox-audit.jsonl') { try { Close-P9SStateHistoryLine (Join-Path $dir $x) } catch { return (& $refuse "HISTORY_NOT_CONTINUED:$x") } }
        $vm = Test-P9SStateInitVms $I $dir $mn.purpose
        $res.vm_states = $vm.states
        if ($vm.reasons.Count) { return (& $refuse $vm.reasons) }
        Invoke-P9SStateInitStep $I 'V'
        if (-not $ours) {
            # ---- M6: the empty ledger, only for a folder that never had one
            if ($v.never_used) {
                $lc = [pscustomobject]@{ LedgerPath = Join-Path $stage 'acpi-validation-ledger.json'; SessionPath = Join-Path $stage 'coordinator-session.json'
                    WriteLedger = { param($p, $t) Write-P9SStateInitAtomic $I $p $t 'M6-ledger' }.GetNewClosure()
                    WriteLedgerRevision = { param($p, $t) Write-P9SStateInitAtomic $I $p $t 'M6-revision' }.GetNewClosure() }
                if (-not (Save-P9SAcpiLedger $lc (New-P9SAcpiLedger))) { return (& $refuse 'LEDGER_NOT_WRITTEN') }
                $res.ledger_created = $true
                Invoke-P9SStateInitStep $I 'M6'
            }
            # ---- M7: the migration record, then the install record
            $obs = @()
            $ro = Read-StateFile -Path (Join-Path $stage 'vm-observations.jsonl')
            if ($ro.ok) { $obs = @($ro.text -split "`n" | Where-Object { $_ } | ForEach-Object { try { $_ | ConvertFrom-Json -AsHashtable -ErrorAction Stop } catch {} } | Where-Object { $_ -is [System.Collections.IDictionary] -and $_.pid -eq $PID -and [string]$_.purpose -ceq $mn.purpose } | ForEach-Object { [ordered]@{ vm = [string]$_.vm; state = [string]$_.state; observed = [bool]$_.observed; utc = [string]$_.utc; operation_id = $(if ($_.operation_id) { [string]$_.operation_id } else { $null }) } }) }
            $M = [ordered]@{ schema = 'p9s-state-migration/1'; migration_id = $op; installation_id = $A.installation_id; source_dir = $src; manifest_sha256 = $v.digest
                files = @(Get-P9SStateManifestFiles $sc); absent = @($sc.absent); ignored = @($sc.ignored); ledger_created = [bool]$res.ledger_created; vm_observations = @($obs); utc = (ConvertTo-P9SStateUtcText (Get-P9SStateInitNow $I)) }
            try { Write-P9SStateNewFile (Join-Path $stage $n.migration) ([Text.UTF8Encoding]::new($false).GetBytes(($M | ConvertTo-Json -Depth 6))) } catch { return (& $refuse 'MIGRATION_RECORD_NOT_WRITTEN') }
            Invoke-P9SStateInitStep $I 'M7-migration'
            $inst = [ordered]@{ schema = 'p9s-state-install/1'; installation_id = $A.installation_id; account = $A.account; state_dir = $A.state_dir; origin = 'migrated'; migration_id = $op; supersedes = $null; created_utc = $A.created_utc }
            try { Write-P9SStateNewFile (Join-Path $stage $n.install) ([Text.UTF8Encoding]::new($false).GetBytes(($inst | ConvertTo-Json -Depth 4))) } catch { return (& $refuse 'INSTALL_RECORD_NOT_WRITTEN') }
            Invoke-P9SStateInitStep $I 'M7'
            # ---- M8: the staging folder checked, then the earlier folder once more
            $chk = Get-P9SStateMigrateFolderCheck $I $A $stage $M
            if ($chk.reasons.Count) { return (& $refuse (@('STAGING_NOT_COMPLETE') + @($chk.reasons))) }
            $s2 = Get-P9SStateSourceScan $src $acct
            if ($s2.reasons.Count -or $s2.digest -cne $v.digest) { return (& $refuse (@('MIGRATION_SOURCE_CHANGED') + @($s2.reasons))) }
            Invoke-P9SStateInitStep $I 'M8'
            # ---- M9: into place (the rename refuses an existing target too)
            if ((Get-P9SSetupEntry $C.StateDir).kind -ne 'absent') { return (& $refuse 'STATE_FOLDER_EXISTS') }
            $id0 = Get-P9SStateIdentity $stage
            $e = Move-P9SStateEntry $stage $C.StateDir
            if (Test-P9SStateExistsError $e) { return (& $refuse "STATE_FOLDER_EXISTS:$e") }
            if ($e) { return (& $refuse "STAGING_NOT_RENAMED:$e") }
            if ((Get-P9SStateIdentity $C.StateDir) -cne $id0) { return (& $refuse 'STATE_FOLDER_RENAME_UNVERIFIED') }
            Invoke-P9SStateInitStep $I 'M9'
        } else {
            $mr = Read-P9SStateRecord (Join-Path $C.StateDir $n.migration) 'migration'
            if (-not $mr.ok) { return (& $refuse (@('STATE_FOLDER_NOT_COMPLETE') + @($mr.reasons | ForEach-Object { "MIGRATION_RECORD_$_" }))) }
            $M = $mr.doc
        }
        # ---- M10: the marker in the earlier folder, created once
        $mp = Join-Path $src $n.marker
        $mk = [ordered]@{ schema = 'p9s-state-moved/1'; installation_id = $A.installation_id; state_dir = $C.StateDir; manifest_sha256 = $A.manifest_sha256
            note = 'Migrated by state.migrate; this folder is no longer the state of the toolkit. Nothing here was deleted or changed.' }
        $me = Get-P9SSetupEntry $mp
        if ($me.kind -eq 'absent') {
            try { Write-P9SStateNewFile $mp ([Text.UTF8Encoding]::new($false).GetBytes(($mk | ConvertTo-Json -Depth 3))) } catch { return (& $refuse 'MARKER_NOT_WRITTEN') }
        } else {
            $mko = Read-P9SStateRecord $mp 'marker'
            if (-not $mko.ok -or [string]$mko.doc.installation_id -cne [string]$A.installation_id -or [string]$mko.doc.manifest_sha256 -cne [string]$A.manifest_sha256 -or -not (Test-P9SStateSamePath $mko.doc.state_dir $C.StateDir)) { return (& $refuse 'MIGRATION_MARKER_CONFLICT') }
        }
        Invoke-P9SStateInitStep $I 'M10'
        # ---- M11: the folder checked, the earlier folder still matching it, the
        # anchor unchanged, then READY
        $chk = Get-P9SStateMigrateFolderCheck $I $A $C.StateDir $M
        foreach ($x in @(Get-P9SStateLocationReasons $C)) { $chk.reasons.Add("LOCATION:$x") }
        if ($chk.reasons.Count) { return (& $refuse (@('STATE_FOLDER_NOT_COMPLETE') + @($chk.reasons))) }
        $s3 = Get-P9SStateSourceScan $src $acct
        $ag = Get-P9SStateSourceAgainst $s3 $M
        if ($s3.reasons.Count -or $ag.reasons.Count) { $res.lines.Add('way out: a record of the earlier folder changed after the copy: put it back as migration.json of the state folder lists it; nothing is chosen automatically'); return (& $refuse (@('MIGRATION_SOURCE_CHANGED') + @($s3.reasons) + @($ag.reasons))) }
        $cur = Read-P9SStateAnchor $C
        if (-not $cur.ok -or [string]$cur.doc.phase -cne 'MIGRATING' -or [string]$cur.doc.op_id -cne $op -or [string]$cur.doc.installation_id -cne [string]$A.installation_id -or [string]$cur.doc.manifest_sha256 -cne [string]$A.manifest_sha256) { return (& $refuse 'ANCHOR_CHANGED') }
        $now = Get-P9SStateInitNow $I
        $created = ConvertFrom-P9SStateUtcText $A.created_utc
        if (-not $created -or $now -lt $created) { return (& $refuse 'CLOCK_BEHIND_EVIDENCE') }
        $R = [ordered]@{}; foreach ($k in $A.Keys) { $R[$k] = $A[$k] }
        $R.phase = 'READY'; $R.initialized_utc = ConvertTo-P9SStateUtcText $now; $R.ledger = $chk.mark
        try { Write-P9SStateAnchor $C $R } catch { return (& $refuse 'ANCHOR_WRITE_FAILED') }
        Invoke-P9SStateInitStep $I 'M11'
        # READY only as the status says it, never as this function assumes it
        $after = Get-P9SStateStatus $C
        $res.status_after = $after.status
        if ($after.status -cne 'READY' -or [string]$after.installation_id -cne [string]$A.installation_id) {
            $res.outcome = 'FAILED'; $res.reasons.Add("STATUS_AFTER_MIGRATE:$($after.status)"); foreach ($x in @($after.reasons)) { $res.reasons.Add($x) }; return $res
        }
        foreach ($x in @($after.notes)) { $res.notes.Add($x) }
        # ---- M12: one line of the new installation's audit
        try {
            $line = [ordered]@{ utc = (ConvertTo-P9SStateUtcText (Get-P9SStateInitNow $I)); pid = $PID; action = 'state.migrate'; outcome = 'DONE'; reasons = @(); installation_id = $A.installation_id; op_id = $op
                source = $src; manifest_sha256 = $A.manifest_sha256; files = @($sc.files).Count; ledger_created = [bool]$M.ledger_created; initialized_utc = $R.initialized_utc; vm_states = $res.vm_states
                notes = @($res.notes | Select-Object -Unique); dispatched = 0 }
            Add-P9SStateHistoryBytes (Join-Path $C.StateDir $mn.audit) ([Text.UTF8Encoding]::new($false).GetBytes(($line | ConvertTo-Json -Compress -Depth 5) + "`n"))
        } catch { $res.lines.Add('the audit line of the new installation was not written') }
        Invoke-P9SStateInitStep $I 'M12'
        $res.outcome = 'DONE'; $res.ledger_created = [bool]$M.ledger_created
        $res.lines.Add("installation $($A.installation_id) ready in $($C.StateDir); migrated from $src ($(@($sc.files).Count) files, manifest $($A.manifest_sha256)); initialized $($R.initialized_utc)")
        $res.lines.Add("the earlier folder is kept as it was, with $($n.marker) added; every ACPI validation authorization approved or valid from before $($R.initialized_utc) is void")
        $res.lines.Add('the coordinator of this version keeps using the earlier folder: nothing was switched; run no live action until a version that uses the new folder')
        return $res
    } finally { if ($ohold) { $ohold.Dispose() }; if ($olock) { $olock.Dispose() }; $lock.Dispose() }
}
# the value names of the anchor key ($null when it cannot be read)
function Get-P9SStateAnchorValueNames($C) {
    $k = $null
    try { $k = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey($C.AnchorKey, $false) } catch { return @() }
    if (-not $k) { return @() }
    try { @($k.GetValueNames()) } finally { $k.Dispose() }
}
