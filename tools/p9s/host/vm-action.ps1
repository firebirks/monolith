#Requires -Version 7.0
# P9-S compatibility shim for an earlier VM-action command: the request goes
# to the coordinator (coordinator-lib.ps1) instead of VBoxManage.
#   -Action Start                  -> vm.start
#   -Action Keys -Scancodes "..."  -> guest.key, only when the scan codes are
#                                     exactly one named key of the allowlist
#                                     (Get-P9SKeyMap); anything else is refused
# A refusal throws the coordinator's line. -DryRun touches no VM.
param([Parameter(Mandatory)][string]$Vm,
      [Parameter(Mandatory)][ValidateSet('Start', 'Keys')][string]$Action,
      [string]$Scancodes, [string]$StatePath, [string]$ConfigPath, [switch]$DryRun)
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'hostmon-lib.ps1')
. (Join-Path $PSScriptRoot 'coordinator-lib.ps1')
# the status of the persistent state folder, which every action needs
. (Join-Path $PSScriptRoot 'p9s-state.ps1')
. (Join-Path $PSScriptRoot 'p9s-state-init.ps1')
$cfg = Read-P9SConfig -Path (Get-P9SConfigPath -ConfigPath $ConfigPath -ToolDir $PSScriptRoot)
Set-P9SVmNames $cfg.vm_names
if ($Action -eq 'Start') { $req = @{ Action = 'vm.start'; Vm = $Vm } }
else {
    $norm = (([string]$Scancodes).Trim().ToLowerInvariant() -split '\s+') -join ' '
    $name = $null
    foreach ($e in (Get-P9SKeyMap).GetEnumerator()) { if ((($e.Value | ForEach-Object { '{0:x2}' -f $_ }) -join ' ') -eq $norm) { $name = $e.Key } }
    if (-not $name) { throw "scancodes '$Scancodes' are not one named key of the coordinator allowlist; nothing sent" }
    $req = @{ Action = 'guest.key'; Vm = $Vm; Key = $name }
}
if ($DryRun) { $req.DryRun = $true }
$r = Invoke-P9SAction -Ctx (New-P9SRealContext -StatePath $StatePath -KitDir $PSScriptRoot -Config $cfg) -Request $req
if ($r.outcome -notin 'DISPATCHED', 'DONE', 'DRYRUN') { throw ($r.lines -join ' ') }
$r.lines
