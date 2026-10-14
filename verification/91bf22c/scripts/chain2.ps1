# The second final verification: waits for the container part, keeps its
# log, stops Docker Desktop to give its memory back, then runs the WSL
# part and the mutation lists. A line per step goes to chain2.log.
$V = $PSScriptRoot
$log = Join-Path $V 'chain2.log'
"started $(Get-Date -Format s)" | Out-File $log
$code = docker wait monolith-final2
"container exit $code $(Get-Date -Format s)" | Out-File $log -Append
docker logs monolith-final2 *> (Join-Path $V 'out-final2\container.log')
docker rm monolith-final2 | Out-Null
docker desktop stop *>> $log
"docker desktop stopped $(Get-Date -Format s)" | Out-File $log -Append
wsl.exe -d Debian -- bash <work>/verify/chain-wsl.sh
"wsl chain exit $LASTEXITCODE $(Get-Date -Format s)" | Out-File $log -Append
