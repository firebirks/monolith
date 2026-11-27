# P9-S host safety toolkit (Windows)

The Windows-host tools of the P9-S clean-machine laboratory (plan in
`docs/CLEAN_MACHINE_TEST.md`, results in `docs/CLEAN_MACHINE_RESULTS.md`).
Two virtual machines run Monolith over Tor on one Windows host. The
toolkit has three parts:

- a resource monitor that samples the host every 5 seconds and turns
  memory, paging and disk measurements into one safety state; it makes
  no VirtualBox call, so a hung or missing VirtualBox cannot stop it;
- a VM-state observer that asks VirtualBox for the state of one
  laboratory VM when the coordinator needs it, through one serialized,
  time-bounded query;
- an action coordinator, the single supported entry point through which
  laboratory automation changes the state of the VMs; it refuses to act
  unless the safety state and a fresh VM observation allow it.

The toolkit is not part of Monolith: nothing in `crates/` uses it, and it
does not touch Tor, Monolith or its profiles.

## What it is not

- Not a security boundary. It runs under the operator's own Windows
  account. That account can still run `VBoxManage`, use the VirtualBox
  window or edit the state files. The coordinator makes the approved
  automation fail closed; it does not prevent anything else, and it
  detects some changes made outside it only afterwards.
- Validated live only in part. The tests are offline: synthetic monitor
  states, mocked dispatchers, dry runs, a simulated calibration and a
  short real-sampling run of the monitor. One supervised session ran the
  monitor live and started and shut down `P9S-D12` once. Section "Not
  validated" lists what still needs a live run.

## Requirements

- Windows 11 and PowerShell 7.4 or later (`pwsh`). Windows PowerShell 5.1
  is not supported.
- VirtualBox 7 with `VBoxManage.exe` in its default folder under
  `%ProgramFiles%\Oracle\VirtualBox` (developed with 7.2). ACPI enabled
  in each VM (`acpi="on"` in `VBoxManage showvminfo`); Guest Additions are
  not needed.
- A standard user account: no elevation, no PowerShell module, no service
  or scheduled task.

## Files

| File | Purpose |
| --- | --- |
| `host-monitor.ps1` | The monitor: samples, policy, state file |
| `hostmon-lib.ps1` | Monitor policy, counter arithmetic, state file, reader |
| `assert-host-safe.ps1` | Read-only check of the state: exit 0 only when READY |
| `p9s-action.ps1` | The coordinator's entry point |
| `coordinator-lib.ps1` | Actions, checks, sessions, emergency path, calibration dry run |
| `p9s-vmstate.ps1` | VM-state observations: one query on request, its times, freshness, journal |
| `p9s-acpival.ps1` | The restricted single-VM ACPI validation of `P9S-D12` or `P9S-U24` (owner authorization required) |
| `p9s-vbox.ps1` | Every VirtualBox operation: one host-wide lock, bounded waits, quarantine, recovery |
| `p9s-config.ps1` | Local configuration and the local secret store |
| `p9s-state.ps1` | Read-only status of the persistent state folder (`state.status`), on which every action is gated |
| `p9s-state-init.ps1` | Explicit initialization of that folder (`state.init`), the anchor mark of every ledger save, and the rule that voids the ACPI validation authorizations issued before it |
| `p9s-state-migrate.ps1` | Explicit migration of the coordinator folder of earlier versions into that folder (`state.migrate`); the anchor written again for a consistent folder (`state.reanchor`) |
| `p9s-local.example.json` | Example of the local configuration |
| `acpi-validation-authorization.example.json` | Example of a version-1 authorization for `P9S-D12` (never valid as it is) |
| `acpi-validation-authorization-u24.example.json` | Example of a version-2 authorization for `P9S-U24` (never valid as it is) |
| `paging-thresholds.json` | Paging values: candidates, not approved |
| `com-type.ps1`, `vm-action.ps1` | Compatibility wrappers that call the coordinator |
| `tests/hostmon-tests.ps1` | Monitor tests |
| `tests/coordinator-tests.ps1` | Coordinator tests |
| `tests/stagec-tests.ps1` | Configuration, credentials, ACPI shutdown, calibration, publication checks |
| `tests/vbox-tests.ps1` | The VirtualBox layer: serialization, timeouts, crashes, recovery, coordinator use |
| `tests/decouple-tests.ps1` | The monitor without VirtualBox; the VM-state observer |
| `tests/acpival-tests.ps1` | The restricted ACPI validation: authorization, targets, budgets, recovery, races, damaged ledgers, low memory |
| `tests/persist-tests.ps1` | The state status: the 17 statuses and their order, access errors, junctions, hard links, ACLs, no side effects; `state.init` in process: refusals, supersede, the VMs, the clock, the authorization rule |
| `tests/setup-tests.ps1` | `state.init` in separate processes: every step as a crash point, the races |
| `tests/migrate-tests.ps1` | `state.migrate` in process: the laboratory shape, every shape it refuses, the readers and the audit witness, locks, VMs, free space, resume and restart, writers at a given step |
| `tests/migproc-tests.ps1` | `state.migrate` in separate processes: every step as a crash point, resume and restart crashes, the races, the coordinator's lock and hold against a migration |
| `tests/enforce-tests.ps1` | The persistent state as the coordinator enforces it: the anchor mark of every ledger save, its faults and crash points, the state changed under an action in process and from another process, `state.reanchor`, the earlier folder after a migration, STOP and ABORT across a restart, the real context and the monitor on the default location |
| `tests/run-all.ps1` | Runs and checks the eleven suites, and the mutants (local and CI) |
| `tests/mutants.json` | The targeted faults and the tests that must detect them |

## Local configuration

Copy `p9s-local.example.json` to `p9s-local.json` in this folder (or set
`P9S_HOST_CONFIG`, or pass `-ConfigPath`) and fill in:

- `vms.debian`, `vms.ubuntu`: the VirtualBox names of the two VMs
  (default `P9S-D12` and `P9S-U24` when no configuration exists);
- `external_disk`: drive letter, model name as Windows reports it
  (`Get-Disk`), and a folder on that disk whose presence proves the disk
  is attached;
- `secrets_dir`: the local secret store (see "Credentials");
- `evidence_dir`: where session outputs are kept (outside the repository);
- `vm_config_reference`: optional records of
  `VBoxManage showvminfo --machinereadable` against which the calibration
  preflight compares the VMs.

`p9s-local.json` is machine-local and ignored by Git. Without it the
monitor cannot confirm the external disk (every sample is UNKNOWN), no
secret can be read, and the preflight host checks stay PENDING. An invalid
configuration file leaves only the emergency and read-only actions
available.

## Credentials

The toolkit reads one kind of secret: the Monolith profile passphrase of a
VM, for `guest.unlock`. It is read from the file
`<secrets_dir>\<vm>-profile-passphrase.txt` (the last line that is not
empty and not a `#` comment) right before it is typed, and it is never
printed, logged, put in an error message or passed on a command line.
Before every read the toolkit refuses the store when:

- the path is relative, in a hidden (dot) folder, in the temporary folder
  or in application data;
- the folder inherits permissions from its parent, or anyone but the
  current account, SYSTEM and Administrators can read the folder or the
  file, or the owner is someone else;
- the file is a link or larger than 4 KiB.

Errors are codes (`SECRET_STORE_NOT_CONFIGURED`, `SECRET_STORE_EXPOSED`,
`SECRET_MISSING`, `SECRET_FORMAT`, ...).

Provisioning, once, by the owner of the laboratory:

    mkdir X:\P9S-Lab\lab-secrets
    icacls X:\P9S-Lab\lab-secrets /inheritance:r /grant:r "%USERNAME%:(OI)(CI)F" "SYSTEM:(OI)(CI)F"

then write each passphrase into its file with a text editor, never on a
command line. The files are plain text: on a removable disk anyone with
the disk can read them. Stronger options (a DPAPI-protected file, the
Windows Credential Manager, disk encryption) are possible and were not
adopted; that is the owner's decision.

Not supported: account and sudo passwords, the guest desktop login and any
privileged guest command. Earlier ad hoc laboratory scripts read such
passwords from local session logs; they are retired and not part of this
toolkit, and the coordinator cannot run them.

## The monitor

    pwsh -NoProfile -File host-monitor.ps1 -OutDir <new folder>
         [-Mode Monitor|Calibrate] [-PagingStop Required|NotRequired]
         [-DurationSeconds 14400] [-Popup]

It writes `run.json`, `samples.csv` (one line per sample), `events.log`,
`processes.csv` (image names only) and `run-end.json` into `-OutDir`, and
replaces `state.json` of the persistent state folder
(`<LocalApplicationData>\Monolith-P9S\state`, see "Persistent state
status") atomically after every sample. Without `-StatePath` it refuses
(exit 2) when that folder does not exist: `state.init` or `state.migrate`
creates it, the monitor never does. `-StatePath` names another state
file, whose folder it creates.
Create `<OutDir>\STOP` to end it; a clean end publishes STOPPED. One
monitor per state file.

The monitor makes no VirtualBox call of any kind: no COM object, no
`VBoxManage`, no VirtualBox lock, no look at the quarantine. VM states
are not part of its samples or of its state; the coordinator observes
them (see "VM states"). Its samples keep only a host figure: the number
of VirtualBox VM processes on the host, of any project, and their
working set, from the Windows process list (`vbox_vm_procs`,
`vbox_vm_ws_mb`). That is layout 2 of `samples.csv` (`samples_schema`
in `run.json`); layout 1, of toolkit `83c9ef8` and earlier, had the
VirtualBox states of the two VMs in the `d12_*` and `u24_*` columns.

The first sample of a run can take several seconds when Windows starts
its performance data provider on demand (7 s once on the laboratory
host); the next samples take a fraction of a second. A first sample
slower than 12 s breaks coverage like any other gap.

Approved limits (in `New-HostMonConfig`): a sample every 5 s; a gap above
12 s breaks coverage; 300 continuous seconds below 90 % commit before a
test; 90 % or more is a WARNING; 95 % for 30 continuous seconds is
STOP_REQUIRED; 97 % at any valid sample is ABORT_REQUIRED; paging windows
of 30 and 120 s. A missing external disk or a storage or USB error in the
System log is STOP_REQUIRED. STOP and ABORT stay latched until the monitor
is restarted, which starts a new 300-second qualification.

Paging: the rules need page-out, low or falling memory, page-file growth
or slow page-file disk reads; page-in alone never fires. The values in
`paging-thresholds.json` are candidates with `"approved": false`: they can
raise a WARNING but never a stop, and while the paging stop is required
and not approved the monitor reports UNCALIBRATED and never READY.

States: INITIALIZING, QUALIFYING, READY, WARNING, UNCALIBRATED,
CALIBRATING (`-Mode Calibrate`, never READY), UNKNOWN, STOP_REQUIRED,
ABORT_REQUIRED, STOPPED; a reader calls a state older than 12 s STALE.

    pwsh -NoProfile -File assert-host-safe.ps1 -Step "<next step>"

prints `HOST GATE OK` (exit 0) or `HOST GATE REFUSED` with every reason.

## The coordinator

    pwsh -NoProfile -File p9s-action.ps1 -Action <name> [-Vm <name>]
         [parameters] [-DryRun] [-ConfigPath <file>]

Exit 0 for DISPATCHED, DONE or DRYRUN, 1 for a refusal or an unfinished
outcome, 2 for FAILED. Next to the state file it keeps
`coordinator-session.json`, `coordinator.lock`,
`coordinator-emergency.hold`, `coordinator-stop.request` and
`coordinator-audit.jsonl`, and the VirtualBox layer keeps its files
there too (see "VirtualBox operations").

Start it only with `pwsh -File` (as above) or dot-sourced at a prompt.
Started with `&` from a prompt or from another script, its closures could
not find the toolkit's functions (the profile passphrase would not be
typed, the calibration host checks would fail); it then prints
`refused: UNSUPPORTED_INVOCATION`, exits 2, and does nothing: no state is
read, no file is written, no input is sent.

Test actions need a READY state with the paging stop enforced, an ACTIVE
test session opened on the same monitor run, both VMs in the state the
session expects, and a second check immediately before dispatch; long
actions check again between chunks or every 5 s.

| Action | What it does |
| --- | --- |
| `session.open -Kind Test` | Opens a test session on the current monitor run |
| `session.close` | Closes it; discards an in-flight record and the stop request once each VM it records a request for is seen off, also when there is no session record; refused while an emergency waits for the lock |
| `vm.start` | `VBoxManage startvm <vm> --type gui` |
| `guest.type -Text` | Letters, digits, space, `-` `.` `~` (em dash); 120 at most; never Enter |
| `guest.key -Key` | One of 18 named keys (Enter, Tab, CtrlQ, ...) |
| `guest.unlock` | Types the profile passphrase from the secret store |
| `scenario.begin`, `phase.advance` | Markers with a full check |
| `wait -Seconds`, `sequence.run -Plan` | Waits and plans of the actions above, checked at every step |
| `emergency.acpi-shutdown` | One ACPI power-button request, then waits for power-off |
| `emergency.wait-off`, `emergency.status` | Read-only |
| `session.status`, `audit.oob`, `audit.vmconfig` | Read-only |
| `vbox.status` | The VirtualBox quarantine, if any; no VirtualBox call |
| `vbox.recover -Operation <id>` | The operator's recovery of a quarantine (see "VirtualBox operations") |
| `calib.preflight`, `calib.dry-run` | Read-only preflight; simulated calibration |
| `acpi.preflight`, `acpi.open` ... `acpi.finalize` | The restricted ACPI validation (see below); owner authorization required |

STOP or ABORT seen in any monitor state halts the session for good:
queued plan steps are cancelled, nothing is retried. After STOP a new
session needs a new monitor run; after ABORT also `-AcknowledgeAbort <id>`
with both VMs off. A coordinator that dies during an action leaves an
in-flight record that blocks test actions until `session.close`.

Emergency shutdown (`emergency.acpi-shutdown`): available in every state,
including STOP, ABORT, UNKNOWN, low memory and with no monitor at all.
It writes a
stop request that every running action honours, waits up to 30 s for any
running action to stop (see "The emergency hold" below), sends one
`VBoxManage controlvm <vm>
acpipowerbutton` (the guest sees a power-button press) and watches the VM
state for up to 180 s. It reports success only when the VM is off. If the
request fails or the VM stays on, the emergency is UNRESOLVED: no second
request, no keyboard input and no forced power-off are attempted; the
operator decides, and the session cannot be closed while that VM runs.
The request is recorded before it is sent; while its end is not
recorded (its coordinator ended, or still watches it), the session
cannot be closed either until the VM is seen off or in the aborted VM
state. A monitor STOP or ABORT changes none of this. The emergency
cannot start a VM or send text.

It sends a request only when it can establish that none has been sent
to that VM: the stop request records each emergency request before it
is sent, and the validation ledger records the request of an
unresolved validation. Both are read again under the coordinator lock,
so that a request another process sent while this one waited is seen.
`session.close` reads the stop request, observes its VMs and removes it
under the same lock, so no request is recorded or sent between its
check and the removal. It does the same when there is no session record
(an emergency under a monitor that has not halted writes none): the
checks are the same and do not read the session record, a dry run
changes nothing, only the stop request the close read is removed, and
its audit record names the VMs that stop request recorded.
When either record cannot be read, is missing after use or does not
match (see "Uncertain request history"), the history is unknown and
nothing is sent (`SHUTDOWN_HISTORY_UNCERTAIN`). A request that cannot be
recorded first is not sent either (`ACPI_REQUEST_NOT_SENT`).

The emergency hold: an emergency takes `coordinator-emergency.hold`
(shared) before it writes or relies on the stop request and keeps it
until it has decided under the coordinator lock. `session.close`, under
the lock and after its checks, tries to take the file exclusively,
without waiting; while an emergency holds it the close is refused
(`EMERGENCY_IN_PROGRESS`), dry run included, and changes nothing, so a
close never removes the stop request an emergency waiting for the lock
raised or relies on. One deadline of 30 s covers the hold and the lock.
An emergency that cannot take the hold in time raises nothing and sends
nothing (`EMERGENCY_LOCK_TIMEOUT`); one that took it but not the lock
sends nothing and says whether the stop request is in place. Neither
retries. An emergency takes the hold before the lock and holds nothing
while it waits for the hold; the close tries the hold only while it
holds the lock, never waiting for it, so the two cannot deadlock. A
process that ends, also by a crash or a forced termination, loses its
handles. The hold file must stay on a local NTFS volume; if it cannot
be opened, emergencies and closes are refused until it can.

Audit records hold action names, reason codes, the monitor run and
sanitized parameters: text only as its length, paths as file names.

## VirtualBox operations

Every VirtualBox operation of the toolkit, read-only ones included, goes
through `p9s-vbox.ps1`:

- VBoxManage runs as a child process for four operations only, and only
  for the two configured VMs: `vm.state` and `vm.config` (`showvminfo`),
  `vm.start` (`startvm`) and `vm.acpi` (`controlvm <vm> acpipowerbutton`);
- the toolkit's only COM call, keyboard input to a guest, runs inside
  its process.

The resource monitor is not one of them: it makes no VirtualBox call
and never takes the lock below, so no VirtualBox operation can delay a
resource sample.

All of them take one host-wide named mutex,
`Global\P9S-VirtualBox-Management-v1`, the same whatever the folder,
process or VM, and hold it for the whole operation: a second operation,
in this or in another toolkit process, waits for it. The waits are
bounded:

| Limit | Default | Why |
| --- | --- | --- |
| Lock wait | 150 s | Longer than the longest command plus its confirmation, so that a healthy holder finishes, or quarantines, first |
| `vm.state`, `vm.config`, `vm.acpi` | 30 s | A query or a posted ACPI event normally takes seconds |
| `vm.start` | 120 s | `startvm` returns once the VM process runs |
| Confirmation after a timeout | 10 s | A late end of the command is still seen |

The defaults bound the waits; they are not shown to suit every condition
of VirtualBox.

Nothing is retried automatically and no process is ever terminated: no
VBoxManage, no VBoxSVC, no VM. A command that times out is not a success.
Whatever cannot be resolved writes `vbox-quarantine.json` next to the
coordinator files:

- the lock was not acquired in time (`VBOX_LOCK_TIMEOUT`);
- the command did not end in time: `VBOX_COMMAND_TIMEOUT` if it ended in
  the confirmation window, `VBOX_CHILD_UNRESOLVED` with its process id if
  it still runs, `VBOX_TERMINATION_UNVERIFIED` if its end cannot be
  confirmed;
- an operation was interrupted (`VBOX_OPERATION_CANCELLED`,
  `VBOX_OPERATION_INTERRUPTED`);
- an earlier owner ended without finishing: its operation record
  (`vbox-operation.json`) is still there and cannot exclude a command
  still running (`VBOX_API_STATE_UNKNOWN`, `VBOX_RECORD_INCOMPLETE`,
  `VBOX_RECORD_UNREADABLE`, `VBOX_CHILD_UNRESOLVED`,
  `VBOX_OPERATION_INTERRUPTED`), or the record belongs to a process that
  still runs (`VBOX_RECORD_STALE`). The record is the evidence: Windows
  reports the mutex as abandoned only while another process holds it
  open, and recreates it otherwise. An owner that ended before it started
  VBoxManage is the only case taken over without a quarantine; until then
  the coordinator refuses with `VBOX_OPERATION_UNFINISHED`.

The mutex is released only once the child has exited or the quarantine is
written and read back; if the quarantine cannot be written, the process
keeps the mutex until it ends. While a quarantine is in place every
VirtualBox operation is refused, in every toolkit process, and so are test
actions, sessions and ACPI requests: an emergency is recorded as
unresolved, and the coordinator claims neither that a request was sent nor
that a guest shut down. A missing `VBoxManage.exe` makes every VM state
`unreadable`, never `off`, without a quarantine.

Recovery, by the operator, once the cause is understood:

1. `p9s-action.ps1 -Action vbox.status` shows the quarantine, its
   operation id, any recorded VBoxManage process and whether it still
   runs; it makes no VirtualBox call.
2. If that process still runs, decide outside the toolkit (wait for it, or
   end it by hand knowing what it was doing); the toolkit never ends it.
3. `p9s-action.ps1 -Action vbox.recover -Operation <id>` (`-DryRun`
   first) refuses if the id does not match or the process still runs,
   then reads the state of each VM once, one query at a time, and clears
   the quarantine only if every query answers; a failed query leaves it.
   For an unfinished operation without a quarantine
   (`VBOX_OPERATION_UNFINISHED`), the id is the one of its record: the
   first `vbox.recover` judges it and quarantines it under the same id
   unless it never started VBoxManage; the second clears it.
4. An open test session is then refused (`SESSION_VBOX_RECOVERED`): close
   it and open a new one, which checks the monitor state and the VM states
   again.

The records and `vbox-audit.jsonl` hold operation names, VM names, codes,
times and process ids, never command lines, command output, paths or
secrets; a failed command appears as the length of its error output and
its VirtualBox error codes.

The lock covers the toolkit only. VBoxManage typed by hand, the VirtualBox
window, other programs and other projects' VMs are outside it. A COM call
made inside a toolkit process cannot be bounded in time: a hung one keeps
its process and the lock, and the other toolkit processes then time out
and quarantine; the resource monitor goes on sampling.

## VM states

The coordinator knows the state of a VM only from an observation: one
`showvminfo` through the VirtualBox layer (`p9s-vmstate.ps1`), made
when it needs one: a session opens, an action is checked before and
after its dispatch, an operator asks (`emergency.status`), a shutdown
is confirmed (one query every 5 s or more, up to a deadline). There is
no background loop and no cache, and nothing is observed because a
resource sample is due.

- An observation records when its query started, how long it took and
  why it failed, and goes to `vm-observations.jsonl` next to the
  coordinator files (VM, state, purpose, times, operation id; never
  command output).
- Anything other than a VirtualBox state name is `unreadable`: no
  `VBoxManage`, a refused, failed or timed-out query, a quarantine, an
  answer about another VM. It is never taken for `off`, and an older
  observation never stands in for a missing one.
- A decision that needs the current state (a start, the ACPI request)
  rejects an observation more than 10 s old or from the future.
- The state a session expects after a dispatch is an assumption;
  `session.status` shows it as such.

| Question | Answered by | VirtualBox |
| --- | --- | --- |
| Is the host under unsafe resource pressure? | the monitor (`state.json`) | never called |
| What was the last reliably observed state of a VM? | the observer, on request | one serialized query |

`audit.oob -MonitorRun <folder>` compares the VM state changes during a
monitor run with the coordinator's dispatches: for layout-2 runs it
uses the observations made during the run, which see a change only at
the next observation; for older runs the VM columns of `samples.csv`.

## Restricted ACPI validation

The normal gate needs approved paging values, and those values need a
calibration with running VMs, which in turn needs a VM shutdown that is
known to work. The `acpi.*` actions break that circle for one purpose
only: one supervised check that the native VirtualBox ACPI power button
shuts one laboratory VM down. It is a separate capability, not a test
mode, and it grants no exception to the normal gate.

Validated live once: on 2026-11-21 (commit `2f4c609`) one session
started `P9S-D12` once and sent it one ACPI request; the guest entered
the ACPI S5 state itself and was observed off 5.2 s after the request.
That says nothing about `P9S-U24`: its validation is prepared in this
version and has never run.

A session has exactly one target, the VM its authorization names, from
a closed table: `P9S-D12` (role `debian`) and `P9S-U24` (role
`ubuntu`). The target must also be the name the local configuration
gives that role; any other name, another project's VM included, is
refused. The target never changes after `acpi.open`: the session keeps
its copy of the authorization, its ledger entry names the same target,
file and session, and a session or ledger edited afterwards is refused
(`ACPI_TARGET_MISMATCH`, `ACPI_AUTHORIZATION_MISMATCH`).

Nothing live runs by default. The owner's authorization file is the
only way in: `acpi.open` without a valid file inside its time window,
and every later live step without a validation session bound to one in
its window, is refused before the coordinator lock, any VirtualBox
query or any dispatch (dry runs too). `acpi.preflight` reads no VM state
without a valid file in its window. Recovery stays available after the
window: `acpi.confirm-off`, `acpi.finalize`, `emergency.acpi-shutdown`.
Once `state.init` has made an installation of the persistent state, an
authorization whose `approved_utc` or `valid_from_utc` is not later than
its `initialized_utc` is refused (`AUTHORIZATION_BEFORE_INSTALLATION`),
at `acpi.open` and at every later step that checks the stored copy; an
installation whose state cannot be established refuses too
(`AUTHORIZATION_INSTALLATION_UNKNOWN`). See "Persistent state
initialization".
A context whose live switch is off (the tests use one) refuses every
live step with `ACPI_VALIDATION_LIVE_DISABLED` before anything is read.

| Action | What it does |
| --- | --- |
| `acpi.preflight [-Authorization <file>]` | Read-only: capability, authorization, ledger, monitor, VirtualBox; with a valid file in its window, one observation of its target and the target's configuration reference |
| `acpi.open -Authorization <file>` | Opens the validation session for the file's target; the authorization id is spent |
| `acpi.start` | The one start of the session's target |
| `acpi.observe -Seconds <n>` | Up to 600 s with a monitor check every 5 s and one observation at each end |
| `acpi.shutdown [-Seconds <n>]` | The one ACPI request, then one observation every 5 s up to the deadline (180 s, at most 600) |
| `acpi.confirm-off -Seconds <n>` | Read only: watches an unresolved validation again; sends nothing |
| `acpi.integrity [-Reference <file>]` | Read only: compares the configuration of the VM, observed off, with its reference |
| `acpi.finalize` | Records the outcome and closes the session |

None of them takes a VM name, text, keys, a plan or a command; the only
dispatches are one `startvm` and one `acpipowerbutton` of the session's
target. Normal test actions are refused in a validation session
(`SESSION_KIND_MISMATCH`), `session.close` sends it to `acpi.finalize`,
and no test session opens until it is finalized and resolved. An
authorization for either VM enables no T-CLEAN step and no READY.

The authorization is a file the owner writes outside this folder for
one session. Version 1 (`acpi-validation-authorization.example.json`)
is the format of the Debian session and can name `P9S-D12` only; its
memory field `provisional_min_available_mb` means MiB. Version 2
(`acpi-validation-authorization-u24.example.json`) can name either VM
and is required for `P9S-U24`: its id carries the target code
(`acpiauth-u24-YYYYMMDD-xxxxxxxx`), so that changing the target of a
file invalidates it, and its memory field is
`provisional_min_available_mib`. Neither example is ever valid as it
is. A file must name the purpose `ACPI_VALIDATION`, one target as a
single string, the permitted actions, one start, one ACPI request, a
window of at most 4 hours, the monitor conditions and preflight checks
of this version, its memory minimum, operator supervision, the stop
policy, the approval of that minimum, and who approved it and when. An
unknown, duplicated or misspelled field is refused, never ignored. The
coordinator keeps a copy in the session; editing the file later changes
nothing. An authorization id opens one session, ever, whatever its
target. The file is not a cryptographic proof: the same Windows
account, or software running as it, can write it. It records the
owner's decision for one session and makes its boundaries checkable; it
is a workflow control, not a security boundary.

No VirtualBox VM process of any VM (`VirtualBoxVM`, `VBoxHeadless`, from
the Windows process list) may run on the host when the session opens
and when the start is dispatched (`VM_PROCESS_RUNNING`): another
project's VM is never touched, the validation waits for the host to be
free of VMs.

The monitor must be a Monitor-mode run with the paging stop required,
fresh and alive, the 300-second commit qualification met, commit below
90 %, no WARNING (a suspected paging pressure with the candidate values
is one), UNKNOWN, STOP or ABORT, the external disk confirmed,
measurements complete and, before the start and while the VM is
observed, at least the memory minimum of the authorization available.
The monitor's `avail_mb` is the Windows `AvailableMBytes` counter, whole
MiB (2^20 bytes): 8192 means 8,589,934,592 bytes, not a decimal value.
The minimum guards what adds or prolongs workload: below it `acpi.open`
and `acpi.start` are refused and `acpi.observe` ends (it still reports
the VM state it read). It does not apply to observation or recovery:
`emergency.status`, `emergency.wait-off`, `session.status`,
`acpi.confirm-off`, `acpi.finalize`, the one ACPI request and
`emergency.acpi-shutdown` work below it. Low memory grants nothing: a
request the ledger does not show as unused stays refused. The code
refuses any minimum below 8192 MiB for either VM; that floor approves
nothing. The owner approved 8192 MiB for the Debian session only; for
`P9S-U24` the value and its approval belong to its own authorization.
Available memory alone is not a paging criterion. The coordinator calls
all this `ACPI_VALIDATION_ELIGIBLE`. It is not READY: the monitor still reports
UNCALIBRATED, nothing is written to its state, and every normal test
action stays refused. The minimum is a validation-only safeguard, not a
paging threshold and not part of the normal gate. The ACPI request
itself also accepts a WARNING or a broken qualification (it lowers the
load), never a STOP, an ABORT or a stale or dead monitor.

The budgets live in `acpi-validation-ledger.json` next to the
coordinator files, keyed by the authorization id, and are spent before
the dispatch: a crash, a restart or a lost session record cannot give
them back, an authorization opens one session only, and an outcome that
cannot be recorded leaves the earlier, unresolved phase. A new session
adds its own entry and never changes an earlier one: the entry of the
Debian session stays as it was recorded. An unresolved validation of
either VM blocks a new validation of both.

The ledger is never taken for empty when it cannot be trusted. Absent,
it is empty only on a host where no validation has used one (no
revision record, no validation session record); absent after use is
`ACPI_LEDGER_MISSING`. Locked, empty, truncated, not JSON, with a
duplicated name, another schema, an entry without its id, a duplicated
id, or a budget count that is not a whole number of 0 or more, it is
`ACPI_LEDGER_UNREADABLE`. Each save raises its revision by one, reads
the file back, then writes `acpi-validation-ledger.revision.json` (the
revision and the SHA-256 of the text). A ledger older than that record,
such as a restored copy, is `ACPI_LEDGER_STALE`; one of the same
revision with other content, or an unreadable record, is
`ACPI_LEDGER_INCONSISTENT`. A ledger written before the record existed
(the Debian session's) is read as it is until its next save. This
guards against accidents (a failed or partial write, a deleted file, a
restored backup), not against someone who edits both files under the
same Windows account.

| Phase | Meaning | What is possible next |
| --- | --- | --- |
| `OPEN` | Session open, nothing started | `acpi.start`, `acpi.finalize` |
| `STARTING` | Start dispatched, outcome not recorded (the coordinator ended) | emergency path, `acpi.confirm-off`, `acpi.finalize` |
| `RUNNING` | Start done | `acpi.observe`, `acpi.shutdown` |
| `START_UNCERTAIN`, `START_FAILED` | Start failed; VM state unknown, or observed off | emergency path, `acpi.confirm-off`, `acpi.finalize` |
| `SHUTDOWN_REQUESTING` | Request recorded, its dispatch never confirmed (the coordinator ended); kept until the VM is seen off | `acpi.confirm-off`, `acpi.finalize` |
| `SHUTDOWN_REQUESTED` | Request sent, guest watched | `acpi.confirm-off` |
| `OFF_CONFIRMED` | Observed off within the deadline | `acpi.integrity`, `acpi.finalize` |
| `ACPI_VALIDATION_UNRESOLVED` | Still running at the deadline, state unreadable, request failed, or VirtualBox in doubt | `acpi.confirm-off`; the operator decides |
| `OFF_AFTER_DEADLINE`, `OFF_REQUEST_UNCONFIRMED`, `OFF_WITHOUT_ACPI`, `OFF_ABNORMAL` | Off, but late, after an unconfirmed request, without the request, or aborted | `acpi.finalize` |

Success is a power-off observed after the request, never the request
itself. When the guest does not power off:

- the request failed: unresolved; no second request, no keyboard
  input, no forced power-off;
- the guest stays on past the deadline, or its state is unreadable:
  unresolved; `acpi.confirm-off` can watch again, read only;
- VirtualBox becomes quarantined: unresolved; nothing is claimed about
  the guest; `vbox.status` and `vbox.recover` as usual;
- STOP or ABORT: the session halts; the validation request is refused
  and `emergency.acpi-shutdown` remains, which sends one request only
  if the validation has not sent its own, and counts it;
- the host is at risk: the operator acts outside the coordinator; no
  STOP or ABORT is held back to finish the validation.

An unresolved validation blocks every new validation and every test
session until `acpi.confirm-off` observes the VM off; `acpi.finalize`
can close it as `ACPI_VALIDATION_UNRESOLVED`, which keeps that block.
A finalized validation is not finalized again: when a later emergency
leaves its session record open, `acpi.finalize` closes the record and
keeps the entry as it is.
Outcomes: `VALIDATED`, `SHUTDOWN_LATE`, `OFF_AFTER_UNCONFIRMED_REQUEST`,
`OFF_AFTER_EMERGENCY_ACPI`, `ENDED_WITHOUT_ACPI`, `OFF_ABNORMAL`,
`START_FAILED`, `NOT_STARTED`, `ACPI_VALIDATION_UNRESOLVED`.

### Uncertain request history

| Kind | Actions | Requires |
| --- | --- | --- |
| Normal validation | `acpi.open` ... `acpi.finalize` | A valid authorization, a readable ledger whose entry matches the session, and the monitor conditions above |
| Emergency observation | `emergency.status`, `emergency.wait-off`, `session.status` | Nothing more: available with a damaged ledger, low memory, STOP, ABORT, or a stale or missing monitor; grants no ACPI request |
| Emergency ACPI dispatch | `emergency.acpi-shutdown` | A readable stop request and ledger, no request recorded for the VM, the VM observed running, VirtualBox not quarantined, the coordinator lock; the request recorded before it is sent |
| Operator-assisted recovery | Outside the coordinator | Whenever the request history is uncertain |

When the ledger is missing after use, unreadable, stale or
inconsistent, when a ledger entry no longer matches its validation
session, or when the stop request cannot be read, the toolkit cannot
tell whether an ACPI request was sent. It reports
`SHUTDOWN_HISTORY_UNCERTAIN` with the reason, and then:

- no ACPI request is sent, by the validation or by the emergency path,
  to either VM; no keyboard input, no forced power-off, no restart of
  VirtualBox;
- the emergency path reads the VM state once (read only), records a VM
  not seen off as unresolved in the stop request when that can be read,
  and marks the session `EMERGENCY_UNRESOLVED`; the monitor keeps
  measuring;
- the files are kept as they are: no action repairs, resets or
  recreates the ledger or the stop request, and `session.close` keeps an
  unreadable stop request;
- `emergency.status` names the reason and the size, write time and
  digest prefix of each ledger file, and the audit records the same;
- every new validation and every test session stays refused.

Recovery is the operator's, outside the coordinator, under a separately
authorized decision: the VM window shows whether the guest is still on,
and only a restored, readable history lets the toolkit act again. A VM
observed off does not restore a missing history, and the toolkit has
no command that resets a ledger.

What follows from these rules:

- an unreadable ledger is not an unused budget;
- a record that exists but cannot be read (access denied, held by
  another program, not UTF-8, a folder in its place) is unreadable,
  never missing: the toolkit opens its records instead of testing
  whether they exist, because an existence test answers false for a
  file whose attributes cannot be read; the checks of the stop request
  and of the VirtualBox quarantine give the same answer whatever
  `$ErrorActionPreference` the caller set;
- low memory blocks new workload, not observation;
- low memory does not authorize a second shutdown request;
- a second ACPI request is never an automatic fallback;
- a failed ledger write is no proof that a request was sent, nor that
  it was not: a budget spent in a ledger that reached the disk stays
  spent;
- a completed ACPI command is no proof that the guest powered off.

## Persistent state status (read only)

The coordinator keeps its files in a folder of the account's profile,
created only by an explicit initialization or migration and recorded by
an anchor in the registry. Earlier versions kept them in
`%TEMP%\p9s-hostmon`, where Windows cleanup can delete them (R14). The
status of the folder decides what the coordinator may do (see
"Persistent state enforcement"):

    pwsh -NoProfile -File p9s-action.ps1 -Action state.status

It reads, and nothing else: it creates no file, folder or registry key,
takes no lock and writes no audit record.

What it reads:

- the state folder `<LocalApplicationData>\Monolith-P9S\state`, where
  LocalApplicationData comes from the Windows known folder API
  (`SHGetKnownFolderPath`), never from the `LOCALAPPDATA` variable, and
  must agree with .NET's value;
- the anchor, the value `StateAnchor` of `HKCU\Software\Monolith-P9S`
  (one JSON document), opened read only;
- the install, migration and ledger records of the state folder; the
  ledger is first read by the toolkit's own reader, and must then be no
  older than the anchor's mark (revision and SHA-256);
- the folders of earlier versions, `%TEMP%\p9s-hostmon` through `TMP`
  and the same name under the known folder's `Temp`, by entry name only
  unless a migration manifest has to be compared;
- the process: its account, whether it has a package identity
  (`GetCurrentPackageFullName`), runs in an app container or with UAC
  virtualization (token information), or starts from `WindowsApps`; any
  of these turns some writes into writes to a private copy;
- for each path component, the state folder's entries and their records:
  reparse points, the final path of a handle (`GetFinalPathNameByHandle`),
  hard links (`GetFileInformationByHandle`), the owner and write entries
  of the ACL, and whether the folder can be listed.

The statuses, in the order in which the first match wins:
`SANDBOX` (a `-StatePath` folder other than the state folder),
`LOCATION_UNSAFE`, `ANCHOR_UNREADABLE`; without an anchor
`ANCHOR_MISSING`, `SETUP_LEFTOVER`, `INSTALLATION_LOST`,
`MIGRATION_SOURCES_AMBIGUOUS`, `MIGRATION_REQUIRED`, `NOT_INITIALIZED`;
with one `INTERRUPTED`, `ACCOUNT_MISMATCH`, `STATE_MISMATCH`,
`STATE_MISSING`, `STATE_INCOMPLETE`, `STATE_UNCERTAIN`,
`STATE_DIVERGED`, `READY`. `READY` needs positive and consistent
evidence: an anchor in phase READY for this account and this folder,
the install record of the same installation, the ledger as the anchor
marks it, and for a migrated installation a manifest that matches the
anchor and an earlier folder that still matches the manifest.

Two rules hold throughout. A missing file never proves that nothing
existed: an earlier-version folder with any coordinator file gives
`MIGRATION_REQUIRED`, never `NOT_INITIALIZED`. An access error is never
read as a missing file: a folder, record or anchor key that cannot be
read gives a status that is not `READY` and names the reason. A check
that cannot be made (no native call, an unknown package state, an
unreadable volume) is not `READY` either.

What it cannot do, by design:

- a state folder and an anchor lost together look exactly like an
  account where the toolkit never ran (`NOT_INITIALIZED`); no reading
  can tell them apart;
- each Windows account has its own folder and anchor; a second account
  does not share the first one's budget;
- an earlier version of the toolkit uses `%TEMP%\p9s-hostmon`; this
  status sees that folder, and a migration leaves a tombstone there that
  the earlier versions with the VirtualBox layer honour, but nothing here
  stops a version that predates that layer (see "Earlier versions after
  a migration").

## Persistent state initialization

    pwsh -NoProfile -File p9s-action.ps1 -Action state.init [-DryRun]
    pwsh -NoProfile -File p9s-action.ps1 -Action state.init -Supersede <installation id>

creates the state folder of a new installation. It is the only way to
one: no other action initializes, repairs or recreates state, and an
absent or empty folder is never taken as permission. It takes no
parameter but `-Supersede`, `-DryRun` and `-ConfigPath`. Every other
action of the coordinator runs on that folder and needs it READY (see
"Persistent state enforcement").

It decides on the status of `state.status`, computed before anything is
written and again under the setup lock:

- `NOT_INITIALIZED`: a new installation;
- `INTERRUPTED` with the anchor in phase INITIALIZING: the same
  initialization, resumed (the same installation id);
- `INSTALLATION_LOST`, `STATE_MISSING`, `STATE_INCOMPLETE`,
  `STATE_MISMATCH`, `STATE_UNCERTAIN`, `STATE_DIVERGED`: only with
  `-Supersede` naming the installation the evidence names (the anchor's
  id, or the id of a migration marker);
- anything else is refused, with the status as the reason
  (`STATE_NOT_NEW:<status>`, `NOT_SUPERSEDABLE:<status>`,
  `NOTHING_TO_SUPERSEDE:<status>`): an installation in place, an
  earlier coordinator folder with evidence (`MIGRATION_REQUIRED`:
  `state.migrate` migrates it), an unreadable anchor, an unsafe location, the
  leftovers of a setup. A state folder already there without an anchor
  is refused too (`STATE_FOLDER_EXISTS`).

Every configured VM must be off. The count of VirtualBox VM processes
(the Windows process list, no VirtualBox call) is checked before
anything is written; the VM states are read through the VirtualBox layer
once the staging folder exists, one query per VM, and the layer's
records and the observations are written into that folder, where they
become the first history of the new installation. A VM in any state but
off (running, paused, saved, aborted, unreadable) refuses. A VirtualBox
operation or quarantine record in any folder under the root, or in an
earlier coordinator folder, refuses every run until the operator
resolves it: a new folder would not see it. The tombstone a migration
leaves in an earlier folder is no such record.

Steps, each a point where the process may end:

1. the root `<LocalApplicationData>\Monolith-P9S`, created with the
   protected ACL (the account, SYSTEM and Administrators; nothing
   inherited), and `setup.lock`, held exclusively, no wait
   (`SETUP_BUSY`);
2. with `-Supersede`, the anchor value it replaces copied to the value
   `Superseded-<operation id>` of the same key; then the anchor in phase
   INITIALIZING with a new installation id, written in one value and
   flushed;
3. the staging folder `state.staging-<operation id>-<n>`, created with
   the protected ACL; it must not exist;
4. the VM states, observed into it;
5. with `-Supersede`, the state folder renamed to
   `state.superseded-<operation id>`, only now that the VMs are off;
6. `install.json` (exclusive create, written through), the empty ledger
   at revision 1 and its revision record by the toolkit's save (each
   file written beside its name, then renamed with write-through);
7. the ACL of the staging folder and of each file checked;
8. the staging folder renamed to `state`, refused if `state` exists;
9. the folder checked against what this run wrote (install record, the
   empty ledger and its record, no other record, the location), the
   anchor checked unchanged, then the anchor in phase READY with
   `initialized_utc` and the ledger's mark; the result is `READY` only if
   `state.status` says so;
10. the first line of the new folder's `coordinator-audit.jsonl`.

An interruption before step 9 leaves `INTERRUPTED` (or, before step 2,
`NOT_INITIALIZED`), never `READY`; the same command completes it. A
staging folder left by an interruption stays where it is; the next run
uses a new one. Nothing is ever deleted: a superseded folder is renamed
aside, a replaced anchor value is kept beside it, the earlier coordinator
folder is not touched.

Time. `created_utc` and `initialized_utc` are written in UTC with seven
decimals (100 ns). A run refuses (`CLOCK_BEHIND_EVIDENCE`) when the clock
is earlier than a time the evidence holds: the anchor's `created_utc` or
`initialized_utc`, the install record's `created_utc`; `initialized_utc`
is never earlier than `created_utc`. Once an installation is READY,
`acpi.open` and every later validation step refuse an authorization
whose `approved_utc` or `valid_from_utc` is not strictly later than
`initialized_utc` (`AUTHORIZATION_BEFORE_INSTALLATION`): times are
compared as instants in UTC at the precision they are written with (a
time without decimals is the start of its second), and equal is not
later. A permit issued, or valid, before an installation can therefore
never be used after it, whatever was lost; one that is still to be used
must be written again. The rule does not depend on the clock at the
time of use, so a restart changes nothing. Without an installation (no
anchor and nothing under the root but the setup lock) no permit is
voided; an installation whose state cannot be established (an
unreadable anchor, one in phase INITIALIZING, a folder without its
anchor, an access error) refuses every authorization
(`AUTHORIZATION_INSTALLATION_UNKNOWN`).

What it does not do: it migrates nothing (`state.migrate`, below), and a
superseding initialization starts a history that knows nothing of the one it
replaces except its id; the VMs being off and every earlier permit being
void are what keep a lost history from giving a budget back.

## Persistent state migration

    pwsh -NoProfile -File p9s-action.ps1 -Action state.migrate [-DryRun]
    pwsh -NoProfile -File p9s-action.ps1 -Action state.migrate -Restart <operation id>

creates the state folder of a new installation from the coordinator
folder of earlier versions (`%TEMP%\p9s-hostmon`), which it copies and
never changes but for two new files, the marker and the tombstone. It is
the only way to migrate: no other action migrates,
nothing migrates by itself, and it never starts a new history over an
empty or new-looking folder. It takes no parameter but `-Restart`,
`-DryRun` and `-ConfigPath`. `-DryRun` reads the earlier folder as the
migration would, prints every file it would copy with its length and
SHA-256, the records it lacks, the files it ignores and the manifest
digest, and writes nothing: it takes no lock and creates no file, not
even a lock file.

It decides on the status of `state.status`, computed before anything is
written and again under the setup lock:

- `MIGRATION_REQUIRED`: a new migration of the one earlier folder that
  holds evidence;
- `INTERRUPTED` with the anchor in phase MIGRATING: the same migration,
  resumed, only while the earlier folder is still exactly what its
  manifest lists (once the state folder is in place, while no record of
  the earlier folder differs from the copy);
- `-Restart <operation id>`: only `INTERRUPTED` with that migration's
  anchor, no state folder in place and no marker written; the same
  migration (same operation and installation id) started again with a
  new manifest, for an earlier folder that changed while the migration
  was interrupted. The anchor value it replaces is first copied to the
  value `MigrationRestarted-<operation id>-<n>`;
- anything else is refused (`NOT_MIGRATABLE:<status>`,
  `NOTHING_TO_RESTART:<status>`, `RESTART_ID_MISMATCH`,
  `RESTART_NOT_APPLICABLE:<why>`), `NOT_INITIALIZED` included: an earlier
  folder holding only the monitor's files, or no earlier folder, is
  nothing to migrate, and this command never initializes.

The earlier folder is one of the two locations `state.status` computes,
never a path read from a record. It is refused as it is, before anything
is written, when it or an entry is a junction or other reparse point, an
entry is a folder, a hard-linked file or a name the toolkit does not
write (`ACPI-VALIDATION-LEDGER.JSON.tmp`, for example), a record or a
history cannot be read, the folder cannot be listed, or it holds a
VirtualBox operation or quarantine record (resolve it first with the
toolkit that wrote it). The monitor's files and the temporary files of
an unfinished atomic write are ignored and listed; `coordinator.lock`
and `coordinator-emergency.hold` are taken, not copied. Write entries
of other accounts on the earlier folder or its files are reported in the
dry run, in the result and in the audit line, not refused: the folder's
ACL is never changed, the copies get the protected ACL of the state
folder, and a writer outside the toolkit is caught by the second reading
before the rename and, afterwards, by `state.status`.

The history must be established before anything is copied:

- the toolkit's own readers: the ledger with its revision record
  (`ACPI_LEDGER_UNREADABLE`, `MISSING`, `STALE` and `INCONSISTENT`
  refuse), the stop request and the session record (unreadable refuses),
  the request history of every configured VM (uncertain refuses);
- the coordinator audit as a second witness: every complete line must be
  one JSON object (the last line may be cut by an interrupted append; it
  is copied as it is and the next line starts on its own line); a
  validation step recorded while the ledger reads "absent, never used"
  (`LEDGER_USED_BUT_MISSING`), or an emergency ACPI request since the last
  `session.close` with no stop request, or with one that does not hold
  its VM (`STOP_REQUEST_USED_BUT_MISSING`), refuses;
- a folder whose ledger never existed gets an empty ledger at revision 1
  only when its audit shows its history: no audit, an empty one or one
  with no complete line refuses (`ACPI_HISTORY_UNVERIFIABLE`). The lock
  files of an earlier coordinator alone are therefore never migrated into
  a new budget;
- the readers' verdict must be about the bytes of the manifest: the
  folder is read again after them, and any difference refuses.

Every configured VM must be off. The count of VirtualBox VM processes is
checked before anything is written; the VM states are read through the
VirtualBox layer once the copies are in the staging folder, and the
observations continue the copied histories. Free space on the state
volume must be at least twice the bytes copied plus 64 MiB.

Steps, each a point where the process may end:

1. the root, created with the protected ACL, and `setup.lock`, held
   exclusively, no wait (`SETUP_BUSY`); the decision taken again;
2. the earlier folder's `coordinator.lock` and `coordinator-emergency.hold`,
   both opened exclusively without waiting (`MIGRATION_SOURCE_BUSY`: a
   coordinator of any version acts there; `EMERGENCY_IN_PROGRESS`: an
   emergency holds the hold) and held to the end, so that no coordinator
   can change the folder meanwhile; the folder read again under them; an
   absent lock file is created empty, as every coordinator action does;
3. the anchor in phase MIGRATING: a new installation id, the computed
   earlier folder, the manifest digest;
4. the staging folder `state.staging-<operation id>-<n>`, protected,
   on the volume of the state folder;
5. each record and history: read, checked against the manifest, written
   beside its name, renamed without replacing, read back and compared;
6. the VM states, observed into it;
7. the empty ledger, only for a folder that never had one (above);
8. `migration.json` (the manifest, the absent records, the ignored files,
   whether the ledger was created, the VM observations) and
   `install.json` (origin migrated), each by exclusive create, written
   through;
9. the staging folder checked (each record byte for byte, each history
   beginning with the copied bytes, no record the manifest lists as
   absent, the ledger as the toolkit reads it, the two records of this
   migration, the protected ACL, plain unlinked files, nothing else), then
   the earlier folder against the manifest once more;
10. the staging folder renamed to `state`, refused if `state` exists;
11. `MIGRATED-TO.json` in the earlier folder, by exclusive create: the
    installation id, the state folder and the manifest digest;
12. `vbox-quarantine.json` in the earlier folder, by exclusive create,
    written through: the tombstone (see "Earlier versions after a
    migration"); one already there must be this migration's
    (`MIGRATION_TOMBSTONE_CONFLICT`);
13. the state folder checked again (the monitor's files there are
    ignored: a monitor may run on the folder once it is in place), the
    earlier folder again (a record that differs from its copy refuses; a
    history that grew is noted; the tombstone is no change),
    the anchor checked unchanged, then the anchor in phase READY with
    `initialized_utc` and the ledger's mark (the laboratory's version 1
    ledger, with no revision field, is revision 0 with the digest of its
    text); the result is `READY` only if `state.status` says so;
14. the line `state.migrate DONE` in the new folder's
    `coordinator-audit.jsonl`.

An interruption before step 13 leaves `INTERRUPTED` (or, before step 3,
`MIGRATION_REQUIRED`), never `READY`, and every ACPI validation
authorization is refused meanwhile (`AUTHORIZATION_INSTALLATION_UNKNOWN`);
the same command completes it. A staging folder left by an interruption
stays where it is; the next run uses a new one. Nothing is ever deleted:
the earlier folder keeps every file as it was and gains only the marker
and the tombstone (and the empty lock files if they were not there). Once READY, the rule
of `state.init` applies: an authorization approved or valid from before
`initialized_utc` is refused.

After the migration `state.status` compares the earlier folder with the
manifest at every start: a record new or changed there gives
`STATE_DIVERGED` (refused; `state.init -Supersede <id>`, or put the
earlier folder back as `migration.json` lists it); a history that grew
is a note; the folder deleted is a note (`OLD_FOLDER_GONE`); the
tombstone missing is a note (`OLD_FOLDER_TOMBSTONE_ABSENT`: a migration
made before the tombstone existed, or one removed by hand), and another
installation's tombstone or a real quarantine record in its place gives
`STATE_DIVERGED`.

## Persistent state enforcement

Every action of the coordinator runs on the persistent state folder, and
its status decides what may run:

- READY: every action;
- any other status: only the observe actions (`session.status`,
  `audit.oob`, `vbox.status`, `calib.dry-run`, `emergency.status`,
  `audit.vmconfig`, `calib.preflight`, `acpi.preflight`). They then write
  nothing (no audit line, no VM observation) and make no VirtualBox call:
  the VM states read as unknown and the host checks of the calibration
  preflight are not run. Every other action is refused before anything is
  read or written, dry runs included, with `STATE_NOT_READY:<status>` and
  the status's reasons. `emergency.acpi-shutdown` then says to shut the VM
  down from the guest or from VirtualBox: outside READY the ACPI request
  history is not known, and nothing is raised, written or sent;
- `state.status`, `state.init`, `state.migrate` and `state.reanchor`
  decide on the status themselves.

A status that cannot be computed (no classifier loaded, an exception, no
answer) refuses like any status but READY. The status is computed when the
action starts; again under the coordinator lock, before the session, the
monitor or the ledger is read for a decision; at each checkpoint of a long
action (`guest.type` between chunks, `wait`, `sequence.run` between steps,
`acpi.observe`); and once more immediately before every VirtualBox write:
every dispatcher of the coordinator and of the ACPI validation is called
through one function that checks it first. Each check needs READY for the
installation the action started on (`STATE_INSTALLATION_CHANGED`
otherwise). A change found under the lock refuses with nothing written; at
the last check before a dispatch it refuses as
`STATE_CHANGED_BEFORE_DISPATCH`; at the dispatch itself the dispatch fails
unsent, and a budget recorded before it stays spent: the start or the
request of a validation in the ledger, the request of an emergency in the
stop request. A refusal decided after the action started is recorded in
the audit of the folder it started on.

`-StatePath` names another state file, for tests and dry runs. Its folder
is a sandbox (status SANDBOX, decided without reading the persistent
state): every action that writes to a VM or sends a live ACPI validation
request (`vm.start`, `guest.*`, `sequence.run`, `emergency.acpi-shutdown`,
`acpi.open` to `acpi.finalize`) is refused there (`STATE_SANDBOX`) unless
it is a dry run, the dispatchers are replaced by ones that refuse, and the
ledger has no anchor mark. A sandbox ledger, an authorization used or
copied there, or a state folder copied there therefore gives no budget
that reaches a VM. The other actions run in the sandbox as before.
`-StatePath` naming the persistent state file itself is no sandbox.

Without `-StatePath` the coordinator never creates the state folder, and
the monitor refuses to start without it.

The ledger and the anchor mark. The anchor records the ledger's mark, its
revision and the SHA-256 of its text. Every save of the ledger, under the
coordinator lock, writes in this order: the ledger (beside its name, then
renamed), read back and compared; its revision record; then the anchor
with the new mark, in one registry value, flushed and read back. The save
succeeds only once the mark is in place, and nothing is dispatched after a
save that failed. Windows gives no atomicity across the file system and
the registry: an interruption between the ledger and the anchor leaves
the anchor behind the ledger, which `state.status` accepts, since the
mark is a lower bound. The anchor is never written ahead of a ledger that
is not on disk, and the mark only moves forward, on an anchor in phase
READY of this account and this folder: a lower revision, other content
for the same revision, an anchor in another phase, of another folder or
of another account takes no mark. A ledger older than the mark (rolled
back, replaced by an older copy), one with other content at the mark's
revision, or one that is gone gives `STATE_UNCERTAIN`: no budget is given
back; the way out is to put the newer ledger back, or
`state.init -Supersede`.

## Writing the anchor again

    pwsh -NoProfile -File p9s-action.ps1 -Action state.reanchor [-DryRun]
    pwsh -NoProfile -File p9s-action.ps1 -Action state.reanchor -Replace

writes the anchor again for a state folder that is in place and
consistent, when the anchor is gone (`ANCHOR_MISSING`) or, with
`-Replace`, when it cannot be parsed (`ANCHOR_UNREADABLE`; its text is
first kept, base64, in a line of the folder's `coordinator-audit.jsonl`).
Every other status refuses (`NOT_REANCHORABLE:<status>`):
`STATE_UNCERTAIN` above all, since marking an uncertain ledger again would
accept a rollback. It takes no parameter but `-Replace`, `-DryRun` and
`-ConfigPath`.

The folder must hold the install record of this account and folder, a
ledger the toolkit's reader establishes, and for a migrated installation
its migration record, with the earlier folder still matching it
(`STATE_FOLDER_NOT_CONSISTENT` otherwise). Every configured VM must be
off, as for `state.init`: the VM process count first, then one query per
VM, observed into the state folder. The decision is taken again under
`setup.lock`. The new anchor takes the installation's id, origin and
`created_utc` from the install record, the ledger's mark as it is, and a
new `initialized_utc`: every ACPI validation authorization approved or
valid from before it is void. One audit line records it.

## Earlier versions after a migration

`state.migrate` leaves two files in the earlier folder: the marker, which
only `state.status` reads, and the tombstone, `vbox-quarantine.json` with
the schema of the VirtualBox layer's quarantine, reason `STATE_MOVED` and
the operation id `state-moved-<installation id>`. Every earlier version
with that layer (from `83c9ef8`) reads any record there as a quarantine:
it starts no VM, sends no key, opens no ACPI request and starts no
`VBoxManage` from that folder, its emergency included, and its
`vbox.recover` cannot clear it (the id is not one of its operation ids).
Such a version can still write its own files there; `state.status`
reports a changed record as `STATE_DIVERGED`, and the coordinator then
refuses everything but the observe actions.

What the tombstone does not cover, and why no live action is authorized
by this version alone:

- versions before `83c9ef8` (the kits of 2026-11-19, for example) have no
  VirtualBox layer and ignore it;
- an earlier version started after Windows cleanup emptied `%TEMP%`
  creates a new `p9s-hostmon` folder without the tombstone, with a new
  budget;
- an earlier version started with `-StatePath` on another folder has a
  budget of its own there; its emergency needs no permit.

The cutover therefore also needs every earlier kit removed from the host
and no earlier version running; nothing in this version can enforce that.

## Calibration (preparation only)

A calibration measures the host with both VMs and both Tor environments
running and nothing else; it is not an acceptance test. This version has
no live calibration: `session.open -Kind Calibration` is refused unless
`-DryRun` is given, and no file or setting enables it.

    pwsh -NoProfile -File p9s-action.ps1 -Action calib.dry-run -Scenario nominal
    pwsh -NoProfile -File p9s-action.ps1 -Action calib.preflight

The dry run plays the planned workflow against the real monitor policy
with synthetic samples (scenarios: `nominal`, `warning-during-start`,
`paging-warning`, `low-memory`, `missing-counters`,
`abort-during-observe`, `monitor-mode`, `stale-monitor`, `acpi-timeout`):

1. preflight: monitor in Calibrate mode, fresh, no warning, commit below
   90 %, 300-second gate met with the VMs off, measurements complete;
2. start the Debian VM, settle 120 s;
3. operator step: start or confirm its system Tor service;
4. at least 8 GiB available (a proposal, not approved), then start the
   Ubuntu VM, settle 120 s;
5. operator step: log in, start Tor Browser, press Connect;
6. 300 continuous seconds below 90 % with both Tor environments up,
   within 900 s;
7. up to 1800 s of observation;
8. ACPI shutdown of both VMs.

A check runs every 5 seconds. Any WARNING (commit, suspected paging
pressure, missing measurements), UNKNOWN or latch ends the calibration
and leaves the ACPI shutdown. The toolkit sends no keyboard or mouse input
during a calibration; it never starts Monolith, sends messages, transfers
files, interrupts Tor, changes VM settings or restores snapshots.

A future live calibration needs the owner's authorization and a version
that implements the live steps. A proposal for that boundary: an
authorization record naming scope, VMs, monitor, limits, expiry, operator
involvement and stop conditions, signed with the owner's key and verified
against a signer list kept in the repository. Even then the same Windows
account could bypass it; it guards against mistakes, not against a
determined local user.

## Tests

    pwsh -NoProfile -File tests/run-all.ps1 -OutDir <new folder>
         [-Mutants [-MutantShard k/n]]

runs the eleven suites (`hostmon`, `coordinator`, `stagec`, `vbox`,
`decouple`, `acpival`, `persist`, `setup`, `migrate`, `migproc`,
`enforce`), each in a new `pwsh -File` process, and judges
them by their `results.json`:
every declared case must be reported once, as PASS, FAIL or NOT_RUN; the
exit code of a suite must agree with its results; anything on the error
stream fails the run; the coordinator files of the default state folder
must not change. `-Mutants` then applies each fault of
`tests/mutants.json` to its own copy of this folder and requires one of
the tests named for it to fail. `-MutantShard k/n` applies only shard k of
n: the faults whose position in `tests/mutants.json`, counted from 0, leaves
k-1 when divided by n; the suites still run in full first. Before any suite
runs it checks that the n shards together hold every fault exactly once and
that none is empty, and refuses (exit 2) a spec that is not k/n with
1 <= k <= n, or one given without `-Mutants`; afterwards every assigned fault
must have exactly one verdict. Without it every fault runs. Exit 0 only when
every check passed. A suite can also be run on its own:

    pwsh -NoProfile -File tests/coordinator-tests.ps1 -OutDir <new folder>

(exit 0 passed, 1 a case failed, 3 a case could not run here). Start
the suites with `pwsh -File` as shown: started with `&` from a prompt,
closures in the tests and in `coordinator-lib.ps1` cannot see the
functions of the script, and cases fail.

Requirements: Windows and PowerShell 7.4 or later; nothing else is
installed. The tests start no VM, send no guest input and no ACPI
request, and read no real secret: the coordinator cases use mock
dispatchers that only record calls, synthetic monitor states, dry runs
and the simulated calibration; secrets are fixture files written by the
test; the monitor and the coordinator run as separate processes only in
fixture mode, for dry runs and refusals, and for the monitor's short
real-sampling run, which reads no VM state. Where VirtualBox is installed
some cases also read the VM states (`VBoxManage showvminfo`, read only,
through `p9s-vbox.ps1`: one query at a time) to show that they did not
change. The `vbox` suite never runs VirtualBox: its "VBoxManage" is a mock
script started with pwsh, steered by named events, in worker processes
that contend for test-only mutex names; a second mutex held by every mock
while it runs detects any overlap. The `decouple` suite runs the real
monitor (real Windows performance classes, no fixture) for about 24 s
while it holds the production lock name, a quarantine and an unfinished
operation record sit in the monitor's state folder and two workers hang
inside mocked VirtualBox calls; holding that name is its only use of a
production name, and only to show that the monitor never needs it. The
`acpival` suite uses mock contexts with a simulated clock, and worker
processes on file-backed mock contexts for races and crashes. The
`coordinator` and `acpival` suites also deny the account access to
synthetic records (their data and attributes, and the listing of their
folder) to show that a record that cannot be read is never taken for a
missing one; the deny entries are removed by the same case. The
`persist` suite builds synthetic state folders, earlier-version folders,
junctions, hard links, ACL entries and test registry keys under
`HKCU\Software\Monolith-P9S-Test` (removed at the end), and brackets
every classification with a snapshot of all of them, taken through file
handles, which must not change; one case starts eight processes that
classify at the same time. Its `state.init` cases, and those of the
`setup` suite, mock the VM states and the VM process count and write the
anchor with the registry API into the test keys; in the `setup` suite
child processes end at each step (TerminateProcess) or wait at one on
named events to show the crash points and the races, never with a
sleep. The `migrate` suite builds earlier coordinator folders with the
laboratory's shape and every shape `state.migrate` refuses (junctions,
hard links, deny entries, damaged records, audit lines that witness a
lost record), runs the migration in process with the VM states, the VM
process count and the free space mocked, and orders writers at a given
step through the step hook of the context; the `migproc` suite ends
migrations at each step (TerminateProcess), races two of them, and runs
the coordinator's own lock and emergency hold functions in other
processes against a migration. The `enforce` suite runs the
coordinator on synthetic state folders with the real classifier and the
real anchor writes into the test keys: the anchor mark of every ledger
save, faults injected at each point of a save (a registry ACL that
refuses the write among them) and child processes ended at each point
(TerminateProcess), the state folder changed under an action in process
and from another process ordered by named events, `state.reanchor`, the
earlier folder after a migration, STOP and ABORT across a restart. The
gate itself is tested status by status in the `coordinator` and
`acpival` suites. Cases P62 and P63 run `state.status` and
`state.init -DryRun`, Q63 `state.migrate -DryRun`, and N13 the
coordinator's entry points without `-StatePath`, against the machine's
real locations, only in the main run of `-CleanRunner` and only where
none of them exists; elsewhere they are NOT_RUN.

CI: the job `p9s-windows-toolkit` of `.github/workflows/ci.yml` runs in three
shards, each on its own GitHub-hosted Windows Server 2025 runner:

    pwsh -NoProfile -File tools/p9s/host/tests/run-all.ps1
         -OutDir <dir> -Mutants -MutantShard <k>/3 -CleanRunner

with k = 1, 2, 3. Every shard runs all eleven suites and a third of the
faults; the three together run each fault exactly once, and each shard
must succeed on its own (one that fails, is cancelled or does not run
leaves the run unsuccessful). `-CleanRunner` first
requires that the runner has no VirtualBox (no `VBoxManage.exe`, no
VirtualBox COM class), no `p9s-local.json`, no `P9S_HOST_CONFIG` and no
default state folder, so that a fault in a test cannot reach a real VM
or secret. The job also checks that Windows PowerShell 5.1 refuses the
entry point. P62 and P63 of the `persist` suite, Q63 of the `migrate`
suite and N13 of the `enforce` suite run only there. One case cannot run there: K37 starts the real coordinator,
which reads the VM states with VBoxManage; it is reported as NOT_RUN and
runs only on a host with VirtualBox (K28 covers the same rule in
process). CI cannot show how a guest answers the ACPI power button, how
the host behaves with both VMs running, or any paging value; see "Not
validated".

## Unsupported on purpose

Free-form commands, keyboard strings outside the allowed characters,
mouse input, account and sudo passwords, the desktop login, forced
power-off, reset, snapshots, VM configuration changes, capture control,
live calibration, a live ACPI validation without the owner's
authorization file, and any override of the safety checks.

## Not validated

- Live on 2026-11-21 (commit `2f4c609`), one supervised session: the
  monitor without VirtualBox sampled for about 2 h 22 min in two runs
  (no invalid sample, largest gap 5.03 s, no STOP or ABORT); `startvm`
  and `acpipowerbutton` ran once each against `P9S-D12` through the
  VirtualBox layer, and the Debian guest powered itself off 5.2 s after
  the request. Keyboard input through the VirtualBox API has not run in
  this version.
- `P9S-U24` has never been started or shut down by the toolkit; how the
  Ubuntu guest answers the power button (shutdown, suspend, a dialog, or
  a delay) is not known. The Debian result does not carry over.
- No paging value is approved; paging stop enforcement is not validated;
  no calibration with both VMs has run.
- Damaged ledgers, low-memory recovery and concurrent emergencies are
  verified with synthetic files, mocks and worker processes only; no
  real disk failure has been reproduced.
- The VirtualBox layer has not met a real timeout or a hung management
  API; its recovery from one is verified only with mocks. Serialization
  reduces the exposure to concurrent management calls; it cannot keep
  VirtualBox from hanging.

## Rollback

The toolkit changes nothing on the host but its own state. To remove it,
stop the monitor, delete the persistent state folder
`<LocalApplicationData>\Monolith-P9S` (state, coordinator files, VM
observations, the validation ledger and its revision record, and the
VirtualBox layer's records), the key `HKCU\Software\Monolith-P9S`,
`%TEMP%\p9s-hostmon` of earlier versions and this folder's
`p9s-local.json`; reverting the commit that added the folder removes the
rest. Deleting the state also deletes the record of every budget spent.
