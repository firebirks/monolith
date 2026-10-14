# Builds the Windows archive of the desktop client (packaging/README.md):
# the release binary, a README and the licences, in one zip that unpacks
# into a folder of its own. Run it from anywhere in the repository with
# PowerShell 7; it needs git and cargo. It writes to target\package\.
$ErrorActionPreference = 'Stop'

$root = git -C $PSScriptRoot rev-parse --show-toplevel
Set-Location $root
$version = (Select-String -Path Cargo.toml -Pattern '^version = "(.*)"$' |
    Select-Object -First 1).Matches.Groups[1].Value
$name = "monolith-desktop-$version-windows-x86_64"
$package = Join-Path $root 'target\package'
$out = Join-Path $package $name
$zip = Join-Path $package "$name.zip"
foreach ($path in @($out, $zip, "$zip.sha256")) {
    if (Test-Path $path) { Remove-Item $path -Recurse -Force }
}
New-Item -ItemType Directory -Path (Join-Path $out 'licenses') | Out-Null

# No symbols in the binary; nothing else differs from a release build.
$env:CARGO_PROFILE_RELEASE_STRIP = 'symbols'
cargo build --release --locked -p monolith-desktop
if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' }
Remove-Item Env:CARGO_PROFILE_RELEASE_STRIP

Copy-Item 'target\release\monolith-desktop.exe' $out
Copy-Item 'packaging\windows\README.txt' $out
Copy-Item 'LICENSE' $out
Copy-Item 'crates\monolith-ui-kit\assets\licenses\*' (Join-Path $out 'licenses')

Compress-Archive -Path $out -DestinationPath $zip
$hash = (Get-FileHash -Algorithm SHA256 $zip).Hash.ToLowerInvariant()
"$hash  $name.zip" | Set-Content -Path "$zip.sha256" -Encoding ascii
Get-Content "$zip.sha256"
