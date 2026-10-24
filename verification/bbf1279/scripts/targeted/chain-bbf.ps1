# The targeted re-verification of a candidate after the test-only fix:
# the Linux container and the Windows checks side by side, then, with both
# ended and nothing else running, test B again and the selected mutation
# faults in WSL (~/reverify-bbf.sh, already in place).
# Usage: pwsh -File chain-bbf.ps1 -Commit <sha>
param([Parameter(Mandatory = $true)][string]$Commit)
$V = $PSScriptRoot
$log = Join-Path $V 'chain-bbf.log'
$repo = '<winhome>\VSCodeProjects\Monolith'
$full = git -C $repo rev-parse $Commit
"start $full $(Get-Date -Format s)" | Out-File $log -Append
docker desktop start *>> $log
"docker started $(Get-Date -Format s)" | Out-File $log -Append
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
pwsh -NoProfile -File (Join-Path $V 'windows-targeted.ps1') -Commit $full *>> $log
"windows ended $(Get-Date -Format s)" | Out-File $log -Append
docker wait monolith-targeted *>> $log
docker logs monolith-targeted *> (Join-Path $C 'container.log')
"container ended $(Get-Date -Format s)" | Out-File $log -Append
wsl.exe -d Debian -- bash -lc "tr -d '\r' < ~/reverify-bbf.sh > ~/reverify-bbf.run && bash ~/reverify-bbf.run $full 4" *>> $log
"wsl ended $(Get-Date -Format s)" | Out-File $log -Append
