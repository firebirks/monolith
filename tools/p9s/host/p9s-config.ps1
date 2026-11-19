#Requires -Version 7.0
# P9-S host toolkit: machine-local configuration and the local secret store.
#
# The configuration file (p9s-local.json next to the scripts, or the path in
# P9S_HOST_CONFIG, or -ConfigPath; never committed, see
# p9s-local.example.json) names the two laboratory VMs, the external disk that
# holds them, the secret store and the evidence folder. Without it the VM
# names default to P9S-D12 and P9S-U24, no secret can be read and the monitor
# cannot confirm the external disk (its state stays UNKNOWN).
#
# Secrets are read only from the configured store, only when the store and the
# file are readable by nobody but the current account, SYSTEM and
# Administrators, never from a hidden (dot) folder, the temporary folder or
# application data, and they are never printed, logged, put in an error
# message or passed on a command line. Errors are codes only.

function Get-P9SDefaultVmNames { 'P9S-D12', 'P9S-U24' }
function Get-P9SConfigPath {
    param([string]$ConfigPath, [Parameter(Mandatory)][string]$ToolDir)
    if ($ConfigPath) { return [IO.Path]::GetFullPath($ConfigPath) }
    if ($env:P9S_HOST_CONFIG) { return [IO.Path]::GetFullPath($env:P9S_HOST_CONFIG) }
    Join-Path $ToolDir 'p9s-local.json'
}
function Test-P9SAbsolutePath($p) { ($p -is [string]) -and $p -ne '' -and [IO.Path]::IsPathFullyQualified($p) }
function Read-P9SConfig {
    param([Parameter(Mandatory)][string]$Path)
    $errors = [System.Collections.Generic.List[string]]::new()
    $o = $null
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) { $errors.Add('CONFIG_MISSING') }
    else { try { $o = Get-Content -Raw -LiteralPath $Path | ConvertFrom-Json -ErrorAction Stop } catch { $errors.Add('CONFIG_INVALID_JSON') } }
    if ($o -and $o.schema -ne 'p9s-host-config/1') { $errors.Add('CONFIG_SCHEMA_UNSUPPORTED') }
    $deb = $null; $ubu = $null; $ext = $null; $refs = [ordered]@{ debian = $null; ubuntu = $null }
    if ($o -and -not $errors.Count) {
        $deb = [string]$o.vms.debian; $ubu = [string]$o.vms.ubuntu
        $re = '^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$'
        if ($deb -cnotmatch $re -or $ubu -cnotmatch $re -or $deb -eq $ubu) { $errors.Add('CONFIG_VM_NAMES') }
        $e = $o.external_disk
        if (-not $e -or [string]$e.drive_letter -cnotmatch '^[A-Z]$' -or [string]::IsNullOrWhiteSpace([string]$e.model) -or [string]$e.model -match '[\x00-\x1f]' -or ([string]$e.model).Length -gt 100) { $errors.Add('CONFIG_EXTERNAL_DISK') }
        elseif (-not (Test-P9SAbsolutePath $e.probe_path)) { $errors.Add('CONFIG_PATH_NOT_ABSOLUTE:external_disk.probe_path') }
        else { $ext = [pscustomobject]@{ drive_letter = [string]$e.drive_letter; model = [string]$e.model; probe_path = [string]$e.probe_path } }
        foreach ($k in 'secrets_dir', 'evidence_dir') { if ($null -ne $o.$k -and -not (Test-P9SAbsolutePath $o.$k)) { $errors.Add("CONFIG_PATH_NOT_ABSOLUTE:$k") } }
        foreach ($k in 'debian', 'ubuntu') {
            $v = if ($o.vm_config_reference) { $o.vm_config_reference.$k } else { $null }
            if ($null -ne $v) { if (Test-P9SAbsolutePath $v) { $refs[$k] = [string]$v } else { $errors.Add("CONFIG_PATH_NOT_ABSOLUTE:vm_config_reference.$k") } }
        }
    }
    $ok = $errors.Count -eq 0
    $names = if ($ok) { @($deb, $ubu) } else { @(Get-P9SDefaultVmNames) }
    [pscustomobject]@{
        ok = $ok; errors = @($errors); path = $Path; present = (Test-Path -LiteralPath $Path -PathType Leaf)
        vm_names = $names; vms = [pscustomobject]@{ debian = $names[0]; ubuntu = $names[1] }
        external_disk = $(if ($ok) { $ext } else { $null })
        secrets_dir = $(if ($ok -and $o.secrets_dir) { [string]$o.secrets_dir } else { $null })
        evidence_dir = $(if ($ok -and $o.evidence_dir) { [string]$o.evidence_dir } else { $null })
        vm_config_reference = $(if ($ok) { [pscustomobject]$refs } else { [pscustomobject]@{ debian = $null; ubuntu = $null } })
    }
}

# ------------------------------------------------------------- secret store
function ConvertTo-P9SSid($Ref) {
    if ($Ref -is [Security.Principal.SecurityIdentifier]) { return $Ref.Value }
    $s = [string]$Ref
    if ($s -match '^S-1-[0-9-]+$') { return $s }
    try { return ([Security.Principal.NTAccount]$s).Translate([Security.Principal.SecurityIdentifier]).Value } catch { return 'UNRESOLVED' }
}
# Readable only by the current account, SYSTEM and Administrators; the store
# folder must not inherit permissions from its parent.
function Test-P9SSecretStoreAcl {
    param([Parameter(Mandatory)][string]$Path, [scriptblock]$GetAcl, [switch]$File)
    $reasons = [System.Collections.Generic.List[string]]::new()
    $acl = if ($GetAcl) { & $GetAcl $Path } else { Get-Acl -LiteralPath $Path }
    $allowed = @([Security.Principal.WindowsIdentity]::GetCurrent().User.Value, 'S-1-5-18', 'S-1-5-32-544')
    $owner = ConvertTo-P9SSid $acl.Owner
    if ($owner -notin $allowed) { $reasons.Add("SECRET_STORE_OWNER:$owner") }
    if (-not $File -and -not $acl.AreAccessRulesProtected) { $reasons.Add('SECRET_STORE_INHERITS_ACL') }
    foreach ($r in @($acl.Access)) {
        if ([string]$r.AccessControlType -ne 'Allow') { continue }
        $rights = [long]([Security.AccessControl.FileSystemRights]$r.FileSystemRights)
        if (-not ($rights -band (0x1 -bor 0x10000000 -bor 0x80000000))) { continue }
        $sid = ConvertTo-P9SSid $r.IdentityReference
        if ($sid -notin $allowed) { $reasons.Add("SECRET_STORE_EXPOSED:$sid") }
    }
    [pscustomobject]@{ ok = ($reasons.Count -eq 0); reasons = @($reasons) }
}
function Get-P9SDefaultForbiddenRoots { @([IO.Path]::GetTempPath(), $env:APPDATA, $env:LOCALAPPDATA) | Where-Object { $_ } }
# Returns the secret (the last line that is not empty and not a # comment) or
# throws one of: SECRET_NAME_INVALID, SECRET_STORE_NOT_CONFIGURED,
# SECRET_STORE_FORBIDDEN_LOCATION, SECRET_STORE_MISSING, SECRET_STORE_EXPOSED,
# SECRET_MISSING, SECRET_FILE_REJECTED. The value is never in a message.
function Read-P9SSecret {
    param($Config, [string]$Name, [scriptblock]$GetAcl, [string[]]$ForbiddenRoots = @(Get-P9SDefaultForbiddenRoots))
    if ($Name -cnotmatch '^[A-Za-z0-9][A-Za-z0-9-]{0,63}$') { throw 'SECRET_NAME_INVALID' }
    $dir = if ($Config) { [string]$Config.secrets_dir } else { '' }
    if (-not (Test-P9SAbsolutePath $dir)) { throw 'SECRET_STORE_NOT_CONFIGURED' }
    $full = [IO.Path]::GetFullPath($dir).TrimEnd('\')
    if (@($full.Split('\') | Where-Object { $_.StartsWith('.') }).Count) { throw 'SECRET_STORE_FORBIDDEN_LOCATION' }
    foreach ($root in @($ForbiddenRoots)) {
        if (-not $root) { continue }
        $r = [IO.Path]::GetFullPath($root).TrimEnd('\')
        if ($full -eq $r -or $full.StartsWith($r + '\', [StringComparison]::OrdinalIgnoreCase)) { throw 'SECRET_STORE_FORBIDDEN_LOCATION' }
    }
    if (-not (Test-Path -LiteralPath $full -PathType Container)) { throw 'SECRET_STORE_MISSING' }
    if (-not (Test-P9SSecretStoreAcl -Path $full -GetAcl $GetAcl).ok) { throw 'SECRET_STORE_EXPOSED' }
    $file = Join-Path $full "$Name.txt"
    if (-not (Test-Path -LiteralPath $file -PathType Leaf)) { throw 'SECRET_MISSING' }
    $item = Get-Item -LiteralPath $file -Force
    if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -or $item.Length -gt 4096) { throw 'SECRET_FILE_REJECTED' }
    if (-not (Test-P9SSecretStoreAcl -Path $file -GetAcl $GetAcl -File).ok) { throw 'SECRET_STORE_EXPOSED' }
    $lines = @(Get-Content -LiteralPath $file | Where-Object { $_ -and $_ -notmatch '^\s*#' })
    if ($lines.Count -lt 1) { throw 'SECRET_MISSING' }
    $lines[-1]
}
