#Requires -Version 7.0
# P9-S VirtualBox invocation layer (library; dot-source hostmon-lib.ps1
# first). Every VirtualBox management operation of the toolkit goes through
# it, read-only ones included:
#   Invoke-P9SVBoxOperation  VBoxManage as a child process: vm.state and
#                            vm.config (showvminfo), vm.start, vm.acpi
#   Invoke-P9SVBoxInProcess  VirtualBox COM calls made in this process: the
#                            guest keyboard input
#   Invoke-P9SVBoxRecovery   the operator's explicit recovery of a
#                            quarantine
# Rules:
#   - one named mutex for the whole host (Get-P9SVBoxMutexName), whatever
#     the working folder, process, output folder or VM; held for the whole
#     operation and released only once the child has exited, or once a
#     quarantine is written and read back;
#   - bounded waits (Get-P9SVBoxLimits): for the lock, for the command, and
#     a confirmation window after a command timeout; no automatic retry;
#     no process is ever terminated (no VBoxManage, VBoxSVC or VM process);
#   - vbox-operation.json, next to the coordinator files, records what the
#     current owner is doing, so that a process that finds the mutex
#     abandoned can tell whether a command may still run;
#   - anything that cannot be resolved writes vbox-quarantine.json: every
#     later operation is refused, in this and in every other toolkit
#     process, until Invoke-P9SVBoxRecovery clears it at the operator's
#     request;
#   - the records and vbox-audit.jsonl hold operation names, VM names,
#     codes, times and process ids; never command lines, command output,
#     paths or secrets.
# In-process COM calls cannot be bounded in time: a hung call keeps its
# process and the mutex, and the other toolkit processes then time out on
# the lock and quarantine. The toolkit cannot serialize what runs outside
# it: VBoxManage typed by hand, the VirtualBox window, other programs, and
# other projects' VMs.

if (-not $script:P9SVBoxRetained) { $script:P9SVBoxRetained = @{} }

function Get-P9SVBoxMutexName { 'Global\P9S-VirtualBox-Management-v1' }
function Get-VBoxManagePath { Join-Path $env:ProgramFiles 'Oracle\VirtualBox\VBoxManage.exe' }
function Get-VBoxStartArgs([string]$Vm) { , @('startvm', $Vm, '--type', 'gui') }
function Get-VBoxAcpiArgs([string]$Vm) { , @('controlvm', $Vm, 'acpipowerbutton') }
# Defaults. A state query normally answers within seconds; startvm returns
# once the VM process runs, which takes longer on this host; acpipowerbutton
# only posts the event. The lock wait is longer than the longest command
# plus its confirmation window, so that a healthy holder always finishes,
# or quarantines, before a waiter gives up. The host resource monitor never
# takes the lock: it makes no VirtualBox call (host-monitor.ps1).
function Get-P9SVBoxLimits {
    [pscustomobject]@{
        exec_ms = [ordered]@{ 'vm.state' = 30000; 'vm.config' = 30000; 'vm.acpi' = 30000; 'vm.start' = 120000 }
        confirm_ms = 10000; output_ms = 5000; lock_wait_ms = 150000
    }
}
function Get-P9SVBoxOperations { 'vm.state', 'vm.config', 'vm.start', 'vm.acpi' }
# monitor.connect and monitor.vmstates (the monitor's COM reads up to toolkit
# 83c9ef8) are retired: the monitor no longer calls VirtualBox.
function Get-P9SVBoxInProcessOperations { 'guest.keys' }
function Get-P9SVBoxIdPattern { '^vbox-[0-9]{8}T[0-9]{9}Z-[0-9]{1,10}-[0-9a-f]{8}$' }
function Get-P9SVBoxArguments([string]$Operation, [string]$Vm) {
    switch -CaseSensitive ($Operation) {
        'vm.state' { return , @('showvminfo', $Vm, '--machinereadable') }
        'vm.config' { return , @('showvminfo', $Vm, '--machinereadable') }
        'vm.start' { return , (Get-VBoxStartArgs $Vm) }
        'vm.acpi' { return , (Get-VBoxAcpiArgs $Vm) }
    }
    throw 'VBOX_OPERATION_NOT_ALLOWED'
}

# ------------------------------------------------------------ context
# Tests replace MutexName, Dir, ExePath/PrefixArgs (a mock command),
# ProcessFactory, OnPhase and FailLatchWrite; the coordinator's command line
# offers none of them.
function New-P9SVBoxContext {
    param([Parameter(Mandatory)][string]$Dir, [string[]]$VmNames = @(), [string]$MutexName, [string]$ExePath, [string[]]$PrefixArgs = @(),
          [scriptblock]$ProcessFactory, [scriptblock]$OnPhase, [switch]$FailLatchWrite,
          [int]$LockWaitMs = 0, [hashtable]$ExecMs = @{}, [int]$ConfirmMs = 0, [int]$OutputMs = 0)
    $l = Get-P9SVBoxLimits
    $e = [ordered]@{}; foreach ($k in $l.exec_ms.Keys) { $e[$k] = $(if ($ExecMs.ContainsKey($k)) { [int]$ExecMs[$k] } else { [int]$l.exec_ms[$k] }) }
    [pscustomobject]@{
        MutexName = $(if ($MutexName) { $MutexName } else { Get-P9SVBoxMutexName }); Dir = [IO.Path]::GetFullPath($Dir); VmNames = @($VmNames)
        ExePath = $(if ($ExePath) { $ExePath } else { Get-VBoxManagePath }); PrefixArgs = @($PrefixArgs)
        ProcessFactory = $ProcessFactory; OnPhase = $OnPhase; FailLatchWrite = [bool]$FailLatchWrite
        LockWaitMs = $(if ($LockWaitMs -gt 0) { $LockWaitMs } else { [int]$l.lock_wait_ms }); ExecMs = $e
        ConfirmMs = $(if ($ConfirmMs -gt 0) { $ConfirmMs } else { [int]$l.confirm_ms }); OutputMs = $(if ($OutputMs -gt 0) { $OutputMs } else { [int]$l.output_ms })
    }
}
function Get-P9SVBoxPath($VBox, [string]$Name) {
    Join-Path $VBox.Dir $(switch ($Name) { 'record' { 'vbox-operation.json' } 'latch' { 'vbox-quarantine.json' } default { 'vbox-audit.jsonl' } })
}
function Set-P9SVBoxDefault($VBox) { $script:P9SVBoxDefault = $VBox }
function Get-P9SVBoxDefault {
    if (-not $script:P9SVBoxDefault) {
        $names = if (Get-Command -Name Get-P9SVmNames -ErrorAction SilentlyContinue) { @(Get-P9SVmNames) } else { @() }
        $script:P9SVBoxDefault = New-P9SVBoxContext -Dir (Split-Path -Parent (Get-DefaultStatePath)) -VmNames $names
    }
    $script:P9SVBoxDefault
}
function New-P9SVBoxOperationId { 'vbox-' + [DateTime]::UtcNow.ToString('yyyyMMddTHHmmssfffZ') + '-' + $PID + '-' + [guid]::NewGuid().ToString('N').Substring(0, 8) }
function New-P9SVBoxResult([string]$Operation, [string]$Vm) {
    [pscustomobject]@{ operation_id = $null; operation = $Operation; vm = $Vm; outcome = $null; reason = $null; exit_code = $null; elapsed_ms = 0; lock_wait_ms = 0
        abandoned = $false; latched = $false; retained = $false; child_pid = $null; lines = @(); diag = $null; note = $null; value = $null }
}

# ------------------------------------------------------------ process identity
# Start time of a running process (UTC ticks), 0 when no such process runs,
# -1 when it runs but cannot be identified.
function Get-P9SProcessStartTicks([int]$Id) {
    $p = $null
    try { $p = [Diagnostics.Process]::GetProcessById($Id) } catch { return 0L }
    try { if ($p.HasExited) { return 0L }; return $p.StartTime.ToUniversalTime().Ticks } catch { return -1L } finally { $p.Dispose() }
}
# 'gone', 'alive' or 'unknown' for a recorded child (pid and start ticks).
function Test-P9SChildAlive($Id, $Ticks) {
    if (-not $Id) { return 'unknown' }
    $t = Get-P9SProcessStartTicks ([int]$Id)
    if ($t -eq 0) { return 'gone' }
    if ($t -lt 0) { return 'unknown' }
    if ($Ticks -and [long]$Ticks -ne $t) { return 'gone' }
    'alive'
}

# ------------------------------------------------------------ records
function Read-P9SVBoxJson([string]$Path, [string]$Schema) {
    if (-not (Test-Path -LiteralPath $Path)) { return [pscustomobject]@{ present = $false; ok = $true; doc = $null } }
    $r = Read-StateFile -Path $Path
    if (-not $r.ok) { return [pscustomobject]@{ present = $true; ok = $false; doc = $null } }
    $d = $null; try { $d = $r.text | ConvertFrom-Json -AsHashtable -ErrorAction Stop } catch {}
    if ($d -isnot [System.Collections.IDictionary] -or $d.schema -ne $Schema) { return [pscustomobject]@{ present = $true; ok = $false; doc = $null } }
    [pscustomobject]@{ present = $true; ok = $true; doc = $d }
}
function Read-P9SVBoxLatch($VBox) { Read-P9SVBoxJson (Get-P9SVBoxPath $VBox 'latch') 'p9s-vbox-quarantine/1' }
function Read-P9SVBoxRecord($VBox) { Read-P9SVBoxJson (Get-P9SVBoxPath $VBox 'record') 'p9s-vbox-operation/1' }
function Save-P9SVBoxRecord($VBox, $Rec) {
    $Rec.updated_utc = [DateTime]::UtcNow.ToString('o')
    New-Item -ItemType Directory -Force -Path $VBox.Dir | Out-Null
    Write-StateAtomic -Path (Get-P9SVBoxPath $VBox 'record') -Text ($Rec | ConvertTo-Json -Depth 4)
}
function Remove-P9SVBoxRecord($VBox) { $p = Get-P9SVBoxPath $VBox 'record'; if (Test-Path -LiteralPath $p -PathType Leaf) { Remove-Item -LiteralPath $p -Force } }
# Writes the quarantine unless one is already in place (the first one stays,
# with its operation id). Returns $true when a quarantine now blocks every
# operation, $false when none could be written: the caller must then keep
# the mutex.
function Set-P9SVBoxLatch($VBox, [string]$Reason, $Rec, [string]$Detail) {
    $p = Get-P9SVBoxPath $VBox 'latch'
    if (Test-Path -LiteralPath $p) { return $true }
    if ($VBox.FailLatchWrite) { return $false }
    $l = [ordered]@{
        schema = 'p9s-vbox-quarantine/1'; reason = $Reason; operation_id = $(if ($Rec -and $Rec.operation_id) { [string]$Rec.operation_id } else { New-P9SVBoxOperationId })
        operation = $(if ($Rec) { $Rec.operation } else { $null }); vm = $(if ($Rec) { $Rec.vm } else { $null })
        child_pid = $(if ($Rec) { $Rec.child_pid } else { $null }); child_start_ticks = $(if ($Rec) { $Rec.child_start_ticks } else { $null })
        owner_pid = $PID; utc = [DateTime]::UtcNow.ToString('o'); detail = $Detail
    }
    try {
        New-Item -ItemType Directory -Force -Path $VBox.Dir | Out-Null
        Write-StateAtomic -Path $p -Text ($l | ConvertTo-Json -Depth 4)
        $c = Read-P9SVBoxLatch $VBox
        return [bool]($c.ok -and [string]$c.doc.operation_id -ceq $l.operation_id -and [string]$c.doc.reason -ceq $Reason)
    } catch { return $false }
}
# Diagnostics of a failed command: its length and any VirtualBox error codes,
# never its text (it can hold paths, VM settings and names).
function ConvertTo-P9SVBoxDiag([string]$Err) {
    if (-not $Err) { return 'no error output' }
    $codes = @([regex]::Matches($Err, '\b(VBOX_E_[A-Z_]{3,40}|NS_ERROR_[A-Z_]{3,40}|E_[A-Z]{3,20}|0x[0-9a-fA-F]{8})\b') | ForEach-Object { $_.Value } | Select-Object -Unique -First 4)
    "error output $($Err.Length) characters$(if ($codes) { '; codes ' + ($codes -join ' ') })"
}
function Add-P9SVBoxAudit($VBox, $Res) {
    $e = [ordered]@{
        utc = [DateTime]::UtcNow.ToString('yyyy-MM-ddTHH:mm:ss.fffZ'); pid = $PID; operation_id = $Res.operation_id; operation = $Res.operation; vm = $Res.vm
        outcome = $Res.outcome; reason = $Res.reason; exit_code = $Res.exit_code; elapsed_ms = $Res.elapsed_ms; lock_wait_ms = $Res.lock_wait_ms
        abandoned = $Res.abandoned; latched = $Res.latched; retained = $Res.retained; child_pid = $Res.child_pid; diag = $Res.diag; note = $Res.note
    }
    $bytes = [Text.UTF8Encoding]::new($false).GetBytes(($e | ConvertTo-Json -Compress -Depth 3) + "`n")
    try { New-Item -ItemType Directory -Force -Path $VBox.Dir | Out-Null } catch {}
    for ($i = 0; $i -lt 40; $i++) {
        try {
            $fs = [IO.FileStream]::new((Get-P9SVBoxPath $VBox 'audit'), [IO.FileMode]::Append, [IO.FileAccess]::Write, [IO.FileShare]::ReadWrite)
            try { $fs.Write($bytes, 0, $bytes.Length) } finally { $fs.Dispose() }
            return
        } catch { Start-Sleep -Milliseconds 25 }
    }
}
function Invoke-P9SVBoxPhase($VBox, [string]$Phase) { if ($VBox.OnPhase) { & $VBox.OnPhase $Phase } }

# ------------------------------------------------------------ the mutex
function Enter-P9SVBoxMutex([string]$Name, [int]$WaitMs) {
    $m = $null
    try { $m = [Threading.Mutex]::new($false, $Name) } catch { return [pscustomobject]@{ ok = $false; reason = 'VBOX_LOCK_UNAVAILABLE'; mutex = $null; abandoned = $false; waited_ms = 0 } }
    $sw = [Diagnostics.Stopwatch]::StartNew(); $got = $false; $ab = $false
    try { $got = $m.WaitOne($WaitMs) }
    catch {
        $x = $_.Exception; while ($x -and $x -isnot [Threading.AbandonedMutexException]) { $x = $x.InnerException }
        if ($x) { $got = $true; $ab = $true } else { $m.Dispose(); return [pscustomobject]@{ ok = $false; reason = 'VBOX_LOCK_UNAVAILABLE'; mutex = $null; abandoned = $false; waited_ms = $sw.ElapsedMilliseconds } }
    }
    if (-not $got) { $m.Dispose(); return [pscustomobject]@{ ok = $false; reason = 'VBOX_LOCK_TIMEOUT'; mutex = $null; abandoned = $false; waited_ms = $sw.ElapsedMilliseconds } }
    [pscustomobject]@{ ok = $true; reason = $null; mutex = $m; abandoned = $ab; waited_ms = $sw.ElapsedMilliseconds }
}
# A mutex that may not be released stays owned by this process until it
# ends; every later operation of this process is then refused.
function Exit-P9SVBoxMutex($Handle, [bool]$Release, [string]$Name) {
    if (-not $Handle -or -not $Handle.mutex) { return }
    if ($Release) { try { $Handle.mutex.ReleaseMutex() } catch {}; $Handle.mutex.Dispose() }
    else { $script:P9SVBoxRetained[$Name] = $Handle.mutex }
}
# Under the mutex, before anything runs: an existing quarantine refuses; an
# operation record left behind means that its owner ended without finishing.
# Windows reports a mutex as abandoned only while another process still
# holds a handle to it; when the owner held the only handle, the next
# process creates the mutex anew. The record is therefore the evidence, and
# the abandoned flag only adds to it. Only an owner that ended before
# starting VBoxManage is resolved here; every other case quarantines.
function Stop-P9SVBoxEntry($VBox, [string]$Reason, $Rec, [string]$Detail) {
    [pscustomobject]@{ ok = $false; reason = $Reason; latched = (Set-P9SVBoxLatch $VBox $Reason $Rec $Detail); note = $Detail }
}
function Resolve-P9SVBoxEntry($VBox, [bool]$Abandoned) {
    if ((Read-P9SVBoxLatch $VBox).present) { return [pscustomobject]@{ ok = $false; reason = 'VBOX_API_QUARANTINED'; latched = $true; note = $null } }
    $r = Read-P9SVBoxRecord $VBox
    if (-not $r.present) {
        if ($Abandoned) { return (Stop-P9SVBoxEntry $VBox 'VBOX_API_STATE_UNKNOWN' $null 'lock abandoned without an operation record') }
        return [pscustomobject]@{ ok = $true; reason = $null; latched = $false; note = $null }
    }
    if (-not $r.ok) { return (Stop-P9SVBoxEntry $VBox 'VBOX_RECORD_UNREADABLE' $null 'operation record unreadable') }
    $rec = $r.doc
    if ([string]$rec.operation_id -cnotmatch (Get-P9SVBoxIdPattern) -or -not $rec.phase -or -not $rec.owner_pid -or -not $rec.owner_start_ticks) { return (Stop-P9SVBoxEntry $VBox 'VBOX_RECORD_INCOMPLETE' $null 'operation record incomplete') }
    # the lock is ours, so its owner no longer holds it; a live owner left its record behind
    $owner = Test-P9SChildAlive $rec.owner_pid $rec.owner_start_ticks
    if ($owner -ne 'gone') { return (Stop-P9SVBoxEntry $VBox 'VBOX_RECORD_STALE' $rec "operation record of a process that is $owner but no longer holds the lock") }
    $how = if ($Abandoned) { 'VBOX_LOCK_ABANDONED' } else { 'VBOX_OWNER_ENDED' }
    switch ([string]$rec.phase) {
        'ACQUIRED' { Remove-P9SVBoxRecord $VBox; return [pscustomobject]@{ ok = $true; reason = $null; latched = $false; note = "$($how)_NO_CHILD" } }
        'STARTING' { return (Stop-P9SVBoxEntry $VBox 'VBOX_API_STATE_UNKNOWN' $rec 'owner ended while starting VBoxManage') }
        'IN_PROCESS' { return (Stop-P9SVBoxEntry $VBox 'VBOX_OPERATION_INTERRUPTED' $rec 'owner ended during an in-process VirtualBox call') }
        { $_ -in 'RUNNING', 'TIMED_OUT' } {
            if (-not $rec.child_pid -or -not $rec.child_start_ticks) { return (Stop-P9SVBoxEntry $VBox 'VBOX_RECORD_INCOMPLETE' $rec 'operation record without its child') }
            $alive = Test-P9SChildAlive $rec.child_pid $rec.child_start_ticks
            if ($alive -eq 'gone') { return (Stop-P9SVBoxEntry $VBox 'VBOX_OPERATION_INTERRUPTED' $rec 'owner ended during the command; the command has ended, its effect is unknown') }
            return (Stop-P9SVBoxEntry $VBox 'VBOX_CHILD_UNRESOLVED' $rec "VBoxManage child $($rec.child_pid) is $alive")
        }
    }
    Stop-P9SVBoxEntry $VBox 'VBOX_RECORD_INCOMPLETE' $rec 'operation record in an unknown phase'
}

# ------------------------------------------------------------ the child
# The real child: VBoxManage with its arguments as a list (no shell, no
# quoting), output read asynchronously so that a full pipe cannot block it.
function Start-P9SVBoxChild($VBox, [string[]]$Arguments) {
    $psi = [Diagnostics.ProcessStartInfo]::new($VBox.ExePath)
    foreach ($a in @($VBox.PrefixArgs) + @($Arguments)) { $psi.ArgumentList.Add([string]$a) }
    $psi.UseShellExecute = $false; $psi.RedirectStandardOutput = $true; $psi.RedirectStandardError = $true; $psi.CreateNoWindow = $true
    if ($VBox.ProcessFactory) { return (& $VBox.ProcessFactory $psi) }
    $p = [Diagnostics.Process]::Start($psi)
    $outT = $p.StandardOutput.ReadToEndAsync(); $errT = $p.StandardError.ReadToEndAsync()
    $ticks = 0L; try { $ticks = $p.StartTime.ToUniversalTime().Ticks } catch {}
    [pscustomobject]@{
        Id = $p.Id; StartTicks = $ticks
        Wait = { param($ms) $p.WaitForExit([int]$ms) }.GetNewClosure()
        Ended = { $p.HasExited }.GetNewClosure()
        Code = { $p.ExitCode }.GetNewClosure()
        Output = { param($ms) $done = [Threading.Tasks.Task]::WaitAll([Threading.Tasks.Task[]]@($outT, $errT), [int]$ms)
            [pscustomobject]@{ complete = $done; out = $(if ($outT.IsCompleted) { $outT.Result } else { '' }); err = $(if ($errT.IsCompleted) { $errT.Result } else { '' }) } }.GetNewClosure()
    }
}
# $true exited, $false still running, $null unknown.
function Wait-P9SVBoxChild($Child, [int]$Ms) {
    try { if (& $Child.Wait $Ms) { if (& $Child.Ended) { return $true } else { return $null } } else { return $false } } catch { return $null }
}
# Runs one command while the caller holds the mutex. Returns $true when the
# caller may release the mutex: the child has exited, or a quarantine is in
# place. $State.resolved stays $false while a started child is unresolved.
function Invoke-P9SVBoxChildLocked($VBox, $Res, [hashtable]$State) {
    $op = $Res.operation; $vm = $Res.vm
    $Res.operation_id = New-P9SVBoxOperationId
    $own = 0L; try { $own = (Get-Process -Id $PID).StartTime.ToUniversalTime().Ticks } catch {}
    $rec = [ordered]@{ schema = 'p9s-vbox-operation/1'; operation_id = $Res.operation_id; operation = $op; vm = $vm; phase = 'ACQUIRED'
        owner_pid = $PID; owner_start_ticks = $own; child_pid = $null; child_start_ticks = $null; started_utc = [DateTime]::UtcNow.ToString('o'); updated_utc = $null
        exec_ms = [int]$VBox.ExecMs[$op] }
    $State.rec = $rec
    try {
        Save-P9SVBoxRecord $VBox $rec
        Invoke-P9SVBoxPhase $VBox 'ACQUIRED'
        $rec.phase = 'STARTING'; Save-P9SVBoxRecord $VBox $rec
        Invoke-P9SVBoxPhase $VBox 'STARTING'
    } catch {
        $Res.outcome = 'FAILED'; $Res.reason = 'VBOX_RECORD_WRITE_FAILED'
        try { Remove-P9SVBoxRecord $VBox } catch {}
        return $true
    }
    $child = $null
    try { $child = Start-P9SVBoxChild $VBox (Get-P9SVBoxArguments $op $vm) }
    catch { $Res.outcome = 'START_FAILED'; $Res.reason = 'VBOX_START_FAILED'; try { Remove-P9SVBoxRecord $VBox } catch {}; return $true }
    $State.resolved = $false
    $Res.child_pid = $child.Id
    $rec.phase = 'RUNNING'; $rec.child_pid = $child.Id; $rec.child_start_ticks = $child.StartTicks
    try { Save-P9SVBoxRecord $VBox $rec } catch {}
    $sw = [Diagnostics.Stopwatch]::StartNew()
    Invoke-P9SVBoxPhase $VBox 'RUNNING'
    $done = Wait-P9SVBoxChild $child ([int]$VBox.ExecMs[$op])
    if ($done -eq $true) {
        $Res.elapsed_ms = $sw.ElapsedMilliseconds
        $code = $null; try { $code = [int](& $child.Code) } catch {}
        if ($null -eq $code) { $done = $null }
        else {
            $o = $null; try { $o = & $child.Output $VBox.OutputMs } catch {}
            $Res.exit_code = $code
            $Res.lines = @(if ($o -and $o.out) { $o.out -split "`r?`n" | Where-Object { $_ } })
            if ($code -eq 0) { $Res.outcome = 'COMPLETED'; if ($o -and -not $o.complete) { $Res.note = 'output incomplete' } }
            else { $Res.outcome = 'ERROR'; $Res.reason = 'VBOX_COMMAND_FAILED'; $Res.diag = ConvertTo-P9SVBoxDiag $(if ($o) { $o.err } else { '' }) }
            $State.resolved = $true
            try { Remove-P9SVBoxRecord $VBox } catch {}
            return $true
        }
    }
    if ($done -eq $false) {
        $rec.phase = 'TIMED_OUT'; try { Save-P9SVBoxRecord $VBox $rec } catch {}
        Invoke-P9SVBoxPhase $VBox 'TIMED_OUT'
        $done = Wait-P9SVBoxChild $child ([int]$VBox.ConfirmMs)
        $Res.elapsed_ms = $sw.ElapsedMilliseconds
        if ($done -eq $true) { $Res.outcome = 'TIMED_OUT_EXITED'; $Res.reason = 'VBOX_COMMAND_TIMEOUT'; $why = "VBoxManage ended only after the $($VBox.ExecMs[$op]) ms limit" }
        elseif ($done -eq $false) { $Res.outcome = 'TIMED_OUT_ALIVE'; $Res.reason = 'VBOX_CHILD_UNRESOLVED'; $why = "VBoxManage child $($child.Id) still running after $($Res.elapsed_ms) ms" }
    }
    if ($null -eq $done) { $Res.elapsed_ms = $sw.ElapsedMilliseconds; $Res.outcome = 'TERMINATION_UNVERIFIED'; $Res.reason = 'VBOX_TERMINATION_UNVERIFIED'; $why = 'the end of the VBoxManage child could not be confirmed' }
    $Res.latched = Set-P9SVBoxLatch $VBox $Res.reason $rec $why
    $State.resolved = $true
    $Res.latched
}

# ------------------------------------------------------------ entry points
function Invoke-P9SVBoxOperation {
    param([Parameter(Mandatory)]$VBox, [Parameter(Mandatory)][string]$Operation, [string]$Vm)
    $res = New-P9SVBoxResult $Operation $Vm
    if ($Operation -cnotin (Get-P9SVBoxOperations)) { $res.outcome = 'REJECTED'; $res.reason = 'VBOX_OPERATION_NOT_ALLOWED'; Add-P9SVBoxAudit $VBox $res; return $res }
    if ($Vm -cnotin $VBox.VmNames) { $res.outcome = 'REJECTED'; $res.reason = 'VBOX_TARGET_REJECTED'; $res.vm = '<rejected>'; Add-P9SVBoxAudit $VBox $res; return $res }
    if ($script:P9SVBoxRetained[$VBox.MutexName]) { $res.outcome = 'RETAINED'; $res.reason = 'VBOX_LOCK_RETAINED'; Add-P9SVBoxAudit $VBox $res; return $res }
    if (-not (Test-Path -LiteralPath $VBox.ExePath -PathType Leaf)) { $res.outcome = 'UNAVAILABLE'; $res.reason = 'VBOX_EXECUTABLE_UNAVAILABLE'; Add-P9SVBoxAudit $VBox $res; return $res }
    if ((Read-P9SVBoxLatch $VBox).present) { $res.outcome = 'QUARANTINED'; $res.reason = 'VBOX_API_QUARANTINED'; $res.latched = $true; Add-P9SVBoxAudit $VBox $res; return $res }
    $h = Enter-P9SVBoxMutex $VBox.MutexName $VBox.LockWaitMs
    $res.lock_wait_ms = $h.waited_ms
    if (-not $h.ok) {
        $res.reason = $h.reason
        if ($h.reason -eq 'VBOX_LOCK_TIMEOUT') { $res.outcome = 'LOCK_TIMEOUT'; $res.latched = Set-P9SVBoxLatch $VBox 'VBOX_LOCK_TIMEOUT' $null "lock not acquired within $($VBox.LockWaitMs) ms" }
        else { $res.outcome = 'LOCK_UNAVAILABLE' }
        Add-P9SVBoxAudit $VBox $res
        return $res
    }
    $res.abandoned = $h.abandoned
    $state = @{ resolved = $true; rec = $null }
    $release = $true
    try {
        $e = Resolve-P9SVBoxEntry $VBox $h.abandoned
        if (-not $e.ok) { $res.outcome = 'QUARANTINED'; $res.reason = $e.reason; $res.latched = $e.latched; $res.note = $e.note; $release = [bool]$e.latched; return $res }
        if ($e.note) { $res.note = $e.note }
        $release = Invoke-P9SVBoxChildLocked $VBox $res $state
        return $res
    } catch {
        if ($state.resolved) { $res.outcome = 'FAILED'; $res.reason = 'VBOX_INTERNAL_ERROR' }
        if ($_.Exception -is [System.Management.Automation.PipelineStoppedException]) { throw }
    } finally {
        if (-not $state.resolved) {
            $res.outcome = 'CANCELLED'; $res.reason = 'VBOX_OPERATION_CANCELLED'
            $release = Set-P9SVBoxLatch $VBox 'VBOX_OPERATION_CANCELLED' $state.rec 'interrupted while VBoxManage ran'
            $res.latched = $release
        }
        if (-not $release) { $res.retained = $true }
        Exit-P9SVBoxMutex $h $release $VBox.MutexName
        Add-P9SVBoxAudit $VBox $res
    }
    $res
}
# A VirtualBox COM call in this process, under the same mutex and records.
function Invoke-P9SVBoxInProcess {
    param([Parameter(Mandatory)]$VBox, [Parameter(Mandatory)][string]$Operation, [string]$Vm, [Parameter(Mandatory)][scriptblock]$Body,
          [object[]]$Arguments = @(), [int]$LockWaitMs = 0)
    $res = New-P9SVBoxResult $Operation $Vm
    if ($Operation -cnotin (Get-P9SVBoxInProcessOperations)) { $res.outcome = 'REJECTED'; $res.reason = 'VBOX_OPERATION_NOT_ALLOWED'; Add-P9SVBoxAudit $VBox $res; return $res }
    if ($Vm -and $Vm -cnotin $VBox.VmNames) { $res.outcome = 'REJECTED'; $res.reason = 'VBOX_TARGET_REJECTED'; $res.vm = '<rejected>'; Add-P9SVBoxAudit $VBox $res; return $res }
    if ($script:P9SVBoxRetained[$VBox.MutexName]) { $res.outcome = 'RETAINED'; $res.reason = 'VBOX_LOCK_RETAINED'; return $res }
    if ((Read-P9SVBoxLatch $VBox).present) { $res.outcome = 'QUARANTINED'; $res.reason = 'VBOX_API_QUARANTINED'; $res.latched = $true; return $res }
    $wait = if ($LockWaitMs -gt 0) { $LockWaitMs } else { $VBox.LockWaitMs }
    $h = Enter-P9SVBoxMutex $VBox.MutexName $wait
    $res.lock_wait_ms = $h.waited_ms
    if (-not $h.ok) {
        $res.reason = $h.reason
        if ($h.reason -ne 'VBOX_LOCK_TIMEOUT') { $res.outcome = 'LOCK_UNAVAILABLE' }
        else { $res.outcome = 'LOCK_TIMEOUT'; $res.latched = Set-P9SVBoxLatch $VBox 'VBOX_LOCK_TIMEOUT' $null "lock not acquired within $wait ms" }
        Add-P9SVBoxAudit $VBox $res
        return $res
    }
    $res.abandoned = $h.abandoned
    $state = @{ resolved = $true; rec = $null }
    $release = $true
    try {
        $e = Resolve-P9SVBoxEntry $VBox $h.abandoned
        if (-not $e.ok) { $res.outcome = 'QUARANTINED'; $res.reason = $e.reason; $res.latched = $e.latched; $res.note = $e.note; $release = [bool]$e.latched; return $res }
        $res.operation_id = New-P9SVBoxOperationId
        $own = 0L; try { $own = (Get-Process -Id $PID).StartTime.ToUniversalTime().Ticks } catch {}
        $rec = [ordered]@{ schema = 'p9s-vbox-operation/1'; operation_id = $res.operation_id; operation = $Operation; vm = $Vm; phase = 'IN_PROCESS'
            owner_pid = $PID; owner_start_ticks = $own; child_pid = $null; child_start_ticks = $null; started_utc = [DateTime]::UtcNow.ToString('o'); updated_utc = $null }
        $state.rec = $rec
        try { Save-P9SVBoxRecord $VBox $rec } catch { $res.outcome = 'FAILED'; $res.reason = 'VBOX_RECORD_WRITE_FAILED'; return $res }
        $state.resolved = $false
        Invoke-P9SVBoxPhase $VBox 'IN_PROCESS'
        $sw = [Diagnostics.Stopwatch]::StartNew()
        try { $res.value = & $Body @Arguments; $res.outcome = 'COMPLETED' }
        catch {
            # the exception type and code only: its message can carry arguments of the call
            $x = $_.Exception
            $res.outcome = 'ERROR'; $res.reason = 'VBOX_CALL_FAILED'; $res.note = "$($x.GetType().Name)$(if ($x.HResult) { ' 0x' + $x.HResult.ToString('x8') })"
        }
        $res.elapsed_ms = $sw.ElapsedMilliseconds
        $state.resolved = $true
        try { Remove-P9SVBoxRecord $VBox } catch {}
        return $res
    } catch {
        if ($state.resolved) { $res.outcome = 'FAILED'; $res.reason = 'VBOX_INTERNAL_ERROR' }
        if ($_.Exception -is [System.Management.Automation.PipelineStoppedException]) { throw }
    } finally {
        if (-not $state.resolved) {
            $res.outcome = 'CANCELLED'; $res.reason = 'VBOX_OPERATION_CANCELLED'
            $release = Set-P9SVBoxLatch $VBox 'VBOX_OPERATION_CANCELLED' $state.rec 'interrupted during an in-process VirtualBox call'
            $res.latched = $release
        }
        if (-not $release) { $res.retained = $true }
        Exit-P9SVBoxMutex $h $release $VBox.MutexName
        Add-P9SVBoxAudit $VBox $res
    }
    $res
}
function ConvertFrom-P9SVBoxState([string[]]$Lines) {
    $l = @($Lines | Where-Object { $_ -like 'VMState=*' }) | Select-Object -First 1
    if ($l -match '^VMState="([a-z]+)"$') { if ($Matches[1] -eq 'poweroff') { 'off' } else { $Matches[1] } } else { $null }
}
# The operator's recovery of a quarantine, on request only: the operation
# id must match; a recorded VBoxManage child must have ended (it is never
# terminated here); then one state query per VM, one at a time, under the
# lock. Only if every query answers is the quarantine removed. Two
# recoveries cannot both clear: the second finds no quarantine, or another
# one.
function Invoke-P9SVBoxRecovery {
    param([Parameter(Mandatory)]$VBox, [Parameter(Mandatory)][string]$OperationId, [switch]$DryRun)
    $out = [pscustomobject]@{ outcome = $null; reasons = [System.Collections.Generic.List[string]]::new(); lines = [System.Collections.Generic.List[string]]::new(); states = [ordered]@{} }
    $audit = New-P9SVBoxResult 'vbox.recover' $null
    $refuse = { param($code, $line) $out.outcome = 'REFUSED'; $out.reasons.Add($code); if ($line) { $out.lines.Add($line) } }
    if ($script:P9SVBoxRetained[$VBox.MutexName]) { & $refuse 'VBOX_LOCK_RETAINED' 'this process keeps the VirtualBox lock after an unresolved operation; end it first'; return $out }
    if (-not (Read-P9SVBoxLatch $VBox).present -and -not (Read-P9SVBoxRecord $VBox).present) { & $refuse 'VBOX_NOT_QUARANTINED' 'no quarantine to clear'; return $out }
    $h = Enter-P9SVBoxMutex $VBox.MutexName $VBox.LockWaitMs
    if (-not $h.ok) { & $refuse $h.reason 'the VirtualBox lock was not acquired; the quarantine stays'; $audit.outcome = 'REFUSED'; $audit.reason = $h.reason; Add-P9SVBoxAudit $VBox $audit; return $out }
    $release = $true
    $state = @{ resolved = $true; rec = $null }
    try {
        $q = Read-P9SVBoxLatch $VBox
        if (-not $q.present) {
            # no quarantine, but possibly an operation left unfinished (vbox.status shows its id)
            $rr = Read-P9SVBoxRecord $VBox
            if (-not $rr.present) { & $refuse 'VBOX_NOT_QUARANTINED' 'no quarantine to clear (another recovery may have cleared it)'; return $out }
            if ($rr.ok -and [string]$rr.doc.operation_id -cne $OperationId) { & $refuse 'VBOX_RECOVERY_ID_MISMATCH' 'the operation id does not match the unfinished operation'; return $out }
            $e = Resolve-P9SVBoxEntry $VBox $h.abandoned
            if (-not $e.ok) {
                $release = [bool]$e.latched
                $nq = Read-P9SVBoxLatch $VBox
                & $refuse $e.reason "the unfinished operation is now quarantined ($($e.reason)$(if ($nq.ok) { ", operation $($nq.doc.operation_id)" })); check it, then run vbox.recover with that id again"
                return $out
            }
            $out.lines.Add("the unfinished operation never started VBoxManage ($($e.note)); its record was removed")
        }
        else {
            if ($h.abandoned) { & $refuse 'VBOX_LOCK_ABANDONED' 'the lock was abandoned again by another process; the quarantine stays'; return $out }
            if (-not $q.ok) { & $refuse 'VBOX_LATCH_UNREADABLE' "the quarantine file is unreadable; inspect $(Split-Path -Leaf (Get-P9SVBoxPath $VBox 'latch')) by hand"; return $out }
            if ([string]$q.doc.operation_id -cne $OperationId) { & $refuse 'VBOX_RECOVERY_ID_MISMATCH' 'the operation id does not match the quarantine'; return $out }
        }
        $rr = Read-P9SVBoxRecord $VBox
        $kids = @(@($(if ($q.present -and $q.ok) { $q.doc })) + @(if ($rr.ok -and $rr.doc) { $rr.doc }) | Where-Object { $_ -and $_.child_pid })
        foreach ($k in $kids) {
            $a = Test-P9SChildAlive $k.child_pid $k.child_start_ticks
            if ($a -ne 'gone') { & $refuse 'VBOX_CHILD_STILL_RUNNING' "VBoxManage child $($k.child_pid) is $a; the toolkit does not end it: the operator decides, then retries"; return $out }
        }
        if ($DryRun) { $out.outcome = 'DRYRUN'; $out.lines.Add("dry run: would read the state of $(@($VBox.VmNames) -join ' and '), one query at a time, and clear the quarantine only if every query answers"); return $out }
        Remove-P9SVBoxRecord $VBox
        foreach ($vm in @($VBox.VmNames)) {
            $pr = New-P9SVBoxResult 'vm.state' $vm
            $release = Invoke-P9SVBoxChildLocked $VBox $pr $state
            Add-P9SVBoxAudit $VBox $pr
            $st = if ($pr.outcome -eq 'COMPLETED' -and $pr.exit_code -eq 0) { ConvertFrom-P9SVBoxState $pr.lines } else { $null }
            if (-not $st) { & $refuse 'VBOX_RECOVERY_PROBE_FAILED' "the state query of $vm ended $($pr.outcome)$(if ($pr.reason) { " ($($pr.reason))" }); the quarantine stays and no other query is made"; return $out }
            $out.states[$vm] = $st
        }
        $lp = Get-P9SVBoxPath $VBox 'latch'
        if (Test-Path -LiteralPath $lp) { Remove-Item -LiteralPath $lp -Force }
        $out.outcome = 'CLEARED'
        $out.lines.Add("quarantine $OperationId cleared; VM states: $(($out.states.GetEnumerator() | ForEach-Object { "$($_.Key) $($_.Value)" }) -join ', ')")
        return $out
    } catch {
        $out.outcome = 'REFUSED'; $out.reasons.Add('VBOX_INTERNAL_ERROR')
        if ($_.Exception -is [System.Management.Automation.PipelineStoppedException]) { throw }
    } finally {
        if (-not $state.resolved) { $release = Set-P9SVBoxLatch $VBox 'VBOX_OPERATION_CANCELLED' $state.rec 'recovery interrupted while VBoxManage ran' }
        Exit-P9SVBoxMutex $h $release $VBox.MutexName
        $audit.outcome = $out.outcome; $audit.reason = $(if ($out.reasons.Count) { $out.reasons[0] } else { $null }); $audit.note = "recovery of $(if ($OperationId -cmatch (Get-P9SVBoxIdPattern)) { $OperationId } else { '<invalid id>' })"
        Add-P9SVBoxAudit $VBox $audit
    }
    $out
}
