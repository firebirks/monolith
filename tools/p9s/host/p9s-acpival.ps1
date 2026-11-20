#Requires -Version 7.0
# P9-S restricted single-VM ACPI validation (library; dot-sourced by
# coordinator-lib.ps1). Purpose: one supervised check that the native
# VirtualBox ACPI power button shuts the Debian VM (P9S-D12) down. The
# paging values cannot be calibrated before such checks exist, and normal
# test actions need approved paging values: this capability breaks that
# circle for exactly one VM, one start and one ACPI request, and for
# nothing else. It is not a test mode and grants no exception to the
# normal gate:
#   - its actions are the acpi.* entries of the coordinator; none accepts
#     a VM name, text, keys, a plan or a command; the only dispatches are
#     one vm.start and one vm.acpi of P9S-D12;
#   - it needs an owner-written authorization file naming that purpose,
#     that VM, the permitted actions, one start, one ACPI request, a window
#     of at most 4 hours and the boundaries below; the file is checked, not
#     trusted as a cryptographic proof: the same Windows account can write
#     it. The live path is also disabled in the code of this version
#     (Get-P9SAcpiValidationCapability): enabling it is a reviewed change
#     the owner authorizes with the session;
#   - the monitor must be a Monitor-mode run with the paging stop required
#     (so it reports UNCALIBRATED while the paging values are unapproved),
#     fresh, alive, the 300-second commit qualification met, commit below
#     90 %, no WARNING, UNKNOWN, STOP or ABORT, measurements complete, and,
#     before the start, at least the provisional available-memory minimum.
#     The coordinator calls that ACPI_VALIDATION_ELIGIBLE; it is not READY
#     and the monitor state is not changed;
#   - the budgets (one start, one ACPI request) live in
#     acpi-validation-ledger.json next to the coordinator files, keyed by
#     the authorization id, and are spent before the dispatch: a restart,
#     a crash or a lost session record cannot give them back, and an
#     authorization id opens one session only;
#   - success is a power-off observed after the request, never the request
#     itself; a VM that stays on, an unreadable state or a VirtualBox
#     failure leaves ACPI_VALIDATION_UNRESOLVED, which blocks every new
#     validation and test session until acpi.confirm-off observes the VM
#     off. No second request, no keyboard input, no forced power-off.

function Get-P9SAcpiValidationCapability {
    [pscustomobject]@{ name = 'ACPI_VALIDATION'; live_enabled = $false; note = 'prepared; the live path is disabled in this version of the toolkit' }
}
function Get-P9SAcpiValidationTarget { 'P9S-D12' }
function Get-P9SAcpiValidationActions { 'acpi.preflight', 'acpi.open', 'acpi.start', 'acpi.observe', 'acpi.shutdown', 'acpi.confirm-off', 'acpi.integrity', 'acpi.finalize' }
# the actions an authorization must list to be usable (the others are
# read-only or close the session)
function Get-P9SAcpiAuthorizedActions { 'acpi.open', 'acpi.start', 'acpi.observe', 'acpi.shutdown', 'acpi.integrity' }
function Get-P9SAcpiValidationLimits {
    [pscustomobject]@{
        max_startups = 1; max_acpi_requests = 1; max_window_hours = 4; provisional_min_available_mb = 8192
        shutdown_wait_s = 180; max_wait_s = 600; authorization_max_bytes = 16384
    }
}
function Get-P9SAcpiMonitorConditions { [ordered]@{ mode = 'Monitor'; paging_stop = 'Required'; commit_gate_s = 300; commit_below_pct = 90 } }
function Get-P9SAcpiPreflightChecks { 'monitor-eligible', 'virtualbox-not-quarantined', 'target-observed-off', 'no-unresolved-validation', 'config-reference-present' }
function Get-P9SAcpiStopPolicy { 'halt-then-emergency-acpi-shutdown-no-repeat' }
# phases in which no VM of the validation can be running unseen
function Get-P9SAcpiResolvedPhases { 'OPEN', 'START_FAILED', 'OFF_CONFIRMED', 'OFF_AFTER_DEADLINE', 'OFF_REQUEST_UNCONFIRMED', 'OFF_WITHOUT_ACPI', 'OFF_ABNORMAL' }
function Test-P9SAcpiEntryResolved($E) { [bool]($E -and [string]$E.phase -cin (Get-P9SAcpiResolvedPhases)) }

# ------------------------------------------------------------ authorization
function ConvertTo-P9SAuthTime($v) {
    if ($v -is [datetime]) { if ($v.Kind -eq [DateTimeKind]::Utc) { return $v } elseif ($v.Kind -eq [DateTimeKind]::Local) { return $v.ToUniversalTime() } else { return $null } }
    if ($v -is [string] -and $v -cmatch '^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(\.\d{1,7})?Z$') {
        $d = [datetime]::MinValue
        if ([datetime]::TryParse($v, [Globalization.CultureInfo]::InvariantCulture, [Globalization.DateTimeStyles]::AdjustToUniversal -bor [Globalization.DateTimeStyles]::AssumeUniversal, [ref]$d)) { return $d }
    }
    $null
}
# Checks one authorization file. Never trusted beyond its fields: it is a
# plain file the operator's account can write.
function Read-P9SAcpiAuthorization {
    param([Parameter(Mandatory)][string]$Path, [string]$KitDir)
    $reasons = [System.Collections.Generic.List[string]]::new()
    $out = { [pscustomobject]@{ ok = ($reasons.Count -eq 0); reasons = @($reasons); auth = $auth; sha256 = $sha; id = $(if ($auth) { $auth.authorization_id } else { $null }) } }
    $auth = $null; $sha = $null
    $lim = Get-P9SAcpiValidationLimits
    $full = $null; try { $full = [IO.Path]::GetFullPath($Path) } catch {}
    if (-not $full -or -not (Test-Path -LiteralPath $full -PathType Leaf)) { $reasons.Add('AUTHORIZATION_MISSING'); return (& $out) }
    if ($KitDir -and $full.StartsWith(([IO.Path]::GetFullPath($KitDir).TrimEnd('\') + '\'), [StringComparison]::OrdinalIgnoreCase)) { $reasons.Add('AUTHORIZATION_IN_TOOLKIT'); return (& $out) }
    $fi = Get-Item -LiteralPath $full
    if ($fi.Length -gt $lim.authorization_max_bytes) { $reasons.Add('AUTHORIZATION_TOO_LARGE'); return (& $out) }
    $sha = Get-FileSha256 $full
    $o = $null
    try { $o = Get-Content -LiteralPath $full -Raw | ConvertFrom-Json -AsHashtable -ErrorAction Stop } catch { $reasons.Add('AUTHORIZATION_INVALID_JSON'); return (& $out) }
    if ($o -isnot [System.Collections.IDictionary] -or $o.schema -cne 'p9s-acpi-validation-authorization/1') { $reasons.Add('AUTHORIZATION_SCHEMA_UNSUPPORTED'); return (& $out) }
    $id = [string]$o.authorization_id
    if ($id -cnotmatch '^acpiauth-[0-9]{8}-[0-9a-f]{8}$') { $reasons.Add('AUTHORIZATION_ID_INVALID') }
    if ([string]$o.purpose -cne 'ACPI_VALIDATION') { $reasons.Add('AUTHORIZATION_PURPOSE') }
    $target = [string]$o.target_vm
    if ($target -cne (Get-P9SAcpiValidationTarget) -or $target -cnotin (Get-P9SVmNames)) { $reasons.Add('AUTHORIZATION_TARGET_NOT_ALLOWED') }
    $acts = @($o.permitted_actions)
    if (-not $acts.Count -or @($acts | Where-Object { $_ -isnot [string] -or $_ -cnotin (Get-P9SAuthorizedActionsList) }).Count -or @($acts | Group-Object | Where-Object Count -gt 1).Count -or $acts -cnotcontains 'acpi.open') { $reasons.Add('AUTHORIZATION_ACTIONS_INVALID') }
    foreach ($k in 'max_startups', 'max_acpi_requests') { if ($o[$k] -isnot [long] -and $o[$k] -isnot [int] -or [long]$o[$k] -ne 1) { $reasons.Add("AUTHORIZATION_BUDGET:$k") } }
    $from = ConvertTo-P9SAuthTime $o.valid_from_utc; $to = ConvertTo-P9SAuthTime $o.expires_utc
    if (-not $from -or -not $to) { $reasons.Add('AUTHORIZATION_TIME_INVALID') }
    elseif ($to -le $from) { $reasons.Add('AUTHORIZATION_TIME_INVALID') }
    elseif (($to - $from).TotalHours -gt $lim.max_window_hours) { $reasons.Add('AUTHORIZATION_WINDOW_TOO_LONG') }
    $mc = $o.monitor_conditions; $want = Get-P9SAcpiMonitorConditions
    $mcOk = $mc -is [System.Collections.IDictionary]
    if ($mcOk) { foreach ($k in $want.Keys) { if ([string]$mc[$k] -cne [string]$want[$k]) { $mcOk = $false } } }
    $minMb = if ($mcOk) { $mc['provisional_min_available_mb'] } else { $null }
    if (-not $mcOk -or ($minMb -isnot [long] -and $minMb -isnot [int]) -or [long]$minMb -lt $lim.provisional_min_available_mb) { $reasons.Add('AUTHORIZATION_MONITOR_CONDITIONS') }
    $pf = @($o.preflight_checks)
    if (@(Compare-Object -ReferenceObject @(Get-P9SAcpiPreflightChecks) -DifferenceObject @($pf | ForEach-Object { [string]$_ }) -CaseSensitive).Count) { $reasons.Add('AUTHORIZATION_PREFLIGHT_CHECKS') }
    if ([string]$o.supervision -cne 'operator-present') { $reasons.Add('AUTHORIZATION_SUPERVISION') }
    if ([string]$o.on_stop_or_abort -cne (Get-P9SAcpiStopPolicy)) { $reasons.Add('AUTHORIZATION_STOP_POLICY') }
    if ($o.provisional_safeguard_approved -isnot [bool] -or -not $o.provisional_safeguard_approved) { $reasons.Add('AUTHORIZATION_SAFEGUARD_NOT_APPROVED') }
    if ([string]::IsNullOrWhiteSpace([string]$o.approved_by) -or [string]$o.approved_by -match '^<.*>$' -or -not (ConvertTo-P9SAuthTime $o.approved_utc)) { $reasons.Add('AUTHORIZATION_APPROVAL_INCOMPLETE') }
    if ($reasons.Count -eq 0) {
        $auth = [ordered]@{
            authorization_id = $id; authorization_sha256 = $sha; target_vm = $target; permitted_actions = @($acts)
            max_startups = 1; max_acpi_requests = 1; valid_from_utc = $from.ToString('o'); expires_utc = $to.ToString('o')
            min_available_mb = [long]$minMb; approved_utc = (ConvertTo-P9SAuthTime $o.approved_utc).ToString('o')
        }
    }
    & $out
}
function Get-P9SAuthorizedActionsList { @(Get-P9SAcpiAuthorizedActions) }
# The time window, checked at every use against the stored copy (editing
# the file later changes nothing).
function Test-P9SAcpiAuthorizationTime($Auth, [datetime]$Now) {
    $from = ConvertTo-P9SAuthTime $Auth.valid_from_utc; $to = ConvertTo-P9SAuthTime $Auth.expires_utc
    if (-not $from -or -not $to) { return 'AUTHORIZATION_TIME_INVALID' }
    if ($Now -lt $from) { return 'AUTHORIZATION_NOT_YET_VALID' }
    if ($Now -ge $to) { return 'AUTHORIZATION_EXPIRED' }
    $null
}

# ------------------------------------------------------------ monitor
# ACPI_VALIDATION_ELIGIBLE: the structural and freshness checks of the
# normal reader with every reason kept except the uncalibrated status,
# plus Monitor mode with the paging stop required, D: confirmed, no storage
# event, complete measurements and, before the start, the provisional
# memory minimum. -ForShutdown (the one ACPI request) also tolerates a
# WARNING, UNKNOWN or broken qualification: the request lowers the load;
# STOP, ABORT, a stale or dead monitor still refuse (the emergency path
# stays available for them).
function Test-P9SAcpiValidationState {
    param([AllowNull()][AllowEmptyString()][string]$Text, [long]$NowQpc, [long]$QpcFrequency, [scriptblock]$IsMonitorAlive, [double]$MinAvailableMb = 0, [switch]$ForShutdown)
    $t = Test-SafetyState -Text $Text -NowQpc $NowQpc -QpcFrequency $QpcFrequency -PagingStop NotRequired -IsMonitorAlive $IsMonitorAlive
    $tolerated = if ($ForShutdown) { @('STATUS_UNCALIBRATED', 'STATUS_WARNING', 'STATUS_QUALIFYING', 'STATUS_UNKNOWN', 'GATE_INCOMPLETE', 'COMMIT_INVALID', 'SAMPLING_GAP') } else { @('STATUS_UNCALIBRATED') }
    $bad = [System.Collections.Generic.List[string]]::new()
    foreach ($r in $t.reasons) { if ($r -cnotin $tolerated) { $bad.Add($r) } }
    $doc = $null; try { $doc = $Text | ConvertFrom-Json -ErrorAction Stop } catch {}
    $commit = $null; $avail = $null; $cal = $null
    if ($doc -is [pscustomobject] -and $doc.schema -eq 'p9s-hostmon-state/1') {
        if ([string]$doc.monitor.mode -cne 'Monitor') { $bad.Add('ACPI_VALIDATION_REQUIRES_MONITOR_MODE') }
        if ([string]$doc.monitor.paging_stop -cne 'Required') { $bad.Add('ACPI_VALIDATION_REQUIRES_PAGING_STOP_REQUIRED') }
        if (-not $ForShutdown) {
            if ($doc.external_disk -and ($doc.external_disk.present -ne $true -or $doc.external_disk.identity_ok -ne $true)) { $bad.Add('EXTERNAL_DISK_NOT_CONFIRMED') }
            if ($doc.storage_events -and [int]$doc.storage_events.count_since_start -gt 0) { $bad.Add('STORAGE_EVENTS') }
            if (($doc.paging -and $doc.paging.inputs_complete -ne $true) -or -not $doc.memory -or -not (Test-Finite $doc.memory.avail_mb)) { $bad.Add('MEASUREMENTS_INCOMPLETE') }
        }
        $commit = $doc.commit.pct; $cal = if ($doc.paging) { [string]$doc.paging.calibration } else { $null }
        if ($doc.memory) { $avail = $doc.memory.avail_mb }
        if ($MinAvailableMb -gt 0 -and (-not (Test-Finite $avail) -or [double]$avail -lt $MinAvailableMb)) { $bad.Add('MEMORY_BELOW_PROVISIONAL_MINIMUM') }
    }
    [pscustomobject]@{ ok = ($bad.Count -eq 0); state = $(if ($bad.Count) { 'ACPI_VALIDATION_NOT_ELIGIBLE' } else { 'ACPI_VALIDATION_ELIGIBLE' }); reasons = @($bad)
        status = $t.status; run_id = $t.run_id; commit_pct = $commit; avail_mb = $avail; paging_calibration = $cal }
}

# ------------------------------------------------------------ ledger
function New-P9SAcpiLedger { [ordered]@{ schema = 'p9s-acpi-validation-ledger/1'; entries = @() } }
function Read-P9SAcpiLedger($Ctx) {
    $p = Get-Prop $Ctx 'LedgerPath'
    if (-not $p) { return [pscustomobject]@{ ok = $false; reason = 'ACPI_LEDGER_UNAVAILABLE'; doc = $null } }
    $r = Read-StateFile -Path $p
    if (-not $r.ok) {
        if ($r.error -eq 'missing') { return [pscustomobject]@{ ok = $true; reason = $null; doc = (New-P9SAcpiLedger) } }
        return [pscustomobject]@{ ok = $false; reason = 'ACPI_LEDGER_UNREADABLE'; doc = $null }
    }
    $d = $null; try { $d = $r.text | ConvertFrom-Json -AsHashtable -ErrorAction Stop } catch {}
    if ($d -isnot [System.Collections.IDictionary] -or $d.schema -cne 'p9s-acpi-validation-ledger/1' -or $null -eq $d.entries) { return [pscustomobject]@{ ok = $false; reason = 'ACPI_LEDGER_UNREADABLE'; doc = $null } }
    $d.entries = @($d.entries)
    foreach ($e in $d.entries) { if ($e -isnot [System.Collections.IDictionary] -or -not $e.authorization_id) { return [pscustomobject]@{ ok = $false; reason = 'ACPI_LEDGER_UNREADABLE'; doc = $null } } }
    [pscustomobject]@{ ok = $true; reason = $null; doc = $d }
}
# $true when written; a failed write is never retried silently: the caller
# then does not dispatch.
function Save-P9SAcpiLedger($Ctx, $Doc) {
    try {
        $text = $Doc | ConvertTo-Json -Depth 8
        $w = Get-Prop $Ctx 'WriteLedger'
        if ($w) { & $w $Ctx.LedgerPath $text } else { Write-StateAtomic -Path $Ctx.LedgerPath -Text $text }
        return $true
    } catch { return $false }
}
function Get-P9SAcpiEntry($Doc, [string]$Id) { foreach ($e in @($Doc.entries)) { if ([string]$e.authorization_id -ceq $Id) { return $e } }; $null }
function Add-P9SAcpiEvent($E, [string]$What, [string]$Detail) {
    $E.events = @($E.events) + @([ordered]@{ utc = [DateTime]::UtcNow.ToString('o'); what = $What; detail = $Detail })
}
# Unresolved entries for one VM (any VM when $Vm is empty).
function Get-P9SAcpiUnresolved($Doc, [string]$Vm) { @(@($Doc.entries) | Where-Object { -not (Test-P9SAcpiEntryResolved $_) -and (-not $Vm -or [string]$_.target_vm -ceq $Vm) }) }
# The unresolved validation of this VM that has already spent its one ACPI
# request (sent, or dispatched without a recorded end), if any: the
# emergency path then sends none. An unreadable ledger blocks nothing here:
# the emergency path stays available.
function Get-P9SAcpiSpentRequest($Ctx, [string]$Vm) {
    $lg = Read-P9SAcpiLedger $Ctx
    if (-not $lg.ok) { return $null }
    $e = @(Get-P9SAcpiUnresolved $lg.doc $Vm) | Select-Object -Last 1
    if ($e -and [int]$e.acpi_used -ge 1) { return $e }
    $null
}
# The end of an emergency ACPI request that a validation entry counted as
# its own (emergency.acpi-shutdown): off, aborted or unresolved. -Locked:
# the caller holds the coordinator lock.
function Set-P9SAcpiAfterEmergency($Ctx, [string]$Vm, [string]$Phase, [switch]$Locked) {
    if (-not (Get-Prop $Ctx 'LedgerPath')) { return $true }
    $lock = if ($Locked) { $null } else { Enter-P9SLock $Ctx 2000 }
    if (-not $Locked -and -not $lock) { return $false }
    try {
        $lg = Read-P9SAcpiLedger $Ctx
        if (-not $lg.ok) { return $false }
        $e = @(@($lg.doc.entries) | Where-Object { [string]$_.target_vm -ceq $Vm -and [string]$_.acpi_source -ceq 'emergency' -and [string]$_.phase -ceq 'SHUTDOWN_REQUESTING' }) | Select-Object -Last 1
        if (-not $e) { return $true }
        $e.phase = $Phase; Add-P9SAcpiEvent $e 'emergency-result' $Phase
        return (Save-P9SAcpiLedger $Ctx $lg.doc)
    } finally { if ($lock) { $lock.Dispose() } }
}

# ------------------------------------------------------------ helpers
function Test-P9SAcpiLive($Ctx) { [bool](Get-Prop $Ctx 'AcpiValidationLive') }
# The validation session, the ledger and its entry, with every reason found.
function Get-P9SAcpiBundle($Ctx) {
    $b = [pscustomobject]@{ reasons = [System.Collections.Generic.List[string]]::new(); session = $null; ledger = $null; entry = $null; auth = $null }
    $sr = Read-P9SSession $Ctx
    if (-not $sr.ok) { $b.reasons.Add('SESSION_UNREADABLE') }
    elseif (-not $sr.session -or [string]$sr.session.kind -cne 'AcpiValidation' -or -not $sr.session.validation) { $b.reasons.Add('ACPI_VALIDATION_SESSION_MISSING') }
    else { $b.session = $sr.session; $b.auth = $sr.session.validation }
    $lg = Read-P9SAcpiLedger $Ctx
    if (-not $lg.ok) { $b.reasons.Add($lg.reason) } else { $b.ledger = $lg.doc }
    if ($b.ledger -and $b.auth) {
        $b.entry = Get-P9SAcpiEntry $b.ledger ([string]$b.auth.authorization_id)
        if (-not $b.entry) { $b.reasons.Add('ACPI_LEDGER_ENTRY_MISSING') }
    }
    $b
}
# One fresh observation of the target, with its freshness and the state
# it must be in; adds reasons, returns the observation.
function Test-P9SAcpiTarget($Ctx, [string]$Vm, [string]$Purpose, [string]$Want, $Reasons) {
    $o = Get-P9SVmObservation $Ctx $Vm $Purpose
    $f = Test-P9SVmObservation -Observation $o -NowQpc ([long](& $Ctx.NowQpc)) -QpcFrequency $Ctx.QpcFrequency -Vm $Vm
    foreach ($r in $f.reasons) { $Reasons.Add($r) }
    if ($f.ok -and $Want -and $o.state -cne $Want) { $Reasons.Add("VM_NOT_$($Want.ToUpperInvariant()):$($o.state)") }
    $o
}
# The checks of every progressing action, again at each checkpoint: the
# monitor (halts recorded), the session, the stop request, VirtualBox and
# the authorization window.
function Test-P9SAcpiConditions($Ctx, $B, $Res, [double]$MinAvailableMb = 0, [switch]$ForShutdown) {
    $reasons = [System.Collections.Generic.List[string]]::new()
    $g = Get-P9SGate $Ctx
    $Res.status = $g.status; $Res.run_id = $g.run_id
    if ($B.session) { $B.session = Update-P9SHalt $Ctx $B.session $g $Res }
    if ($B.session -and [string]$B.session.lifecycle -cne 'ACTIVE') { $reasons.Add("SESSION_$($B.session.lifecycle)") }
    if ($B.session -and $g.run_id -and $g.run_id -ne $B.session.monitor_run_id) { $reasons.Add('MONITOR_RUN_CHANGED') }
    $e = Test-P9SAcpiValidationState -Text $g.text -NowQpc (& $Ctx.NowQpc) -QpcFrequency $Ctx.QpcFrequency -IsMonitorAlive $Ctx.IsMonitorAlive -MinAvailableMb $MinAvailableMb -ForShutdown:$ForShutdown
    foreach ($r in $e.reasons) { $reasons.Add($r) }
    if (Test-Path -LiteralPath $Ctx.StopRequestPath) { $reasons.Add('EMERGENCY_STOP_REQUESTED') }
    foreach ($r in (Get-P9SVBoxGate $Ctx).reasons) { $reasons.Add($r) }
    if ($B.auth) { $w = Test-P9SAcpiAuthorizationTime $B.auth (Get-P9SUtcNow $Ctx); if ($w) { $reasons.Add($w) } }
    [pscustomobject]@{ ok = ($reasons.Count -eq 0); reasons = @($reasons); gate = $g; eligibility = $e }
}
function Add-P9SAcpiReasons($List, $Items) { foreach ($r in @($Items)) { if ($r -and -not $List.Contains([string]$r)) { $List.Add([string]$r) } } }
# The ledger first, then the session. -BeforeDispatch: when the ledger
# cannot be written nothing else is written either (no in-flight record is
# left for a dispatch that will not happen).
function Save-P9SAcpiState($Ctx, $B, [switch]$BeforeDispatch) {
    $ok = Save-P9SAcpiLedger $Ctx $B.ledger
    if (-not $ok -and $BeforeDispatch) { return $false }
    try { Save-P9SSession $Ctx $B.session } catch { $ok = $false }
    $ok
}

# ------------------------------------------------------------ entry
function Invoke-P9SValidation($Ctx, [hashtable]$Request, $V, $Res) {
    $a = [string]$Request.Action; $dry = [bool]$Request.DryRun
    $Res.vm = Get-P9SAcpiValidationTarget
    if ($a -ceq 'acpi.preflight') { return (Invoke-P9SAcpiPreflight $Ctx $Request $V $Res) }
    # the live path: disabled in this version; refused before anything is
    # read, so that a disabled capability makes no VirtualBox call
    if ($a -cne 'acpi.confirm-off' -and -not $dry -and -not (Test-P9SAcpiLive $Ctx)) {
        Set-P9SRefused $Res @('ACPI_VALIDATION_LIVE_DISABLED') $null
        $Res.lines.Add("the restricted ACPI validation is prepared but its live path is disabled in this version of the toolkit; -DryRun shows what $a would do. Enabling it is a reviewed change authorized by the owner together with the session.")
        Add-P9SAudit -Ctx $Ctx -Res $Res -Request $Request -V $V
        return $Res
    }
    $hold = [pscustomobject]@{ lock = (Enter-P9SLock $Ctx 0) }
    if (-not $hold.lock) {
        Set-P9SRefused $Res @('COORDINATOR_BUSY') $null
        Add-P9SAudit -Ctx $Ctx -Res $Res -Request $Request -V $V
        return $Res
    }
    try {
        switch -CaseSensitive ($a) {
            'acpi.open' { $null = Invoke-P9SAcpiOpen $Ctx $Request $Res }
            'acpi.start' { $null = Invoke-P9SAcpiStart $Ctx $Request $Res }
            'acpi.observe' { $null = Invoke-P9SAcpiObserve $Ctx $Request $Res }
            'acpi.shutdown' { $null = Invoke-P9SAcpiShutdown $Ctx $Request $Res $hold }
            'acpi.confirm-off' { $null = Invoke-P9SAcpiConfirmOff $Ctx $Request $Res $hold }
            'acpi.integrity' { $null = Invoke-P9SAcpiIntegrity $Ctx $Request $Res }
            'acpi.finalize' { $null = Invoke-P9SAcpiFinalize $Ctx $Request $Res }
        }
    } catch {
        if (-not $Res.outcome -or $Res.outcome -in 'DISPATCHED', 'DONE') { $Res.outcome = 'FAILED' }
        $Res.reasons.Add('ACPI_VALIDATION_INTERNAL_ERROR'); $Res.lines.Add("internal error: $($_.Exception.Message)")
    } finally { if ($hold.lock) { $hold.lock.Dispose(); $hold.lock = $null } }
    Add-P9SAudit -Ctx $Ctx -Res $Res -Request $Request -V $V
    $Res
}

# ------------------------------------------------------------ preflight
function Invoke-P9SAcpiPreflight($Ctx, [hashtable]$Request, $V, $Res) {
    $target = Get-P9SAcpiValidationTarget
    $cap = Get-P9SAcpiValidationCapability
    $lim = Get-P9SAcpiValidationLimits
    $live = Test-P9SAcpiLive $Ctx
    $Res.lines.Add("capability $($cap.name): live path $(if ($live) { 'ENABLED' } else { 'DISABLED in this version (dry runs only)' }); target $target; one start, one ACPI request")
    $minMb = $lim.provisional_min_available_mb
    $au = $null
    if ($Request.Authorization) {
        $au = Read-P9SAcpiAuthorization -Path ([string]$Request.Authorization) -KitDir (Split-Path -Parent $Ctx.ThresholdsPath)
        if ($au.ok) {
            $w = Test-P9SAcpiAuthorizationTime $au.auth (Get-P9SUtcNow $Ctx)
            $Res.lines.Add("authorization $($au.id): valid$(if ($w) { ", but $w" } else { '' }); window $($au.auth.valid_from_utc) to $($au.auth.expires_utc); actions $(@($au.auth.permitted_actions) -join ' ')")
            if ($w) { $Res.reasons.Add($w) }
            $minMb = [double]$au.auth.min_available_mb
        } else { $Res.lines.Add("authorization: NOT VALID ($($au.reasons -join ', '))"); Add-P9SAcpiReasons $Res.reasons $au.reasons }
    } else { $Res.lines.Add('authorization: none given (-Authorization <file>); a session cannot open without one') }
    $lg = Read-P9SAcpiLedger $Ctx
    if (-not $lg.ok) { $Res.lines.Add("ledger: $($lg.reason) -> FAIL"); $Res.reasons.Add($lg.reason) }
    else {
        $un = @(Get-P9SAcpiUnresolved $lg.doc '')
        $Res.lines.Add("ledger: $(@($lg.doc.entries).Count) earlier validations, $($un.Count) unresolved -> $(if ($un.Count) { 'FAIL' } else { 'PASS' })")
        if ($un.Count) { $Res.reasons.Add('ACPI_VALIDATION_UNRESOLVED_PREVIOUS') }
        if ($Request.Authorization -and $au -and $au.ok -and (Get-P9SAcpiEntry $lg.doc $au.id)) { $Res.lines.Add("authorization $($au.id) was already used -> FAIL"); $Res.reasons.Add('AUTHORIZATION_ALREADY_USED') }
    }
    $sr = Read-P9SSession $Ctx
    $Res.lines.Add("session: $(if (-not $sr.ok) { 'UNREADABLE' } elseif ($sr.session) { "$($sr.session.session_id) kind $($sr.session.kind) $($sr.session.lifecycle)" } else { 'none' })")
    $g = Get-P9SGate $Ctx; $Res.status = $g.status; $Res.run_id = $g.run_id
    $e = Test-P9SAcpiValidationState -Text $g.text -NowQpc (& $Ctx.NowQpc) -QpcFrequency $Ctx.QpcFrequency -IsMonitorAlive $Ctx.IsMonitorAlive -MinAvailableMb $minMb
    $Res.lines.Add("monitor: status $(if ($e.status) { $e.status } else { '-' }), run $(if ($e.run_id) { $e.run_id } else { '-' }); commit $(if (Test-Finite $e.commit_pct) { ([double]$e.commit_pct).ToString('F1', [Globalization.CultureInfo]::InvariantCulture) + ' %' } else { '-' }); available $(if (Test-Finite $e.avail_mb) { [math]::Round([double]$e.avail_mb) } else { '-' }) MB (provisional minimum $minMb MB before the start, validation only, owner approval required)")
    $Res.lines.Add("paging: $(if ($e.paging_calibration) { $e.paging_calibration } else { '-' }) (recorded, never a paging pass; normal READY stays unavailable)")
    $Res.lines.Add("$($e.state)$(if (-not $e.ok) { ': ' + ($e.reasons -join ', ') })")
    Add-P9SAcpiReasons $Res.reasons $e.reasons
    $vg = Get-P9SVBoxGate $Ctx
    $Res.lines.Add("VirtualBox API: $($vg.text) -> $(if ($vg.ok) { 'PASS' } else { 'FAIL (no VirtualBox query is made)' })")
    Add-P9SAcpiReasons $Res.reasons $vg.reasons
    if ($vg.ok) {
        $tr = [System.Collections.Generic.List[string]]::new()
        $o = Test-P9SAcpiTarget $Ctx $target 'preflight' 'off' $tr
        $Res.lines.Add("target: $(Format-P9SVmObservation $o) -> $(if ($tr.Count) { 'FAIL' } else { 'PASS' })")
        Add-P9SAcpiReasons $Res.reasons $tr
    }
    $ref = if ($Ctx.Config -and $Ctx.Config.vm_config_reference) { $Ctx.Config.vm_config_reference.debian } else { $null }
    $Res.lines.Add("configuration reference: $(if ($ref -and (Test-Path -LiteralPath $ref)) { "$([IO.Path]::GetFileName($ref)) -> PASS" } else { 'not configured or missing -> FAIL' })")
    if (-not ($ref -and (Test-Path -LiteralPath $ref))) { $Res.reasons.Add('ACPI_REFERENCE_MISSING') }
    $Res.lines.Add("ACPI VALIDATION PREFLIGHT: $(if ($Res.reasons.Count) { 'NOT READY TO OPEN' } else { 'all checks passed' }); live path $(if ($live) { 'enabled' } else { 'DISABLED' })")
    $Res.outcome = 'DONE'
    Add-P9SAudit -Ctx $Ctx -Res $Res -Request $Request -V $V
    $Res
}

# ------------------------------------------------------------ open
function Invoke-P9SAcpiOpen($Ctx, [hashtable]$Request, $Res) {
    $dry = [bool]$Request.DryRun
    $reasons = [System.Collections.Generic.List[string]]::new()
    $target = Get-P9SAcpiValidationTarget
    $au = Read-P9SAcpiAuthorization -Path ([string]$Request.Authorization) -KitDir (Split-Path -Parent $Ctx.ThresholdsPath)
    Add-P9SAcpiReasons $reasons $au.reasons
    if ($au.ok) {
        $w = Test-P9SAcpiAuthorizationTime $au.auth (Get-P9SUtcNow $Ctx); if ($w) { $reasons.Add($w) }
        if (@($au.auth.permitted_actions) -cnotcontains 'acpi.open') { $reasons.Add('ACTION_NOT_AUTHORIZED:acpi.open') }
    }
    $lg = Read-P9SAcpiLedger $Ctx
    if (-not $lg.ok) { $reasons.Add($lg.reason) }
    else {
        if ($au.ok -and (Get-P9SAcpiEntry $lg.doc $au.id)) { $reasons.Add('AUTHORIZATION_ALREADY_USED') }
        if (@(Get-P9SAcpiUnresolved $lg.doc '').Count) { $reasons.Add('ACPI_VALIDATION_UNRESOLVED_PREVIOUS') }
    }
    $sr = Read-P9SSession $Ctx
    $sess = $null
    if (-not $sr.ok) { $reasons.Add('SESSION_UNREADABLE') } else { $sess = $sr.session }
    if ($sess -and $sess.session_id -and [string]$sess.lifecycle -ceq 'ACTIVE') { $reasons.Add('SESSION_ALREADY_OPEN') }
    if ($sess -and [string]$sess.kind -ceq 'AcpiValidation' -and [string]$sess.lifecycle -cne 'CLOSED') { $reasons.Add('ACPI_VALIDATION_NOT_FINALIZED') }
    if ($sess -and $sess.inflight) { $reasons.Add('INFLIGHT_UNRESOLVED') }
    if (Test-Path -LiteralPath $Ctx.StopRequestPath) { $reasons.Add('STOP_REQUEST_PENDING') }
    $g = Get-P9SGate $Ctx; $Res.status = $g.status; $Res.run_id = $g.run_id
    if ($sr.ok) { $sess = Update-P9SHalt $Ctx $sess $g $Res }
    $ackOk = $false
    if ($sess -and $sess.halt) {
        if ($g.run_id -and $g.run_id -eq [string]$sess.halt.monitor_run_id) { $reasons.Add('STOP_REQUIRES_NEW_MONITOR_RUN') }
        if ([string]$sess.halt.status -eq 'ABORT_REQUIRED' -and -not $sess.halt.acknowledged) {
            if ([string]$Request.AcknowledgeAbort -cne [string]$sess.halt.abort_id) { $reasons.Add('ABORT_NOT_ACKNOWLEDGED') } else { $ackOk = $true }
        }
    }
    $minMb = if ($au.ok) { [double]$au.auth.min_available_mb } else { [double](Get-P9SAcpiValidationLimits).provisional_min_available_mb }
    $e = Test-P9SAcpiValidationState -Text $g.text -NowQpc (& $Ctx.NowQpc) -QpcFrequency $Ctx.QpcFrequency -IsMonitorAlive $Ctx.IsMonitorAlive -MinAvailableMb $minMb
    Add-P9SAcpiReasons $reasons $e.reasons
    $vg = Get-P9SVBoxGate $Ctx
    Add-P9SAcpiReasons $reasons $vg.reasons
    $states = [ordered]@{}
    if ($vg.ok) {
        $o = Test-P9SAcpiTarget $Ctx $target 'session' 'off' $reasons
        $states[$target] = $o.state
        if ($ackOk) { foreach ($n in Get-P9SVmNames) { if ($n -cne $target) { $x = Get-P9SVmObservation $Ctx $n 'session'; $states[$n] = $x.state; if ($x.state -cne 'off') { $reasons.Add('VMS_NOT_OFF') } } } }
    }
    if ($reasons.Count) { Set-P9SRefused $Res @($reasons) $g; return $Res }
    if ($dry) { $Res.outcome = 'DRYRUN'; $Res.lines.Add("dry run: would open an ACPI validation session for $target on monitor run $($g.run_id) with authorization $($au.id) (one start, one ACPI request, until $($au.auth.expires_utc))"); return $Res }
    $prev = if ($sess) { $sess.halt } else { $null }
    if ($ackOk) { $prev.acknowledged = $true; $prev.acknowledged_utc = [DateTime]::UtcNow.ToString('o') }
    $new = New-P9SSessionRecord 'AcpiValidation' $g.run_id 'ACTIVE'
    $new.expected_vm = [ordered]@{ $target = 'off' }; $new.previous_halt = $prev
    $new.validation = $au.auth
    $entry = [ordered]@{
        authorization_id = $au.id; authorization_sha256 = $au.sha256; session_id = $new.session_id; target_vm = $target; monitor_run_id = $g.run_id
        opened_utc = [DateTime]::UtcNow.ToString('o'); valid_from_utc = $au.auth.valid_from_utc; expires_utc = $au.auth.expires_utc
        startups_used = 0; acpi_used = 0; acpi_source = $null; phase = 'OPEN'; deadline_utc = $null; integrity = $null; outcome = $null; finalized_utc = $null; events = @()
    }
    Add-P9SAcpiEvent $entry 'opened' "monitor run $($g.run_id); commit $([math]::Round([double]$e.commit_pct, 1)) %; available $([math]::Round([double]$e.avail_mb)) MB"
    $lg.doc.entries = @($lg.doc.entries) + @($entry)
    # the ledger first: if the session record then fails, the
    # authorization is spent and nothing can start
    if (-not (Save-P9SAcpiLedger $Ctx $lg.doc)) { Set-P9SRefused $Res @('EVIDENCE_WRITE_FAILED') $g; $Res.lines.Add('the ledger could not be written; no session opened'); return $Res }
    try { Save-P9SSession $Ctx $new } catch { $Res.outcome = 'FAILED'; $Res.reasons.Add('EVIDENCE_WRITE_FAILED'); $Res.lines.Add("the session record could not be written; authorization $($au.id) is spent and no start is possible with it"); return $Res }
    $Res.session = $new.session_id; $Res.outcome = 'DONE'
    $Res.lines.Add("ACPI validation session $($new.session_id) opened on monitor run $($g.run_id): authorization $($au.id), target $target, one start, one ACPI request, until $($au.auth.expires_utc); $($e.state), paging $($e.paging_calibration)$(if ($ackOk) { "; abort $($prev.abort_id) acknowledged" })")
    $Res
}

# ------------------------------------------------------------ start
function Invoke-P9SAcpiStart($Ctx, [hashtable]$Request, $Res) {
    $dry = [bool]$Request.DryRun
    $b = Get-P9SAcpiBundle $Ctx
    $reasons = [System.Collections.Generic.List[string]]::new(); Add-P9SAcpiReasons $reasons $b.reasons
    if ($b.session) { $Res.session = $b.session.session_id; if ($b.session.inflight) { $reasons.Add('INFLIGHT_UNRESOLVED') } }
    if ($b.auth -and @($b.auth.permitted_actions) -cnotcontains 'acpi.start') { $reasons.Add('ACTION_NOT_AUTHORIZED:acpi.start') }
    if ($b.entry -and $b.auth -and [int]$b.entry.startups_used -ge [int]$b.auth.max_startups) { $reasons.Add('ACPI_STARTUP_BUDGET_EXHAUSTED') }
    $minMb = if ($b.auth) { [double]$b.auth.min_available_mb } else { [double](Get-P9SAcpiValidationLimits).provisional_min_available_mb }
    $c = Test-P9SAcpiConditions $Ctx $b $Res -MinAvailableMb $minMb
    Add-P9SAcpiReasons $reasons $c.reasons
    $target = if ($b.auth) { [string]$b.auth.target_vm } else { Get-P9SAcpiValidationTarget }
    $o = $null
    if ((Get-P9SVBoxGate $Ctx).ok) { $o = Test-P9SAcpiTarget $Ctx $target 'start' 'off' $reasons }
    if ($reasons.Count) { Set-P9SRefused $Res @($reasons) $c.gate; return $Res }
    # last check, immediately before the dispatch: the monitor again, and
    # the observation must still be fresh
    $c2 = Test-P9SAcpiConditions $Ctx $b $Res -MinAvailableMb $minMb
    $f = Test-P9SVmObservation -Observation $o -NowQpc ([long](& $Ctx.NowQpc)) -QpcFrequency $Ctx.QpcFrequency -Vm $target
    if (-not $c2.ok -or -not $f.ok) { Set-P9SRefused $Res (@('STATE_CHANGED_BEFORE_DISPATCH') + @($c2.reasons) + @($f.reasons)) $c2.gate; return $Res }
    if ($dry) { $Res.outcome = 'DRYRUN'; $Res.lines.Add("dry run: would start $target once (VBoxManage $((Get-VBoxStartArgs $target) -join ' ')); the start budget would then be spent"); return $Res }
    # the budget is spent before the dispatch
    $b.entry.startups_used = [int]$b.entry.startups_used + 1; $b.entry.phase = 'STARTING'
    Add-P9SAcpiEvent $b.entry 'start-dispatching' "observed $($o.state) at $($o.observed_utc)"
    $b.session.inflight = [ordered]@{ action = 'acpi.start'; vm = $target; pid = $Ctx.Pid; start_ticks = $Ctx.ProcessStartTicks; utc = [DateTime]::UtcNow.ToString('o') }
    if (-not (Save-P9SAcpiState $Ctx $b -BeforeDispatch)) { Set-P9SRefused $Res @('EVIDENCE_WRITE_FAILED') $c2.gate; $Res.lines.Add("the ledger or the session could not be written: $target not started (the start budget counts as spent if the ledger was written)"); return $Res }
    try {
        & $Ctx.Dispatch['vm.start'] $target
        $Res.dispatched = 1; $Res.outcome = 'DISPATCHED'
        $b.entry.phase = 'RUNNING'; $b.session.expected_vm[$target] = 'running'
        Add-P9SAcpiEvent $b.entry 'started' 'VBoxManage startvm completed'
        $Res.lines.Add("started $target (the only start of this validation)")
    } catch {
        $Res.outcome = 'FAILED'; $Res.reasons.Add('DISPATCH_FAILED')
        $Res.lines.Add("start of $target failed ($($_.Exception.Message)); the start budget is spent")
        $b.entry.phase = 'START_UNCERTAIN'
        if ((Get-P9SVBoxGate $Ctx).ok) {
            $x = Get-P9SVmObservation $Ctx $target 'reconcile'
            if ($x.observed -and $x.state -ceq 'off') { $b.entry.phase = 'START_FAILED'; $b.session.expected_vm[$target] = 'off' }
            elseif ($x.observed -and $x.state -ceq 'running') { $b.entry.phase = 'RUNNING'; $b.session.expected_vm[$target] = 'running' }
            $Res.lines.Add("after the failure: $(Format-P9SVmObservation $x)")
        }
        Add-P9SAcpiEvent $b.entry 'start-failed' $b.entry.phase
        if ($b.entry.phase -ceq 'START_UNCERTAIN') { $Res.reasons.Add('ACPI_VALIDATION_UNRESOLVED') }
    }
    $g2 = Get-P9SGate $Ctx
    $b.session = Update-P9SHalt $Ctx $b.session $g2 $Res
    $b.session.inflight = $null
    if (-not (Save-P9SAcpiState $Ctx $b)) { $Res.reasons.Add('EVIDENCE_WRITE_FAILED'); $Res.lines.Add('the outcome of the start could not be recorded; the ledger keeps the earlier phase (the budget stays spent)') }
    if ([string]$b.session.lifecycle -like 'HALTED*') { $Res.lines.Add("the monitor halted the session during the start: no further validation step; shut $target down with emergency.acpi-shutdown") }
    $Res
}

# ------------------------------------------------------------ observe
function Invoke-P9SAcpiObserve($Ctx, [hashtable]$Request, $Res) {
    $dry = [bool]$Request.DryRun
    $b = Get-P9SAcpiBundle $Ctx
    $reasons = [System.Collections.Generic.List[string]]::new(); Add-P9SAcpiReasons $reasons $b.reasons
    if ($b.session) { $Res.session = $b.session.session_id; if ($b.session.inflight) { $reasons.Add('INFLIGHT_UNRESOLVED') } }
    if ($b.auth -and @($b.auth.permitted_actions) -cnotcontains 'acpi.observe') { $reasons.Add('ACTION_NOT_AUTHORIZED:acpi.observe') }
    if ($b.entry -and [int]$b.entry.startups_used -lt 1) { $reasons.Add('ACPI_NOT_STARTED') }
    $c = Test-P9SAcpiConditions $Ctx $b $Res
    Add-P9SAcpiReasons $reasons $c.reasons
    $target = if ($b.auth) { [string]$b.auth.target_vm } else { Get-P9SAcpiValidationTarget }
    if ((Get-P9SVBoxGate $Ctx).ok) { $o = Test-P9SAcpiTarget $Ctx $target 'observe' 'running' $reasons; if ($o.observed) { $Res.lines.Add("begin: $(Format-P9SVmObservation $o)") } }
    if ($reasons.Count) { Set-P9SRefused $Res @($reasons) $c.gate; return $Res }
    $secs = [int]$Request.Seconds
    if ($dry) { $Res.outcome = 'DRYRUN'; $Res.lines.Add("dry run: would observe $target for $secs s with a monitor check every 5 s and one VM-state query at each end"); return $Res }
    $left = $secs; $stopped = $false
    while ($left -gt 0) {
        $step = [math]::Min(5, $left)
        & $Ctx.Sleep ($step * 1000)
        $left -= $step
        $k = Test-P9SAcpiConditions $Ctx $b $Res
        if (-not $k.ok) { $stopped = $true; $Res.outcome = 'STOPPED_AT_CHECKPOINT'; Add-P9SAcpiReasons $Res.reasons $k.reasons; $Res.lines.Add("observation stopped with $left s left ($($k.reasons -join ', '))"); break }
    }
    $vgEnd = Get-P9SVBoxGate $Ctx
    if ($vgEnd.ok) { $x = Get-P9SVmObservation $Ctx $target 'observe'; $Res.lines.Add("end: $(Format-P9SVmObservation $x)") } else { $Res.lines.Add("end: $target not read (VirtualBox $($vgEnd.text))") }
    Add-P9SAcpiEvent $b.entry 'observed' "$($secs - $left) of $secs s$(if ($stopped) { '; stopped at a checkpoint' })"
    if (-not (Save-P9SAcpiLedger $Ctx $b.ledger)) { $Res.reasons.Add('EVIDENCE_WRITE_FAILED') }
    if (-not $stopped) { $Res.outcome = 'DONE' }
    $Res
}

# ------------------------------------------------------------ the one ACPI request
function Invoke-P9SAcpiShutdown($Ctx, [hashtable]$Request, $Res, $Hold) {
    $dry = [bool]$Request.DryRun
    $lim = Get-P9SAcpiValidationLimits
    $b = Get-P9SAcpiBundle $Ctx
    $reasons = [System.Collections.Generic.List[string]]::new(); Add-P9SAcpiReasons $reasons $b.reasons
    if ($b.session) { $Res.session = $b.session.session_id; if ($b.session.inflight) { $reasons.Add('INFLIGHT_UNRESOLVED') } }
    if ($b.auth -and @($b.auth.permitted_actions) -cnotcontains 'acpi.shutdown') { $reasons.Add('ACTION_NOT_AUTHORIZED:acpi.shutdown') }
    if ($b.entry -and [int]$b.entry.startups_used -lt 1) { $reasons.Add('ACPI_NOT_STARTED') }
    if ($b.entry -and $b.auth -and [int]$b.entry.acpi_used -ge [int]$b.auth.max_acpi_requests) { $reasons.Add('ACPI_BUDGET_EXHAUSTED') }
    $c = Test-P9SAcpiConditions $Ctx $b $Res -ForShutdown
    Add-P9SAcpiReasons $reasons $c.reasons
    $target = if ($b.auth) { [string]$b.auth.target_vm } else { Get-P9SAcpiValidationTarget }
    $o = $null
    if ((Get-P9SVBoxGate $Ctx).ok) { $o = Test-P9SAcpiTarget $Ctx $target 'shutdown' 'running' $reasons }
    if ($reasons.Count) {
        Set-P9SRefused $Res @($reasons) $c.gate
        if (@($reasons | Where-Object { $_ -like 'SESSION_HALTED*' -or $_ -eq 'EMERGENCY_STOP_REQUESTED' -or $_ -like 'AUTHORIZATION_*' -or $_ -like 'STATE_*' -or $_ -eq 'MONITOR_NOT_RUNNING' }).Count) { $Res.lines.Add("the validation request is not available now; to shut $target down use emergency.acpi-shutdown (one request; it knows whether this validation already sent one)") }
        return $Res
    }
    $limit = if ($Request.ContainsKey('Seconds') -and $Request.Seconds) { [int]$Request.Seconds } else { [int]$lim.shutdown_wait_s }
    $c2 = Test-P9SAcpiConditions $Ctx $b $Res -ForShutdown
    $f = Test-P9SVmObservation -Observation $o -NowQpc ([long](& $Ctx.NowQpc)) -QpcFrequency $Ctx.QpcFrequency -Vm $target
    if (-not $c2.ok -or -not $f.ok) { Set-P9SRefused $Res (@('STATE_CHANGED_BEFORE_DISPATCH') + @($c2.reasons) + @($f.reasons)) $c2.gate; return $Res }
    if ($dry) { $Res.outcome = 'DRYRUN'; $Res.lines.Add("dry run: would send one ACPI power-button request to $target (VBoxManage $((Get-VBoxAcpiArgs $target) -join ' ')) and observe it every 5 s for up to $limit s; success only when it is observed off; no keyboard input, no forced power-off"); return $Res }
    # the budget is spent before the dispatch
    $b.entry.acpi_used = [int]$b.entry.acpi_used + 1; $b.entry.acpi_source = 'validation'; $b.entry.phase = 'SHUTDOWN_REQUESTING'
    $b.entry.deadline_utc = (Get-P9SUtcNow $Ctx).AddSeconds($limit).ToString('o')
    Add-P9SAcpiEvent $b.entry 'acpi-dispatching' "observed $($o.state) at $($o.observed_utc); deadline $limit s"
    $b.session.inflight = [ordered]@{ action = 'acpi.shutdown'; vm = $target; pid = $Ctx.Pid; start_ticks = $Ctx.ProcessStartTicks; utc = [DateTime]::UtcNow.ToString('o') }
    if (-not (Save-P9SAcpiState $Ctx $b -BeforeDispatch)) { Set-P9SRefused $Res @('EVIDENCE_WRITE_FAILED') $c2.gate; $Res.lines.Add("the ledger or the session could not be written: no request sent to $target (the ACPI budget counts as spent if the ledger was written)"); return $Res }
    try { & $Ctx.Dispatch['vm.acpi'] $target }
    catch {
        $Res.outcome = 'FAILED'; Add-P9SAcpiReasons $Res.reasons @('ACPI_REQUEST_FAILED', 'ACPI_VALIDATION_UNRESOLVED')
        $Res.lines.Add("*** the ACPI request to $target failed ($($_.Exception.Message)). ACPI_VALIDATION_UNRESOLVED: no second request, no keyboard input, no forced power-off; the operator decides outside the coordinator. ***")
        $b.entry.phase = 'ACPI_VALIDATION_UNRESOLVED'; Add-P9SAcpiEvent $b.entry 'acpi-failed' 'request not confirmed'
        $b.session.inflight = $null
        if (-not (Save-P9SAcpiState $Ctx $b)) { $Res.reasons.Add('EVIDENCE_WRITE_FAILED') }
        return $Res
    }
    $Res.dispatched = 1
    $Res.lines.Add("ACPI power-button request sent to $target (the only request of this validation)")
    $b.entry.phase = 'SHUTDOWN_REQUESTED'; Add-P9SAcpiEvent $b.entry 'acpi-sent' 'VBoxManage controlvm acpipowerbutton completed'
    $b.session.inflight = $null
    if (-not (Save-P9SAcpiState $Ctx $b)) { $Res.reasons.Add('EVIDENCE_WRITE_FAILED') }
    # the lock is released while the VM is watched, so that the emergency
    # path stays available
    $Hold.lock.Dispose(); $Hold.lock = $null
    $w = Watch-P9SAcpiTarget $Ctx $target $limit
    $Hold.lock = Enter-P9SLock $Ctx ([int]$Ctx.EmergencyLockWaitMs)
    Complete-P9SAcpiWatch $Ctx $Res $w $target $limit $(if ($Hold.lock) { $true } else { $false }) -Deadline
    $Res
}
# Observes the target every 5 s for up to $Limit s (at most
# floor($Limit / 5) + 1 queries): off, aborted, still running at the
# deadline, unreadable, or VirtualBox in doubt. Sends nothing.
function Watch-P9SAcpiTarget($Ctx, [string]$Vm, [int]$Limit) {
    $elapsed = 0; $last = $null; $n = 0
    $max = [math]::Floor($Limit / (Get-P9SVmObservationInterval)) + 1
    while ($true) {
        $vg = Get-P9SVBoxGate $Ctx
        if (-not $vg.ok) { return [pscustomobject]@{ end = 'VBOX'; elapsed = $elapsed; obs = $last; vbox = $vg; queries = $n } }
        $last = Get-P9SVmObservation $Ctx $Vm 'confirm'; $n++
        if (-not $last.observed) { return [pscustomobject]@{ end = 'UNREADABLE'; elapsed = $elapsed; obs = $last; vbox = $vg; queries = $n } }
        if ($last.state -ceq 'off') { return [pscustomobject]@{ end = 'OFF'; elapsed = $elapsed; obs = $last; vbox = $vg; queries = $n } }
        if ($last.state -ceq 'aborted') { return [pscustomobject]@{ end = 'ABORTED'; elapsed = $elapsed; obs = $last; vbox = $vg; queries = $n } }
        if ($elapsed -ge $Limit -or $n -ge $max) { return [pscustomobject]@{ end = 'DEADLINE'; elapsed = $elapsed; obs = $last; vbox = $vg; queries = $n } }
        & $Ctx.Sleep ((Get-P9SVmObservationInterval) * 1000); $elapsed += (Get-P9SVmObservationInterval)
    }
}
# Records the end of a watch in the ledger and the session (under the
# coordinator lock). -Deadline: the watch of the request itself, whose
# deadline it is; otherwise a later confirmation, judged against the
# recorded deadline.
function Complete-P9SAcpiWatch($Ctx, $Res, $W, [string]$Target, [int]$Limit, [bool]$Locked, [switch]$Deadline) {
    $phase = $null
    switch ($W.end) {
        'OFF' {
            $Res.outcome = 'DONE'
            $Res.lines.Add("$Target observed off $(if ($Deadline) { "about $($W.elapsed) s after the ACPI request (deadline $Limit s)" } else { 'by the confirmation' }): $(Format-P9SVmObservation $W.obs)")
        }
        'ABORTED' { $Res.outcome = 'OFF_ABNORMAL'; $Res.reasons.Add('VM_ABORTED'); $Res.lines.Add("$Target ended as aborted, not by an orderly shutdown"); $phase = 'OFF_ABNORMAL' }
        'DEADLINE' {
            $Res.outcome = 'TIMEOUT'; Add-P9SAcpiReasons $Res.reasons @('VM_NOT_OFF', $(if ($Deadline) { 'ACPI_VALIDATION_UNRESOLVED' }))
            $Res.lines.Add("*** $Target is still $($W.obs.state) after $($W.elapsed) s ($($W.queries) queries). $(if ($Deadline) { 'ACPI_VALIDATION_UNRESOLVED: no second request, no keyboard input, no forced power-off; the operator decides outside the coordinator; acpi.confirm-off can watch again, read only.' } else { 'Still unresolved.' }) ***")
        }
        'UNREADABLE' {
            $Res.outcome = 'FAILED'; Add-P9SAcpiReasons $Res.reasons @('VM_STATE_UNOBSERVED', 'ACPI_VALIDATION_UNRESOLVED')
            $Res.lines.Add("*** the state of $Target could not be read ($($W.obs.reason)); it is unknown, never taken for off. ACPI_VALIDATION_UNRESOLVED: no further query, no second request. ***")
        }
        'VBOX' {
            $Res.outcome = 'FAILED'; Add-P9SAcpiReasons $Res.reasons (@($W.vbox.reasons) + @('ACPI_VALIDATION_UNRESOLVED'))
            $Res.lines.Add("*** VirtualBox became $($W.vbox.text) while $Target was watched; its state is unknown. ACPI_VALIDATION_UNRESOLVED: no further query, no second request; vbox.status shows the quarantine. ***")
        }
    }
    if (-not $Locked) { $Res.reasons.Add('EVIDENCE_WRITE_FAILED'); $Res.lines.Add('the coordinator lock was not free in time: the outcome is not recorded; the ledger keeps the earlier phase (unresolved until acpi.confirm-off)'); return }
    $b = Get-P9SAcpiBundle $Ctx
    if (-not $b.entry) {
        # the session record may be gone (after acpi.finalize): the latest unresolved entry for the target
        $lg = Read-P9SAcpiLedger $Ctx
        if ($lg.ok) { $b.ledger = $lg.doc; $b.entry = @(Get-P9SAcpiUnresolved $lg.doc $Target) | Select-Object -Last 1 }
    }
    if (-not $b.entry) { $Res.reasons.Add('EVIDENCE_WRITE_FAILED'); $Res.lines.Add('the ledger entry was not found: the outcome is not recorded'); return }
    if ($W.end -ceq 'OFF') {
        $late = $false
        if (-not $Deadline) { $dl = ConvertTo-P9SAuthTime $b.entry.deadline_utc; $late = -not $dl -or (Get-P9SUtcNow $Ctx) -gt $dl }
        $phase = switch ([string]$b.entry.phase) {
            'SHUTDOWN_REQUESTED' { if ($late) { 'OFF_AFTER_DEADLINE' } else { 'OFF_CONFIRMED' } }
            # the dispatch was never confirmed (its coordinator ended during it)
            'SHUTDOWN_REQUESTING' { 'OFF_REQUEST_UNCONFIRMED' }
            'ACPI_VALIDATION_UNRESOLVED' { if ([int]$b.entry.acpi_used -ge 1 -and [string]$b.entry.acpi_source) { 'OFF_AFTER_DEADLINE' } else { 'OFF_WITHOUT_ACPI' } }
            default { 'OFF_WITHOUT_ACPI' }
        }
        if ($phase -ceq 'OFF_AFTER_DEADLINE' -and $Res.outcome -ceq 'DONE') { $Res.reasons.Add('SHUTDOWN_AFTER_DEADLINE'); $Res.lines.Add('the power-off was observed after the deadline: the validation records it as late') }
        if ($b.session -and $b.session.expected_vm) { $b.session.expected_vm[$Target] = 'off' }
    } elseif ($W.end -ceq 'ABORTED') { if ($b.session -and $b.session.expected_vm) { $b.session.expected_vm[$Target] = 'aborted' } }
    elseif ($Deadline -or [string]$b.entry.phase -cin 'SHUTDOWN_REQUESTED', 'SHUTDOWN_REQUESTING') { $phase = 'ACPI_VALIDATION_UNRESOLVED' }
    if ($phase) { $b.entry.phase = $phase }
    Add-P9SAcpiEvent $b.entry $(if ($Deadline) { 'acpi-watch' } else { 'confirm-off' }) "$($W.end) after $($W.elapsed) s, $($W.queries) queries; phase $($b.entry.phase)"
    $ok = Save-P9SAcpiLedger $Ctx $b.ledger
    if ($b.session) { try { Save-P9SSession $Ctx $b.session } catch { $ok = $false } }
    if (-not $ok) { $Res.reasons.Add('EVIDENCE_WRITE_FAILED'); $Res.lines.Add('the outcome could not be recorded; the ledger keeps the earlier phase (unresolved until acpi.confirm-off)') }
}

# ------------------------------------------------------------ confirmation (read only)
function Invoke-P9SAcpiConfirmOff($Ctx, [hashtable]$Request, $Res, $Hold) {
    $dry = [bool]$Request.DryRun
    $lg = Read-P9SAcpiLedger $Ctx
    if (-not $lg.ok) { Set-P9SRefused $Res @($lg.reason) $null; return $Res }
    $sr = Read-P9SSession $Ctx
    $entry = $null
    if ($sr.ok -and $sr.session -and [string]$sr.session.kind -ceq 'AcpiValidation' -and $sr.session.validation) { $entry = Get-P9SAcpiEntry $lg.doc ([string]$sr.session.validation.authorization_id); $Res.session = $sr.session.session_id }
    if (-not $entry -or (Test-P9SAcpiEntryResolved $entry)) { $u = @(Get-P9SAcpiUnresolved $lg.doc ''); if ($u.Count) { $entry = $u[-1] } }
    if (-not $entry -or (Test-P9SAcpiEntryResolved $entry)) { Set-P9SRefused $Res @('ACPI_NOTHING_TO_CONFIRM') $null; $Res.lines.Add('no ACPI validation is waiting for a confirmation'); return $Res }
    $target = [string]$entry.target_vm
    $vg = Get-P9SVBoxGate $Ctx
    if (-not $vg.ok) { Set-P9SRefused $Res @($vg.reasons) $null; $Res.lines.Add("VirtualBox is $($vg.text): the state of $target is not read and stays unknown"); return $Res }
    $secs = [int]$Request.Seconds
    if ($dry) { $Res.outcome = 'DRYRUN'; $Res.lines.Add("dry run: would observe $target every 5 s for up to $secs s (validation $($entry.authorization_id), phase $($entry.phase)); read only, nothing is sent"); return $Res }
    $Hold.lock.Dispose(); $Hold.lock = $null
    $w = Watch-P9SAcpiTarget $Ctx $target $secs
    $Hold.lock = Enter-P9SLock $Ctx ([int]$Ctx.EmergencyLockWaitMs)
    Complete-P9SAcpiWatch $Ctx $Res $w $target $secs $(if ($Hold.lock) { $true } else { $false })
    $Res
}

# ------------------------------------------------------------ integrity (read only)
function Invoke-P9SAcpiIntegrity($Ctx, [hashtable]$Request, $Res) {
    $dry = [bool]$Request.DryRun
    $b = Get-P9SAcpiBundle $Ctx
    $reasons = [System.Collections.Generic.List[string]]::new(); Add-P9SAcpiReasons $reasons $b.reasons
    if ($b.session) { $Res.session = $b.session.session_id; if ([string]$b.session.lifecycle -ceq 'CLOSED') { $reasons.Add('SESSION_CLOSED') }; if ($b.session.inflight) { $reasons.Add('INFLIGHT_UNRESOLVED') } }
    if ($b.auth -and @($b.auth.permitted_actions) -cnotcontains 'acpi.integrity') { $reasons.Add('ACTION_NOT_AUTHORIZED:acpi.integrity') }
    $ref = if ($Request.Reference) { [string]$Request.Reference } elseif ($Ctx.Config -and $Ctx.Config.vm_config_reference) { [string]$Ctx.Config.vm_config_reference.debian } else { $null }
    if (-not $ref -or -not (Test-Path -LiteralPath $ref -PathType Leaf)) { $reasons.Add('ACPI_REFERENCE_MISSING') }
    $vg = Get-P9SVBoxGate $Ctx; Add-P9SAcpiReasons $reasons $vg.reasons
    $target = if ($b.auth) { [string]$b.auth.target_vm } else { Get-P9SAcpiValidationTarget }
    if ($vg.ok) { $null = Test-P9SAcpiTarget $Ctx $target 'integrity' 'off' $reasons }
    if ($reasons.Count) { Set-P9SRefused $Res @($reasons) $null; return $Res }
    if ($dry) { $Res.outcome = 'DRYRUN'; $Res.lines.Add("dry run: would read the configuration of $target once and compare it with $([IO.Path]::GetFileName($ref)) (key names only)"); return $Res }
    $cur = @(& $Ctx.ReadVmConfig $target)
    if (-not $cur.Count) { Set-P9SRefused $Res @('VM_CONFIG_UNREADABLE') $null; return $Res }
    $d = @(Compare-VmConfig -Reference (Get-Content -LiteralPath $ref) -Current $cur)
    $b.entry.integrity = [ordered]@{ utc = [DateTime]::UtcNow.ToString('o'); reference = [IO.Path]::GetFileName($ref); differing = $d.Count; keys = @($d | ForEach-Object { [string]$_.key }) }
    Add-P9SAcpiEvent $b.entry 'integrity' "$($d.Count) differing keys"
    if (-not (Save-P9SAcpiLedger $Ctx $b.ledger)) { $Res.reasons.Add('EVIDENCE_WRITE_FAILED') }
    $Res.lines.Add("$target configuration against $([IO.Path]::GetFileName($ref)): $($d.Count) differing keys")
    foreach ($x in $d) { $Res.lines.Add("  $($x.key) $($x.change)") }
    if ($d.Count) { $Res.reasons.Add("VM_CONFIG_CHANGED:$($d.Count)") }
    $Res.outcome = 'DONE'
    $Res
}

# ------------------------------------------------------------ finalize
function Get-P9SAcpiOutcome($E) {
    switch ([string]$E.phase) {
        'OPEN' { 'NOT_STARTED' }
        'START_FAILED' { 'START_FAILED' }
        'OFF_CONFIRMED' { if ([string]$E.acpi_source -ceq 'validation') { 'VALIDATED' } else { 'OFF_AFTER_EMERGENCY_ACPI' } }
        'OFF_AFTER_DEADLINE' { 'SHUTDOWN_LATE' }
        'OFF_REQUEST_UNCONFIRMED' { 'OFF_AFTER_UNCONFIRMED_REQUEST' }
        'OFF_WITHOUT_ACPI' { 'ENDED_WITHOUT_ACPI' }
        'OFF_ABNORMAL' { 'OFF_ABNORMAL' }
        default { 'ACPI_VALIDATION_UNRESOLVED' }
    }
}
function Invoke-P9SAcpiFinalize($Ctx, [hashtable]$Request, $Res) {
    $dry = [bool]$Request.DryRun
    $b = Get-P9SAcpiBundle $Ctx
    $reasons = [System.Collections.Generic.List[string]]::new(); Add-P9SAcpiReasons $reasons $b.reasons
    if ($b.session -and [string]$b.session.lifecycle -ceq 'CLOSED') { $reasons.Add('SESSION_CLOSED') }
    if ($reasons.Count) { Set-P9SRefused $Res @($reasons) $null; return $Res }
    $Res.session = $b.session.session_id
    $target = [string]$b.auth.target_vm
    $note = $null
    if ($b.session.inflight) {
        $note = "in-flight $($b.session.inflight.action) of an ended coordinator discarded; its effect is unknown"
        if ([string]$b.entry.phase -ceq 'STARTING') { $b.entry.phase = 'START_UNCERTAIN' }
        elseif ([string]$b.entry.phase -ceq 'SHUTDOWN_REQUESTING') { $b.entry.phase = 'ACPI_VALIDATION_UNRESOLVED' }
    }
    $obs = if ((Get-P9SVBoxGate $Ctx).ok) { Get-P9SVmObservation $Ctx $target 'finalize' } else { $null }
    $outcome = Get-P9SAcpiOutcome $b.entry
    if ($dry) { $Res.outcome = 'DRYRUN'; $Res.lines.Add("dry run: would close the validation with outcome $outcome (phase $($b.entry.phase))"); return $Res }
    $b.entry.outcome = $outcome; $b.entry.finalized_utc = [DateTime]::UtcNow.ToString('o')
    if ($b.session.halt) { $b.entry.halt = [string]$b.session.halt.status }
    Add-P9SAcpiEvent $b.entry 'finalized' "$outcome; final observation $(if ($obs) { $obs.state } else { 'not made (VirtualBox in doubt)' })$(if ($note) { "; $note" })"
    if (-not (Save-P9SAcpiLedger $Ctx $b.ledger)) { Set-P9SRefused $Res @('EVIDENCE_WRITE_FAILED') $null; $Res.lines.Add('the ledger could not be written; the session stays open'); return $Res }
    $b.session.lifecycle = 'CLOSED'; $b.session.inflight = $null
    try { Save-P9SSession $Ctx $b.session } catch { $Res.reasons.Add('EVIDENCE_WRITE_FAILED') }
    $Res.outcome = 'DONE'
    $Res.lines.Add("ACPI validation $($b.entry.authorization_id) closed: outcome $outcome; starts $($b.entry.startups_used) of 1, ACPI requests $($b.entry.acpi_used) of 1$(if ($b.entry.integrity) { "; configuration $($b.entry.integrity.differing) differing keys" } else { '; configuration not compared' }); final state $(if ($obs) { Format-P9SVmObservation $obs } else { 'not read' })$(if ($note) { "; $note" })")
    if (-not (Test-P9SAcpiEntryResolved $b.entry)) {
        Add-P9SAcpiReasons $Res.reasons @('ACPI_VALIDATION_UNRESOLVED')
        $Res.lines.Add("*** UNRESOLVED: no new ACPI validation and no test session can open until acpi.confirm-off observes $target off. ***")
    }
    $Res
}
