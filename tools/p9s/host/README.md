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
| `p9s-config.ps1` | Local configuration and the local secret store |
| `p9s-local.example.json` | Example of the local configuration |
| `paging-thresholds.json` | Paging values: candidates, not approved |
| `com-type.ps1`, `vm-action.ps1` | Compatibility wrappers that call the coordinator |
| `tests/hostmon-tests.ps1` | Monitor tests |
| `tests/coordinator-tests.ps1` | Coordinator tests |
| `tests/stagec-tests.ps1` | Configuration, credentials, ACPI shutdown, calibration, publication checks |

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
and `coordinator-audit.jsonl`.

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

    pwsh -NoProfile -File tests/hostmon-tests.ps1 -OutDir <new folder>
    pwsh -NoProfile -File tests/coordinator-tests.ps1 -OutDir <new folder>
    pwsh -NoProfile -File tests/stagec-tests.ps1 -OutDir <new folder>

They need no VM: mock dispatchers, synthetic states, dry runs, read-only
`VBoxManage showvminfo`, and in a few cases the monitor or the
coordinator run as separate processes for a few seconds. They are not
part of the repository's CI, which runs on Linux; run them on the
Windows host.

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

## Rollback

The toolkit changes nothing on the host. To remove it, stop the monitor,
delete `%TEMP%\p9s-hostmon` and this folder's `p9s-local.json`; reverting
the commit that added the folder removes the rest.
