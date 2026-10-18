# Runs a script in WSL Debian and waits for it, so that a wsl.exe client
# stays alive for the whole run. Started detached:
#   Start-Process pwsh -ArgumentList '-NoProfile','-File',<this>,<script>,<args...> -WindowStyle Hidden
param([Parameter(Mandatory = $true)][string]$Script, [Parameter(ValueFromRemainingArguments = $true)][string[]]$Rest)
$log = Join-Path $PSScriptRoot 'launch-wsl.log'
"$(Get-Date -Format s) start $Script $($Rest -join ' ')" | Out-File $log -Append -Encoding ascii
& wsl.exe -d Debian -- bash $Script @Rest
"$(Get-Date -Format s) end $Script exit $LASTEXITCODE" | Out-File $log -Append -Encoding ascii
