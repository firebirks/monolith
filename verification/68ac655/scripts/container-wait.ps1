# Waits for the container stage of the final verification and keeps its
# log. Stops Docker Desktop to give its memory back only if no other
# container runs: other work may use the same Docker. A line per step goes
# to final.log next to this file.
$V = $PSScriptRoot
$log = Join-Path $V 'final.log'
$code = docker wait monolith-final6
"container exit $code $(Get-Date -Format s)" | Out-File $log -Append
docker logs monolith-final6 *> (Join-Path $V 'out-final\container.log')
docker rm monolith-final6 | Out-Null
$others = @(docker ps -q)
if ($others.Count -eq 0) {
    docker desktop stop *>> $log
    "docker desktop stopped $(Get-Date -Format s)" | Out-File $log -Append
} else {
    "docker desktop left running: $($others.Count) other container(s) $(Get-Date -Format s)" | Out-File $log -Append
}
# The mutation lists wait for this file to go.
Remove-Item (Join-Path $V 'container.running') -ErrorAction SilentlyContinue
