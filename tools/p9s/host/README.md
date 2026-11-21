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
| `tests/acpival-tests.ps1` | The restricted ACPI validation: authorization, targets, budgets, recovery, races |
| `tests/run-all.ps1` | Runs and checks the six suites, and the mutants (local and CI) |
| `tests/mutants.json` | The 74 targeted faults and the tests that must detect them |

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
replaces `%TEMP%\p9s-hostmon\state.json` atomically after every sample.
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
`coordinator-session.json`, `coordinator.lock`, `coordinator-stop.request`
and `coordinator-audit.jsonl`, and the VirtualBox layer keeps its files
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
| `session.close` | Closes it; discards an in-flight record and the stop request |
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
including STOP, ABORT, UNKNOWN and with no monitor at all. It writes a
stop request that every running action honours, waits up to 30 s for any
running action to stop, sends one `VBoxManage controlvm <vm>
acpipowerbutton` (the guest sees a power-button press) and watches the VM
state for up to 180 s. It reports success only when the VM is off. If the
request fails or the VM stays on, the emergency is UNRESOLVED: no second
request, no keyboard input and no forced power-off are attempted; the
operator decides, and the session cannot be closed while that VM runs. It
cannot start a VM or send text.

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
Below the minimum the start is refused and an observation ends; the one
ACPI request stays available. The code refuses any minimum below 8192
MiB for either VM; that floor approves nothing. The owner approved 8192
MiB for the Debian session only; for `P9S-U24` the value and its
approval belong to its own authorization. Available memory alone is not
a paging criterion. The coordinator calls all this
`ACPI_VALIDATION_ELIGIBLE`. It is not READY: the monitor still reports
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

| Phase | Meaning | What is possible next |
| --- | --- | --- |
| `OPEN` | Session open, nothing started | `acpi.start`, `acpi.finalize` |
| `STARTING` | Start dispatched, outcome not recorded (the coordinator ended) | emergency path, `acpi.confirm-off`, `acpi.finalize` |
| `RUNNING` | Start done | `acpi.observe`, `acpi.shutdown` |
| `START_UNCERTAIN`, `START_FAILED` | Start failed; VM state unknown, or observed off | emergency path, `acpi.confirm-off`, `acpi.finalize` |
| `SHUTDOWN_REQUESTING` | Request dispatched, not confirmed | `acpi.confirm-off`, `acpi.finalize` |
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
Outcomes: `VALIDATED`, `SHUTDOWN_LATE`, `OFF_AFTER_UNCONFIRMED_REQUEST`,
`OFF_AFTER_EMERGENCY_ACPI`, `ENDED_WITHOUT_ACPI`, `OFF_ABNORMAL`,
`START_FAILED`, `NOT_STARTED`, `ACPI_VALIDATION_UNRESOLVED`.

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
         [-Mutants]

runs the six suites (`hostmon`, `coordinator`, `stagec`, `vbox`,
`decouple`, `acpival`), each in a new `pwsh -File` process, and judges
them by their `results.json`:
every declared case must be reported once, as PASS, FAIL or NOT_RUN; the
exit code of a suite must agree with its results; anything on the error
stream fails the run; the coordinator files of the default state folder
must not change. `-Mutants` then applies each fault of
`tests/mutants.json` to its own copy of this folder and requires one of
the tests named for it to fail. Exit 0 only when every check passed. A
suite can also be run on its own:

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
processes on file-backed mock contexts for races and crashes.

CI: the job `p9s-windows-toolkit` of `.github/workflows/ci.yml` runs

    pwsh -NoProfile -File tools/p9s/host/tests/run-all.ps1
         -OutDir <dir> -Mutants -CleanRunner

on a GitHub-hosted Windows Server 2025 runner. `-CleanRunner` first
requires that the runner has no VirtualBox (no `VBoxManage.exe`, no
VirtualBox COM class), no `p9s-local.json`, no `P9S_HOST_CONFIG` and no
default state folder, so that a fault in a test cannot reach a real VM
or secret. The job also checks that Windows PowerShell 5.1 refuses the
entry point. One case cannot run there: K37 starts the real coordinator,
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
- The VirtualBox layer has not met a real timeout or a hung management
  API; its recovery from one is verified only with mocks. Serialization
  reduces the exposure to concurrent management calls; it cannot keep
  VirtualBox from hanging.

## Rollback

The toolkit changes nothing on the host. To remove it, stop the monitor,
delete `%TEMP%\p9s-hostmon` (state, coordinator files, VM observations,
the validation ledger and the VirtualBox layer's records) and this
folder's `p9s-local.json`; reverting the commit that added the folder
removes the rest.
