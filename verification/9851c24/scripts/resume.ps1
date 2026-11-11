# The mutation baseline of Phases 1 to 8 alone, after the chain of
# master.ps1 ended: final-mutation.sh in WSL on the commit named in
# wsl\mutation-go. Logs to baseline.log.
# Usage: pwsh -File baseline.ps1 -Commit <sha>
param([Parameter(Mandatory = $true)][string]$Commit)
$V = $PSScriptRoot
$log = Join-Path $V 'baseline.log'
$wslV = '<work>/verify8'
function Note($text) { "$(Get-Date -Format s) $text" | Out-File $log -Append -Encoding ascii }
Note "mutation baseline of $Commit resumed"
wsl.exe -d Debian -- bash "$wslV/wsl/resume-mutation.sh" $Commit 2 *>> $log
Note "mutation baseline ended"
