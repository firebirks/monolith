# P9-S host safety toolkit (Windows)

The Windows-host tools of the P9-S clean-machine laboratory (plan in
`docs/CLEAN_MACHINE_TEST.md`, results in `docs/CLEAN_MACHINE_RESULTS.md`).
Two virtual machines run Monolith over Tor on one Windows host. The
toolkit has two parts:

- a resource monitor that samples the host every 5 seconds and turns
  memory, paging and disk measurements into one safety state, and
- an action coordinator, the single supported entry point through which
  laboratory automation changes the state of the VMs; it refuses to act
  unless the safety state allows it.

The toolkit is not part of Monolith: nothing in `crates/` uses it, and it
does not touch Tor, Monolith or its profiles.

## What it is not

- Not a security boundary. It runs under the operator's own Windows
  account. That account can still run `VBoxManage`, use the VirtualBox
  window or edit the state files. The coordinator makes the approved
  automation fail closed; it does not prevent anything else, and it
  detects some changes made outside it only afterwards.
- Not validated with running VMs. Every test is offline: synthetic
  monitor states, mocked dispatchers, dry runs and a simulated
  calibration. Section "Not validated" lists what still needs a live run.

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
| `p9s-vbox.ps1` | Every VirtualBox operation: one host-wide lock, bounded waits, quarantine, recovery |
| `p9s-config.ps1` | Local configuration and the local secret store |
| `p9s-local.example.json` | Example of the local configuration |
| `paging-thresholds.json` | Paging values: candidates, not approved |
| `com-type.ps1`, `vm-action.ps1` | Compatibility wrappers that call the coordinator |
| `tests/hostmon-tests.ps1` | Monitor tests |
| `tests/coordinator-tests.ps1` | Coordinator tests |
| `tests/stagec-tests.ps1` | Configuration, credentials, ACPI shutdown, calibration, publication checks |
| `tests/vbox-tests.ps1` | The VirtualBox layer: serialization, timeouts, crashes, recovery, coordinator use |
| `tests/run-all.ps1` | Runs and checks the four suites, and the mutants (local and CI) |
| `tests/mutants.json` | The 39 targeted faults and the tests that must detect them |

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

It writes `run.json`, `samples.csv` (one line per sample; the `d12_*` and
`u24_*` columns are the first and second configured VM), `events.log`,
`processes.csv` (image names only) and `run-end.json` into `-OutDir`, and
replaces `%TEMP%\p9s-hostmon\state.json` atomically after every sample.
Create `<OutDir>\STOP` to end it; a clean end publishes STOPPED. One
monitor per state file.

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
- the toolkit's COM calls (keyboard input to a guest, the monitor's VM
  states) run inside their process.

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
| The monitor's lock try | 1 s | The monitor skips a sample's VM states instead of waiting |

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
and quarantine.

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

runs the four suites (`hostmon`, `coordinator`, `stagec`, `vbox`), each
in a new `pwsh -File` process, and judges them by their `results.json`:
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
fixture mode or for dry runs and refusals. Where VirtualBox is installed
some cases also read the VM states (`VBoxManage showvminfo`, read only,
through `p9s-vbox.ps1`: one query at a time) to show that they did not
change. The `vbox` suite never runs VirtualBox: its "VBoxManage" is a mock
script started with pwsh, steered by named events, in worker processes
that contend for test-only mutex names; a second mutex held by every mock
while it runs detects any overlap.

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
power-off, reset, snapshots, VM configuration changes, capture control and
live calibration.

## Not validated

- The real dispatchers (`startvm`, keyboard input through the VirtualBox
  API, `acpipowerbutton`) have not run against a VM in this version.
- How each guest answers the ACPI power button (shutdown, suspend or a
  dialog) depends on its desktop settings and is not verified.
- No paging value is approved; paging stop enforcement is not validated.
- The monitor was validated live once, with both VMs off.
- The VirtualBox layer has not run against a real VirtualBox: its tests
  use a mock VBoxManage. How long the real commands take on this host,
  whether the defaults above fit them, and whether VirtualBox recovers by
  itself from a hung management API are not known. Serialization reduces
  the exposure to concurrent management calls; it cannot keep VirtualBox
  from hanging.

## Rollback

The toolkit changes nothing on the host. To remove it, stop the monitor,
delete `%TEMP%\p9s-hostmon` and this folder's `p9s-local.json`; reverting
the commit that added the folder removes the rest.
