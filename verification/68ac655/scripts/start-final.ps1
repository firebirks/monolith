# Starts the final verification of Phase 6 on one frozen commit:
# exports the commit, starts the container stage, and leaves a detached
# chain that waits for it, stops Docker Desktop, then runs the WSL stage
# and the mutation lists. The Windows stage is started apart
# (windows-stage.ps1), since it competes for the same cores.
# Usage: pwsh -File start-final.ps1 -Commit <sha>
param([Parameter(Mandatory = $true)][string]$Commit, [switch]$NoChain)
$ErrorActionPreference = 'Stop'
$V = $PSScriptRoot
$repo = '<winhome>\VSCodeProjects\Monolith'
$full = git -C $repo rev-parse $Commit
$src = Join-Path $V 'src-final'
$out = Join-Path $V 'out-final'
foreach ($d in $src, $out) { if (Test-Path $d) { Remove-Item $d -Recurse -Force } ; New-Item -ItemType Directory -Path $d | Out-Null }
# The tree of the commit, without .git, and its name.
$tar = Join-Path $V 'final.tar'
git -C $repo archive --format=tar -o $tar $full
tar -xf $tar -C $src
Remove-Item $tar
Set-Content -Path (Join-Path $src 'COMMIT') -Value $full -Encoding ascii -NoNewline
"exported $full $(Get-Date -Format s)" | Out-File (Join-Path $V 'final.log') -Append
docker rm -f monolith-final6 2>$null | Out-Null
docker run -d --name monolith-final6 --cap-add SYS_PTRACE --security-opt seccomp=unconfined `
    -v "${src}:/src:ro" -v "${out}:/out" -v "${V}:/scripts:ro" -v monolith-vol:/vol `
    monolith-verify:phase5 bash /scripts/in-container.sh bash /scripts/stage-container.sh | Out-Null
"container started $(Get-Date -Format s)" | Out-File (Join-Path $V 'final.log') -Append
if ($NoChain) {
    # The WSL stage and the mutation lists already run: only wait for the
    # container, keep its log and stop Docker Desktop.
    Start-Process pwsh -ArgumentList '-NoProfile', '-File', (Join-Path $V 'container-wait.ps1') -WindowStyle Hidden
} else {
    Start-Process pwsh -ArgumentList '-NoProfile', '-File', (Join-Path $V 'chain6.ps1'), $full -WindowStyle Hidden
}
