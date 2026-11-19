#Requires -Version 7.0
# P9-S host gate check: reads the state file of host-monitor.ps1 and
# prints one line, HOST GATE OK or HOST GATE REFUSED with every reason.
# Exit 0 only when the monitor is running, its state is fresh (at most
# 12 s old on the monotonic clock), the status is READY, the 300-s gate is
# complete, commit is valid and below 90 % and, unless -PagingStop
# NotRequired is given for a test whose brief does not require it, the
# paging stop rule is approved and enforced. Anything else (no file,
# invalid JSON, other schema, stale, dead or unknown monitor, any other
# status) exits 1. Read only.
#   pwsh -NoProfile -File assert-host-safe.ps1 -Step "<what is about to happen>" [-RunId <id>] [-Log <file>]
param(
    [string]$StatePath,
    [string]$RunId,
    [ValidateSet('Required', 'NotRequired')][string]$PagingStop = 'Required',
    [string]$Step = 'unnamed step',
    [string]$Log
)
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'hostmon-lib.ps1')
$g = Invoke-HostGate -StatePath $StatePath -RunId $RunId -PagingStop $PagingStop -Step $Step
$line = Format-HostGate $g
if ($Log) { [IO.File]::AppendAllText($Log, [DateTime]::UtcNow.ToString('HH:mm:ss') + ' ' + $line + "`r`n", [Text.Encoding]::ASCII) }
$line
if ($g.ok) { exit 0 } else { exit 1 }
