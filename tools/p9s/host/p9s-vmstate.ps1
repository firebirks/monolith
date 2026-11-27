#Requires -Version 7.0
# P9-S VM-state observer (library; dot-sourced by coordinator-lib.ps1).
# It answers one question: what is the last reliably observed state of one
# laboratory VM, and how old is that observation. The host resource
# monitor (host-monitor.ps1) answers another one, whether the Windows host
# is under unsafe resource pressure, and makes no VirtualBox call; the two
# answers never depend on each other.
#   - An observation is one state query made on request through the
#     context's ReadVmState. In the real context that is one VBoxManage
#     showvminfo through p9s-vbox.ps1: the host-wide VirtualBox lock, a
#     bounded wait, the quarantine, no retry. There is no background loop
#     and no cache: the coordinator observes when a session opens, before
#     and after a dispatch, on a status request and while it confirms a
#     shutdown (one query every 5 s or more, up to a deadline), never
#     because a resource sample is due.
#   - Anything other than a VirtualBox state name is 'unreadable' with
#     observed = $false (missing VBoxManage, a refused, failed or timed-out
#     query, a quarantine, a busy lock, another VM's name in the answer).
#     It is never taken for 'off', and a previous observation never stands
#     in for a current one.
#   - The time of an observation is the moment its query started: the
#     state it reports is at least that old. A decision that needs the
#     current state rejects an observation older than
#     Get-P9SVmObservationMaxAgeSeconds, or one from the future.
#   - The state the coordinator expects (session.expected_vm, set after a
#     dispatch) is an assumption; only an observation shows the VM state.
#   - Each observation is appended to vm-observations.jsonl next to the
#     coordinator files: VM, state, purpose, times and the VirtualBox
#     operation id; never command output.

function Get-P9SVmObservationMaxAgeSeconds { 10 }
function Get-P9SVmObservationInterval { 5 }
# Words that are no VM state, whatever a reader returns.
function Get-P9SNonObservations { 'unreadable', 'unknown', 'busy', 'quarantined', 'none' }
function Test-P9SObservedVmState($s) { ($s -is [string]) -and $s -cmatch '^[a-z]{2,40}$' -and $s -cnotin (Get-P9SNonObservations) }
function Get-P9SUtcNow($Ctx) { $f = Get-Prop $Ctx 'UtcNow'; if ($f) { [datetime](& $f) } else { [DateTime]::UtcNow } }

# One observation: a fresh query, its times, the journal entry.
function Get-P9SVmObservation($Ctx, [string]$Vm, [string]$Purpose) {
    $f = [double]$Ctx.QpcFrequency
    $utc = (Get-P9SUtcNow $Ctx).ToString('yyyy-MM-ddTHH:mm:ss.fffZ')
    $script:P9SLastVBoxResult = $null
    $q0 = [long](& $Ctx.NowQpc)
    $raw = $null
    try { $raw = & $Ctx.ReadVmState $Vm } catch { $raw = $null }
    $q1 = [long](& $Ctx.NowQpc)
    $lr = $script:P9SLastVBoxResult
    $raw = if ($raw -is [array]) { $null } else { $raw }
    $ok = Test-P9SObservedVmState $raw
    $o = [pscustomobject]@{
        schema = 'p9s-vm-observation/1'; vm = $Vm; state = $(if ($ok) { [string]$raw } else { 'unreadable' }); observed = $ok; purpose = $Purpose
        observed_qpc = $q0; completed_qpc = $q1; observed_utc = $utc; elapsed_ms = [math]::Round(1000.0 * ($q1 - $q0) / $f)
        operation_id = $(if ($lr) { $lr.operation_id } else { $null }); reason = $(if ($ok) { $null } elseif ($lr -and $lr.reason) { [string]$lr.reason } else { 'VM_STATE_UNREADABLE' })
    }
    Add-P9SVmObservationRecord $Ctx $o
    $o
}
# Whether an observation may decide something now: it must be an
# observation of this VM, not older than the limit, not from the future.
function Test-P9SVmObservation {
    param($Observation, [Parameter(Mandatory)][long]$NowQpc, [Parameter(Mandatory)][long]$QpcFrequency, [string]$Vm, [double]$MaxAgeSeconds = (Get-P9SVmObservationMaxAgeSeconds))
    $reasons = [System.Collections.Generic.List[string]]::new()
    $age = $null
    $name = if ($Vm) { $Vm } elseif ($Observation) { [string]$Observation.vm } else { '?' }
    if (-not $Observation -or $Observation.schema -ne 'p9s-vm-observation/1') { $reasons.Add("VM_STATE_UNOBSERVED:$name") }
    else {
        if ($Vm -and [string]$Observation.vm -cne $Vm) { $reasons.Add("VM_IDENTITY_MISMATCH:$name") }
        if ($Observation.observed -ne $true -or -not (Test-P9SObservedVmState $Observation.state)) { $reasons.Add("VM_STATE_UNOBSERVED:$name") }
        else {
            $age = ($NowQpc - [long]$Observation.observed_qpc) / [double]$QpcFrequency
            if ($age -lt 0) { $reasons.Add("VM_STATE_FROM_FUTURE:$name") }
            elseif ($age -gt $MaxAgeSeconds) { $reasons.Add("VM_STATE_STALE:$name") }
        }
    }
    [pscustomobject]@{ ok = ($reasons.Count -eq 0); reasons = @($reasons); age_s = $age; state = $(if ($Observation) { [string]$Observation.state } else { 'unreadable' }) }
}
function Format-P9SVmObservation($O) {
    if (-not $O.observed) { return "$($O.vm) unreadable (not observed: $($O.reason); never taken for off)" }
    "$($O.vm) $($O.state) (observed $($O.observed_utc), query $($O.elapsed_ms) ms)"
}
function Add-P9SVmObservationRecord($Ctx, $O) {
    $p = Get-Prop $Ctx 'ObservationPath'
    # an action outside READY writes nothing (coordinator-lib.ps1)
    if (-not $p -or (Get-Prop $Ctx 'StateQuiet')) { return }
    $e = [ordered]@{ schema = 'p9s-vm-observation/1'; utc = $O.observed_utc; pid = $PID; vm = $O.vm; state = $O.state; observed = $O.observed; purpose = $O.purpose
        elapsed_ms = $O.elapsed_ms; operation_id = $O.operation_id; reason = $O.reason }
    $bytes = [Text.UTF8Encoding]::new($false).GetBytes(($e | ConvertTo-Json -Compress) + "`n")
    for ($i = 0; $i -lt 40; $i++) {
        try {
            $fs = [IO.FileStream]::new($p, [IO.FileMode]::Append, [IO.FileAccess]::Write, [IO.FileShare]::ReadWrite)
            try { $fs.Write($bytes, 0, $bytes.Length) } finally { $fs.Dispose() }
            return
        } catch { Start-Sleep -Milliseconds 25 }
    }
    Write-Warning "P9-S coordinator: VM observation not recorded in $p"
}
# The journal as observations (utc, vm, state, observed); unreadable lines
# are skipped.
function Read-P9SVmObservations([string]$Path) {
    if (-not $Path -or -not (Test-Path -LiteralPath $Path)) { return @() }
    @(Get-Content -LiteralPath $Path | ForEach-Object { try { $x = $_ | ConvertFrom-Json -ErrorAction Stop; if ($x.schema -eq 'p9s-vm-observation/1') { $x } } catch {} })
}
