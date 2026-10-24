# Runs the targeted re-verification of one candidate: the Linux container
# and the Windows checks side by side, then, with both ended and nothing
# else running, the selected mutation faults in WSL.
# Usage: pwsh -File chain-targeted.ps1 -Commit <sha>
param([Parameter(Mandatory = $true)][string]$Commit)
$V = $PSScriptRoot
$log = Join-Path $V 'chain.log'
$repo = '<winhome>\VSCodeProjects\Monolith'
$full = git -C $repo rev-parse $Commit
"start $full $(Get-Date -Format s)" | Out-File $log -Append
# The container: the export of the commit, a volume of its own, owned by
# the user the image runs as.
$C = Join-Path $V 'container'
$src = Join-Path $C 'src'
$out = Join-Path $C 'out-container'
foreach ($d in $src, $out) { if (Test-Path $d) { Remove-Item $d -Recurse -Force }; New-Item -ItemType Directory -Path $d | Out-Null }
$tar = Join-Path $C 'candidate.tar'
git -C $repo archive --format=tar -o $tar $full
tar -xf $tar -C $src
Remove-Item $tar
Set-Content -Path (Join-Path $src 'COMMIT') -Value $full -Encoding ascii -NoNewline
docker rm -f monolith-targeted 2>$null | Out-Null
docker volume create monolith-vol-targeted *>> $log
docker run --rm -u root -v monolith-vol-targeted:/vol monolith-verify:phase7 chown 1000:1000 /vol *>> $log
docker run -d --name monolith-targeted -v "${src}:/src:ro" -v "${out}:/out" -v "${C}:/scripts:ro" -v monolith-vol-targeted:/vol `
    monolith-verify:phase7 bash /scripts/in-container.sh bash /scripts/stage-targeted.sh *>> $log
"container started $(Get-Date -Format s)" | Out-File $log -Append
# Windows, while the container runs.
pwsh -NoProfile -File (Join-Path $V 'windows-targeted.ps1') -Commit $full *>> $log
"windows ended $(Get-Date -Format s)" | Out-File $log -Append
docker wait monolith-targeted *>> $log
docker logs monolith-targeted *> (Join-Path $C 'container.log')
"container ended $(Get-Date -Format s)" | Out-File $log -Append
# The mutation, alone.
$w = '/mnt/c' + ($V.Substring(2) -replace '\', '/') + '/wsl/targeted-mutation.sh'
wsl.exe -d Debian -- bash -c "tr -d '\r' < '$w' > ~/targeted-mutation.sh && chmod +x ~/targeted-mutation.sh" *>> $log
wsl.exe -d Debian -- bash -lc "~/targeted-mutation.sh $full 4" *>> $log
"mutation ended $(Get-Date -Format s)" | Out-File $log -Append
