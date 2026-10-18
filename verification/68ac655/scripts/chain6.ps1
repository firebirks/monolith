# Waits for the container stage of the final verification, keeps its log,
# stops Docker Desktop to give its memory back, then runs the WSL stage and
# the mutation lists on the same commit. A line per step goes to
# final.log next to this file.
param([Parameter(Mandatory = $true)][string]$Commit)
$V = $PSScriptRoot
$log = Join-Path $V 'final.log'
$code = docker wait monolith-final6
"container exit $code $(Get-Date -Format s)" | Out-File $log -Append
docker logs monolith-final6 *> (Join-Path $V 'out-final\container.log')
docker rm monolith-final6 | Out-Null
docker desktop stop *>> $log
"docker desktop stopped $(Get-Date -Format s)" | Out-File $log -Append
wsl.exe -d Debian -- bash <work>/chain6-wsl.sh $Commit
"wsl chain exit $LASTEXITCODE $(Get-Date -Format s)" | Out-File $log -Append
