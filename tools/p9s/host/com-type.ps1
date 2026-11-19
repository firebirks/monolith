#Requires -Version 7.0
# P9-S compatibility shim for an earlier typing command: it types nothing
# itself and hands the request to the coordinator (coordinator-lib.ps1):
#   -Text <synthetic text>  -> guest.type (READY state, active test session,
#                              checks between chunks, no Enter)
#   -Shutdown               -> retired: a guest is no longer shut down by
#                              typing into it; use p9s-action.ps1 -Action
#                              emergency.acpi-shutdown
# A refusal throws the coordinator's line (HOST GATE REFUSED / ACTION
# REJECTED ...). -DryRun prints what would happen and touches no VM.
param([Parameter(Mandatory)][string]$Vm,
      [Parameter(Mandatory)][string]$Text,
      [string]$StatePath, [string]$ConfigPath, [switch]$Shutdown, [switch]$DryRun)
$ErrorActionPreference = 'Stop'
if ($Shutdown) { throw 'retired: shutting a guest down by typing into it is no longer supported; use p9s-action.ps1 -Action emergency.acpi-shutdown' }
. (Join-Path $PSScriptRoot 'hostmon-lib.ps1')
. (Join-Path $PSScriptRoot 'coordinator-lib.ps1')
$cfg = Read-P9SConfig -Path (Get-P9SConfigPath -ConfigPath $ConfigPath -ToolDir $PSScriptRoot)
Set-P9SVmNames $cfg.vm_names
$req = @{ Action = 'guest.type'; Vm = $Vm; Text = $Text }
if ($DryRun) { $req.DryRun = $true }
$r = Invoke-P9SAction -Ctx (New-P9SRealContext -StatePath $StatePath -KitDir $PSScriptRoot -Config $cfg) -Request $req
if ($r.outcome -notin 'DISPATCHED', 'DONE', 'DRYRUN') { throw ($r.lines -join ' ') }
$r.lines
