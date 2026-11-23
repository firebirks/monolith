#Requires -Version 7.0
# P9-S action coordinator (library; dot-source hostmon-lib.ps1 first).
# The supported way for P9-S automation to change laboratory state. Every
# action is a named entry of Get-P9SActionSpec with validated parameters; no
# free command, no passthrough. Classes:
#   normal       vm.start, guest.type/key/unlock, scenario.begin,
#                phase.advance, wait, sequence.run: need a fresh READY
#                monitor state with the paging stop enforced, an ACTIVE test
#                session bound to that monitor run, VM states as the session
#                expects, and a second check immediately before dispatch;
#                long actions recheck at checkpoints
#   session      session.open (Test; Calibration only as a dry run),
#                session.close
#   emergency    emergency.status, emergency.acpi-shutdown (one ACPI
#                power-button request per VM, then the VM state is watched;
#                success only when the VM is off), emergency.wait-off: no gate
#   observe      session.status, audit.oob, audit.vmconfig, vbox.status:
#                read only
#   calibration  calib.preflight (read only), calib.dry-run (simulated): no
#                live calibration exists in this version
#   validation   acpi.*: the restricted single-VM ACPI validation of
#                p9s-acpival.ps1 (one start and one ACPI request of
#                P9S-D12 under an owner authorization); without a valid
#                authorization nothing live runs
# STOP or ABORT seen in any monitor state halts the session for good; ABORT
# must be acknowledged with its id, with every laboratory VM off, before a
# new session opens. VM states are observations made on demand
# (p9s-vmstate.ps1), never taken from the resource monitor, which makes no
# VirtualBox call. Every VirtualBox call goes through p9s-vbox.ps1 (one
# host-wide lock, bounded waits, quarantine); while a quarantine is in place
# no test action, session or ACPI request is possible, and vbox.recover
# clears it at the operator's request. The coordinator cannot stop anyone
# from running VBoxManage or other tools directly: it governs the approved
# automation only. Machine-local settings and secrets come from
# p9s-config.ps1.
. (Join-Path $PSScriptRoot 'p9s-config.ps1')
. (Join-Path $PSScriptRoot 'p9s-vbox.ps1')
. (Join-Path $PSScriptRoot 'p9s-vmstate.ps1')
. (Join-Path $PSScriptRoot 'p9s-acpival.ps1')
$script:P9SCoordinatorLibFile = $PSCommandPath

# ------------------------------------------------------------ allowlists
# VM names: from the local configuration (Set-P9SVmNames), else the defaults.
function Set-P9SVmNames([string[]]$Names) { $script:P9SVmNames = @($Names) }
function Get-P9SVmNames { if ($script:P9SVmNames) { $script:P9SVmNames } else { Get-P9SDefaultVmNames } }
# Actions that existed earlier or were asked for and are deliberately absent.
function Get-P9SRetiredActions {
    [ordered]@{
        'emergency.guest-poweroff' = 'typing into a guest window is not a safe shutdown; use emergency.acpi-shutdown'
        'guest.password'           = 'account and sudo passwords are not handled by the toolkit'
        'guest.sudo'               = 'privileged guest commands are not supported'
        'guest.login'              = 'the guest desktop login is an operator step'
    }
}
function Get-P9SKeyMap {
    [ordered]@{
        Enter = @(0x1c, 0x9c); Escape = @(0x01, 0x81); Tab = @(0x0f, 0x8f); ShiftTab = @(0x2a, 0x0f, 0x8f, 0xaa)
        AltTab = @(0x38, 0x0f, 0x8f, 0xb8); Space = @(0x39, 0xb9); Backspace = @(0x0e, 0x8e); Down = @(0xe0, 0x50, 0xe0, 0xd0)
        KeepAlive = @(0x2a, 0xaa); CtrlAltT = @(0x1d, 0x38, 0x14, 0x94, 0xb8, 0x9d); CtrlShiftT = @(0x1d, 0x2a, 0x14, 0x94, 0xaa, 0x9d)
        Super = @(0xe0, 0x5b, 0xe0, 0xdb); SuperUp = @(0xe0, 0x5b, 0xe0, 0x48, 0xe0, 0xc8, 0xe0, 0xdb); AltF4 = @(0x38, 0x3e, 0xbe, 0xb8)
        CtrlQ = @(0x1d, 0x10, 0x90, 0x9d); CtrlU = @(0x1d, 0x16, 0x96, 0x9d); CtrlC = @(0x1d, 0x2e, 0xae, 0x9d); CtrlA = @(0x1d, 0x1e, 0x9e, 0x9d)
    }
}
function Get-P9SScenarios { 'nominal', 'warning-during-start', 'paging-warning', 'low-memory', 'missing-counters', 'abort-during-observe', 'monitor-mode', 'stale-monitor', 'acpi-timeout' }
function Get-P9SPlanStepActions { 'vm.start', 'guest.type', 'guest.key', 'guest.unlock', 'scenario.begin', 'phase.advance', 'wait' }
function Get-P9SCalibrationStepKinds { 'preflight', 'vm-start', 'settle', 'operator-assist', 'memory-check', 'qualify-300s', 'observe', 'acpi-shutdown', 'wait-off' }
function Test-P9SCalibrationStepKind([string]$Kind) { [bool](@(Get-P9SCalibrationStepKinds) -ccontains $Kind) }
# Planning values of the future supervised calibration (owner brief of stage
# C). The approved monitor limits are those of hostmon-lib.ps1; the 8 GiB
# before the second VM is a proposal that needs validation and approval.
function Get-P9SCalibrationLimits {
    [pscustomobject]@{
        settle_s = 120; qualify_gate_s = 300; qualify_cap_s = 900; observe_s = 1800
        second_vm_min_available_mb = 8192; operator_assist_max_s = 600; shutdown_wait_s = 180
    }
}
function Get-P9SActionSpec {
    [ordered]@{
        'session.open'             = @{ class = 'session'; vm = $false; required = @('Kind'); optional = @('AcknowledgeAbort') }
        'session.close'            = @{ class = 'session'; vm = $false; required = @(); optional = @() }
        'session.status'           = @{ class = 'observe'; vm = $false; required = @(); optional = @() }
        'vm.start'                 = @{ class = 'normal'; vm = $true; required = @(); optional = @() }
        'guest.type'               = @{ class = 'normal'; vm = $true; required = @('Text'); optional = @() }
        'guest.key'                = @{ class = 'normal'; vm = $true; required = @('Key'); optional = @() }
        'guest.unlock'             = @{ class = 'normal'; vm = $true; required = @(); optional = @() }
        'scenario.begin'           = @{ class = 'normal'; vm = $false; required = @('Name'); optional = @() }
        'phase.advance'            = @{ class = 'normal'; vm = $false; required = @('Name'); optional = @() }
        'wait'                     = @{ class = 'normal'; vm = $false; required = @('Seconds'); optional = @() }
        'sequence.run'             = @{ class = 'normal'; vm = $false; required = @('Plan'); optional = @() }
        'emergency.status'         = @{ class = 'emergency'; vm = $false; required = @(); optional = @() }
        'emergency.acpi-shutdown'  = @{ class = 'emergency'; vm = $true; required = @(); optional = @('Seconds') }
        'emergency.wait-off'       = @{ class = 'emergency'; vm = $true; required = @('Seconds'); optional = @() }
        'calib.preflight'          = @{ class = 'calibration'; vm = $false; required = @(); optional = @() }
        'calib.dry-run'            = @{ class = 'calibration'; vm = $false; required = @('Scenario'); optional = @() }
        'audit.oob'                = @{ class = 'observe'; vm = $false; required = @('MonitorRun'); optional = @() }
        'audit.vmconfig'           = @{ class = 'observe'; vm = $true; required = @('Reference'); optional = @() }
        'vbox.status'              = @{ class = 'observe'; vm = $false; required = @(); optional = @() }
        'vbox.recover'             = @{ class = 'session'; vm = $false; required = @('Operation'); optional = @() }
        'acpi.preflight'           = @{ class = 'validation'; vm = $false; required = @(); optional = @('Authorization') }
        'acpi.open'                = @{ class = 'validation'; vm = $false; required = @('Authorization'); optional = @('AcknowledgeAbort') }
        'acpi.start'               = @{ class = 'validation'; vm = $false; required = @(); optional = @() }
        'acpi.observe'             = @{ class = 'validation'; vm = $false; required = @('Seconds'); optional = @() }
        'acpi.shutdown'            = @{ class = 'validation'; vm = $false; required = @(); optional = @('Seconds') }
        'acpi.confirm-off'         = @{ class = 'validation'; vm = $false; required = @('Seconds'); optional = @() }
        'acpi.integrity'           = @{ class = 'validation'; vm = $false; required = @(); optional = @('Reference') }
        'acpi.finalize'            = @{ class = 'validation'; vm = $false; required = @(); optional = @() }
    }
}

# ------------------------------------------------------------ validation
# Checks the request against the allowlists before anything else happens:
# no state is read and nothing is dispatched for a rejected request.
function Test-P9SRequest {
    param([hashtable]$Request, [switch]$AsPlanStep)
    $reasons = [System.Collections.Generic.List[string]]::new()
    $valid = @{}
    $a = [string]$Request['Action']
    $specs = Get-P9SActionSpec
    $spec = if ($a -cmatch '^[a-z][a-z.-]{0,40}$' -and $specs.Contains($a)) { $specs[$a] } else { $null }
    if (-not $spec -and $a -cmatch '^[a-z][a-z.-]{0,40}$' -and (Get-P9SRetiredActions).Contains($a)) { return [pscustomobject]@{ ok = $false; reasons = @("ACTION_RETIRED:$a"); spec = $null; valid = $valid; plan = $null } }
    if (-not $spec) { return [pscustomobject]@{ ok = $false; reasons = @('UNKNOWN_ACTION'); spec = $null; valid = $valid; plan = $null } }
    if ($AsPlanStep -and $a -cnotin (Get-P9SPlanStepActions)) { return [pscustomobject]@{ ok = $false; reasons = @('PLAN_STEP_NOT_ALLOWED'); spec = $spec; valid = $valid; plan = $null } }
    $valid.Action = $true
    $allowed = [System.Collections.Generic.List[string]]::new(); foreach ($x in @('Action', 'DryRun') + @($spec.required) + @($spec.optional)) { $allowed.Add($x) }
    $needed = [System.Collections.Generic.List[string]]::new(); foreach ($x in @($spec.required)) { $needed.Add($x) }
    if ($spec.vm) { $allowed.Add('Vm'); $needed.Add('Vm') }
    foreach ($k in @($Request.Keys)) { if (-not ($allowed -contains $k)) { $reasons.Add("PARAMETER_NOT_ALLOWED:$k") } }
    foreach ($k in $needed) { if ($null -eq $Request[$k] -or [string]$Request[$k] -eq '') { $reasons.Add("PARAMETER_MISSING:$k") } }
    $plan = $null
    foreach ($k in @($Request.Keys)) {
        if (-not ($allowed -contains $k)) { continue }
        $v = $Request[$k]
        if ($null -eq $v -or ($v -is [string] -and $v -eq '')) { continue }
        $ok = switch ($k) {
            'Action' { $true }
            'DryRun' { $v -is [bool] -or $v -is [switch] }
            'Vm' { [string]$v -cin (Get-P9SVmNames) }
            'Text' { $v -is [string] -and $v -cmatch '^[A-Za-z0-9 .~-]{1,120}$' }
            'Key' { $v -is [string] -and @((Get-P9SKeyMap).Keys) -ccontains $v }
            'Name' { $v -is [string] -and $v -cmatch '^[a-z0-9][a-z0-9-]{0,40}$' }
            'Seconds' { $n = 0; ([int]::TryParse([string]$v, [ref]$n)) -and $n -ge 1 -and $n -le $(if ($a -like 'emergency.*' -or $a -like 'acpi.*') { 600 } else { 3600 }) }
            'Authorization' { $v -is [string] -and (Test-Path -LiteralPath $v -PathType Leaf) }
            'Kind' { [string]$v -cin 'Test', 'Calibration' }
            'Scenario' { [string]$v -cin (Get-P9SScenarios) }
            'AcknowledgeAbort' { [string]$v -cmatch '^abort-[0-9a-f]{12}$' }
            'Plan' { $v -is [string] -and (Test-Path -LiteralPath $v -PathType Leaf) }
            'Reference' { $v -is [string] -and (Test-Path -LiteralPath $v -PathType Leaf) }
            'MonitorRun' { $v -is [string] -and (Test-Path -LiteralPath $v -PathType Container) }
            'Operation' { [string]$v -cmatch (Get-P9SVBoxIdPattern) }
            default { $false }
        }
        if ($ok) { $valid[$k] = $true }
        else {
            $code = switch ($k) { 'Vm' { 'VM_NOT_ALLOWED' } 'Text' { 'TEXT_NOT_ALLOWED' } 'Key' { 'KEY_NOT_ALLOWED' } 'Name' { 'NAME_NOT_ALLOWED' } 'Seconds' { 'SECONDS_NOT_ALLOWED' } 'Plan' { 'PATH_NOT_FOUND:Plan' } 'Reference' { 'PATH_NOT_FOUND:Reference' } 'MonitorRun' { 'PATH_NOT_FOUND:MonitorRun' } 'Authorization' { 'PATH_NOT_FOUND:Authorization' } default { "PARAMETER_INVALID:$k" } }
            $reasons.Add($code)
        }
    }
    if ($a -eq 'sequence.run' -and $valid.Plan) {
        $p = $null
        try { $p = Get-Content -LiteralPath $Request.Plan -Raw | ConvertFrom-Json -AsHashtable -ErrorAction Stop } catch { $reasons.Add('PLAN_INVALID_JSON') }
        if ($p) {
            $steps = @($p.steps)
            if ($p.schema -ne 'p9s-plan/1') { $reasons.Add('PLAN_SCHEMA_UNSUPPORTED') }
            elseif ($steps.Count -lt 1 -or $steps.Count -gt 100) { $reasons.Add('PLAN_STEP_COUNT') }
            else {
                for ($i = 0; $i -lt $steps.Count; $i++) {
                    $s = $steps[$i]
                    if ($s -isnot [System.Collections.IDictionary]) { $reasons.Add("PLAN_STEP_INVALID:$($i + 1)"); continue }
                    $h = @{}; foreach ($k in $s.Keys) { $h[$k] = $s[$k] }
                    if ($h.ContainsKey('DryRun')) { $reasons.Add("PLAN_STEP_INVALID:$($i + 1):DryRun"); continue }
                    $sv = Test-P9SRequest -Request $h -AsPlanStep
                    if (-not $sv.ok) { $reasons.Add("PLAN_STEP_INVALID:$($i + 1):$($sv.reasons[0])") }
                }
                $name = if ([string]$p.name -cmatch '^[a-z0-9][a-z0-9-]{0,40}$') { [string]$p.name } else { 'unnamed' }
                $plan = [pscustomobject]@{ name = $name; steps = $steps }
            }
        }
    }
    [pscustomobject]@{ ok = ($reasons.Count -eq 0); reasons = @($reasons); spec = $spec; valid = $valid; plan = $plan }
}

# ------------------------------------------------------- results and audit
function New-P9SResult([string]$Action, [string]$Vm) {
    [pscustomobject]@{ action = $Action; vm = $Vm; outcome = $null; reasons = [System.Collections.Generic.List[string]]::new(); lines = [System.Collections.Generic.List[string]]::new(); dispatched = 0; status = $null; run_id = $null; session = $null }
}
function Set-P9SRefused($Res, [string[]]$Reasons, $Gate, [string]$Outcome = 'REFUSED') {
    $Res.outcome = $Outcome
    foreach ($r in $Reasons) { if ($r -and -not $Res.reasons.Contains($r)) { $Res.reasons.Add($r) } }
    $label = if ($Outcome -eq 'REJECTED') { 'ACTION REJECTED' } else { 'HOST GATE REFUSED' }
    $Res.lines.Add(('{0} ({1}{2}): status {3}, run {4}; {5}' -f $label, $Res.action, $(if ($Res.vm) { " $($Res.vm)" } else { '' }),
            $(if ($Gate -and $Gate.status) { $Gate.status } else { '-' }), $(if ($Gate -and $Gate.run_id) { $Gate.run_id } else { '-' }), ($Res.reasons -join ', ')))
}
# Parameters as they may appear in the audit log: text only by its length,
# paths only by file name, anything that failed validation as <invalid>.
function ConvertTo-P9SAuditParams([hashtable]$Request, $V) {
    $p = [ordered]@{}
    if (-not $Request) { return $p }
    foreach ($k in @($Request.Keys)) {
        if ($k -in 'Action', 'Vm', 'DryRun') { continue }
        if ($k -eq 'Text') { $p.text_length = ([string]$Request.Text).Length; continue }
        if (-not ($V -and $V.valid.ContainsKey($k))) { $p[$k] = '<invalid>'; continue }
        switch ($k) {
            'Plan' { $p.plan = [IO.Path]::GetFileName([string]$Request.Plan) }
            'Reference' { $p.reference = [IO.Path]::GetFileName([string]$Request.Reference) }
            'MonitorRun' { $p.monitor_run = [IO.Path]::GetFileName([string]$Request.MonitorRun) }
            'Authorization' { $p.authorization = [IO.Path]::GetFileName([string]$Request.Authorization) }
            default { $p[$k] = [string]$Request[$k] }
        }
    }
    $p
}
function Add-P9SAudit {
    param($Ctx, $Res, [hashtable]$Request, $V, [string]$Outcome, [string[]]$Reasons, [string]$Note, [string]$Action, [string]$Vm)
    $e = [ordered]@{
        utc = [DateTime]::UtcNow.ToString('yyyy-MM-ddTHH:mm:ss.fffZ'); pid = $Ctx.Pid; session = $Res.session
        action = $(if ($Action) { $Action } else { $Res.action }); vm = $(if ($Vm) { $Vm } else { $Res.vm })
        outcome = $(if ($Outcome) { $Outcome } else { $Res.outcome }); reasons = @($(if ($PSBoundParameters.ContainsKey('Reasons')) { $Reasons } else { $Res.reasons }))
        monitor_status = $Res.status; monitor_run = $Res.run_id; dry_run = [bool]($Request -and $Request.DryRun); dispatched = $Res.dispatched
        params = (ConvertTo-P9SAuditParams $Request $V); note = $Note
    }
    $bytes = [Text.UTF8Encoding]::new($false).GetBytes(($e | ConvertTo-Json -Compress -Depth 5) + "`n")
    for ($i = 0; $i -lt 40; $i++) {
        try {
            $fs = [IO.FileStream]::new($Ctx.AuditPath, [IO.FileMode]::Append, [IO.FileAccess]::Write, [IO.FileShare]::ReadWrite)
            try { $fs.Write($bytes, 0, $bytes.Length) } finally { $fs.Dispose() }
            return
        } catch { Start-Sleep -Milliseconds 25 }
    }
    Write-Warning "P9-S coordinator: audit record not written to $($Ctx.AuditPath)"
}

# ------------------------------------------------------------ session file
function New-P9SSessionRecord([string]$Kind, [string]$RunId, [string]$Lifecycle) {
    [ordered]@{
        schema = 'p9s-coordinator-session/1'
        session_id = $(if ($Kind) { 'ses-' + [DateTime]::UtcNow.ToString('yyyyMMddTHHmmssfffZ') + '-' + $PID } else { $null })
        kind = $(if ($Kind) { $Kind } else { $null }); opened_utc = [DateTime]::UtcNow.ToString('o'); monitor_run_id = $(if ($RunId) { $RunId } else { $null })
        lifecycle = $Lifecycle; halt = $null; previous_halt = $null; expected_vm = @{}; inflight = $null; last_marker = $null
    }
}
function Read-P9SSession($Ctx) {
    $r = Read-StateFile -Path $Ctx.SessionPath
    if (-not $r.ok) { return [pscustomobject]@{ ok = ($r.error -eq 'missing'); session = $null } }
    try { $s = $r.text | ConvertFrom-Json -AsHashtable -ErrorAction Stop } catch { return [pscustomobject]@{ ok = $false; session = $null } }
    if ($s -isnot [System.Collections.IDictionary] -or $s.schema -ne 'p9s-coordinator-session/1') { return [pscustomobject]@{ ok = $false; session = $null } }
    if ($null -eq $s.expected_vm) { $s.expected_vm = @{} }
    [pscustomobject]@{ ok = $true; session = $s }
}
function Save-P9SSession($Ctx, $Session) { Write-StateAtomic -Path $Ctx.SessionPath -Text ($Session | ConvertTo-Json -Depth 6) }
# The stop request: written by an emergency action, honoured by every
# checkpoint, removed only by session.close. It also records the ACPI
# request sent to each VM (never repeated in the same emergency) and the VMs
# whose shutdown is unresolved. present and not ok: it exists but cannot be
# read (locked, not JSON, another shape), so the requests it records are
# unknown; it is then kept as it is, never replaced.
function Read-P9SStopRequestState($Ctx) {
    $r = Read-StateFile -Path $Ctx.StopRequestPath
    if (-not $r.ok) { return [pscustomobject]@{ present = ($r.error -ne 'missing'); ok = $false; doc = $null } }
    $x = $null; try { $x = $r.text | ConvertFrom-Json -AsHashtable -ErrorAction Stop } catch {}
    $bad = [pscustomobject]@{ present = $true; ok = $false; doc = $null }
    if ($x -isnot [System.Collections.IDictionary]) { return $bad }
    foreach ($k in 'acpi', 'unresolved') {
        if ($null -eq $x[$k]) { $x[$k] = [ordered]@{} } elseif ($x[$k] -isnot [System.Collections.IDictionary]) { return $bad }
    }
    [pscustomobject]@{ present = $true; ok = $true; doc = $x }
}
function Read-P9SStopRequest($Ctx) {
    $s = Read-P9SStopRequestState $Ctx
    if ($s.ok) { return $s.doc }
    if ($s.present) { return [ordered]@{ reason = 'UNREADABLE'; acpi = [ordered]@{}; unresolved = [ordered]@{} } }
    $null
}
function New-P9SStopRequestDoc($Ctx) { [ordered]@{ reason = 'EMERGENCY_ACPI_SHUTDOWN'; utc = [DateTime]::UtcNow.ToString('o'); pid = $Ctx.Pid; acpi = [ordered]@{}; unresolved = [ordered]@{} } }
function Save-P9SStopRequest($Ctx, $Req) { Write-StateAtomic -Path $Ctx.StopRequestPath -Text ($Req | ConvertTo-Json -Depth 5) }
# Raises the stop request when there is none, without the lock and without
# replacing one that exists (another emergency's record of its request, or
# an unreadable file). $true when this call created it.
function Add-P9SStopRequestIfAbsent($Ctx) {
    $full = [IO.Path]::GetFullPath($Ctx.StopRequestPath)
    if ([IO.File]::Exists($full)) { return $false }
    $tmp = [IO.Path]::Combine([IO.Path]::GetDirectoryName($full), '.' + [IO.Path]::GetFileName($full) + '.' + [guid]::NewGuid().ToString('N') + '.tmp')
    try {
        [IO.File]::WriteAllText($tmp, ((New-P9SStopRequestDoc $Ctx) | ConvertTo-Json -Depth 5), [Text.UTF8Encoding]::new($false))
        [IO.File]::Move($tmp, $full, $false)
        return $true
    } catch { return $false }
    finally { try { if ([IO.File]::Exists($tmp)) { [IO.File]::Delete($tmp) } } catch {} }
}
# Records a VM whose emergency shutdown is unresolved, merged into the stop
# request as it is now, under the lock (-Locked: the caller holds it);
# never over an unreadable one. $false when it was not recorded.
function Set-P9SStopRequestUnresolved($Ctx, [string]$Vm, [switch]$Locked) {
    $lock = if ($Locked) { $null } else { Enter-P9SLock $Ctx 2000 }
    if (-not $Locked -and -not $lock) { return $false }
    try {
        $sq = Read-P9SStopRequestState $Ctx
        if ($sq.present -and -not $sq.ok) { return $false }
        $req = if ($sq.ok) { $sq.doc } else { New-P9SStopRequestDoc $Ctx }
        $req.unresolved[$Vm] = [DateTime]::UtcNow.ToString('o')
        Save-P9SStopRequest $Ctx $req
        return $true
    } catch { return $false }
    finally { if ($lock) { $lock.Dispose() } }
}
# After an emergency wait (the lock was released while waiting): record the
# VM state the session should now expect, or the unresolved lifecycle.
function Set-P9SSessionAfterEmergency($Ctx, [string]$Vm, [string]$VmState, [string]$Lifecycle) {
    $lock = Enter-P9SLock $Ctx 2000
    if (-not $lock) { return }
    try {
        $sr = Read-P9SSession $Ctx
        if ($sr.ok -and $sr.session -and $sr.session.session_id) {
            if ($Vm) { $sr.session.expected_vm[$Vm] = $VmState }
            if ($Lifecycle) { $sr.session.lifecycle = $Lifecycle }
            Save-P9SSession $Ctx $sr.session
        }
    } finally { $lock.Dispose() }
}
function Enter-P9SLock($Ctx, [int]$WaitMs = 0) {
    $sw = [Diagnostics.Stopwatch]::StartNew()
    while ($true) {
        try { return [IO.FileStream]::new($Ctx.LockPath, [IO.FileMode]::OpenOrCreate, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None) }
        catch { if ($sw.ElapsedMilliseconds -ge $WaitMs) { return $null }; [Threading.Thread]::Sleep(50) }
    }
}

# ------------------------------------------------------------ monitor state
function Get-P9SGate($Ctx) {
    $r = & $Ctx.ReadState $Ctx.StatePath
    if (-not $r.ok) {
        return [pscustomobject]@{ ok = $false; reasons = @($(if ($r.error -eq 'missing') { 'STATE_MISSING' } else { 'STATE_UNREADABLE' })); status = $null; run_id = $null; halt_status = $null; halt_reasons = @(); mode = $null; text = $null; doc = $null }
    }
    $t = Test-SafetyState -Text $r.text -NowQpc (& $Ctx.NowQpc) -QpcFrequency $Ctx.QpcFrequency -PagingStop Required -IsMonitorAlive $Ctx.IsMonitorAlive
    $doc = $null; try { $doc = $r.text | ConvertFrom-Json -ErrorAction Stop } catch {}
    $hs = $null; $hr = @()
    if ($doc -is [pscustomobject] -and $doc.schema -eq 'p9s-hostmon-state/1') {
        if ($doc.latched -and $doc.latched.status) { $hs = [string]$doc.latched.status; $hr = @($doc.latched.reasons) }
        elseif ([string]$doc.status -in 'STOP_REQUIRED', 'ABORT_REQUIRED') { $hs = [string]$doc.status; $hr = @($doc.reasons) }
    }
    [pscustomobject]@{ ok = $t.ok; reasons = @($t.reasons); status = $t.status; run_id = $t.run_id; halt_status = $hs; halt_reasons = $hr
        mode = $(if ($doc -is [pscustomobject] -and $doc.monitor) { [string]$doc.monitor.mode } else { $null }); text = $r.text; doc = $doc }
}
# The VirtualBox management state of the context (p9s-vbox.ps1): a
# quarantine, or a lock this process could not release, refuses every test
# action, session and ACPI request. A context without a VirtualBox part (the
# tests' mock contexts) is not tracked.
function Get-P9SVBoxGate($Ctx) {
    $vb = Get-Prop $Ctx 'VBox'
    if (-not $vb) { return [pscustomobject]@{ ok = $true; reasons = @(); latch = $null; text = 'not tracked' } }
    if ($script:P9SVBoxRetained[$vb.MutexName]) { return [pscustomobject]@{ ok = $false; reasons = @('VBOX_API_QUARANTINED', 'VBOX_LOCK_RETAINED'); latch = $null; text = 'QUARANTINED (this process keeps the VirtualBox lock)' } }
    $q = Read-P9SVBoxLatch $vb
    if (-not $q.present) {
        # a record whose owner is gone: an operation ended unfinished;
        # the next VirtualBox operation quarantines it
        $rr = Read-P9SVBoxRecord $vb
        if ($rr.present -and (-not $rr.ok -or (Test-P9SChildAlive $rr.doc.owner_pid $rr.doc.owner_start_ticks) -ne 'alive')) {
            return [pscustomobject]@{ ok = $false; reasons = @('VBOX_OPERATION_UNFINISHED'); latch = $null; text = 'UNFINISHED OPERATION (its owner ended; the next VirtualBox operation quarantines)' }
        }
        return [pscustomobject]@{ ok = $true; reasons = @(); latch = $null; text = 'not quarantined' }
    }
    $why = if ($q.ok) { [string]$q.doc.reason } else { 'VBOX_LATCH_UNREADABLE' }
    [pscustomobject]@{ ok = $false; reasons = @('VBOX_API_QUARANTINED', $why); latch = $q.doc
        text = "QUARANTINED ($why$(if ($q.ok) { ", operation $($q.doc.operation_id)$(if ($q.doc.operation) { " $($q.doc.operation) $($q.doc.vm)" })" }))" }
}
# Closures built with GetNewClosure (the unlock dispatcher, the calibration
# host checks) find functions through the global scope. That holds when
# p9s-action.ps1 is started with pwsh -File (or dot-sourced at a prompt),
# not when it is started with & from a prompt or another script. The check
# asks a closure, the same way, for the functions those closures call.
function Test-P9SSupportedInvocation {
    $lib = [string]$script:P9SCoordinatorLibFile
    $need = [ordered]@{ 'Read-P9SSecret' = (Join-Path (Split-Path -Parent $lib) 'p9s-config.ps1'); 'Get-P9SScancodesForText' = $lib; 'Get-P9SCalibrationHostChecks' = $lib }
    $probe = { param($n) Get-Command -Name $n -CommandType Function -ErrorAction SilentlyContinue }.GetNewClosure()
    foreach ($n in $need.Keys) {
        $f = & $probe $n
        if (-not $f -or -not $f.ScriptBlock.File -or -not ([string]$f.ScriptBlock.File).Equals([string]$need[$n], [StringComparison]::OrdinalIgnoreCase)) { return [pscustomobject]@{ ok = $false; reason = 'UNSUPPORTED_INVOCATION'; missing = $n } }
    }
    [pscustomobject]@{ ok = $true; reason = $null; missing = $null }
}
# A STOP or ABORT seen in any parseable state (fresh or not) halts the
# session record; a stronger halt replaces a weaker one; the first reasons
# of each level are kept. Returns the session record (created if needed).
function Update-P9SHalt($Ctx, $Session, $Gate, $Res) {
    if (-not $Gate.halt_status) { return $Session }
    $rank = @{ STOP_REQUIRED = 1; ABORT_REQUIRED = 2 }
    $cur = if ($Session -and $Session.halt -and $Session.halt.status) { [int]$rank[[string]$Session.halt.status] } else { 0 }
    if ([int]$rank[$Gate.halt_status] -le $cur) { return $Session }
    if (-not $Session) { $Session = New-P9SSessionRecord $null $null 'CLOSED' }
    $utc = [DateTime]::UtcNow.ToString('o')
    $id = $null
    if ($Gate.halt_status -eq 'ABORT_REQUIRED') {
        $h = [Security.Cryptography.SHA256]::HashData([Text.Encoding]::UTF8.GetBytes("$($Gate.run_id)|$utc|$($Ctx.Pid)"))
        $id = 'abort-' + ([BitConverter]::ToString($h) -replace '-', '').Substring(0, 12).ToLowerInvariant()
    }
    $Session.halt = [ordered]@{ status = $Gate.halt_status; reasons = @($Gate.halt_reasons); utc = $utc; monitor_run_id = $Gate.run_id; abort_id = $id; acknowledged = $false }
    if ($Session.lifecycle -ne 'CLOSED') { $Session.lifecycle = $(if ($Gate.halt_status -eq 'ABORT_REQUIRED') { 'HALTED_ABORT' } else { 'HALTED_STOP' }) }
    Save-P9SSession $Ctx $Session
    $Res.session = $Session.session_id
    Add-P9SAudit -Ctx $Ctx -Res $Res -Outcome 'HALTED' -Reasons @($Gate.halt_reasons) -Note "session halted: monitor $($Gate.halt_status) on run $($Gate.run_id)$(if ($id) { "; abort id $id" })"
    $Res.lines.Add("*** $($Gate.halt_status): the coordinator halted the session ($(@($Gate.halt_reasons) -join ', ')). No further test action; orderly shutdown through emergency.acpi-shutdown. ***")
    $Session
}
function Test-P9SCheckpoint($Ctx, $Session, $Res) {
    $g = Get-P9SGate $Ctx
    $Res.status = $g.status; $Res.run_id = $g.run_id
    $null = Update-P9SHalt $Ctx $Session $g $Res
    $reasons = [System.Collections.Generic.List[string]]::new()
    if (-not $g.ok) { foreach ($r in $g.reasons) { $reasons.Add($r) } }
    if ($Session.lifecycle -ne 'ACTIVE') { $reasons.Add("SESSION_$($Session.lifecycle)") }
    if ($g.run_id -and $g.run_id -ne $Session.monitor_run_id) { $reasons.Add('MONITOR_RUN_CHANGED') }
    if (Test-Path -LiteralPath $Ctx.StopRequestPath) { $reasons.Add('EMERGENCY_STOP_REQUESTED') }
    foreach ($r in (Get-P9SVBoxGate $Ctx).reasons) { $reasons.Add($r) }
    [pscustomobject]@{ ok = ($reasons.Count -eq 0); reasons = @($reasons); gate = $g }
}

# ----------------------------------------------------------------- entry
function Invoke-P9SAction {
    param([Parameter(Mandatory)]$Ctx, [Parameter(Mandatory)][hashtable]$Request)
    $v = Test-P9SRequest -Request $Request
    $res = New-P9SResult $(if ($v.spec) { [string]$Request.Action } else { '<unknown>' }) $(if ($v.valid.Vm) { [string]$Request.Vm } elseif ($Request.ContainsKey('Vm')) { '<invalid>' } else { $null })
    if (-not $v.ok) {
        Set-P9SRefused $res $v.reasons $null 'REJECTED'
        Add-P9SAudit -Ctx $Ctx -Res $res -Request $Request -V $v
        return $res
    }
    $class = $v.spec.class
    $cfgErr = Get-Prop $Ctx 'ConfigError'
    if ($cfgErr -and ($class -in 'normal', 'session', 'calibration' -or ($class -eq 'validation' -and [string]$Request.Action -cne 'acpi.confirm-off'))) {
        Set-P9SRefused $res @([string]$cfgErr) $null 'REJECTED'
        Add-P9SAudit -Ctx $Ctx -Res $res -Request $Request -V $v
        return $res
    }
    if ($class -eq 'emergency') { return (Invoke-P9SEmergency $Ctx $Request $v $res) }
    if ($class -eq 'observe') { return (Invoke-P9SObserve $Ctx $Request $v $res) }
    if ($class -eq 'calibration') { return (Invoke-P9SCalibration $Ctx $Request $v $res) }
    if ($class -eq 'validation') { return (Invoke-P9SValidation $Ctx $Request $v $res) }
    $lock = Enter-P9SLock $Ctx 0
    if (-not $lock) {
        Set-P9SRefused $res @('COORDINATOR_BUSY') $null
        Add-P9SAudit -Ctx $Ctx -Res $res -Request $Request -V $v
        return $res
    }
    try {
        if ($class -eq 'session') { return (Invoke-P9SSessionAction $Ctx $Request $v $res) }
        return (Invoke-P9SNormal -Ctx $Ctx -Request $Request -V $v -Res $res)
    } finally { $lock.Dispose() }
}
function Get-P9SDescription([hashtable]$R) {
    switch ([string]$R.Action) {
        'vm.start' { "start $($R.Vm): VBoxManage $((Get-VBoxStartArgs $R.Vm) -join ' ')" }
        'guest.type' { "type $(([string]$R.Text).Length) characters into $($R.Vm), 8 at a time with a safety check between chunks, no Enter" }
        'guest.key' { "press $($R.Key) in $($R.Vm)" }
        'guest.unlock' { "type the profile passphrase of $($R.Vm) from the local secret store (never logged)" }
        'scenario.begin' { "begin scenario $($R.Name)" }
        'phase.advance' { "advance to phase $($R.Name)" }
        'wait' { "wait $($R.Seconds) s with a safety check every 5 s" }
        default { [string]$R.Action }
    }
}

# ------------------------------------------------------------ normal actions
function Invoke-P9SNormal {
    param($Ctx, [hashtable]$Request, $V, $Res, $Session, [switch]$InSequence)
    $a = [string]$Request.Action; $vm = [string]$Request.Vm; $dry = [bool]$Request.DryRun
    $reasons = [System.Collections.Generic.List[string]]::new()
    $sessOk = $true
    if (-not $InSequence) { $sr = Read-P9SSession $Ctx; $sessOk = $sr.ok; $Session = $sr.session; if (-not $sessOk) { $reasons.Add('SESSION_UNREADABLE') } }
    $g = Get-P9SGate $Ctx
    $Res.status = $g.status; $Res.run_id = $g.run_id
    if ($sessOk) { $Session = Update-P9SHalt $Ctx $Session $g $Res }
    if ($Session) { $Res.session = $Session.session_id }
    if (-not $g.ok) { foreach ($r in $g.reasons) { $reasons.Add($r) } }
    $vg = Get-P9SVBoxGate $Ctx
    foreach ($r in $vg.reasons) { $reasons.Add($r) }
    $active = $Session -and $Session.session_id
    if ($sessOk -and -not $active) { $reasons.Add('SESSION_MISSING') }
    if ($active) {
        if ($Session.lifecycle -ne 'ACTIVE') { $reasons.Add("SESSION_$($Session.lifecycle)") }
        if ($Session.kind -ne 'Test') { $reasons.Add('SESSION_KIND_MISMATCH') }
        if ($g.run_id -and $g.run_id -ne $Session.monitor_run_id) { $reasons.Add('MONITOR_RUN_CHANGED') }
        if ($Session.inflight) { $reasons.Add('INFLIGHT_UNRESOLVED') }
    }
    if (Test-Path -LiteralPath $Ctx.StopRequestPath) { $reasons.Add('EMERGENCY_STOP_REQUESTED') }
    if ($active -and $vg.ok) {
        $actual = @{}
        foreach ($n in Get-P9SVmNames) {
            $actual[$n] = (Get-P9SVmObservation $Ctx $n 'action').state
            if ($actual[$n] -ne [string]$Session.expected_vm[$n]) { $reasons.Add("OUT_OF_BAND_VM_STATE:$n") }
        }
        if ($a -eq 'vm.start' -and $actual[$vm] -ne 'off') { $reasons.Add('VM_NOT_OFF') }
        if ($a -like 'guest.*' -and $actual[$vm] -ne 'running') { $reasons.Add('VM_NOT_RUNNING') }
    }
    if ($reasons.Count) {
        Set-P9SRefused $Res @($reasons) $g
        if (-not $InSequence) { Add-P9SAudit -Ctx $Ctx -Res $Res -Request $Request -V $V }
        return $Res
    }
    if ($a -eq 'sequence.run') { return (Invoke-P9SSequence $Ctx $Request $V $Res $Session) }
    # last check, immediately before dispatch
    $cp = Test-P9SCheckpoint $Ctx $Session $Res
    if (-not $cp.ok) {
        Set-P9SRefused $Res (@('STATE_CHANGED_BEFORE_DISPATCH') + @($cp.reasons)) $cp.gate
        if (-not $InSequence) { Add-P9SAudit -Ctx $Ctx -Res $Res -Request $Request -V $V }
        return $Res
    }
    if ($dry) {
        $Res.outcome = 'DRYRUN'; $Res.lines.Add("dry run: would $(Get-P9SDescription $Request)")
        Add-P9SAudit -Ctx $Ctx -Res $Res -Request $Request -V $V
        return $Res
    }
    $Session.inflight = [ordered]@{ action = $a; vm = $(if ($vm) { $vm } else { $null }); pid = $Ctx.Pid; start_ticks = $Ctx.ProcessStartTicks; utc = [DateTime]::UtcNow.ToString('o') }
    Save-P9SSession $Ctx $Session
    try {
        switch ($a) {
            'vm.start' { & $Ctx.Dispatch['vm.start'] $vm; $Res.dispatched = 1; $Session.expected_vm[$vm] = 'running' }
            'guest.key' { & $Ctx.Dispatch['guest.key'] $vm ([int[]]((Get-P9SKeyMap)[[string]$Request.Key])); $Res.dispatched = 1 }
            'guest.unlock' { & $Ctx.Dispatch['guest.unlock'] $vm; $Res.dispatched = 1 }
            'guest.type' {
                $text = [string]$Request.Text
                $chunks = @(for ($i = 0; $i -lt $text.Length; $i += 8) { $text.Substring($i, [math]::Min(8, $text.Length - $i)) })
                $typed = 0
                for ($i = 0; $i -lt $chunks.Count; $i++) {
                    if ($i -gt 0) {
                        $c = Test-P9SCheckpoint $Ctx $Session $Res
                        if (-not $c.ok) {
                            $Res.outcome = 'STOPPED_AT_CHECKPOINT'; foreach ($r in $c.reasons) { $Res.reasons.Add($r) }
                            $Res.lines.Add("stopped after $typed of $($text.Length) characters ($($c.reasons -join ', ')); nothing more typed, no Enter")
                            break
                        }
                    }
                    & $Ctx.Dispatch['guest.text'] $vm $chunks[$i]
                    $typed += $chunks[$i].Length; $Res.dispatched++
                }
            }
            'wait' {
                $left = [int]$Request.Seconds
                while ($left -gt 0) {
                    $step = [math]::Min(5, $left)
                    & $Ctx.Sleep ($step * 1000)
                    $left -= $step
                    $c = Test-P9SCheckpoint $Ctx $Session $Res
                    if (-not $c.ok) {
                        $Res.outcome = 'STOPPED_AT_CHECKPOINT'; foreach ($r in $c.reasons) { $Res.reasons.Add($r) }
                        $Res.lines.Add("wait stopped with $left s left ($($c.reasons -join ', '))")
                        break
                    }
                }
            }
            default { $Session.last_marker = "$a $($Request.Name)" }
        }
        if (-not $Res.outcome) { $Res.outcome = $(if ($Res.dispatched) { 'DISPATCHED' } else { 'DONE' }); $Res.lines.Add("done: $(Get-P9SDescription $Request)") }
    } catch {
        $Res.outcome = 'FAILED'; $Res.reasons.Add('DISPATCH_FAILED'); $Res.lines.Add("dispatch failed: $($_.Exception.Message)")
    } finally {
        $Session.inflight = $null
        Save-P9SSession $Ctx $Session
    }
    Add-P9SAudit -Ctx $Ctx -Res $Res -Request $Request -V $V
    $Res
}
function Invoke-P9SSequence($Ctx, [hashtable]$Request, $V, $Res, $Session) {
    $steps = @($V.plan.steps); $n = $steps.Count; $done = 0
    $Res.lines.Add("sequence $($V.plan.name): $n steps")
    for ($i = 0; $i -lt $n; $i++) {
        $h = @{}; foreach ($k in $steps[$i].Keys) { $h[$k] = $steps[$i][$k] }
        if ($Request.DryRun) { $h.DryRun = $true }
        $sv = Test-P9SRequest -Request $h -AsPlanStep
        $sres = New-P9SResult ([string]$h.Action) $(if ($h.Vm) { [string]$h.Vm } else { $null })
        $r = Invoke-P9SNormal -Ctx $Ctx -Request $h -V $sv -Res $sres -Session $Session -InSequence
        $Res.dispatched += $r.dispatched
        if ($r.outcome -in 'DISPATCHED', 'DONE', 'DRYRUN') { $done++; $Res.lines.Add("step $($i + 1)/$n $($h.Action): $($r.outcome)"); continue }
        foreach ($x in $r.reasons) { if (-not $Res.reasons.Contains($x)) { $Res.reasons.Add($x) } }
        $Res.status = $r.status; $Res.run_id = $r.run_id
        if ($r.outcome -eq 'REFUSED') {
            Add-P9SAudit -Ctx $Ctx -Res $sres -Request $h -V $sv -Outcome 'CANCELLED' -Note "sequence $($V.plan.name) step $($i + 1) of $n not started"
            $Res.lines.Add("step $($i + 1)/$n $($h.Action): CANCELLED ($($r.reasons -join ', '))")
        } else { $Res.lines.Add("step $($i + 1)/$n $($h.Action): $($r.outcome)") }
        for ($j = $i + 1; $j -lt $n; $j++) {
            $hj = @{}; foreach ($k in $steps[$j].Keys) { $hj[$k] = $steps[$j][$k] }
            $rj = New-P9SResult ([string]$hj.Action) $(if ($hj.Vm) { [string]$hj.Vm } else { $null }); $rj.session = $Session.session_id
            Add-P9SAudit -Ctx $Ctx -Res $rj -Request $hj -V (Test-P9SRequest -Request $hj -AsPlanStep) -Outcome 'CANCELLED' -Reasons @('SEQUENCE_STOPPED') -Note "sequence $($V.plan.name) step $($j + 1) of $n not started"
            $Res.lines.Add("step $($j + 1)/$n $($hj.Action): CANCELLED")
        }
        $Res.outcome = 'STOPPED_AT_CHECKPOINT'
        break
    }
    if (-not $Res.outcome) { $Res.outcome = $(if ($Request.DryRun) { 'DRYRUN' } else { 'DONE' }) }
    $Res.lines.Add("sequence $($V.plan.name): $done of $n steps done")
    Add-P9SAudit -Ctx $Ctx -Res $Res -Request $Request -V $V
    $Res
}

# ------------------------------------------------------------ session actions
function Invoke-P9SSessionAction($Ctx, [hashtable]$Request, $V, $Res) {
    $a = [string]$Request.Action; $dry = [bool]$Request.DryRun
    $sr = Read-P9SSession $Ctx
    if (-not $sr.ok) { Set-P9SRefused $Res @('SESSION_UNREADABLE') $null; Add-P9SAudit -Ctx $Ctx -Res $Res -Request $Request -V $V; return $Res }
    $sess = $sr.session
    if ($sess) { $Res.session = $sess.session_id }
    if ($a -eq 'vbox.recover') {
        # the operator's recovery of a VirtualBox quarantine (p9s-vbox.ps1)
        $vb = Get-Prop $Ctx 'VBox'
        if (-not $vb) { Set-P9SRefused $Res @('VBOX_NOT_TRACKED') $null; Add-P9SAudit -Ctx $Ctx -Res $Res -Request $Request -V $V; return $Res }
        $rc = Invoke-P9SVBoxRecovery -VBox $vb -OperationId ([string]$Request.Operation) -DryRun:$dry
        foreach ($l in $rc.lines) { $Res.lines.Add($l) }
        if ($rc.outcome -eq 'CLEARED') {
            $Res.outcome = 'DONE'
            if ($sess -and $sess.session_id -and $sess.lifecycle -eq 'ACTIVE') {
                $sess.lifecycle = 'VBOX_RECOVERED'; Save-P9SSession $Ctx $sess
                $Res.lines.Add("session $($sess.session_id) must be closed and a new one opened: the monitor, the VM states and the session are qualified again before any test action")
            }
        } elseif ($rc.outcome -eq 'DRYRUN') { $Res.outcome = 'DRYRUN' }
        else { Set-P9SRefused $Res @($rc.reasons) $null }
        Add-P9SAudit -Ctx $Ctx -Res $Res -Request $Request -V $V
        return $Res
    }
    if ($a -eq 'session.close') {
        # a restricted ACPI validation closes through acpi.finalize, which
        # records its outcome in the ledger
        if ($sess -and [string]$sess.kind -ceq 'AcpiValidation' -and [string]$sess.lifecycle -cne 'CLOSED') {
            Set-P9SRefused $Res @('ACPI_VALIDATION_USE_FINALIZE') $null
            $Res.lines.Add("session $($sess.session_id) is an ACPI validation: close it with acpi.finalize")
            Add-P9SAudit -Ctx $Ctx -Res $Res -Request $Request -V $V
            return $Res
        }
        # an emergency whose VM did not power off cannot be closed away,
        # nor one whose record cannot be read (it is kept for the operator)
        $sq = Read-P9SStopRequestState $Ctx
        if ($sq.present -and -not $sq.ok) {
            Set-P9SRefused $Res @('STOP_REQUEST_UNREADABLE', 'SHUTDOWN_HISTORY_UNCERTAIN') $null
            $Res.lines.Add("not closed: the stop request cannot be read, so the ACPI requests it records and the VMs whose shutdown is unresolved are unknown. It is kept as it is; the operator inspects it and decides outside the coordinator.")
            Add-P9SAudit -Ctx $Ctx -Res $Res -Request $Request -V $V
            return $Res
        }
        # A VM that got a request whose end was never recorded (its
        # coordinator ended, or still watches it) counts as unresolved too,
        # until it is seen off or aborted.
        $stopReq = $sq.doc
        if ($stopReq) {
            $still = @(foreach ($k in @($stopReq.unresolved.Keys)) { if ((Get-P9SVmObservation $Ctx $k 'session').state -ne 'off') { $k } })
            $still += @(foreach ($k in @($stopReq.acpi.Keys)) { if (-not $stopReq.unresolved.Contains($k) -and (Get-P9SVmObservation $Ctx $k 'session').state -notin 'off', 'aborted') { $k } })
            if ($still.Count) {
                Set-P9SRefused $Res @('EMERGENCY_UNRESOLVED') $null
                $Res.lines.Add("not closed: $($still -join ', ') did not power off after the ACPI request; the operator decides outside the coordinator, then closes the session")
                Add-P9SAudit -Ctx $Ctx -Res $Res -Request $Request -V $V
                return $Res
            }
        }
        $note = $null
        if ($sess -and $sess.inflight) { $note = "in-flight record of $($sess.inflight.action) discarded by the operator (its effect is unknown)" }
        if (-not $dry -and $sess) {
            $sess.lifecycle = 'CLOSED'; $sess.inflight = $null
            Save-P9SSession $Ctx $sess
            if (Test-Path -LiteralPath $Ctx.StopRequestPath) { Remove-Item -LiteralPath $Ctx.StopRequestPath }
        }
        $Res.outcome = $(if ($dry) { 'DRYRUN' } else { 'DONE' })
        $Res.lines.Add($(if ($sess) { "session $($sess.session_id) closed$(if ($note) { "; $note" })" } else { 'no session to close' }))
        Add-P9SAudit -Ctx $Ctx -Res $Res -Request $Request -V $V -Note $note
        return $Res
    }
    $g = Get-P9SGate $Ctx
    $Res.status = $g.status; $Res.run_id = $g.run_id
    $sess = Update-P9SHalt $Ctx $sess $g $Res
    if ([string]$Request.Kind -eq 'Calibration') {
        if (-not $dry) { Set-P9SRefused $Res @('CALIBRATION_LIVE_DISABLED') $g; Add-P9SAudit -Ctx $Ctx -Res $Res -Request $Request -V $V; return $Res }
        $c = Test-P9SCalibrationState -Text $g.text -NowQpc (& $Ctx.NowQpc) -QpcFrequency $Ctx.QpcFrequency -IsMonitorAlive $Ctx.IsMonitorAlive
        $Res.outcome = 'DRYRUN'
        $Res.lines.Add("dry run: a calibration session would $(if ($c.ok) { 'meet' } else { 'not meet' }) the monitor conditions$(if (-not $c.ok) { ' (' + ($c.reasons -join ', ') + ')' }); live calibration is not available in this kit version")
        Add-P9SAudit -Ctx $Ctx -Res $Res -Request $Request -V $V
        return $Res
    }
    $reasons = [System.Collections.Generic.List[string]]::new()
    if (-not $g.ok) { foreach ($r in $g.reasons) { $reasons.Add($r) } }
    $vg = Get-P9SVBoxGate $Ctx
    foreach ($r in $vg.reasons) { $reasons.Add($r) }
    if ($sess -and $sess.session_id -and $sess.lifecycle -eq 'ACTIVE') { $reasons.Add('SESSION_ALREADY_OPEN') }
    if ($sess -and $sess.inflight) { $reasons.Add('INFLIGHT_UNRESOLVED') }
    if (Test-Path -LiteralPath $Ctx.StopRequestPath) { $reasons.Add('STOP_REQUEST_PENDING') }
    # a restricted ACPI validation must be finalized, and resolved, first
    if ($sess -and [string]$sess.kind -ceq 'AcpiValidation' -and [string]$sess.lifecycle -cne 'CLOSED') { $reasons.Add('ACPI_VALIDATION_NOT_FINALIZED') }
    if (Get-Prop $Ctx 'LedgerPath') {
        $lg = Read-P9SAcpiLedger $Ctx
        if (-not $lg.ok) { $reasons.Add($lg.reason) } elseif (@(Get-P9SAcpiUnresolved $lg.doc '').Count) { $reasons.Add('ACPI_VALIDATION_UNRESOLVED') }
    }
    $ackOk = $false
    if ($sess -and $sess.halt) {
        if ($g.run_id -and $g.run_id -eq [string]$sess.halt.monitor_run_id) { $reasons.Add('STOP_REQUIRES_NEW_MONITOR_RUN') }
        if ([string]$sess.halt.status -eq 'ABORT_REQUIRED' -and -not $sess.halt.acknowledged) {
            if ([string]$Request.AcknowledgeAbort -cne [string]$sess.halt.abort_id) { $reasons.Add('ABORT_NOT_ACKNOWLEDGED') } else { $ackOk = $true }
        }
    }
    $actual = @{}
    if ($vg.ok) {
        foreach ($n in Get-P9SVmNames) {
            $actual[$n] = (Get-P9SVmObservation $Ctx $n 'session').state
            if ($actual[$n] -notin 'off', 'running') { $reasons.Add("VM_STATE_UNEXPECTED:$n") }
        }
    }
    if ($ackOk -and @($actual.Values | Where-Object { $_ -ne 'off' }).Count) { $reasons.Add('VMS_NOT_OFF') }
    if ($reasons.Count) { Set-P9SRefused $Res @($reasons) $g; Add-P9SAudit -Ctx $Ctx -Res $Res -Request $Request -V $V; return $Res }
    $g2 = Get-P9SGate $Ctx
    if (-not $g2.ok -or $g2.run_id -ne $g.run_id) {
        $null = Update-P9SHalt $Ctx $sess $g2 $Res
        Set-P9SRefused $Res (@('STATE_CHANGED_BEFORE_DISPATCH') + @($g2.reasons)) $g2; Add-P9SAudit -Ctx $Ctx -Res $Res -Request $Request -V $V; return $Res
    }
    if ($dry) { $Res.outcome = 'DRYRUN'; $Res.lines.Add("dry run: would open a test session on monitor run $($g.run_id)"); Add-P9SAudit -Ctx $Ctx -Res $Res -Request $Request -V $V; return $Res }
    $prev = if ($sess) { $sess.halt } else { $null }
    if ($ackOk) { $prev.acknowledged = $true; $prev.acknowledged_utc = [DateTime]::UtcNow.ToString('o') }
    $new = New-P9SSessionRecord 'Test' $g.run_id 'ACTIVE'
    $new.expected_vm = $actual; $new.previous_halt = $prev
    Save-P9SSession $Ctx $new
    $Res.session = $new.session_id; $Res.outcome = 'DONE'
    $Res.lines.Add("session $($new.session_id) opened on monitor run $($g.run_id); VMs: $(($actual.GetEnumerator() | Sort-Object Key | ForEach-Object { "$($_.Key) $($_.Value)" }) -join ', ')$(if ($ackOk) { "; abort $($prev.abort_id) acknowledged" })")
    Add-P9SAudit -Ctx $Ctx -Res $Res -Request $Request -V $V
    $Res
}

# ------------------------------------------------------------ emergency path
# An emergency whose request history cannot be established (the ledger
# unreadable, missing, stale or inconsistent, or the stop request
# unreadable): no request is sent, under the lock. The VM state is read
# once when VirtualBox allows it (read only); a VM not seen off is recorded
# as unresolved in the stop request (when it can be read) and in the
# session; the files are kept as they are. Returns the audit note.
function Invoke-P9SEmergencyUncertain($Ctx, $Res, [string]$Vm, $H, $Vg, $S, $G) {
    $obs = if ($Vg.ok) { Get-P9SVmObservation $Ctx $Vm 'shutdown' } else { $null }
    $off = [bool]($obs -and $obs.observed -and [string]$obs.state -ceq 'off')
    Set-P9SRefused $Res (@($H.reasons) + @($(if (-not $Vg.ok) { $Vg.reasons })) + @($(if (-not $off) { 'EMERGENCY_UNRESOLVED' }))) $G
    $what = @($H.reasons | Where-Object { $_ -cne 'SHUTDOWN_HISTORY_UNCERTAIN' -and $_ -cne 'ACPI_ALREADY_REQUESTED' }) -join ' / '
    $Res.lines.Add("*** $what / SHUTDOWN_HISTORY_UNCERTAIN: the toolkit cannot establish whether an ACPI request was already sent to $Vm. NO REQUEST IS SENT: an unknown history is not an unused budget. No keyboard input, no forced power-off, no restart of VirtualBox. ***")
    $Res.lines.Add("${Vm}: $(if ($obs) { Format-P9SVmObservation $obs } else { "not read (VirtualBox $($Vg.text))" })$(if ($off) { '; no request is needed, but the history still has to be reconciled' })")
    $diag = [System.Collections.Generic.List[string]]::new()
    if ($H.diagnosis) { $diag.Add("ledger: $($H.diagnosis)") }
    if ($H.reasons -ccontains 'STOP_REQUEST_UNREADABLE') {
        $sp = [string]$Ctx.StopRequestPath
        $diag.Add("stop request: $(try { $i = [IO.FileInfo]::new($sp); "$($i.Length) bytes, written $($i.LastWriteTimeUtc.ToString('yyyy-MM-ddTHH:mm:ssZ', [Globalization.CultureInfo]::InvariantCulture))" } catch { 'not inspected' })")
    }
    foreach ($d in $diag) { $Res.lines.Add($d) }
    $Res.lines.Add("monitor: status $(if ($G.status) { $G.status } else { '-' }), run $(if ($G.run_id) { $G.run_id } else { '-' }) (it keeps measuring on its own; nothing of it is changed)")
    if (-not $off) {
        if (-not (Set-P9SStopRequestUnresolved $Ctx $Vm -Locked)) { $Res.lines.Add('the stop request is not updated (it cannot be read or written); it is kept as it is') }
        if ($S -and $S.session_id) { try { $S.lifecycle = 'EMERGENCY_UNRESOLVED'; Save-P9SSession $Ctx $S } catch { $Res.reasons.Add('EVIDENCE_WRITE_FAILED') } }
    }
    $Res.lines.Add('The ledger and the stop request are kept as they are for inspection: never repaired, reset or recreated. emergency.status and emergency.wait-off still read the VM state (read only); no request is sent through the toolkit until a separately authorized recovery restores a readable history. The operator decides outside the coordinator.')
    "request history uncertain ($(@($H.reasons) -join ', ')); $($diag -join '; ')"
}
function Invoke-P9SEmergency($Ctx, [hashtable]$Request, $V, $Res) {
    $a = [string]$Request.Action; $vm = [string]$Request.Vm; $dry = [bool]$Request.DryRun
    if ($a -eq 'emergency.status') {
        $g = Get-P9SGate $Ctx; $Res.status = $g.status; $Res.run_id = $g.run_id
        $sr = Read-P9SSession $Ctx
        $Res.lines.Add("monitor: status $(if ($g.status) { $g.status } else { '-' }), run $(if ($g.run_id) { $g.run_id } else { '-' }); $(if ($g.ok) { 'READY for test actions' } else { $g.reasons -join ', ' })")
        $Res.lines.Add("session: $(if (-not $sr.ok) { 'UNREADABLE' } elseif ($sr.session) { "$($sr.session.session_id) $($sr.session.lifecycle)$(if ($sr.session.halt) { " halt $($sr.session.halt.status) $($sr.session.halt.abort_id)" })$(if ($sr.session.inflight) { ' IN-FLIGHT ' + $sr.session.inflight.action })" } else { 'none' })")
        $vg = Get-P9SVBoxGate $Ctx
        $Res.lines.Add("VirtualBox API: $($vg.text)")
        if ($vg.ok) { foreach ($n in Get-P9SVmNames) { $Res.lines.Add((Format-P9SVmObservation (Get-P9SVmObservation $Ctx $n 'status'))) } }
        else { foreach ($n in Get-P9SVmNames) { $Res.lines.Add("$n unknown (not read while VirtualBox is quarantined; see vbox.status)") } }
        $lg = Read-P9SAcpiLedger $Ctx
        if ($lg.ok -and @(Get-P9SAcpiUnresolved $lg.doc '').Count) { foreach ($u in @(Get-P9SAcpiUnresolved $lg.doc '')) { $Res.lines.Add("ACPI validation $($u.authorization_id) of $($u.target_vm): $($u.phase) (unresolved; acpi.confirm-off)") } }
        elseif (-not $lg.ok -and (Get-Prop $Ctx 'LedgerPath')) {
            $Res.lines.Add("ACPI validation ledger: $($lg.reason) / SHUTDOWN_HISTORY_UNCERTAIN: $(Get-P9SAcpiLedgerDiagnosis $Ctx $lg). No ACPI request is sent through the toolkit while the history is unknown; the files are kept as they are for the operator.")
            foreach ($r in $lg.reason, 'SHUTDOWN_HISTORY_UNCERTAIN') { if (-not $Res.reasons.Contains($r)) { $Res.reasons.Add($r) } }
        }
        $sq = Read-P9SStopRequestState $Ctx
        if ($sq.present -and -not $sq.ok) {
            $Res.lines.Add('stop request: UNREADABLE / SHUTDOWN_HISTORY_UNCERTAIN: the ACPI requests it records are unknown; it is kept as it is and no ACPI request is sent through the toolkit.')
            foreach ($r in 'STOP_REQUEST_UNREADABLE', 'SHUTDOWN_HISTORY_UNCERTAIN') { if (-not $Res.reasons.Contains($r)) { $Res.reasons.Add($r) } }
        } elseif ($sq.ok) { $Res.lines.Add("stop request: ACPI requests recorded for $(if ($sq.doc.acpi.Count) { @($sq.doc.acpi.Keys) -join ', ' } else { 'none' }); unresolved $(if ($sq.doc.unresolved.Count) { @($sq.doc.unresolved.Keys) -join ', ' } else { 'none' })") }
        $Res.outcome = 'DONE'
        Add-P9SAudit -Ctx $Ctx -Res $Res -Request $Request -V $V
        return $Res
    }
    if ($a -eq 'emergency.acpi-shutdown') {
        $limit = if ($Request.ContainsKey('Seconds') -and $Request.Seconds) { [int]$Request.Seconds } else { (Get-P9SCalibrationLimits).shutdown_wait_s }
        if ($dry) {
            $Res.outcome = 'DRYRUN'
            $Res.lines.Add("dry run: would send one ACPI power-button request to $vm (VBoxManage $((Get-VBoxAcpiArgs $vm) -join ' ')) and watch it for up to $limit s; no keyboard input, no forced power-off")
            $vg = Get-P9SVBoxGate $Ctx
            if (-not $vg.ok) { $Res.lines.Add("dry run: VirtualBox is $($vg.text): no request could be sent now") }
            $dh = Get-P9SAcpiRequestHistory $Ctx $vm
            if (-not $dh.ok) { $Res.lines.Add("dry run: no request could be sent now ($(@($dh.reasons) -join ', '))") }
            Add-P9SAudit -Ctx $Ctx -Res $Res -Request $Request -V $V
            return $Res
        }
        # a request already sent to this VM, by this emergency or by an
        # unresolved validation, is not repeated: refused at once, nothing
        # written. Decided again under the lock below (a validation releases
        # the lock while it watches its request, another emergency may send
        # one meanwhile); an unknown history refuses there too.
        $h = Get-P9SAcpiRequestHistory $Ctx $vm
        if ($h.spent) {
            Set-P9SRefused $Res @($h.reasons) $null
            $Res.lines.Add("$($h.spent); it is not repeated. Watch it with emergency.wait-off$(if ($h.entry) { ' or acpi.confirm-off' }); if it stays on, the operator decides outside the coordinator.")
            Add-P9SAudit -Ctx $Ctx -Res $Res -Request $Request -V $V
            return $Res
        }
        # the stop request halts every other action at its next checkpoint;
        # one that exists is left as it is
        $null = Add-P9SStopRequestIfAbsent $Ctx
        $lock = Enter-P9SLock $Ctx $Ctx.EmergencyLockWaitMs
        if (-not $lock) {
            Set-P9SRefused $Res (@('EMERGENCY_LOCK_TIMEOUT') + @($h.reasons)) $null
            $Res.lines.Add('another coordinator action did not reach a checkpoint in time; no request sent. The stop request stays in place.')
            Add-P9SAudit -Ctx $Ctx -Res $Res -Request $Request -V $V
            return $Res
        }
        $sent = $false; $note = $null
        try {
            $g = Get-P9SGate $Ctx; $Res.status = $g.status; $Res.run_id = $g.run_id
            $s = $null; $sr = Read-P9SSession $Ctx
            if ($sr.ok) {
                $s = Update-P9SHalt $Ctx $sr.session $g $Res
                if ($s -and $s.session_id) { $Res.session = $s.session_id; if ($s.lifecycle -eq 'ACTIVE') { $s.lifecycle = 'SHUTDOWN'; Save-P9SSession $Ctx $s } }
            }
            # the decision, from the records as they are now
            $h = Get-P9SAcpiRequestHistory $Ctx $vm
            $vg = Get-P9SVBoxGate $Ctx
            if ($h.uncertain) { $note = Invoke-P9SEmergencyUncertain $Ctx $Res $vm $h $vg $s $g }
            elseif ($h.spent) {
                Set-P9SRefused $Res @($h.reasons) $g
                $Res.lines.Add("$($h.spent), while this emergency waited for the lock; it is not repeated. Watch it with emergency.wait-off$(if ($h.entry) { ' or acpi.confirm-off' }).")
            }
            elseif (-not $vg.ok) {
                # the management plane is in doubt: nothing can be sent,
                # and nothing is known about the guest
                $Res.outcome = 'FAILED'; foreach ($r in $vg.reasons) { $Res.reasons.Add($r) }; $Res.reasons.Add('EMERGENCY_UNRESOLVED')
                $Res.lines.Add("*** VirtualBox is $($vg.text). No ACPI request can be sent through the toolkit and the state of $vm is unknown. EMERGENCY UNRESOLVED: the stop request stays in place; the operator decides outside the coordinator (vbox.status shows the quarantine; vbox.recover clears it once the cause is resolved). ***")
                $null = Set-P9SStopRequestUnresolved $Ctx $vm -Locked
                if ($s -and $s.session_id) { $s.lifecycle = 'EMERGENCY_UNRESOLVED'; Save-P9SSession $Ctx $s }
            }
            else {
                $st = (Get-P9SVmObservation $Ctx $vm 'shutdown').state
                if ($st -eq 'off') { $Res.outcome = 'DONE'; $Res.lines.Add("$vm is already off; no request sent") }
                elseif ($st -ne 'running') { Set-P9SRefused $Res @("VM_NOT_RUNNING:$st") $g; $Res.lines.Add("$vm is $st; an ACPI request is sent only to a running VM") }
                else {
                    # recorded before it is sent: first in the unresolved
                    # validation of this VM, whose one request it spends,
                    # then in the stop request; when either cannot be
                    # written nothing is sent
                    $recorded = $true
                    if ($h.entry) {
                        $ve = $h.entry
                        $ve.acpi_used = [int]$ve.acpi_used + 1; $ve.acpi_source = 'emergency'; $ve.phase = 'SHUTDOWN_REQUESTING'
                        $ve.deadline_utc = (Get-P9SUtcNow $Ctx).AddSeconds($limit).ToString('o')
                        Add-P9SAcpiEvent $ve 'emergency-acpi' 'emergency.acpi-shutdown sends the request'
                        if (-not (Save-P9SAcpiLedger $Ctx $h.ledger)) { $recorded = $false; $Res.lines.Add("the ACPI validation ledger could not be written (validation $($ve.authorization_id)); if the write reached the file, the request counts as spent there") }
                    }
                    if ($recorded) {
                        $req = if ($h.stop) { $h.stop } else { New-P9SStopRequestDoc $Ctx }
                        $req.acpi[$vm] = [DateTime]::UtcNow.ToString('o')
                        try { Save-P9SStopRequest $Ctx $req } catch { $recorded = $false; $Res.lines.Add("the stop request could not be written ($($_.Exception.Message))") }
                    }
                    if (-not $recorded) {
                        $Res.outcome = 'FAILED'; foreach ($r in 'EVIDENCE_WRITE_FAILED', 'ACPI_REQUEST_NOT_SENT', 'EMERGENCY_UNRESOLVED') { $Res.reasons.Add($r) }
                        $Res.lines.Add("*** the request to $vm could not be recorded before it is sent: NOTHING WAS SENT, and nothing is claimed. No keyboard input, no forced power-off. EMERGENCY UNRESOLVED: the operator decides outside the coordinator. ***")
                        $null = Set-P9SStopRequestUnresolved $Ctx $vm -Locked
                        if ($s -and $s.session_id) { try { $s.lifecycle = 'EMERGENCY_UNRESOLVED'; Save-P9SSession $Ctx $s } catch {} }
                    } else {
                        try {
                            & $Ctx.Dispatch['vm.acpi'] $vm
                            $Res.dispatched = 1; $sent = $true
                            $Res.lines.Add("ACPI power-button request sent to $vm")
                        } catch {
                            $Res.outcome = 'FAILED'; $Res.reasons.Add('ACPI_REQUEST_FAILED'); $Res.reasons.Add('EMERGENCY_UNRESOLVED')
                            $Res.lines.Add("*** ACPI request to $vm failed ($($_.Exception.Message)). EMERGENCY UNRESOLVED: nothing else is tried; the operator decides outside the coordinator. ***")
                            $null = Set-P9SStopRequestUnresolved $Ctx $vm -Locked
                            if ($s -and $s.session_id) { $s.lifecycle = 'EMERGENCY_UNRESOLVED'; Save-P9SSession $Ctx $s }
                            $null = Set-P9SAcpiAfterEmergency $Ctx $vm 'ACPI_VALIDATION_UNRESOLVED' -Locked
                        }
                    }
                }
            }
        } finally { $lock.Dispose() }
        if ($sent) {
            $elapsed = 0; $st = $null; $vq = $null
            while ($true) {
                $vq = Get-P9SVBoxGate $Ctx
                if (-not $vq.ok) { $st = $null; break }
                $st = (Get-P9SVmObservation $Ctx $vm 'confirm').state
                if ($st -in 'off', 'aborted' -or $elapsed -ge $limit) { break }
                & $Ctx.Sleep 2000; $elapsed += 2
            }
            $null = Set-P9SAcpiAfterEmergency $Ctx $vm $(if ($vq.ok -and $st -eq 'off') { 'OFF_CONFIRMED' } elseif ($vq.ok -and $st -eq 'aborted') { 'OFF_ABNORMAL' } else { 'ACPI_VALIDATION_UNRESOLVED' })
            if (-not $vq.ok) {
                $Res.outcome = 'FAILED'; foreach ($r in $vq.reasons) { $Res.reasons.Add($r) }; $Res.reasons.Add('EMERGENCY_UNRESOLVED')
                $Res.lines.Add("*** VirtualBox became $($vq.text) while $vm was watched (about $elapsed s after the request). Its state is unknown. EMERGENCY UNRESOLVED: no further query, no second request; the operator decides outside the coordinator. ***")
                $null = Set-P9SStopRequestUnresolved $Ctx $vm
                Set-P9SSessionAfterEmergency $Ctx $null $null 'EMERGENCY_UNRESOLVED'
            }
            elseif ($st -eq 'off') {
                $Res.outcome = 'DONE'; $Res.lines.Add("$vm powered off after the ACPI request (about $elapsed s)")
                Set-P9SSessionAfterEmergency $Ctx $vm 'off' $null
            } elseif ($st -eq 'aborted') {
                $Res.outcome = 'OFF_ABNORMAL'; $Res.reasons.Add('VM_ABORTED'); $Res.lines.Add("$vm ended as aborted, not by an orderly shutdown")
                Set-P9SSessionAfterEmergency $Ctx $vm 'aborted' $null
            } else {
                $Res.outcome = 'TIMEOUT'; $Res.reasons.Add('EMERGENCY_UNRESOLVED')
                $Res.lines.Add("*** $vm is still $st $limit s after the ACPI request. EMERGENCY UNRESOLVED: no forced power-off and no keyboard input are attempted; the operator decides outside the coordinator. ***")
                $null = Set-P9SStopRequestUnresolved $Ctx $vm
                Set-P9SSessionAfterEmergency $Ctx $null $null 'EMERGENCY_UNRESOLVED'
            }
        }
        Add-P9SAudit -Ctx $Ctx -Res $Res -Request $Request -V $V -Note $note
        return $Res
    }
    # emergency.wait-off: read only, bounded
    $vg = Get-P9SVBoxGate $Ctx
    if (-not $vg.ok) {
        Set-P9SRefused $Res @($vg.reasons) $null
        $Res.lines.Add("VirtualBox is $($vg.text): the state of $vm is not read and is unknown")
        Add-P9SAudit -Ctx $Ctx -Res $Res -Request $Request -V $V
        return $Res
    }
    $limit = [int]$Request.Seconds; $elapsed = 0; $st = $null
    while ($true) {
        $st = (Get-P9SVmObservation $Ctx $vm 'confirm').state
        if ($st -in 'off', 'aborted' -or $elapsed -ge $limit) { break }
        & $Ctx.Sleep 2000; $elapsed += 2
    }
    if ($st -in 'off', 'aborted') {
        $Res.outcome = 'DONE'; $Res.lines.Add("$vm is $st$(if ($st -eq 'aborted') { ' (abnormal end, not an orderly shutdown)' })")
        $lock = Enter-P9SLock $Ctx 2000
        if ($lock) {
            try { $sr = Read-P9SSession $Ctx; if ($sr.ok -and $sr.session -and $sr.session.session_id) { $sr.session.expected_vm[$vm] = $st; Save-P9SSession $Ctx $sr.session } } finally { $lock.Dispose() }
        }
    } else { $Res.outcome = 'TIMEOUT'; $Res.reasons.Add('VM_NOT_OFF'); $Res.lines.Add("$vm still $st after $limit s") }
    Add-P9SAudit -Ctx $Ctx -Res $Res -Request $Request -V $V
    $Res
}

# ------------------------------------------------------------ observation
function ConvertTo-P9SUtc($v) {
    if ($v -is [datetime]) { return $v.ToUniversalTime() }
    [DateTime]::Parse([string]$v, [Globalization.CultureInfo]::InvariantCulture, [Globalization.DateTimeStyles]::AdjustToUniversal -bor [Globalization.DateTimeStyles]::AssumeUniversal)
}
# VM state changes that no dispatched coordinator action explains within
# the window. Detection after the fact, not prevention. Input: the
# coordinator's observations (-Observations: utc, vm, state, observed, from
# vm-observations.jsonl), or -Samples, the rows of a samples.csv of layout
# 1 (toolkit 83c9ef8 and earlier, whose monitor read the VM states:
# d12_state and u24_state are the first and second configured VM). On
# demand observations see a change only at the next observation.
function ConvertFrom-P9SLegacySamples([object[]]$Samples) {
    $names = @(Get-P9SVmNames); $cols = @('d12_state', 'u24_state')
    foreach ($s in @($Samples)) { for ($i = 0; $i -lt [math]::Min(2, $names.Count); $i++) { [pscustomobject]@{ utc = $s.utc; vm = $names[$i]; state = [string]$s.($cols[$i]); observed = $null } } }
}
function Find-OutOfBandVmChanges {
    param([object[]]$Samples, [object[]]$Observations, [object[]]$Audit, [double]$WindowSeconds = 120)
    $obs = if ($PSBoundParameters.ContainsKey('Observations')) { @($Observations) } else { @(ConvertFrom-P9SLegacySamples $Samples) }
    $ev = [System.Collections.Generic.List[object]]::new()
    foreach ($vm in Get-P9SVmNames) {
        $prev = $null
        foreach ($s in @($obs | Where-Object { [string]$_.vm -ceq $vm })) {
            $cur = [string]$s.state
            # no observation: the lock was busy, VirtualBox was quarantined, or the read failed
            if (-not $cur -or $s.observed -eq $false -or $cur -in 'busy', 'quarantined', 'unreadable') { continue }
            if ($prev -and $cur -ne $prev) {
                $t = ConvertTo-P9SUtc $s.utc
                $want = if ($cur -in 'starting', 'running') { @('vm.start', 'acpi.start') } elseif ($cur -in 'stopping', 'off') { @('emergency.acpi-shutdown', 'acpi.shutdown') } else { @() }
                $by = $null
                if ($want.Count) {
                    foreach ($x in @($Audit)) {
                        if ($x.action -in $want -and $x.vm -eq $vm -and ($x.outcome -eq 'DISPATCHED' -or [int]$x.dispatched -gt 0)) {
                            $ta = ConvertTo-P9SUtc $x.utc
                            if ($ta -le $t -and ($t - $ta).TotalSeconds -le $WindowSeconds) { $by = "$($x.action) at $($ta.ToString('HH:mm:ss'))Z"; break }
                        }
                    }
                }
                $ev.Add([pscustomobject]@{ utc = $t.ToString('yyyy-MM-ddTHH:mm:ssZ'); vm = $vm; from = $prev; to = $cur; explained = [bool]$by; by = $by })
            }
            $prev = $cur
        }
    }
    $ev.ToArray()
}
# Machine-readable VirtualBox configuration against a reference record:
# changed keys only (no values), runtime keys ignored, capture keys flagged.
function Compare-VmConfig {
    param([string[]]$Reference, [string[]]$Current)
    $vol = '^(VMState|VMStateChangeTime|VideoMode|SessionName|GuestAdditionsRunLevel|GuestAdditionsVersion|GuestAdditionsFacility_[^=]*)='
    $map = { param($lines) $m = [ordered]@{}; foreach ($l in @($lines)) { if ($l -notmatch $vol -and $l -match '^([^=]+)=(.*)$') { $m[$Matches[1]] = $Matches[2] } }; $m }
    $x = & $map $Reference; $y = & $map $Current
    foreach ($k in (@($x.Keys) + @($y.Keys) | Select-Object -Unique)) {
        $p = $x[$k]; $q = $y[$k]
        if ($p -cne $q) { [pscustomobject]@{ key = $k; change = $(if ($null -eq $p) { 'added' } elseif ($null -eq $q) { 'removed' } else { 'changed' }); capture = ($k -match 'nictrace') } }
    }
}
function Invoke-P9SObserve($Ctx, [hashtable]$Request, $V, $Res) {
    switch ([string]$Request.Action) {
        'session.status' {
            $sr = Read-P9SSession $Ctx; $g = Get-P9SGate $Ctx; $Res.status = $g.status; $Res.run_id = $g.run_id
            $Res.lines.Add("monitor: status $(if ($g.status) { $g.status } else { '-' }), run $(if ($g.run_id) { $g.run_id } else { '-' }); $(if ($g.ok) { 'READY' } else { $g.reasons -join ', ' })")
            if (-not $sr.ok) { $Res.lines.Add('session: UNREADABLE') }
            elseif (-not $sr.session) { $Res.lines.Add('session: none') }
            else { $s = $sr.session; $Res.lines.Add("session: $($s.session_id) kind $($s.kind) lifecycle $($s.lifecycle) run $($s.monitor_run_id)$(if ($s.halt) { "; halt $($s.halt.status) $($s.halt.abort_id) acknowledged $($s.halt.acknowledged)" })$(if ($s.inflight) { "; IN-FLIGHT $($s.inflight.action)" })") }
            if ($sr.ok -and $sr.session -and [string]$sr.session.kind -ceq 'AcpiValidation' -and $sr.session.validation) {
                $va = $sr.session.validation; $lg = Read-P9SAcpiLedger $Ctx
                $en = if ($lg.ok) { Get-P9SAcpiEntry $lg.doc ([string]$va.authorization_id) } else { $null }
                $Res.lines.Add("ACPI validation: authorization $($va.authorization_id), target $($va.target_vm), until $((ConvertTo-P9SAuthTime $va.expires_utc).ToString('o')); $(if ($en) { "phase $($en.phase), starts $($en.startups_used) of $($va.max_startups), ACPI requests $($en.acpi_used) of $($va.max_acpi_requests)$(if ($en.outcome) { ", outcome $($en.outcome)" })" } else { "ledger $(if ($lg.ok) { 'entry missing' } else { $lg.reason })" })")
                foreach ($n in @($sr.session.expected_vm.Keys)) { $Res.lines.Add("  $n expected $($sr.session.expected_vm[$n]) (an assumption; acpi.preflight or emergency.status observe it)") }
            }
        }
        'audit.oob' {
            $csv = Join-Path $Request.MonitorRun 'samples.csv'
            $samples = @(if (Test-Path -LiteralPath $csv) { Import-Csv -LiteralPath $csv })
            $aud = @(if (Test-Path -LiteralPath $Ctx.AuditPath) { Get-Content -LiteralPath $Ctx.AuditPath | ForEach-Object { try { $_ | ConvertFrom-Json } catch {} } })
            if ($samples.Count -and @($samples[0].PSObject.Properties.Name) -contains 'd12_state') {
                $ev = @(Find-OutOfBandVmChanges -Samples $samples -Audit $aud); $src = "$($samples.Count) samples"
            } else {
                # samples layout 2: the monitor holds no VM state; the
                # coordinator's own observations made during the run
                $from = $null; $to = [DateTime]::UtcNow
                try { $from = ConvertTo-P9SUtc (Get-Content -Raw -LiteralPath (Join-Path $Request.MonitorRun 'run.json') | ConvertFrom-Json).start_utc } catch {}
                try { $to = ConvertTo-P9SUtc (Get-Content -Raw -LiteralPath (Join-Path $Request.MonitorRun 'run-end.json') | ConvertFrom-Json).end_utc } catch {}
                $obs = @(Read-P9SVmObservations (Get-Prop $Ctx 'ObservationPath') | Where-Object { $u = ConvertTo-P9SUtc $_.utc; (-not $from -or $u -ge $from) -and $u -le $to })
                $ev = @(Find-OutOfBandVmChanges -Observations $obs -Audit $aud); $src = "$($obs.Count) VM observations of the coordinator during the run"
            }
            $un = @($ev | Where-Object { -not $_.explained })
            $Res.lines.Add("VM state changes in $($src): $($ev.Count), not explained by a coordinator action: $($un.Count)")
            foreach ($e in $ev) { $Res.lines.Add("  $($e.utc) $($e.vm) $($e.from) -> $($e.to): $(if ($e.explained) { $e.by } else { 'POSSIBLE OUT-OF-BAND CHANGE' })") }
            if ($un.Count) { $Res.reasons.Add("OUT_OF_BAND_CHANGES:$($un.Count)") }
        }
        'vbox.status' {
            # read only and without any VirtualBox call
            $vg = Get-P9SVBoxGate $Ctx
            $Res.lines.Add("VirtualBox API: $($vg.text)")
            $vb = Get-Prop $Ctx 'VBox'
            if ($vb) {
                $rr = Read-P9SVBoxRecord $vb
                if ($rr.present) { $Res.lines.Add($(if ($rr.ok) { "operation record: $($rr.doc.operation_id) $($rr.doc.operation) $($rr.doc.vm) phase $($rr.doc.phase)$(if ($rr.doc.child_pid) { "; VBoxManage child $($rr.doc.child_pid) $(Test-P9SChildAlive $rr.doc.child_pid $rr.doc.child_start_ticks)" })" } else { 'operation record: UNREADABLE' })) }
                if ($vg.latch -and $vg.latch.child_pid) { $Res.lines.Add("quarantined child: VBoxManage $($vg.latch.child_pid) $(Test-P9SChildAlive $vg.latch.child_pid $vg.latch.child_start_ticks)") }
                if (-not $vg.ok) { $Res.lines.Add('recovery: see "VirtualBox operations" in README.md; p9s-action.ps1 -Action vbox.recover -Operation <operation id> clears it only after its checks') }
            }
            if (-not $vg.ok) { foreach ($r in $vg.reasons) { $Res.reasons.Add($r) } }
        }
        'audit.vmconfig' {
            $vg = Get-P9SVBoxGate $Ctx
            if (-not $vg.ok) { Set-P9SRefused $Res @($vg.reasons) $null; Add-P9SAudit -Ctx $Ctx -Res $Res -Request $Request -V $V; return $Res }
            $d = @(Compare-VmConfig -Reference (Get-Content -LiteralPath $Request.Reference) -Current @(& $Ctx.ReadVmConfig ([string]$Request.Vm)))
            $Res.lines.Add("$($Request.Vm) configuration against $([IO.Path]::GetFileName([string]$Request.Reference)): $($d.Count) differing keys")
            foreach ($x in $d) { $Res.lines.Add("  $($x.key) $($x.change)$(if ($x.capture) { ' (CAPTURE SETTING)' })") }
            if ($d.Count) { $Res.reasons.Add("VM_CONFIG_CHANGED:$($d.Count)") }
        }
    }
    $Res.outcome = 'DONE'
    Add-P9SAudit -Ctx $Ctx -Res $Res -Request $Request -V $V
    $Res
}

# ------------------------------------------------------------ calibration
# Monitor conditions for a calibration step: the structural and freshness
# checks of the normal reader, the monitor in Calibrate mode, status
# QUALIFYING, UNCALIBRATED (Calibrate mode with the paging stop required and
# no approved values: the premise of a calibration) or CALIBRATING (with
# -PagingStop NotRequired); never WARNING, UNKNOWN or a latch; commit below
# 90 %, D: confirmed, no storage event. The 95/97 % rules stay with the
# monitor.
function Test-P9SCalibrationState {
    param([AllowNull()][AllowEmptyString()][string]$Text, [long]$NowQpc, [long]$QpcFrequency, [scriptblock]$IsMonitorAlive)
    $t = Test-SafetyState -Text $Text -NowQpc $NowQpc -QpcFrequency $QpcFrequency -PagingStop NotRequired -IsMonitorAlive $IsMonitorAlive
    $bad = [System.Collections.Generic.List[string]]::new()
    foreach ($r in $t.reasons) { if ($r -notin 'STATUS_QUALIFYING', 'STATUS_UNCALIBRATED', 'STATUS_CALIBRATING', 'GATE_INCOMPLETE') { $bad.Add($r) } }
    $doc = $null; try { $doc = $Text | ConvertFrom-Json -ErrorAction Stop } catch {}
    $q = $false; $since = $null; $commit = $null; $avail = $null
    if ($doc -is [pscustomobject] -and $doc.schema -eq 'p9s-hostmon-state/1') {
        if ([string]$doc.monitor.mode -ne 'Calibrate') { $bad.Add('CALIBRATION_REQUIRES_CALIBRATE_MODE') }
        if ($doc.external_disk -and ($doc.external_disk.present -ne $true -or $doc.external_disk.identity_ok -ne $true)) { $bad.Add('EXTERNAL_DISK_NOT_CONFIRMED') }
        if ($doc.storage_events -and [int]$doc.storage_events.count_since_start -gt 0) { $bad.Add('STORAGE_EVENTS') }
        if (($doc.paging -and $doc.paging.inputs_complete -ne $true) -or -not $doc.memory -or -not (Test-Finite $doc.memory.avail_mb)) { $bad.Add('MEASUREMENTS_INCOMPLETE') }
        $q = ($doc.gate.qualified -eq $true); $since = $doc.gate.since_mono_s; $commit = $doc.commit.pct
        if ($doc.memory) { $avail = $doc.memory.avail_mb }
    }
    [pscustomobject]@{ ok = ($bad.Count -eq 0); reasons = @($bad); qualified = $q; gate_since = $since; status = $t.status; commit_pct = $commit; avail_mb = $avail }
}
function New-P9SSimSample([double]$T, [double]$Commit, $Vms, [hashtable]$Knobs = @{}) {
    $g = [math]::Pow(2, 30)
    $base = if ($Knobs.ContainsKey('avail_base_mb')) { [double]$Knobs.avail_base_mb } else { 18000.0 }
    $running = @($Vms.Values | Where-Object { $_ -eq 'running' }).Count
    [pscustomobject]@{
        seq = 0; mono = $T; utc = 'simulated'; read_ok = $true; read_error = $null
        committed_bytes = $Commit * $g; commit_limit_bytes = 100 * $g; avail_mb = $base - 4096.0 * $running; modified_list_mb = 200.0
        pages = 120.0; pages_in = 120.0; pages_out = $(if ($Knobs.ContainsKey('pages_out')) { [double]$Knobs.pages_out } else { 0.0 }); page_reads = 30.0; page_writes = 0.0
        page_faults = 9000.0; transition_faults = 3000.0; cache_faults = 1000.0; demand_zero_faults = 3000.0; rate_issues = @()
        pagefile_used_mb = 90.0; pagefile_alloc_mb = 5888.0; pagefile_pct = 1.5; pagefile_disks = $(if ($Knobs.no_pagefile_disk) { $null } else { @(0) }); system_disk = 0; external_disk = 2
        disks = @{ 0 = [pscustomobject]@{ read_ms = 1.5; write_ms = 1.0; read_mbps = 8.0; write_mbps = 2.0; queue = 0; avg_queue = 0.3 }
                   2 = [pscustomobject]@{ read_ms = 2.0; write_ms = 1.0; read_mbps = 0.5; write_mbps = 0.5; queue = 0; avg_queue = 0.05 } }
        external_present = $true; external_identity_ok = $true; eventlog_ok = $true; storage_events_new = 0; storage_event_ids = @(); cpu_pct = 10.0; vm = @{}
    }
}
# The whole calibration workflow against a simulated monitor (the real
# policy of hostmon-lib.ps1 fed with synthetic samples) and simulated VMs.
# It has no access to any dispatcher: nothing real can be started, typed or
# shut down. The first configured VM is the Debian one, the second the
# Ubuntu one. Durations and the memory condition are the planning values of
# Get-P9SCalibrationLimits (proposals until approved).
function Invoke-P9SCalibrationDryRun {
    param([Parameter(Mandatory)][string]$Scenario, [Parameter(Mandatory)]$Thresholds, [int]$SettleSeconds = 120, [int]$ObserveSeconds = 1800,
          [int]$QualifyTimeoutSeconds = 900, [int]$ShutdownWaitSeconds = 180, [int]$SecondVmMinAvailableMb = 8192)
    if ($Scenario -cnotin (Get-P9SScenarios)) { throw "unknown scenario $Scenario" }
    $names = @(Get-P9SVmNames); $deb = $names[0]; $ubu = $names[1]
    $mode = if ($Scenario -eq 'monitor-mode') { 'Monitor' } else { 'Calibrate' }
    $mst = New-MonitorState -RunId "sim-$Scenario" -Config (New-HostMonConfig -PagingStop Required -Mode $mode -Thresholds $Thresholds)
    $sim = @{ t = 0.0; doc = $null; vms = [ordered]@{ $deb = 'off'; $ubu = 'off' }; phase = 'idle'; observeStart = 0.0; lag = $(if ($Scenario -eq 'stale-monitor') { 20.0 } else { 0.0 }); dispatches = 0 }
    $knobs = {
        $k = @{}
        $debOn = $sim.vms[$deb] -eq 'running'
        if ($Scenario -eq 'low-memory') { $k.avail_base_mb = 11000.0 }
        if ($Scenario -eq 'paging-warning' -and $debOn) { $k.pages_out = 3000.0 }
        if ($Scenario -eq 'missing-counters' -and $debOn) { $k.no_pagefile_disk = $true }
        $k
    }
    $commit = {
        $c = 54.0 + 9.0 * @($sim.vms.Values | Where-Object { $_ -eq 'running' }).Count
        if ($Scenario -eq 'warning-during-start' -and $sim.vms[$ubu] -eq 'running') { $c = 91.0 }
        if ($Scenario -eq 'abort-during-observe' -and $sim.phase -eq 'observe' -and ($sim.t - $sim.observeStart) -ge 600) { $c = 97.0 }
        $c
    }
    $advance = { param([double]$secs) for ($k = 0; $k -lt [math]::Round($secs / 5); $k++) { $sim.t += 5; $sim.doc = Step-Monitor -State $mst -Sample (New-P9SSimSample $sim.t (& $commit) $sim.vms (& $knobs)) } }
    $check = {
        $info = [ordered]@{ pid = 0; process_start_utc = 'simulated'; process_start_ticks = 0; mode = $mode; paging_stop = 'Required'; thresholds_sha256 = $Thresholds.sha256; thresholds_approved = $Thresholds.approved; test = $false; kit = 'simulation' }
        $txt = ConvertTo-StateJson -Doc $sim.doc -Monitor $info -WrittenQpc ([long](($sim.t - $sim.lag) * 1e7)) -QpcFrequency 10000000 -WrittenUtc 'simulated'
        Test-P9SCalibrationState -Text $txt -NowQpc ([long]($sim.t * 1e7)) -QpcFrequency 10000000 -IsMonitorAlive { $true }
    }
    $steps = [System.Collections.Generic.List[object]]::new()
    $add = { param($step, $kind, $outcome, $detail) $steps.Add([pscustomobject]@{ step = $step; kind = $kind; t = $sim.t; outcome = $outcome; detail = $detail }) }
    # a timed stretch with a monitor check every 5 s; returns the reasons of the first failed check
    $waitChecked = { param([int]$secs) for ($s = 0; $s -lt $secs; $s += 5) { & $advance 5; $c = & $check; if (-not $c.ok) { return , @($c.reasons) } }; $null }
    $planned = @(@('preflight', 'preflight'), @("vm-start $deb", 'vm-start'), @("settle $deb", 'settle'), @('operator-assist debian-tor', 'operator-assist'),
        @('memory-check', 'memory-check'), @("vm-start $ubu", 'vm-start'), @("settle $ubu", 'settle'), @('operator-assist ubuntu-tor-browser', 'operator-assist'),
        @('qualify-300s', 'qualify-300s'), @('observe', 'observe'))
    $pending = @(
        'a live calibration needs the owner''s separate authorization and a toolkit version that implements the live steps (none does)',
        "the Debian VM's system Tor is started or confirmed by the operator (service management may need privileges the toolkit does not hold)",
        'the Ubuntu desktop login, the Tor Browser launch and Connect are operator steps; the toolkit sends no keyboard or mouse input during calibration',
        "at least $SecondVmMinAvailableMb MB available before the second VM is a proposal that needs validation and approval",
        "durations: settle $SettleSeconds s per VM, qualification cap $QualifyTimeoutSeconds s, observation $ObserveSeconds s, shutdown wait $ShutdownWaitSeconds s are planning values",
        'a WARNING of any kind (commit 90 % or more, suspected paging pressure with the unapproved candidate values, missing measurements) ends the calibration here; whether to hold instead is an owner decision',
        'how each guest reacts to the ACPI power button (shutdown, suspend or a dialog) is not verified yet',
        'the frozen snapshot images are hashed before a live start; not part of this dry run')
    $halt = $null; $why = @(); $unresolved = $false
    & $advance 305
    $c = & $check
    if (-not $c.ok -or -not $c.qualified) { $halt = 'preflight'; $why = @($c.reasons) + $(if (-not $c.qualified -and $c.ok) { @('GATE_INCOMPLETE') } else { @() }); & $add 'preflight' 'preflight' 'HALTED' ($why -join ', ') }
    else { & $add 'preflight' 'preflight' 'PASS' "monitor in Calibrate mode, status $($c.status), commit $([math]::Round($c.commit_pct, 1)) %, 300-s gate met with the VMs off; disk, VM configuration and authorization simulated as present" }
    foreach ($pair in @(@($deb, 'operator-assist debian-tor', 30, 'the operator starts or confirms the system Tor service of the Debian VM through its normal service management and confirms it from the guest console; the toolkit sends no input; checks continue every 5 s'),
                        @($ubu, 'operator-assist ubuntu-tor-browser', 90, 'the operator logs in to the Ubuntu desktop, starts Tor Browser from its launcher and presses Connect, then confirms the bootstrap; the toolkit sends no keyboard or mouse input; checks continue every 5 s'))) {
        $vm = $pair[0]
        if ($halt) { break }
        if ($vm -eq $ubu) {
            $c = & $check
            if (-not $c.ok) { $halt = 'memory-check'; $why = @($c.reasons); & $add 'memory-check' 'memory-check' 'HALTED' ($why -join ', '); break }
            if (-not (Test-Finite $c.avail_mb)) { $halt = 'memory-check'; $why = @('MEASUREMENTS_INCOMPLETE'); & $add 'memory-check' 'memory-check' 'HALTED' 'available memory not measured'; break }
            if ([double]$c.avail_mb -lt $SecondVmMinAvailableMb) { $halt = 'memory-check'; $why = @('MEMORY_BELOW_PROPOSED_MINIMUM'); & $add 'memory-check' 'memory-check' 'HALTED' "available $([math]::Round([double]$c.avail_mb)) MB below $SecondVmMinAvailableMb MB (proposed condition)"; break }
            & $add 'memory-check' 'memory-check' 'PASS' "available $([math]::Round([double]$c.avail_mb)) MB, at least $SecondVmMinAvailableMb MB (proposed condition, not yet approved)"
        }
        $c = & $check
        if (-not $c.ok) { $halt = "vm-start $vm"; $why = @($c.reasons); & $add "vm-start $vm" 'vm-start' 'HALTED' ($why -join ', '); break }
        $sim.vms[$vm] = 'running'; $sim.dispatches++
        & $add "vm-start $vm" 'vm-start' 'SIMULATED' "VBoxManage $((Get-VBoxStartArgs $vm) -join ' ') (not executed)"
        $r = & $waitChecked $SettleSeconds
        if ($r) { $halt = "settle $vm"; $why = @($r); & $add "settle $vm" 'settle' 'HALTED' ($why -join ', '); break }
        & $add "settle $vm" 'settle' 'PASS' "$SettleSeconds s with the monitor conditions met at every 5-s check"
        $r = & $waitChecked $pair[2]
        if ($r) { $halt = $pair[1]; $why = @($r); & $add $pair[1] 'operator-assist' 'HALTED' ($why -join ', '); break }
        & $add $pair[1] 'operator-assist' 'OPERATOR-ASSISTED (simulated)' $pair[3]
    }
    $torT = $sim.t
    if (-not $halt) {
        $ok = $false
        for ($s = 0; $s -lt $QualifyTimeoutSeconds; $s += 5) {
            & $advance 5; $c = & $check
            if (-not $c.ok) { $halt = 'qualify-300s'; $why = @($c.reasons); & $add 'qualify-300s' 'qualify-300s' 'HALTED' ($why -join ', '); break }
            if ($c.qualified -and $null -ne $c.gate_since -and [double]$c.gate_since -le $torT -and ($sim.t - $torT) -ge 300) { $ok = $true; break }
        }
        if (-not $halt -and -not $ok) { $halt = 'qualify-300s'; $why = @('QUALIFICATION_TIMEOUT'); & $add 'qualify-300s' 'qualify-300s' 'HALTED' "QUALIFICATION_TIMEOUT after $QualifyTimeoutSeconds s" }
        if ($ok) { & $add 'qualify-300s' 'qualify-300s' 'PASS' "300 continuous s below 90 % with both VMs and both Tor environments up (from t=$torT s)" }
    }
    if (-not $halt) {
        $sim.phase = 'observe'; $sim.observeStart = $sim.t
        $r = & $waitChecked $ObserveSeconds
        if ($r) { $halt = 'observe'; $why = @($r); & $add 'observe' 'observe' 'HALTED' "after $($sim.t - $sim.observeStart) s: $($why -join ', ')" }
        else { & $add 'observe' 'observe' 'PASS' "$ObserveSeconds s of host measurements kept by the monitor (samples.csv), no test action" }
    }
    if ($halt) { foreach ($p in $planned) { if (-not @($steps | Where-Object { $_.step -eq $p[0] }).Count) { & $add $p[0] $p[1] 'CANCELLED' "not started after the halt at $halt" } } }
    foreach ($vm in $ubu, $deb) {
        if ($sim.vms[$vm] -ne 'running') { continue }
        if ($halt) { & $add "acpi-shutdown $vm" 'acpi-shutdown' 'EMERGENCY (simulated)' "emergency.acpi-shutdown: one ACPI power-button request, then the VM state is watched for up to $ShutdownWaitSeconds s" }
        else { & $add "acpi-shutdown $vm" 'acpi-shutdown' 'SIMULATED' "VBoxManage $((Get-VBoxAcpiArgs $vm) -join ' ') (not executed)" }
        $sim.dispatches++
        if ($Scenario -eq 'acpi-timeout' -and $vm -eq $ubu) {
            $sim.t += $ShutdownWaitSeconds
            & $add "wait-off $vm" 'wait-off' 'UNRESOLVED (operator decision)' "still running $ShutdownWaitSeconds s after the request; no second request, no forced power-off, no keyboard input"
            $unresolved = $true
        } else { & $add "wait-off $vm" 'wait-off' 'SIMULATED' 'VM state read until off (read only)'; $sim.vms[$vm] = 'off' }
    }
    [pscustomobject]@{
        scenario = $Scenario; mode = $mode; verdict = $(if ($halt) { 'HALTED' } elseif ($unresolved) { 'SHUTDOWN_UNRESOLVED' } else { 'COMPLETED' }); halted_at = $halt; reasons = @($why)
        steps = $steps.ToArray(); pending = $pending; real_dispatches = 0; simulated_dispatches = $sim.dispatches; simulated_seconds = $sim.t
    }
}
function Invoke-P9SCalibration($Ctx, [hashtable]$Request, $V, $Res) {
    if ([string]$Request.Action -eq 'calib.dry-run') {
        $th = Get-PagingThresholds -Path $Ctx.ThresholdsPath
        $d = Invoke-P9SCalibrationDryRun -Scenario ([string]$Request.Scenario) -Thresholds $th
        $Res.lines.Add("SIMULATED calibration dry run, scenario $($d.scenario) (monitor $($d.mode) mode, policy of hostmon-lib.ps1): $($d.verdict)$(if ($d.halted_at) { " at $($d.halted_at): $($d.reasons -join ', ')" }); simulated time $($d.simulated_seconds) s; real dispatches $($d.real_dispatches)")
        foreach ($s in $d.steps) { $Res.lines.Add(('  t={0,6} s  {1,-20} {2}  {3}' -f $s.t, $s.step, $s.outcome, $s.detail)) }
        $Res.lines.Add('  pending before any live calibration:')
        foreach ($p in $d.pending) { $Res.lines.Add("    - $p") }
        $Res.outcome = 'DRYRUN'
        Add-P9SAudit -Ctx $Ctx -Res $Res -Request $Request -V $V -Note "calibration dry run $($d.scenario): $($d.verdict)"
        return $Res
    }
    # calib.preflight: read only
    $g = Get-P9SGate $Ctx; $Res.status = $g.status; $Res.run_id = $g.run_id
    $c = Test-P9SCalibrationState -Text $g.text -NowQpc (& $Ctx.NowQpc) -QpcFrequency $Ctx.QpcFrequency -IsMonitorAlive $Ctx.IsMonitorAlive
    $Res.lines.Add("monitor state: $(if ($c.ok) { "OK ($($c.status))" } else { 'NOT OK: ' + ($c.reasons -join ', ') })")
    $Res.lines.Add("300-s gate before the first VM start: $(if ($c.qualified) { 'met' } else { 'not met' })")
    $vg = Get-P9SVBoxGate $Ctx
    $Res.lines.Add("VirtualBox API: $($vg.text) -> $(if ($vg.ok) { 'PASS' } else { 'FAIL (no VirtualBox query is made)' })")
    foreach ($r in $vg.reasons) { $Res.reasons.Add($r) }
    $mem = $null
    if ($vg.ok) { $mem = 0; foreach ($n in Get-P9SVmNames) { $l = @(& $Ctx.ReadVmConfig $n) | Where-Object { $_ -match '^memory=(\d+)$' } | Select-Object -First 1; if ($l -match '^memory=(\d+)$') { $mem += [int]$Matches[1] } else { $mem = $null; break } } }
    $lim = Get-P9SCalibrationLimits
    $Res.lines.Add("available memory: $(if (-not (Test-Finite $c.avail_mb)) { 'PENDING (no monitor value)' } else { "$([math]::Round([double]$c.avail_mb)) MB now$(if ($null -ne $mem) { "; the two VMs are configured with $mem MB" }); at least $($lim.second_vm_min_available_mb) MB is required before the second VM starts (PROPOSAL, not approved)" })")
    if ($vg.ok) { foreach ($x in @(& $Ctx.CalibrationHostChecks)) { $Res.lines.Add("host: $x") } }
    else { $Res.lines.Add('host: VM checks not run while VirtualBox is quarantined -> FAIL') }
    $Res.lines.Add('authorization: none (a live calibration needs the owner''s separate authorization and a new kit version)')
    $Res.lines.Add('LIVE CALIBRATION: NOT AVAILABLE IN THIS KIT VERSION')
    if (-not $c.ok) { foreach ($r in $c.reasons) { $Res.reasons.Add($r) } }
    $Res.outcome = 'DONE'
    Add-P9SAudit -Ctx $Ctx -Res $Res -Request $Request -V $V
    $Res
}

# ------------------------------------------------------------ real context
# VM state through the VirtualBox layer: 'off', the VirtualBox state name,
# or 'unreadable' for anything else (no VBoxManage, a refused, failed or
# timed-out query, a quarantine, no VMState line). 'unreadable' is never
# taken for 'off'. The last layer result stays in $script:P9SLastVBoxResult.
function Get-P9SVmStateReal([string]$Vm, $VBox) {
    if ($Vm -cnotin (Get-P9SVmNames)) { return 'unreadable' }
    $r = $null
    try { $r = Invoke-P9SVBoxOperation -VBox $(if ($VBox) { $VBox } else { Get-P9SVBoxDefault }) -Operation 'vm.state' -Vm $Vm } catch {}
    $script:P9SLastVBoxResult = $r
    if (-not $r -or $r.outcome -ne 'COMPLETED' -or $r.exit_code -ne 0) { return 'unreadable' }
    # the answer must be about the VM asked for
    $nm = @($r.lines | Where-Object { $_ -like 'name=*' }) | Select-Object -First 1
    if ($nm -cne ('name="' + $Vm + '"')) { return 'unreadable' }
    $s = ConvertFrom-P9SVBoxState $r.lines
    if ($s) { $s } else { 'unreadable' }
}
function Get-P9SVmConfigReal([string]$Vm, $VBox) {
    if ($Vm -cnotin (Get-P9SVmNames)) { return @() }
    $r = $null
    try { $r = Invoke-P9SVBoxOperation -VBox $(if ($VBox) { $VBox } else { Get-P9SVBoxDefault }) -Operation 'vm.config' -Vm $Vm } catch {}
    $script:P9SLastVBoxResult = $r
    if (-not $r -or $r.outcome -ne 'COMPLETED' -or $r.exit_code -ne 0) { return @() }
    @($r.lines)
}
# A dispatch succeeded only if VBoxManage ran and exited 0; otherwise the
# reason code (never the command's output) is the error.
function Assert-P9SVBoxCompleted($R) {
    if ($R -and $R.outcome -eq 'COMPLETED' -and $R.exit_code -eq 0) { return }
    throw "$(if ($R) { $R.reason } else { 'VBOX_INTERNAL_ERROR' }) ($(if ($R) { $R.outcome } else { '-' })$(if ($R -and $null -ne $R.exit_code) { ", exit $($R.exit_code)" }))"
}
# Text -> groups of scan codes for the guests' Italian keyboard layout
# (letters, digits, space, '-' and '.'; '~' = em dash via Ctrl+Shift+U 2014
# Space).
function Get-P9SScancodesForText([string]$Text) {
    if ($Text -cnotmatch '^[A-Za-z0-9 .~-]+$') { throw 'text outside the allowed characters; nothing typed' }
    $codes = @{ q = 0x10; w = 0x11; e = 0x12; r = 0x13; t = 0x14; y = 0x15; u = 0x16; i = 0x17; o = 0x18; p = 0x19
        a = 0x1e; s = 0x1f; d = 0x20; f = 0x21; g = 0x22; h = 0x23; j = 0x24; k = 0x25; l = 0x26
        z = 0x2c; x = 0x2d; c = 0x2e; v = 0x2f; b = 0x30; n = 0x31; m = 0x32; ' ' = 0x39; '-' = 0x35; '.' = 0x34
        '1' = 0x02; '2' = 0x03; '3' = 0x04; '4' = 0x05; '5' = 0x06; '6' = 0x07; '7' = 0x08; '8' = 0x09; '9' = 0x0a; '0' = 0x0b }
    $seq = [System.Collections.Generic.List[object]]::new()
    foreach ($ch in $Text.ToCharArray()) {
        if ($ch -eq '~') { $seq.Add(@(0x1d, 0x2a, 0x16, 0x96, 0xaa, 0x9d)); foreach ($d in '2', '0', '1', '4') { $k = $codes[$d]; $seq.Add(@($k, ($k -bor 0x80))) }; $seq.Add(@(0x39, 0xb9)); continue }
        $k = $codes[([string]$ch).ToLowerInvariant()]
        if ([char]::IsUpper($ch)) { $seq.Add(@(0x2a, $k, ($k -bor 0x80), 0xaa)) } else { $seq.Add(@($k, ($k -bor 0x80))) }
    }
    , $seq.ToArray()
}
# Keyboard input through the VirtualBox COM API, under the VirtualBox lock
# (p9s-vbox.ps1 Invoke-P9SVBoxInProcess); the call itself cannot be bounded
# in time.
function Send-P9SGuestScancodes([string]$Vm, [object[]]$Groups) {
    $r = Invoke-P9SVBoxInProcess -VBox (Get-P9SVBoxDefault) -Operation 'guest.keys' -Vm $Vm -Arguments @($Vm, $Groups) -Body {
        param($name, $groupList)
        $vbc = New-Object -ComObject VirtualBox.VirtualBoxClient
        $m = $vbc.VirtualBox.FindMachine($name)
        if ($m.State -ne 6) { throw "the VM is not running (state $($m.State)); nothing sent" }
        $s = $vbc.Session
        $m.LockMachine($s, 1)
        try { $kb = $s.Console.Keyboard; foreach ($g in $groupList) { foreach ($c in @($g)) { $kb.PutScancode([int]$c); Start-Sleep -Milliseconds 60 }; Start-Sleep -Milliseconds 140 } }
        finally { $s.UnlockMachine() }
    }
    if ($r.outcome -ne 'COMPLETED') { throw "$($r.reason) ($($r.outcome))$(if ($r.note) { ": $($r.note)" })" }
}
# guest.unlock: the profile passphrase of the VM from the local secret store
# (p9s-config.ps1), typed as scan codes. Errors are codes; the value is never
# in a message. -Sender and -GetAcl are replaced by mocks in the tests.
function New-P9SUnlockDispatcher {
    param($Config, [scriptblock]$GetAcl, [Parameter(Mandatory)][scriptblock]$Sender, [string[]]$ForbiddenRoots = @(Get-P9SDefaultForbiddenRoots))
    $cfg = $Config; $acl = $GetAcl; $send = $Sender; $roots = $ForbiddenRoots
    {
        param($vm)
        $p = $null
        try {
            $p = Read-P9SSecret -Config $cfg -Name "$vm-profile-passphrase" -GetAcl $acl -ForbiddenRoots $roots
            if ($p -cnotmatch '^[a-z ]{20,}$') { throw 'SECRET_FORMAT' }
            & $send $vm (Get-P9SScancodesForText $p)
        } finally { $p = $null }
    }.GetNewClosure()
}
# The only real dispatchers. There is none for typing a shutdown command, for
# a forced power-off, a reset, a snapshot or a raw scan-code string.
function Get-P9SRealDispatch($Config) {
    @{
        'vm.start' = { param($vm) Assert-P9SVBoxCompleted (Invoke-P9SVBoxOperation -VBox (Get-P9SVBoxDefault) -Operation 'vm.start' -Vm $vm) }
        'guest.text' = { param($vm, $text) Send-P9SGuestScancodes $vm (Get-P9SScancodesForText $text) }
        'guest.key' = { param($vm, $codes) Send-P9SGuestScancodes $vm @(, [int[]]$codes) }
        'guest.unlock' = (New-P9SUnlockDispatcher -Config $Config -Sender { param($vm, $groups) Send-P9SGuestScancodes $vm $groups })
        'vm.acpi' = { param($vm) Assert-P9SVBoxCompleted (Invoke-P9SVBoxOperation -VBox (Get-P9SVBoxDefault) -Operation 'vm.acpi' -Vm $vm) }
    }
}
function Get-P9SCalibrationHostChecks($Config) {
    $out = [System.Collections.Generic.List[string]]::new()
    $ext = if ($Config) { $Config.external_disk } else { $null }
    if (-not $ext) { $out.Add('external disk: not configured -> PENDING') }
    else {
        try {
            $d = Get-Disk -Number (Get-Partition -DriveLetter $ext.drive_letter -ErrorAction Stop).DiskNumber -ErrorAction Stop
            $out.Add("external disk $($ext.drive_letter): $($d.FriendlyName), $($d.BusType), $($d.HealthStatus) -> $(if ($d.FriendlyName -eq $ext.model -and [string]$d.HealthStatus -eq 'Healthy') { 'PASS' } else { 'FAIL' })")
        } catch { $out.Add("external disk $($ext.drive_letter): not found -> FAIL") }
    }
    $names = @(Get-P9SVmNames)
    for ($i = 0; $i -lt $names.Count; $i++) {
        $vm = $names[$i]
        $st = Get-P9SVmStateReal $vm
        $out.Add("$vm state $st -> $(if ($st -eq 'off') { 'PASS' } else { 'FAIL' })")
        $ref = if ($Config -and $Config.vm_config_reference) { $(if ($i -eq 0) { $Config.vm_config_reference.debian } else { $Config.vm_config_reference.ubuntu }) } else { $null }
        if ($ref -and (Test-Path -LiteralPath $ref)) {
            $diff = @(Compare-VmConfig -Reference (Get-Content -LiteralPath $ref) -Current @(Get-P9SVmConfigReal $vm))
            $out.Add("$vm configuration against $([IO.Path]::GetFileName($ref)): $($diff.Count) differing keys$(if ($diff.Count) { ' (' + (($diff | ForEach-Object key) -join ', ') + ')' }) -> $(if ($diff.Count) { 'FAIL' } else { 'PASS' })")
        } else { $out.Add("$vm configuration reference not configured or missing -> PENDING") }
    }
    $out.Add('frozen snapshot images: PENDING (hashed before a live calibration; not run by the preflight)')
    $out
}
function New-P9SRealContext([string]$StatePath, [string]$AuditPath, [Parameter(Mandatory)][string]$KitDir, $Config) {
    if (-not $StatePath) { $StatePath = Get-DefaultStatePath }
    $full = [IO.Path]::GetFullPath($StatePath); $dir = [IO.Path]::GetDirectoryName($full)
    New-Item -ItemType Directory -Force -Path $dir | Out-Null
    $me = Get-Process -Id $PID
    $cfg = $Config
    # the VirtualBox layer keeps its records next to the coordinator files
    $vbx = New-P9SVBoxContext -Dir $dir -VmNames @(Get-P9SVmNames)
    Set-P9SVBoxDefault $vbx
    [pscustomobject]@{
        StatePath = $full; SessionPath = Join-Path $dir 'coordinator-session.json'; LockPath = Join-Path $dir 'coordinator.lock'
        StopRequestPath = Join-Path $dir 'coordinator-stop.request'; AuditPath = $(if ($AuditPath) { $AuditPath } else { Join-Path $dir 'coordinator-audit.jsonl' })
        ThresholdsPath = Join-Path $KitDir 'paging-thresholds.json'
        ReadState = { param($p) Read-StateFile -Path $p }; NowQpc = { [Diagnostics.Stopwatch]::GetTimestamp() }; QpcFrequency = [Diagnostics.Stopwatch]::Frequency; IsMonitorAlive = $null
        ReadVmState = { param($vm) Get-P9SVmStateReal $vm }
        ReadVmConfig = { param($vm) Get-P9SVmConfigReal $vm }
        VBox = $vbx
        Dispatch = (Get-P9SRealDispatch $cfg)
        Sleep = { param($ms) Start-Sleep -Milliseconds $ms }; Pid = $PID; ProcessStartTicks = $me.StartTime.ToUniversalTime().Ticks
        EmergencyLockWaitMs = 30000; CalibrationHostChecks = { Get-P9SCalibrationHostChecks $cfg }.GetNewClosure()
        ObservationPath = Join-Path $dir 'vm-observations.jsonl'; LedgerPath = Join-Path $dir 'acpi-validation-ledger.json'
        UtcNow = { [DateTime]::UtcNow }; AcpiValidationLive = [bool](Get-P9SAcpiValidationCapability).live_enabled
        # running VirtualBox VMs of the whole host, from the Windows process
        # list (the names the monitor counts); no VirtualBox call
        CountVmProcesses = { @(Get-Process -Name 'VirtualBoxVM', 'VBoxHeadless' -ErrorAction SilentlyContinue).Count }
        Config = $cfg; ConfigError = $(if ($cfg -and $cfg.present -and -not $cfg.ok) { 'CONFIG_INVALID' } else { $null })
    }
}
