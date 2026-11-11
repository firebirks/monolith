# Starts the container stage of the Phase 8 final verification on one
# frozen commit, in the image of the Phase 7 verification (no new image is
# built): exports the commit and the tag phase-7-complete without .git,
# marks each with its hash, and runs stage-container.sh in a container left
# to run. Its output goes to out-container.
# Usage: pwsh -File start-container.ps1 -Commit <sha> [-Cycles <n>]
param([Parameter(Mandatory = $true)][string]$Commit, [int]$Cycles = 3)
$ErrorActionPreference = 'Stop'
$V = $PSScriptRoot
$repo = '<repo>'
$log = Join-Path $V 'launch.log'
$full = git -C $repo rev-parse $Commit
$seven = git -C $repo rev-parse 'phase-7-complete^{commit}'
$src = Join-Path $V 'src'
$src7 = Join-Path $V 'src7'
$out = Join-Path $V 'out-container'
foreach ($d in $src, $src7, $out) { if (Test-Path $d) { Remove-Item $d -Recurse -Force }; New-Item -ItemType Directory -Path $d | Out-Null }
foreach ($pair in @(@($full, $src), @($seven, $src7))) {
    $tar = Join-Path $V 'tree.tar'
    git -C $repo archive --format=tar -o $tar $pair[0]
    tar -xf $tar -C $pair[1]
    Remove-Item $tar
    Set-Content -Path (Join-Path $pair[1] 'COMMIT') -Value $pair[0] -Encoding ascii -NoNewline
}
"exported $full and phase-7-complete $seven $(Get-Date -Format s)" | Out-File $log -Append
docker rm -f monolith-final8 2>$null | Out-Null
docker run -d --name monolith-final8 --cap-add SYS_PTRACE --security-opt seccomp=unconfined `
    -e RESTART_CYCLES=$Cycles `
    -v "${src}:/src:ro" -v "${src7}:/src7:ro" -v "${out}:/out" -v "${V}:/scripts:ro" -v monolith-vol8:/vol `
    monolith-verify:phase7 bash /scripts/in-container.sh bash /scripts/stage-container.sh *>> $log
"container started $(Get-Date -Format s)" | Out-File $log -Append
