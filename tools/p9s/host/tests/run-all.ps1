#Requires -Version 7.0
# Entry point of the toolkit's tests, for the Windows host and for CI.
# Runs the ten suites, each in a new pwsh process started with -File, and
# judges them by their structured results (results.json), not by their
# console text: every case declared in a suite must be reported exactly
# once, as PASS, FAIL or NOT_RUN; NOT_RUN is accepted only for the cases
# this machine cannot run (K37 without VirtualBox; P62, P63 and Q63, which
# read the real state locations, outside a clean runner); the exit code of the
# suite must agree with its results; anything written to the error stream
# fails the run.
#   -Mutants      also applies the targeted faults of tests/mutants.json,
#                 each to its own copy of this folder (never to it), and
#                 requires each to make one of its named tests fail.
#   -MutantShard k/n
#                 with -Mutants, applies only shard k of n: the faults whose
#                 position in tests/mutants.json, counted from 0, leaves k-1
#                 when divided by n. The n shards together hold every fault
#                 exactly once; this is checked for all n shards before any
#                 suite runs, and every assigned fault must end with one
#                 verdict. The suites always run in full. Without it every
#                 fault runs, as before.
#   -CleanRunner  first requires a machine without VirtualBox, laboratory
#                 configuration or secret store (a CI runner), so that no
#                 test can reach a real VM or a real secret.
# The coordinator files of the default state folder must not change.
#   pwsh -NoProfile -File tests/run-all.ps1 -OutDir <new folder> [-Mutants [-MutantShard k/n]] [-CleanRunner]
# Exit codes: 0 every check passed; 1 a check failed; 2 refused arguments
# or runtime.
param(
    [Parameter(Mandatory)][string]$OutDir,
    [switch]$Mutants,
    [string]$MutantShard,
    [switch]$CleanRunner,
    [int]$Throttle = 0
)
$ErrorActionPreference = 'Stop'
if ($PSVersionTable.PSEdition -ne 'Core' -or $PSVersionTable.PSVersion -lt [version]'7.4') { "refused: PowerShell 7.4 or later is required (this is $($PSVersionTable.PSEdition) $($PSVersionTable.PSVersion))"; exit 2 }
if (-not $IsWindows) { 'refused: the toolkit and its tests run on Windows only'; exit 2 }
$kit = Split-Path $PSScriptRoot -Parent
. (Join-Path $kit 'hostmon-lib.ps1')

# ---- the shards of the targeted faults ----------------------------------------------
# The fault ids by position, as n lists: list k-1 holds the ids whose
# position leaves k-1 when divided by n.
function Get-P9SMutantShardPlan([string[]]$Ids, [int]$N) {
    $plan = @(for ($k = 0; $k -lt $N; $k++) { , [System.Collections.Generic.List[string]]::new() })
    for ($i = 0; $i -lt $Ids.Count; $i++) { $plan[$i % $N].Add($Ids[$i]) }
    , @($plan | ForEach-Object { , [string[]]$_.ToArray() })
}
# A shard spec checked against the fault ids: ok with k, n and the ids of
# shard k, or the reason it is refused. Every shard must hold at least one
# fault, and the n shards together every fault exactly once.
function Get-P9SMutantShardSpec([string]$Spec, [bool]$WithMutants, [string[]]$Ids) {
    $no = { param([string]$Why) [pscustomobject]@{ ok = $false; error = $Why; k = 0; n = 0; assigned = @() } }
    if (-not $WithMutants) { return (& $no '-MutantShard needs -Mutants') }
    if ($Spec -cnotmatch '^([1-9][0-9]{0,2})/([1-9][0-9]{0,2})$') { return (& $no "-MutantShard takes k/n (whole numbers, 1 <= k <= n), not '$Spec'") }
    $k = [int]$Matches[1]; $n = [int]$Matches[2]
    if ($k -gt $n) { return (& $no "-MutantShard ${Spec}: k is greater than n") }
    if (-not $Ids -or $Ids.Count -eq 0) { return (& $no '-MutantShard: no fault ids to divide') }
    if (@($Ids | Group-Object -CaseSensitive | Where-Object Count -gt 1).Count) { return (& $no '-MutantShard: tests/mutants.json repeats a fault id') }
    if ($n -gt $Ids.Count) { return (& $no "-MutantShard ${Spec}: $n shards for $($Ids.Count) faults would leave a shard empty") }
    $plan = Get-P9SMutantShardPlan $Ids $n
    $seen = @{}
    foreach ($s in $plan) { if (-not $s.Count) { return (& $no "-MutantShard ${Spec}: a shard is empty") }; foreach ($id in $s) { $seen[$id] = 1 + [int]$seen[$id] } }
    if (@($Ids | Where-Object { $seen[$_] -ne 1 }).Count -or @($seen.Keys).Count -ne $Ids.Count) { return (& $no "-MutantShard ${Spec}: the shards do not hold every fault exactly once") }
    [pscustomobject]@{ ok = $true; error = $null; k = $k; n = $n; assigned = [string[]]$plan[$k - 1] }
}
# Each assigned fault must have exactly one verdict, and no other fault one.
function Get-P9SMutantShardProblems([string[]]$Assigned, $Results) {
    $got = @($Results | ForEach-Object { [string]$_.id })
    @(foreach ($id in $Assigned) { $c = @($got | Where-Object { $_ -ceq $id }).Count; if ($c -ne 1) { "mutant $id has $c verdicts" } }) +
    @(foreach ($id in @($got | Select-Object -Unique)) { if ($Assigned -cnotcontains $id) { "mutant $id ran but was not assigned" } })
}
$shard = $null
if ($PSBoundParameters.ContainsKey('MutantShard')) {
    $mids = @()
    try {
        $md = Get-Content -Raw -LiteralPath (Join-Path $PSScriptRoot 'mutants.json') | ConvertFrom-Json
        if ($md.schema -ne 'p9s-mutants/1' -or @($md.mutants).Count -ne [int]$md.count) { "refused: -MutantShard: tests/mutants.json has the wrong schema or count"; exit 2 }
        $mids = [string[]]@($md.mutants | ForEach-Object { [string]$_.id })
    } catch { "refused: -MutantShard: tests/mutants.json cannot be read ($($_.Exception.Message))"; exit 2 }
    $shard = Get-P9SMutantShardSpec $MutantShard ([bool]$Mutants) $mids
    if (-not $shard.ok) { "refused: $($shard.error)"; exit 2 }
    $shard | Add-Member -NotePropertyName total -NotePropertyValue $mids.Count
}
if (Test-Path -LiteralPath $OutDir) { "refused: $OutDir exists"; exit 2 }
$OutDir = [IO.Path]::GetFullPath($OutDir)
if ($OutDir.StartsWith($kit + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) { 'refused: -OutDir must be outside the toolkit folder'; exit 2 }
New-Item -ItemType Directory -Path $OutDir | Out-Null
$pwsh = (Get-Process -Id $PID).Path
$problems = [System.Collections.Generic.List[string]]::new()
$suites = 'hostmon', 'coordinator', 'stagec', 'vbox', 'decouple', 'acpival', 'persist', 'setup', 'migrate', 'migproc'
$required = @($suites | ForEach-Object { "tests\$_-tests.ps1" }) + @('hostmon-lib.ps1', 'coordinator-lib.ps1', 'p9s-config.ps1', 'p9s-vbox.ps1', 'p9s-vmstate.ps1', 'p9s-acpival.ps1', 'p9s-state.ps1', 'p9s-state-init.ps1', 'p9s-state-migrate.ps1', 'paging-thresholds.json') + @(if ($Mutants) { 'tests\mutants.json' })
$missing = @($required | Where-Object { -not (Test-Path -LiteralPath (Join-Path $kit $_)) })
if ($missing.Count) { "FAIL required files missing: $($missing -join ', ')"; [ordered]@{ schema = 'p9s-test-run/1'; ok = $false; problems = @("required files missing: $($missing -join ', ')") } | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $OutDir 'summary.json') -Encoding utf8; exit 1 }

# ---- the machine ------------------------------------------------------------------
$vbox = Join-Path $env:ProgramFiles 'Oracle\VirtualBox\VBoxManage.exe'
$envFacts = [ordered]@{
    os = [string][Environment]::OSVersion.VersionString; pwsh = [string]$PSVersionTable.PSVersion; edition = $PSVersionTable.PSEdition
    processors = [Environment]::ProcessorCount; vboxmanage_present = (Test-Path -LiteralPath $vbox)
    virtualbox_com_registered = (Test-Path -LiteralPath 'Registry::HKEY_CLASSES_ROOT\VirtualBox.VirtualBoxClient')
    local_config_present = (Test-Path -LiteralPath (Join-Path $kit 'p9s-local.json')); p9s_host_config_set = [bool]$env:P9S_HOST_CONFIG
    tor_on_path = [bool](Get-Command tor -ErrorAction SilentlyContinue)
}
"== P9-S toolkit tests: pwsh $($envFacts.pwsh) on $($envFacts.os), $($envFacts.processors) processors"
"   VBoxManage present: $($envFacts.vboxmanage_present); VirtualBox COM registered: $($envFacts.virtualbox_com_registered); local configuration: $($envFacts.local_config_present); P9S_HOST_CONFIG set: $($envFacts.p9s_host_config_set); tor on PATH: $($envFacts.tor_on_path)"
if ($shard) { "   mutant shard $($shard.k)/$($shard.n): $($shard.assigned.Count) of $($shard.total) faults: $($shard.assigned -join ' ')" }
$defaultDir = Split-Path (Get-DefaultStatePath) -Parent
$envFacts.default_state_folder_present = (Test-Path -LiteralPath $defaultDir)
if ($CleanRunner) {
    foreach ($k in 'vboxmanage_present', 'virtualbox_com_registered', 'local_config_present', 'p9s_host_config_set', 'default_state_folder_present') {
        if ($envFacts[$k]) { $problems.Add("clean runner required, but ${k}: true") }
    }
    if ($problems.Count) { $problems | ForEach-Object { "FAIL $_" }; $envFacts | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $OutDir 'environment.json') -Encoding utf8; exit 1 }
}
$envFacts | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $OutDir 'environment.json') -Encoding utf8
$allowedNotRun = @{ hostmon = @(); coordinator = @(); stagec = @(); vbox = @(); decouple = @(); acpival = @(); persist = @(); setup = @(); migrate = @(); migproc = @() }
if (-not $envFacts.vboxmanage_present) { $allowedNotRun.coordinator = @('K37') }
# P62, P63 and Q63 run the entry point against the machine's real state
# locations: only on a clean runner, and only in the main run (never in a
# mutant's copy)
if (-not $CleanRunner) { $allowedNotRun.persist = @('P62', 'P63'); $allowedNotRun.migrate = @('Q63') }

# the coordinator files of the default state folder, before
function Get-DefaultCoordinatorFiles {
    foreach ($n in 'coordinator-session.json', 'coordinator-audit.jsonl', 'coordinator-stop.request', 'coordinator.lock') {
        $p = Join-Path $defaultDir $n
        if (Test-Path -LiteralPath $p) { $i = Get-Item -LiteralPath $p; "$n $($i.Length) $($i.LastWriteTimeUtc.Ticks)" } else { "$n absent" }
    }
}
$coordBefore = @(Get-DefaultCoordinatorFiles) -join '; '

# ---- one suite in a new process, judged by its results ------------------------------
function Invoke-P9SSuite {
    param([string]$Pwsh, [string]$KitDir, [string]$Suite, [string]$Dir, [string[]]$AllowedNotRun, [int]$TimeoutSeconds = 900)
    $file = Join-Path $KitDir "tests\$Suite-tests.ps1"
    $declared = @([regex]::Matches([IO.File]::ReadAllText($file), "(?m)^\s*Case '([A-Za-z0-9-]+)'") | ForEach-Object { $_.Groups[1].Value })
    $r = [ordered]@{ suite = $Suite; declared = $declared.Count; exit = $null; pass = 0; fail = 0; not_run = 0; failed = @(); not_run_ids = @(); problems = [System.Collections.Generic.List[string]]::new() }
    if (@($declared | Group-Object | Where-Object Count -gt 1).Count) { $r.problems.Add("$Suite declares a case id twice") }
    $sd = Join-Path $Dir $Suite
    $q = { param($s) '"' + $s + '"' }
    $p = Start-Process -FilePath $Pwsh -ArgumentList @('-NoProfile', '-NonInteractive', '-File', (& $q $file), '-OutDir', (& $q $sd)) -NoNewWindow -PassThru `
        -RedirectStandardOutput (Join-Path $Dir "$Suite-console.txt") -RedirectStandardError (Join-Path $Dir "$Suite-stderr.txt")
    $null = $p.Handle
    if (-not $p.WaitForExit($TimeoutSeconds * 1000)) { try { $p.Kill($true) } catch {}; $r.problems.Add("$Suite did not finish within $TimeoutSeconds s") }
    $p.WaitForExit()
    $r.exit = $p.ExitCode
    $err = Get-Content -Raw -LiteralPath (Join-Path $Dir "$Suite-stderr.txt")
    if ($err -and $err.Trim()) { $r.problems.Add("$Suite wrote to the error stream: $($err.Trim().Substring(0, [math]::Min(300, $err.Trim().Length)))") }
    $res = $null
    try { $res = @(Get-Content -Raw -LiteralPath (Join-Path $sd 'results.json') | ConvertFrom-Json) } catch { $r.problems.Add("$Suite results.json missing or unreadable") }
    if ($null -ne $res) {
        $ids = @($res | ForEach-Object { [string]$_.id })
        foreach ($d in $declared) { if (@($ids | Where-Object { $_ -ceq $d }).Count -ne 1) { $r.problems.Add("$Suite case $d reported $(@($ids | Where-Object { $_ -ceq $d }).Count) times") } }
        foreach ($i in $ids) { if ($declared -cnotcontains $i) { $r.problems.Add("$Suite reported an undeclared case $i") } }
        foreach ($x in $res) {
            switch ([string]$x.result) {
                'PASS' { $r.pass++ }
                'FAIL' { $r.fail++; $r.failed += "$($x.id) $($x.name) :: $($x.detail)" }
                'NOT_RUN' { $r.not_run++; $r.not_run_ids += [string]$x.id; if ($AllowedNotRun -cnotcontains [string]$x.id) { $r.problems.Add("$Suite case $($x.id) not run here: $($x.detail)") } }
                default { $r.problems.Add("$Suite case $($x.id) has result '$($x.result)'") }
            }
        }
        $expect = if ($r.fail) { 1 } elseif ($r.not_run) { 3 } else { 0 }
        if ($r.exit -ne $expect) { $r.problems.Add("$Suite exit code $($r.exit) does not match its results (expected $expect)") }
    }
    [pscustomobject]$r
}

# ---- the suites, one after the other ---------------------------------------------------
$suiteResults = [System.Collections.Generic.List[object]]::new()
foreach ($s in $suites) {
    $sw = [Diagnostics.Stopwatch]::StartNew()
    if ($CleanRunner -and $s -in 'persist', 'migrate') { $env:P9S_TEST_REAL_LOCATIONS = '1' }
    try { $x = Invoke-P9SSuite -Pwsh $pwsh -KitDir $kit -Suite $s -Dir $OutDir -AllowedNotRun $allowedNotRun[$s] } finally { Remove-Item Env:P9S_TEST_REAL_LOCATIONS -ErrorAction SilentlyContinue }
    $x | Add-Member -NotePropertyName seconds -NotePropertyValue ([math]::Round($sw.Elapsed.TotalSeconds, 1))
    $suiteResults.Add($x)
    "== $s : declared $($x.declared), passed $($x.pass), failed $($x.fail), not run $($x.not_run) $(if ($x.not_run_ids) { '(' + ($x.not_run_ids -join ', ') + ')' }), exit $($x.exit), $($x.seconds) s"
    foreach ($f in $x.failed) { "   FAIL $f" }
    foreach ($pr in $x.problems) { "   PROBLEM $pr"; $problems.Add($pr) }
    if ($x.problems.Count) { "   --- last lines of the $s console:"; Get-Content -LiteralPath (Join-Path $OutDir "$s-console.txt") -Tail 40 | ForEach-Object { "   | $_" } }
    if ($x.fail) { $problems.Add("$s : $($x.fail) failed") }
}
$suitesOk = -not $problems.Count

# ---- targeted mutants ----------------------------------------------------------------------
$mutantResults = @()
if ($Mutants) {
    if (-not $suitesOk) { 'mutants: not run, because the suites did not pass'; $problems.Add('mutants not run: the suites did not pass') }
    else {
        $def = Get-Content -Raw -LiteralPath (Join-Path $PSScriptRoot 'mutants.json') | ConvertFrom-Json
        if ($def.schema -ne 'p9s-mutants/1' -or @($def.mutants).Count -ne [int]$def.count) { $problems.Add('mutants.json: wrong schema or count') }
        if (@($def.mutants | Group-Object id | Where-Object Count -gt 1).Count) { $problems.Add('mutants.json: duplicate id') }
        $todo = @($def.mutants)
        if ($shard) {
            # the same shard of the faults as checked at the start
            $again = Get-P9SMutantShardSpec $MutantShard $true ([string[]]@($def.mutants | ForEach-Object { [string]$_.id }))
            if (-not $again.ok -or ($again.assigned -join ',') -cne ($shard.assigned -join ',')) { $problems.Add('mutants.json changed during the run: the shard is no longer the one checked at the start') }
            $todo = @($def.mutants | Where-Object { $shard.assigned -ccontains [string]$_.id })
        }
        $n = if ($Throttle -gt 0) { $Throttle } else { [math]::Max(1, [math]::Min(4, [Environment]::ProcessorCount)) }
        $mdir = Join-Path $OutDir 'mutants'; New-Item -ItemType Directory -Path $mdir | Out-Null
        $fn = ${function:Invoke-P9SSuite}.ToString()
        $kitDir = $kit; $pw = $pwsh; $allow = $allowedNotRun
        "== mutants: $($todo.Count) of $(@($def.mutants).Count)$(if ($shard) { " (shard $($shard.k)/$($shard.n))" }), $n at a time"
        $mutantResults = @($todo | ForEach-Object -ThrottleLimit $n -Parallel {
            $ErrorActionPreference = 'Stop'
            ${function:Invoke-P9SSuite} = $using:fn
            $m = $_; $root = Join-Path $using:mdir $m.id
            $o = [ordered]@{ id = $m.id; suite = $m.suite; what = $m.what; killers = @($m.killers); verdict = 'ERROR'; failed = @(); note = '' }
            try {
                $copy = Join-Path $root 'kit'; New-Item -ItemType Directory -Path $copy -Force | Out-Null
                Get-ChildItem -LiteralPath $using:kitDir -Force | Where-Object { $_.Name -notin 'p9s-local.json', 'KIT-SHA256SUMS.txt' } | Copy-Item -Destination $copy -Recurse
                $f = Join-Path $copy $m.file
                if ($m.file -match '[\\/]|\.\.' -or -not (Test-Path -LiteralPath $f)) { throw "file $($m.file) not in the toolkit" }
                $t = [IO.File]::ReadAllText($f).Replace("`r`n", "`n")
                $c = ([regex]::Matches($t, [regex]::Escape([string]$m.find))).Count
                if ($c -ne 1) { throw "the text to replace occurs $c times in $($m.file)" }
                [IO.File]::WriteAllText($f, $t.Replace([string]$m.find, [string]$m.replace), [Text.UTF8Encoding]::new($false))
                $x = Invoke-P9SSuite -Pwsh $using:pw -KitDir $copy -Suite $m.suite -Dir $root -AllowedNotRun ($using:allow)[$m.suite]
                $o.failed = @($x.failed | ForEach-Object { ($_ -split ' ')[0] })
                $hit = @($o.failed | Where-Object { $m.killers -ccontains $_ })
                if ($x.exit -eq 1 -and $hit.Count) { $o.verdict = 'KILLED'; $o.note = "detected by $($hit -join ', ')" }
                else { $o.verdict = 'SURVIVED'; $o.note = "exit $($x.exit); failed: $($o.failed -join ', ')" }
            } catch { $o.verdict = 'ERROR'; $o.note = $_.Exception.Message }
            [pscustomobject]$o
        } | Sort-Object { $_.id.Substring(0, 2) }, { [int]($_.id -replace '\D', '') })
        foreach ($m in $mutantResults) {
            '   {0,-5} {1,-8} {2} ({3}){4}' -f $m.id, $m.verdict, $m.what, $m.note, $(if ($m.verdict -eq 'KILLED' -and @($m.failed).Count -gt 1) { '; all failing: ' + ($m.failed -join ', ') } else { '' })
            if ($m.verdict -ne 'KILLED') { $problems.Add("mutant $($m.id) $($m.verdict): $($m.note)") }
        }
        foreach ($p in @(Get-P9SMutantShardProblems ([string[]]@($todo | ForEach-Object { [string]$_.id })) $mutantResults)) { $problems.Add($p) }
        "== mutants: $(@($mutantResults | Where-Object verdict -eq 'KILLED').Count) of $($todo.Count) killed$(if ($shard) { " (shard $($shard.k)/$($shard.n) of $(@($def.mutants).Count))" })"
    }
}

# ---- the default state folder, after ------------------------------------------------------
$coordAfter = @(Get-DefaultCoordinatorFiles) -join '; '
if ($coordAfter -ne $coordBefore) { $problems.Add("the coordinator files of $defaultDir changed during the tests: $coordBefore -> $coordAfter") }
if ($CleanRunner -and (Test-Path -LiteralPath $defaultDir)) { $problems.Add("the tests created the default state folder $defaultDir") }

$summary = [ordered]@{
    schema = 'p9s-test-run/1'; environment = $envFacts; allowed_not_run = $allowedNotRun
    mutant_shard = $(if ($shard) { [ordered]@{ k = $shard.k; n = $shard.n; total = $shard.total; assigned = @($shard.assigned) } } else { $null })
    suites = @($suiteResults | ForEach-Object { [ordered]@{ suite = $_.suite; declared = $_.declared; passed = $_.pass; failed = $_.fail; not_run = $_.not_run; not_run_ids = @($_.not_run_ids); exit = $_.exit; seconds = $_.seconds } })
    mutants = @($mutantResults); problems = @($problems); ok = (-not $problems.Count)
}
$summary | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath (Join-Path $OutDir 'summary.json') -Encoding utf8
if ($env:GITHUB_STEP_SUMMARY) {
    $md = @('### P9-S Windows toolkit tests', '', "pwsh $($envFacts.pwsh), $($envFacts.os)", '', '| Suite | Declared | Passed | Failed | Not run | Exit |', '| --- | --- | --- | --- | --- | --- |')
    foreach ($x in $suiteResults) { $md += "| $($x.suite) | $($x.declared) | $($x.pass) | $($x.fail) | $($x.not_run) $(if ($x.not_run_ids) { '(' + ($x.not_run_ids -join ', ') + ')' }) | $($x.exit) |" }
    if ($Mutants) { $md += ''; $md += "Mutants killed: $(@($mutantResults | Where-Object verdict -eq 'KILLED').Count) of $(@($mutantResults).Count)$(if ($shard) { " (shard $($shard.k)/$($shard.n): $($shard.assigned.Count) of $($shard.total) assigned)" })" }
    $md += ''; $md += $(if ($problems.Count) { 'Result: FAILED' } else { 'Result: passed' })
    $md | Add-Content -LiteralPath $env:GITHUB_STEP_SUMMARY -Encoding utf8
}
if ($problems.Count) { "== RESULT: FAILED ($($problems.Count) problems)"; exit 1 }
'== RESULT: all checks passed'
exit 0
