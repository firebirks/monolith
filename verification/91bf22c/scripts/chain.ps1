# Waits for the container part of the final verification, keeps its log,
# stops Docker Desktop to give its memory back, then runs the WSL part and
# the mutation lists. A line per step goes to chain.log next to this file.
$V = $PSScriptRoot
$log = Join-Path $V 'chain.log'
"started $(Get-Date -Format s)" | Out-File $log
$code = docker wait monolith-final
"container exit $code $(Get-Date -Format s)" | Out-File $log -Append
docker logs monolith-final *> (Join-Path $V 'out-final\container.log')
docker rm monolith-final | Out-Null
docker desktop stop *>> $log
"docker desktop stopped $(Get-Date -Format s)" | Out-File $log -Append
wsl.exe -d Debian -- bash <work>/verify/chain-wsl.sh
"wsl chain exit $LASTEXITCODE $(Get-Date -Format s)" | Out-File $log -Append
