# Starts the container stage of the Phase 7 final verification on one
# frozen commit: builds the image (the Phase 5 image and the stable of the
# day), exports the commit without .git, marks it with its hash, and runs
# stage-container.sh in a container. The container is left to run; its
# output goes to out-container.
# Usage: pwsh -File start-container.ps1 -Commit <sha>
param([Parameter(Mandatory = $true)][string]$Commit)
$ErrorActionPreference = 'Stop'
$V = $PSScriptRoot
$repo = '<winhome>\VSCodeProjects\Monolith'
$log = Join-Path $V 'launch.log'
$full = git -C $repo rev-parse $Commit
$src = Join-Path $V 'src'
$out = Join-Path $V 'out-container'
foreach ($d in $src, $out) { if (Test-Path $d) { Remove-Item $d -Recurse -Force }; New-Item -ItemType Directory -Path $d | Out-Null }
docker build -t monolith-verify:phase7 $V *>> $log
"image built $(Get-Date -Format s)" | Out-File $log -Append
$tar = Join-Path $V 'final.tar'
git -C $repo archive --format=tar -o $tar $full
tar -xf $tar -C $src
Remove-Item $tar
Set-Content -Path (Join-Path $src 'COMMIT') -Value $full -Encoding ascii -NoNewline
"exported $full $(Get-Date -Format s)" | Out-File $log -Append
docker rm -f monolith-final7 2>$null | Out-Null
docker run -d --name monolith-final7 --cap-add SYS_PTRACE --security-opt seccomp=unconfined `
    -v "${src}:/src:ro" -v "${out}:/out" -v "${V}:/scripts:ro" -v monolith-vol7:/vol `
    monolith-verify:phase7 bash /scripts/in-container.sh bash /scripts/stage-container.sh *>> $log
"container started $(Get-Date -Format s)" | Out-File $log -Append
