#Requires -Version 7.0
# P9-S action coordinator: the one supported entry point for P9-S
# automation that changes laboratory state (see coordinator-lib.ps1 and
# README.md). Prints what it did or why it refused, then "outcome: ...".
# Exit codes: 0 DISPATCHED, DONE or DRYRUN; 1 REFUSED, REJECTED, TIMEOUT,
# OFF_ABNORMAL or STOPPED_AT_CHECKPOINT; 2 FAILED.
#   pwsh -NoProfile -File p9s-action.ps1 -Action session.open -Kind Test
#   pwsh -NoProfile -File p9s-action.ps1 -Action guest.key -Vm P9S-U24 -Key CtrlQ
#   pwsh -NoProfile -File p9s-action.ps1 -Action emergency.acpi-shutdown -Vm P9S-D12
#   pwsh -NoProfile -File p9s-action.ps1 -Action calib.dry-run -Scenario nominal
#   pwsh -NoProfile -File p9s-action.ps1 -Action vbox.status
#   pwsh -NoProfile -File p9s-action.ps1 -Action acpi.preflight [-Authorization <file>]
# Start it with pwsh -File (as above). Started with & from a prompt or
# another script it refuses with exit 2 and does nothing: its closures could
# not find the toolkit's functions there.
# Machine-local settings: -ConfigPath, else P9S_HOST_CONFIG, else
# p9s-local.json next to this script (see p9s-local.example.json).
# Coordinator files live next to the monitor state file (default
# %TEMP%\p9s-hostmon\): coordinator-session.json, coordinator.lock,
# coordinator-stop.request, coordinator-audit.jsonl, vm-observations.jsonl,
# acpi-validation-ledger.json, and the VirtualBox layer's
# vbox-operation.json, vbox-quarantine.json, vbox-audit.jsonl.
param(
    [Parameter(Mandatory)][string]$Action,
    [string]$Vm, [string]$Text, [string]$Key, [string]$Name, [int]$Seconds, [string]$Plan, [string]$Kind,
    [string]$Scenario, [string]$MonitorRun, [string]$Reference, [string]$AcknowledgeAbort, [string]$Operation, [string]$Authorization,
    [switch]$DryRun, [string]$StatePath, [string]$AuditPath, [string]$ConfigPath
)
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'hostmon-lib.ps1')
. (Join-Path $PSScriptRoot 'coordinator-lib.ps1')
$inv = Test-P9SSupportedInvocation
if (-not $inv.ok) {
    "refused: $($inv.reason) ($($inv.missing) not visible to the toolkit's closures). Start the coordinator with: pwsh -NoProfile -File p9s-action.ps1 -Action <name> ... Nothing was done."
    exit 2
}
$cfg = Read-P9SConfig -Path (Get-P9SConfigPath -ConfigPath $ConfigPath -ToolDir $PSScriptRoot)
Set-P9SVmNames $cfg.vm_names
if ($cfg.present -and -not $cfg.ok) { "configuration $($cfg.path) is invalid: $($cfg.errors -join ', '); only emergency and read-only actions are available" }
$req = @{}
foreach ($k in $PSBoundParameters.Keys) {
    if ($k -in 'StatePath', 'AuditPath', 'ConfigPath') { continue }
    if ($k -eq 'DryRun') { $req.DryRun = [bool]$DryRun; continue }
    $req[$k] = $PSBoundParameters[$k]
}
$ctx = New-P9SRealContext -StatePath $StatePath -AuditPath $AuditPath -KitDir $PSScriptRoot -Config $cfg
$r = Invoke-P9SAction -Ctx $ctx -Request $req
foreach ($l in $r.lines) { $l }
"outcome: $($r.outcome)$(if ($r.reasons.Count) { '; ' + ($r.reasons -join ', ') })"
if ($r.outcome -in 'DISPATCHED', 'DONE', 'DRYRUN') { exit 0 } elseif ($r.outcome -eq 'FAILED') { exit 2 } else { exit 1 }
