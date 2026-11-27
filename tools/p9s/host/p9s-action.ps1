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
#   pwsh -NoProfile -File p9s-action.ps1 -Action state.status
#   pwsh -NoProfile -File p9s-action.ps1 -Action state.init [-Supersede <installation id>] [-DryRun]
#   pwsh -NoProfile -File p9s-action.ps1 -Action state.migrate [-Restart <operation id>] [-DryRun]
# state.status only reads (p9s-state.ps1): it prints the status of the
# persistent state folder of later versions, creates nothing, writes no
# audit record and changes nothing any other action does.
# state.init (p9s-state-init.ps1) creates that folder explicitly, with
# every configured VM off; no other action ever does. state.migrate
# (p9s-state-migrate.ps1) creates it from the coordinator folder below,
# which it copies and never changes but for one marker file. The
# coordinator of this version keeps using its folder below either way; a
# new installation voids every ACPI validation authorization issued before
# it.
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
    [switch]$DryRun, [string]$StatePath, [string]$AuditPath, [string]$ConfigPath, [string]$Supersede, [string]$Restart
)
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'hostmon-lib.ps1')
. (Join-Path $PSScriptRoot 'coordinator-lib.ps1')
$inv = Test-P9SSupportedInvocation
if (-not $inv.ok) {
    "refused: $($inv.reason) ($($inv.missing) not visible to the toolkit's closures). Start the coordinator with: pwsh -NoProfile -File p9s-action.ps1 -Action <name> ... Nothing was done."
    exit 2
}
. (Join-Path $PSScriptRoot 'p9s-state.ps1')
. (Join-Path $PSScriptRoot 'p9s-state-init.ps1')
. (Join-Path $PSScriptRoot 'p9s-state-migrate.ps1')
if ($Action -ceq 'state.status') {
    $extra = @($PSBoundParameters.Keys | Where-Object { $_ -notin 'Action', 'StatePath' })
    if ($extra.Count) { "refused: state.status takes no parameter but -StatePath (given: $($extra -join ', ')). Nothing was read or written."; 'outcome: REFUSED; STATE_STATUS_PARAMETER'; exit 1 }
    try { $sctx = New-P9SStateContext -AskedStatePath $StatePath; $st = Get-P9SStateStatus $sctx; $lines = @(Format-P9SStateStatus $st $sctx) }
    catch { "the state status was not computed: $($_.Exception.Message)"; 'outcome: FAILED'; exit 2 }
    $lines
    'outcome: DONE'
    exit 0
}
if ($Action -ceq 'state.init') {
    $extra = @($PSBoundParameters.Keys | Where-Object { $_ -notin 'Action', 'Supersede', 'DryRun', 'ConfigPath' })
    if ($extra.Count) { "refused: state.init takes only -Supersede <installation id>, -DryRun and -ConfigPath (given: $($extra -join ', ')). Nothing was read or written."; 'outcome: REFUSED; STATE_INIT_PARAMETER'; exit 1 }
    $cfg = Read-P9SConfig -Path (Get-P9SConfigPath -ConfigPath $ConfigPath -ToolDir $PSScriptRoot)
    if ($cfg.present -and -not $cfg.ok) { "refused: configuration $($cfg.path) is invalid ($($cfg.errors -join ', ')): the VMs that must be off are not known. Nothing was written."; 'outcome: REFUSED; CONFIG_INVALID'; exit 1 }
    Set-P9SVmNames $cfg.vm_names
    try {
        $ia = @{ I = (New-P9SStateInitRealContext (New-P9SStateContext)); DryRun = [bool]$DryRun }
        if ($PSBoundParameters.ContainsKey('Supersede')) { $ia.Supersede = $Supersede }
        $r = Invoke-P9SStateInit @ia
    } catch { "state.init stopped: $($_.Exception.Message)"; 'outcome: FAILED'; exit 2 }
    foreach ($l in $r.lines) { $l }
    "outcome: $($r.outcome)$(if ($r.reasons.Count) { '; ' + ($r.reasons -join ', ') })"
    if ($r.outcome -in 'DONE', 'DRYRUN') { exit 0 } elseif ($r.outcome -eq 'FAILED') { exit 2 } else { exit 1 }
}
if ($Action -ceq 'state.migrate') {
    $extra = @($PSBoundParameters.Keys | Where-Object { $_ -notin 'Action', 'Restart', 'DryRun', 'ConfigPath' })
    if ($extra.Count) { "refused: state.migrate takes only -Restart <operation id>, -DryRun and -ConfigPath (given: $($extra -join ', ')). Nothing was read or written."; 'outcome: REFUSED; STATE_MIGRATE_PARAMETER'; exit 1 }
    $cfg = Read-P9SConfig -Path (Get-P9SConfigPath -ConfigPath $ConfigPath -ToolDir $PSScriptRoot)
    if ($cfg.present -and -not $cfg.ok) { "refused: configuration $($cfg.path) is invalid ($($cfg.errors -join ', ')): the VMs that must be off are not known. Nothing was written."; 'outcome: REFUSED; CONFIG_INVALID'; exit 1 }
    Set-P9SVmNames $cfg.vm_names
    try {
        $ma = @{ I = (New-P9SStateMigrateRealContext (New-P9SStateContext)); DryRun = [bool]$DryRun }
        if ($PSBoundParameters.ContainsKey('Restart')) { $ma.Restart = $Restart }
        $r = Invoke-P9SStateMigrate @ma
    } catch { "state.migrate stopped: $($_.Exception.Message)"; 'outcome: FAILED'; exit 2 }
    foreach ($l in $r.lines) { $l }
    if ($r.notes.Count) { "notes: $($r.notes -join ', ')" }
    "outcome: $($r.outcome)$(if ($r.reasons.Count) { '; ' + ($r.reasons -join ', ') })"
    if ($r.outcome -in 'DONE', 'DRYRUN') { exit 0 } elseif ($r.outcome -eq 'FAILED') { exit 2 } else { exit 1 }
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
