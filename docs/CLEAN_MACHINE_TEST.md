# Clean-machine test plan (P9-S)

Status: proposed on 2026-11-11 for the owner's approval; not run. No
machine has been prepared, no archive built for it and no scenario run.
It replaces the first outline of P9-S (`STATUS.md` section 6) with the
platform priorities the owner set on 2026-11-11. When it runs, the results
go to `STATUS.md`, and this file is corrected where the run shows it wrong.

## 1. Purpose and limits

Run the desktop client as a tester would receive it, on machines that have
never seen the repository or a development tool, over the public Tor
network, and find what stops a person unfamiliar with the project: the
start, Tor, the profile, contacts, messages, files, restarts, removal. The
run also collects the first network evidence for the packaged desktop
binary on Linux and Windows (`STATUS.md` section 5, blocker 8).

What it is not:

- It changes no code, packaging or user document. A defect is reported
  with its evidence and fixed only under a separate authorization, in the
  workstream it belongs to (P9-D, P9-E, P9-G, P9-B).
- It distributes nothing. The archives stay on the owner's machines.
- It is not the beta runbook (P9-H) and not the network-isolation work of
  P9-G. Passing it does not make a platform supported.

## 2. Platforms

| Id | System | Priority | Tor | Updates until |
| --- | --- | --- | --- | --- |
| W11 | Windows 11 26H2 (build 26300), x86_64, Pro | primary | Tor Browser; the Tor Expert Bundle in session B | 2028-10-10 |
| D12 | Debian 12, latest point release, amd64, GNOME on Wayland | primary | the `tor` package of Debian 12 | 2028-06-30 (LTS) |
| U24 | Ubuntu 24.04 LTS, latest point release, amd64, GNOME on Wayland; one start under Xorg | primary | Tor Browser for Linux | May 2029 |
| W10 | Windows 10 22H2, x86_64, enrolled in Extended Security Updates | optional | Tor Browser | 2027-10-12 (ESU) |

Lifecycle dates, checked on 2026-11-11:

- Microsoft, "Windows 11 - release information" (updated 2026-11-10):
  26H2 available since 2026-09-29, updates for Home and Pro until
  2028-10-10; 25H2 (build 26200) until 2027-10-12; 24H2 has reached the end
  of updates for Home and Pro; 26H1 is only for new devices sold with it.
- Microsoft: Windows 10 support ended on 2025-10-14. The consumer ESU page
  says that enrolled PCs receive security updates through 2027-10-12.
  Organizations buy ESU by the year, for at most three years.
- Debian, wiki.debian.org/LTS: Debian 12 is in LTS from 2026-06-11 to
  2028-06-30. It is oldstable, with security updates from the LTS team.
- Canonical, release cycle: Ubuntu 24.04 LTS has standard support until
  May 2029.

Choices:

- W11 uses 26H2, the current release and the build of the development
  host. If 26H2 media cannot be had, or 26H2 needs something the test
  machine cannot give, 25H2 is used and the reason recorded. 24H2 is not
  used: Home and Pro no longer receive updates.
- W10 is a compatibility check, not a primary platform: only with ESU
  active (the device enrolled, and Windows Update showing it), and only
  if decision S3 keeps it. A result on Windows 11 says nothing about
  Windows 10, nor the reverse.
- Debian 13 is left out: it is the development system (WSL). Ubuntu 22.04
  (glibc 2.35) is left out: the archive built on Debian 12 needs glibc
  2.36 (section 6).
- Tor: the `tor` package of Debian 12 and that of Ubuntu 24.04
  (noble-updates) were both 0.4.9.11 on 2026-11-11, above Monolith's
  baseline 0.4.9.5 and below the 0.4.9.13 that `TOR_INTEGRATION.md`
  recommends (TROVE-2026-053). The run records the version used. That the
  Linux README sends users to the distribution's package goes to P9-D as a
  finding, whatever the run shows.

## 3. Machines

Two sessions, so that every message and transfer crosses platforms:

| Session | Machines | What runs |
| --- | --- | --- |
| A | W11 and D12 | every scenario on both; W11 with Tor Browser, D12 with the system `tor` |
| B | U24 and W11 | every scenario on U24; W11 as its peer, set up again with the Tor Expert Bundle (T-CLEAN-3) |
| C, optional | W10 and D12 | every scenario on W10 |

- Each system is installed fresh from the vendor's media with all updates,
  a local account (no Microsoft account, no OneDrive sign-in) and the
  default security settings: Microsoft Defender, SmartScreen and Smart App
  Control as installation leaves them. Nothing is installed by hand: no
  Rust, Git, compiler, Visual Studio, Visual C++ Redistributable, Python,
  Docker or GPU tool. On Linux, the desktop task of the installer and
  nothing else.
- Snapshots: "pristine" right after installation and updates, before any
  guest tools or graphics driver; "clean" after what section 4 allows.
  Each session starts from "clean". After the write-up the machines are
  destroyed or wiped.
- VMs get 4 GB of memory and 2 CPUs each, two at a time, one session after
  the other: the host is shared with other work. Network: NAT with
  Internet access. No shared folders, clipboard sharing, drag and drop
  between host and guest, or bridge to the host's LAN.
- Files go in on a read-only ISO image made on the host: the archives, the
  Tor installers and the scripts of section 10. Cards go between the two
  machines on a small virtual disk attached to one machine at a time. A
  network share is not used: it would add traffic to the evidence.
- Every machine keeps the default time service and is set to UTC, so that
  the logs of both machines and of the host line up.

## 4. Graphics

Monolith needs OpenGL 2.1 on Windows, and OpenGL 2.1 or OpenGL ES 2.0 on
Linux. A start that fails for want of it exits with code 1 and shows
nothing (`STATUS.md` 3.5). That is a finding of P9-S whatever the remedy.

Linux VMs: Mesa's software renderer (llvmpipe) gives more than 2.1
without a GPU, under Wayland and X11. Nothing to do.

Windows: a VM without a GPU usually has the Microsoft Basic Display
Adapter and only the OpenGL 1.1 of Windows' own software renderer, so
Monolith is not expected to start there. Options, in order of preference:

1. A physical Windows 11 PC that can be wiped, with the vendor's graphics
   driver, as testers have it. Recommended.
2. A hypervisor with 3D acceleration for Windows guests, for example VMware
   Workstation with "Accelerate 3D graphics", whose driver comes with its
   guest tools. The guest tools install more than a driver: what they
   install is recorded (section 5).
3. GPU partitioning (GPU-PV) on the host's Hyper-V. Microsoft documents it
   for Windows Server and Azure Local hosts; on a Windows 11 host the
   host's driver files have to be copied into the guest by hand, a set-up
   no tester has. Last resort.

If none of them gives OpenGL 2.1, the Windows sessions run on a physical
machine. The graphics stack of Monolith is not changed to suit a VM.

The check, on every machine, before the first start:

- Windows: the adapter and its driver, and whether a vendor OpenGL driver
  is registered. Without `OpenGLDriverName` only the software OpenGL 1.1
  is there.

      Get-CimInstance Win32_VideoController |
          Select-Object Name, DriverVersion, DriverDate
      Get-ItemProperty 'HKLM:\SYSTEM\CurrentControlSet\Control\Class\{4d36e968-e325-11ce-bfc1-08002be10318}\0*' -ErrorAction SilentlyContinue |
          Select-Object PSChildName, DriverDesc, OpenGLDriverName

- Linux: `echo $XDG_SESSION_TYPE`, and `glxinfo -B` (package `mesa-utils`,
  installed after the "clean" snapshot and named in the record) for the
  renderer and the OpenGL version.
- Everywhere: the first start of Monolith itself, with its exit code. On
  Windows, `(Start-Process .\monolith-desktop.exe -PassThru -Wait).ExitCode`
  gives it when the program fails or is closed.

One extra run: a start on the "pristine" Windows VM with the Basic
Display Adapter, to record what a user without OpenGL 2.1 sees. It is the
manual check that P9-E names among its gates.

## 5. Windows runtime dependencies

The import table of `monolith-desktop.exe` (x86_64, GUI subsystem), read
on 2026-11-11 from a build of this tree. A release build links the same C
runtime.

| DLL | Part of Windows 10 and 11 |
| --- | --- |
| `kernel32`, `user32`, `gdi32`, `advapi32`, `shell32`, `shlwapi`, `ole32`, `oleaut32`, `imm32`, `ws2_32`, `ntdll`, `dwmapi`, `uxtheme`, `uiautomationcore`, `bcryptprimitives`, `api-ms-win-core-synch-l1-2-0` | yes |
| `opengl32` | yes, but alone it gives OpenGL 1.1 in software; 2.1 needs a vendor driver (section 4) |
| `api-ms-win-crt-*`, the Universal C Runtime | yes, since Windows 10 |
| `VCRUNTIME140.dll` (nine functions: `memcpy`, `memmove`, `memset`, `memcmp` and exception handling) | no: it belongs to the Visual C++ Redistributable |

Nothing is delay-loaded. `libEGL.dll` is named in the binary but only
tried at run time.

Microsoft's "Deployment in Microsoft C++" says that the Universal CRT is
part of Windows and that the version-specific libraries are deployed with
the application: centrally with the Redistributable, locally beside the
executable, or by static linking. The zip of `packaging/windows/build.ps1`
holds the executable and texts only, and `packaging/windows/README.txt`
names no runtime. On the development host the DLL comes from the
Redistributable 14.51 that Visual Studio installed, which says nothing
about a clean machine.

On each Windows machine, on the "pristine" snapshot, before anything else:

- whether `C:\Windows\System32\vcruntime140.dll` exists and its version,
  and whether the Redistributable is registered
  (`HKLM:\SOFTWARE\Microsoft\VisualStudio\14.0\VC\Runtimes\x64`) or listed
  among the installed programs;
- the same after each later installation (guest tools, graphics driver,
  Tor Browser, Expert Bundle), noting which one brought the DLL;
- the edition, S mode (an edition in S mode runs no program from outside
  the Store), the state of Smart App Control (Windows Security > App and
  browser control) and Defender's real-time protection.

Classification. If a clean, updated Windows lacks the DLL and Monolith
does not start, the requirement is undocumented and blocks the beta
(section 13) until the owner chooses a remedy: name the Redistributable as
a prerequisite in the README, ship the DLL beside the executable, or link
the C runtime statically. Each is a change of packaging or build, outside
P9-S. The Redistributable is not installed to make the test pass; it may
be installed afterwards, and recorded, to find out whether anything else
is missing. The loader's message is recorded word for word.

Smart App Control, when on, blocks programs that are neither signed nor
known, and the archive is unsigned (D3). If it is on and blocks Monolith,
that is a blocker tied to D3; it is recorded, not cleared by turning the
feature off.

An archive that reaches a tester through a browser carries the mark of
the web, which brings SmartScreen and Smart App Control into play; one
copied from an ISO does not. Before it is extracted, the zip gets the mark
a download leaves, so that the prompts a tester would see are seen:

    Set-Content -Path .\monolith-desktop-<version>-windows-x86_64.zip `
        -Stream Zone.Identifier -Value "[ZoneTransfer]`r`nZoneId=3"

## 6. Linux glibc and libraries

Three things are kept apart.

- Building from source: Rust 1.85.1 or newer and a linker. The build needs
  no development package of the windowing system, whose libraries are
  loaded at run time. P9-S does not test it.
- The archive: one binary that links only `libc.so.6`, `libm.so.6`,
  `libgcc_s.so.1` and the loader (`readelf -d` of a build of this tree,
  2026-11-11), with the glibc symbol versions of the system it was built
  on. A build on Debian 13 needs `GLIBC_2.39` and does not start on Debian
  12 (2.36). The archive for P9-S is built on Debian 12 (section 8) and
  must need at most `GLIBC_2.36`; it then fits Debian 12 and Ubuntu 24.04
  (2.39).
- The system: what the binary loads when the window opens, by the names in
  the binary: `libEGL.so.1`, `libGL.so.1`, `libwayland-client.so.0`,
  `libwayland-egl.so.1`, `libxkbcommon.so.0`, `libxkbcommon-x11.so.0`,
  `libX11.so.6`, `libX11-xcb.so.1`, `libxcb.so.1`, `libXcursor.so.1`,
  `libXi.so.6`, `libXinerama.so.1`, `libXrender.so.1`, and a Mesa driver.
  A desktop installation of Debian or Ubuntu is expected to have them
  (packages `libegl1`, `libgl1`, `libwayland-client0`, `libwayland-egl1`,
  `libxkbcommon0`, `libxkbcommon-x11-0`, `libx11-6`, `libx11-xcb1`,
  `libxcb1`, `libxcursor1`, `libxi6`, `libxinerama1`, `libxrender1`,
  `libegl-mesa0`, `libgl1-mesa-dri`). The Linux README names only
  `libxkbcommon` and OpenGL.

Checks:

- on the built binary, recorded with the archive:

      objdump -T monolith-desktop | grep -o 'GLIBC_[0-9.]*' | sort -uV | tail -1
      readelf -d monolith-desktop | grep NEEDED

  the first at most `GLIBC_2.36`, the second the four libraries above;
- on each Linux machine: `ldd --version`, `ldd ./monolith-desktop` with no
  "not found", and `ldconfig -p` for each library loaded at run time.

## 7. Tor

Monolith uses a Tor that already runs on the machine, through its SOCKS
and control endpoints on loopback, or Tor's control socket on Debian. It
never bundles or starts one, and it has no way out of the machine without
Tor. P9-S changes none of that: no fallback outside Tor, no DNS, nothing
new in the client. The scenarios use the public Tor network from the
owner's connection (decision S4): the private network of
`tests/tor-network/` cannot test Tor Browser or a distribution's service
as users run them.

| Machine | Tor | From | Settings of Monolith |
| --- | --- | --- | --- |
| W11 in session A, W10 | Tor Browser, current stable release | torproject.org, downloaded on the host, signature checked there | the defaults: SOCKS 127.0.0.1:9150, control 127.0.0.1:9151, cookie in the Tor Browser folder on the desktop |
| W11 in session B | Tor Expert Bundle, current stable release | torproject.org, as above | "Tor Expert Bundle": SOCKS 127.0.0.1:9050, control 127.0.0.1:9051, cookie `%APPDATA%\tor\control_auth_cookie`; a torrc with `ControlPort 9051` and `CookieAuthentication 1` |
| D12 | the `tor` package of Debian 12 | the Debian archive | the defaults: SOCKS 127.0.0.1:9050, control socket `/run/tor/control`, cookie `/run/tor/control.authcookie`; the user in the group `debian-tor` |
| U24 | Tor Browser for Linux, current stable release | torproject.org, as above | "Tor Browser", with the path of the cookie under `Browser/TorBrowser/Data/Tor/` |

For all of them:

- Tor 0.4.9.5 or later, or Monolith refuses it; 0.4.9.13 or later is
  recommended (section 2). The version used is recorded.
- Tor is connected before an identity is made: "Connect" pressed in Tor
  Browser, the service bootstrapped. Tor Browser stays open while
  Monolith runs.
- A change in Settings > Tor applies when Monolith starts again.
- The signature of each Tor download is checked on the host with the Tor
  Project's key; the version and the key's fingerprint go in the record.
- No bridge is used. If the network of the test needs one, that is
  recorded.

## 8. Archive provenance

- One commit for both archives: the head of `phase-9-beta-readiness` when
  P9-S is approved, built from a clean tree. Recorded: `git rev-parse
  HEAD` and `git status`.
- Linux: `packaging/linux/build.sh` in a Debian 12 container (the image of
  the Phase 8 verification, `monolith-verify:phase7`, if it has git,
  Python 3 and rustup; otherwise a new `debian:12` image), with the
  toolchain `rust-toolchain.toml` pins (1.95.0). Recorded: the image
  digest, `rustc -vV`, `cargo -V`, the archive's SHA-256 and file list,
  and the checks of section 6.
- Windows: `pwsh packaging/windows/build.ps1` on the Windows 11 host.
  Recorded: `rustc -vV`, the MSVC linker's version, the zip's SHA-256 and
  file list, and the import table of the executable (section 5).
- The SHA-256 is checked again inside each machine before extraction
  (T-CLEAN-1).
- Not covered: a signed manifest and its key (P9-F, P9-B; D2, D3). P9-S
  shows only that the bytes on a test machine are the bytes the owner
  built. A tester still cannot tell that an archive is the owner's; that
  blocker (`STATUS.md` section 5, blocker 2) stays open and is not tested
  here.
- The archives are not uploaded, attached or given to anyone. They are
  deleted with the machines after the write-up; their SHA-256 and the
  records are kept.

## 9. Test identities and privacy

- Only identities made for P9-S on the test machines, named like
  "P9S W11 A". Passphrases used for nothing else. No card, invitation or
  contact of a real person. Message texts are fixed test strings with no
  personal data.
- Test files are random bytes of fixed sizes made for the run, never
  personal files.
- Never in the evidence: passphrases; the contents of the profile folder
  (its names, sizes and permissions only); whole cards, invitation cards
  or their QR codes; onion addresses; fingerprints beyond their first
  group; the Tor control cookie; Tor's state and data folders; a capture
  of loopback traffic, because the control connection carries the onion
  service's private key (`ADD_ONION`) and the cookie proofs; system call
  buffers (`strace -s 0`); memory or crash dumps.
- Screenshots are cropped or covered where a card, a QR code, an address
  or a fingerprint shows.
- The evidence stays on the owner's machines. `STATUS.md` gets a summary.
  Nothing is published without the owner's review. After the write-up the
  profiles, the machines and the archives are destroyed.

## 10. Network observation

The aim: show that the Monolith process connects only to the Tor endpoints
it is configured for, on loopback (or to Tor's control socket on Debian),
makes no DNS query and causes no other traffic; and see whether anything
else on the machine sends traffic that a Monolith action causes. A
per-process view is not enough on Windows: the DNS Client service resolves
names on behalf of processes, so a query made for Monolith would show as
the service's (`STATUS.md` section 4).

Observation windows, each start and end written in the session log:

| Window | Monolith | Length |
| --- | --- | --- |
| W0 baseline | not started; Tor running and connected; nothing else open | 15 minutes |
| W1 start | from launch to the welcome screen (T-CLEAN-4), then through the new profile to "Tor connected" (T-CLEAN-5) | as long as it takes |
| W2 contacts and messages | T-CLEAN-8 to T-CLEAN-12 | as long as they take |
| W3 files | T-CLEAN-13 to T-CLEAN-15 | as long as they take |
| W4 idle | running, nothing done | 15 minutes |
| W5 after exit | closed | 5 minutes |
| W6 deny, Windows only | a firewall rule blocks the program outside loopback; T-CLEAN-4, 10 and 13 again | about 20 minutes |

Windows, set up before W0 (as administrator) and undone after W6:

    auditpol /set /subcategory:"Filtering Platform Connection" /success:enable /failure:enable
    wevtutil sl Security /ms:1073741824
    wevtutil sl Microsoft-Windows-DNS-Client/Operational /e:true
    pktmon start --capture --comp nics --pkt-size 128 --file-name C:\p9s\host.etl

- The process: the Security log's events 5156 (connection permitted), 5157
  (blocked) and 5158 (bind) whose application is `monolith-desktop.exe`,
  exported with `wevtutil epl Security`; and, every five seconds while it
  runs, `Get-NetTCPConnection` and `Get-NetUDPEndpoint` with
  `-OwningProcess` of its process id, from a script on the ISO.
- The machine: the packet capture of the network adapters (`pktmon stop`,
  then `pktmon etl2pcap`), headers only; for a VM, also a capture taken on
  the host side of its virtual adapter, which the guest cannot influence.
- DNS: the DNS Client operational log, exported after W5. Each query is
  matched by time with the session log, and with the process the event
  names where it names one.
- W6: `New-NetFirewallRule -DisplayName "P9-S block" -Direction Outbound
  -Program <full path of monolith-desktop.exe> -Action Block`. Firewall
  rules leave loopback traffic alone by default, so Monolith should keep
  working through Tor if it needs nothing else; the run checks that it
  does. Every 5157 event for the program outside loopback is an attempt to
  leave the machine. The rule is removed after W6.

Linux, with `strace` and `tcpdump` installed after the "clean" snapshot
and named in the record:

    strace -f -tt -s 0 -o p9s-strace.txt \
        -e trace=socket,connect,bind,listen,accept,accept4,sendto,sendmsg,sendmmsg,open,openat \
        ./monolith-desktop
    sudo tcpdump -i <external interface> -s 128 -w p9s-host.pcap

- The process: every `socket`, `connect`, `bind` and send with an address
  in the strace output, and `ss -tunxp` every five seconds. `open` and
  `openat` give the names of the files it opens (never their contents),
  which tell what a call to nscd was for.
- The machine: the capture of the external interface, never of `lo`; for a
  VM also one on the host side. On U24, `resolvectl monitor` as root for
  the queries that systemd-resolved answers.

Rules for reading the evidence:

- The Monolith process may connect only to 127.0.0.1 or ::1 on the
  configured SOCKS and control ports, or over Unix sockets to Tor's control
  socket, the display server (Wayland, X11), the D-Bus session bus and the
  accessibility bus. Anything else is a finding: another address, a UDP
  socket, a netlink socket, port 53, or the resolver's socket under
  `/run/systemd/resolve/`. A connection to anything but loopback is a
  blocker.
- glibc asks nscd first for any lookup in its databases and, with nscd
  absent, reads the files itself. A failed call to nscd followed by a read
  of the user or group database (`/etc/passwd`, `/etc/group`) is a local
  lookup and is noted, not a finding: on D12 the graphics library does one
  at start to find the home folder. A call to nscd followed by a name
  lookup (`/etc/hosts`, `/etc/resolv.conf`, the dns, resolve or mdns NSS
  modules, a UDP socket, port 53), a call nscd answered, and one whose
  database cannot be told are findings.
- For the machine, the DNS names and remote endpoints of W1 to W6 that are
  not in W0 are listed and each explained: Windows Update, Defender, the
  time service, the connectivity test, Tor's own connections to relays
  (from `tor.exe` or `firefox.exe` in the Windows log). A name that ends in
  `.onion` in any DNS query is a blocker until explained. An unexplained
  query that follows a Monolith action in time (start, import, send, save,
  exit) is investigated.
- The claim stays within what was seen: the capture shows what left the
  machine during the windows. It does not attribute every packet to a
  process; the system's own traffic is explained, not removed.

T-NET-1 (`tests/network/fail-closed.sh`) shows the command-line client and
the Tor crate in a namespace with only loopback. It does not stand for
this run of the packaged window, and P9-G still owes the namespace trace
of the desktop binary.

## 11. Scenarios

Every scenario runs on every machine of its session, unless it says
otherwise. "Peer" is the other machine of the session. "Who": human means
the owner at the machine; scripted means a script on the ISO, read by a
human.

### T-CLEAN-1 Check the archive

- Start: the archive and its `.sha256` file on the ISO; the build record
  with the SHA-256 the owner wrote down when building.
- Do: compute the SHA-256 (`Get-FileHash -Algorithm SHA256`,
  `sha256sum`) and compare it with the record and the `.sha256` file.
- Expect: the three values equal.
- Property: the bytes tested are the bytes built. Not authenticity
  (section 8).
- Evidence: the three values.
- Fails if: any value differs. The session stops.
- Who: scripted.

### T-CLEAN-2 Extract or install

- Start: the checked archive; on Windows the mark of the web set
  (section 5).
- Do: Windows: "Extract all" in Explorer to a folder under Documents.
  Linux: `tar -xzf`, then `./install.sh`.
- Expect: the files `packaging/README.md` lists and no others;
  `install.sh` puts the program in `~/.local/bin` and the menu entry and
  icon under `~/.local/share`; nothing asks for administrator rights; every
  SmartScreen, Smart App Control or antivirus prompt is recorded word for
  word.
- Property: no installer, no service, no privileges.
- Evidence: the file list with sizes; screenshots of prompts.
- Fails if: a file is missing or added; anything needs administrator
  rights; a security product removes or blocks the program and a tester
  could not clear it.
- Who: human.

### T-CLEAN-3 Set up Tor

- Start: no Tor installed.
- Do: what the user README says, timing each step (section 7 gives the
  set-up of each machine). W11 in session A: Tor Browser from its
  installer, default place on the desktop, "Connect". W11 in session B:
  the Tor Expert Bundle with the torrc of the README, then
  Settings > Tor > "Tor Expert Bundle" and a restart of Monolith. D12:
  `sudo apt install tor`, `sudo adduser "$USER" debian-tor`, log in
  again. U24: Tor Browser unpacked in the home folder, started,
  "Connect", then Settings > Tor > "Tor Browser" with the path of the
  cookie and a restart of Monolith. W10: as W11 in session A.
- Expect: Tor running and connected; every step a newcomer needs is in the
  README, or is recorded as missing.
- Property: Monolith uses the Tor the system runs; it bundles none.
- Evidence: the steps and their times; the Tor version (`tor --version`,
  or the version Tor Browser shows) and where it came from.
- Fails if: Tor cannot be set up from the README alone; Monolith refuses
  the Tor the README sends the user to.
- Who: human.

### T-CLEAN-4 First start

- Start: Tor running; no profile; W0 done.
- Do: start `monolith-desktop`. Windows: from PowerShell with the exit
  code (section 4). Linux: once from the menu entry, once from a terminal.
- Expect: the welcome screen; on Windows no console window. Monolith asks
  Tor nothing and uses no network before a profile or a session is open:
  no socket to Tor, no inet socket, no DNS query; its other sockets are
  those section 10 allows (the display server, the session and
  accessibility buses). The window closes at once with exit code 0.
- Property: a start that fails is visible; nothing leaves the machine
  before the user opens a profile or a session.
- Evidence: screenshot; exit code; time to the window; the OpenGL record;
  the strace summary and the capture summary of W1.
- Fails if: no window (the exit code and any loader message recorded); a
  socket to Tor, an inet socket, a DNS query, or any other finding under
  the rules of section 10.
- Who: human; the exit code scripted.
- Changed after the first run on D12 (2026-11-13): the welcome screen has
  no Tor indicator and Monolith does not look for Tor before a session is
  open, so the Tor checks moved to T-CLEAN-5. That run was made against
  the earlier wording; its record stays as written.

### T-CLEAN-5 Create a profile and an identity

- Start: the welcome screen; Tor running; W0 done.
- Do: create a profile with a long test passphrase, then an identity "P9S
  <machine> A". Open "Tor details" once the profile is open.
- Expect: the warning that a lost passphrase cannot be recovered; the
  profile under the documented folder (`%LOCALAPPDATA%\Monolith\Profile`,
  `~/.local/share/monolith`), on Linux the folder 0700 and its files 0600;
  once the profile is open, the Tor indicator goes from "Checking Tor" to
  a state that "Tor details" explains in plain words, and Tor is found
  when the settings match the Tor set up as the README says; the identity
  published: "Tor connected". The process reaches Tor only on the
  configured ports or control socket; Tor's own connections to relays are
  told apart by process (`ss -tnp` as root on Linux).
- Property: data only where documented, with the documented permissions;
  Tor only, fail closed; the onion service published through Tor.
- Evidence: the listing of the data folder; screenshots with the card and
  address covered; the text of "Tor details"; the time to "Tor
  connected"; the strace and capture summaries.
- Fails if: the profile is elsewhere; Unix permissions are wider; a Tor set
  up as the README says is not found; the identity is not published within
  10 minutes while Tor is connected; the strace summary of the process has
  a finding under the rules of section 10.
- Who: human.

### T-CLEAN-6 Close and reopen

- Do: close with the window's close button; start again; give the
  passphrase.
- Expect: the close happens at once, or asks as `DESKTOP.md` section 6
  says; the same identity (first group of the fingerprint) and "Tor
  connected" again.
- Property: nothing is lost across a normal restart.
- Evidence: fingerprint group before and after; screenshot.
- Fails if: the profile does not open; an identity differs or is missing;
  the profile is called damaged.
- Who: human.

### T-CLEAN-7 One instance per profile

- Do: while Monolith runs, start a second copy with the same profile.
- Expect: the second says "Monolith is already open with this profile."
  and changes nothing; the first keeps working and sends a message
  afterwards.
- Property: never two processes on one profile, so never two copies of one
  onion service.
- Evidence: screenshot; the message sent by the first copy afterwards.
- Fails if: the second copy opens the profile, or the first is disturbed.
- Who: human.

### T-CLEAN-8 Exchange contact cards

- Start: identity A on both machines, both "Tor connected".
- Do: on each machine, "Share my card", save it to a file, carry the file
  to the peer on the transfer disk, "Add contact" from that file on the
  peer. Compare the fingerprints both sides show, read aloud by the owner.
- Expect: each side lists the other as a contact; until the other side has
  added the card, "Add contact" and the conversation say that it must;
  the fingerprints match. Note where the card was saved (Downloads, or a
  folder that OneDrive redirects).
- Property: a contact only from a card the user brings; the fingerprint
  check is offered; a card file is read as `DESKTOP.md` section 10 says.
- Evidence: screenshots with cards covered; the fingerprint groups; the
  path the card was saved to.
- Fails if: a valid card is refused; the fingerprints differ.
- Who: human.

### T-CLEAN-9 Accept an invitation

- Start: T-CLEAN-8 done.
- Do: one machine makes an invitation labelled "P9S" and its invitation
  card goes to the peer; the peer makes a second identity, B, adds the
  invitation card and sends a message; the first machine accepts the
  request; then it revokes the invitation.
- Expect: the request listed with the fingerprint and the invitation that
  admitted it; after acceptance the conversation shows the message; the
  toast says that the invitation still admits requests; revoking asks
  first and the invitation is listed as revoked.
- Property: requests only through a capability; acceptance is explicit;
  several identities on one profile.
- Evidence: screenshots, covered.
- Fails if: the request does not arrive within 15 minutes while both sides
  are connected; the accepted contact cannot exchange messages.
- Who: human.

### T-CLEAN-10 Messages both ways

- Do: five short messages each way, one of several lines and one in a
  non-Latin script.
- Expect: on the sender Queued, then Delivered (two ticks); on the
  receiver each message once, in order.
- Property: Delivered only from the receiver's durable acknowledgement; no
  loss, duplicate or reordering.
- Evidence: screenshot; the time from Send to Delivered.
- Fails if: a message is lost, duplicated or out of order, or shown
  Delivered while the receiver does not show it.
- Who: human.

### T-CLEAN-11 Queue while the peer is away

- Do: close Monolith on the peer; send three messages; wait 10 minutes;
  open Monolith on the peer again.
- Expect: Queued, with the notice that messages are delivered only while
  Monolith is open on both sides and when the next try is due; all three
  delivered after the peer is back, within the retry delay; none lost or
  duplicated.
- Property: a durable queue on the sender; no relay.
- Evidence: the notice's text; the times.
- Fails if: a message is lost or duplicated, or not delivered within 15
  minutes of both sides running.
- Who: human.

### T-CLEAN-12 Tor interrupted

- Do: on one machine stop Tor (close Tor Browser; `sudo systemctl stop
  tor`; end `tor.exe`); watch the indicator; send a message; start Tor
  again.
- Expect: "Tor unavailable"; the message Queued, with the notice that Tor
  is down; nothing dialed; once Tor is back, "Tor connected" again without
  a restart of Monolith, and the message Delivered.
- Property: fail closed without Tor; recovery when it returns.
- Evidence: screenshots; times; the network evidence of the window.
- Fails if: Monolith tries any connection outside loopback; it does not
  recover after Tor returns (recorded; a blocker if a restart of Monolith
  does not help either).
- Who: human.

### T-CLEAN-13 A small file

- Do: send a 1 MiB file of random bytes; the peer accepts it and saves it
  to Downloads.
- Expect: the offer shows the name and size; no byte moves before the
  acceptance; progress, then complete; the file saved as a new file and
  never opened.
- Property: nothing transferred without consent; integrity; a received
  file is never run or previewed.
- Evidence: screenshots; the SHA-256 (T-CLEAN-15).
- Fails if: bytes move before acceptance; the file is opened; an error.
- Who: human.

### T-CLEAN-14 A large file, resumed after a restart

- Do: send a 200 MiB file (Monolith accepts up to 256 MiB). At about a
  third, close Monolith on the receiver and open it again after two
  minutes; at about two thirds, close the sender and open it again.
- Expect: after each restart the transfer is shown as being restored, then
  goes on from where it stopped, not from zero, and completes.
- Property: a transfer is durable state of both sides and survives a
  restart of either (`PROTOCOL.md` section 13).
- Evidence: the progress before and after each restart; the total time.
- Fails if: it starts again from zero, stalls for more than 15 minutes
  while both sides are connected, or completes with other content.
- Who: human.

### T-CLEAN-15 Check the received files

- Do: the SHA-256 and size of each original and of each saved file.
- Expect: equal.
- Property: a file is accepted only with the digest of the bytes offered.
- Evidence: the values.
- Fails if: any differs. Blocker.
- Who: scripted.

### T-CLEAN-16 Wrong passphrase

- Do: list the data folder; start Monolith; give a wrong passphrase twice,
  then the right one; list the folder again.
- Expect: "That passphrase does not open this profile." under the field;
  the folder unchanged by the wrong attempts; the profile opens with the
  right one.
- Property: the vault refuses without changing anything.
- Evidence: the two listings (names, sizes, times); screenshot.
- Fails if: a file changes; the profile is damaged; the message misleads.
- Who: human.

### T-CLEAN-17 Unexpected termination

- Do: end the process by force (`Stop-Process -Force`, `kill -9`): once
  just after Send while the peer is closed, once during a transfer. Start
  again each time.
- Expect: the profile opens; the message is Queued or Delivered, once; the
  transfer goes on.
- Property: the crash model of `STORAGE.md`: nothing durable is lost and
  nothing is duplicated.
- Evidence: screenshots; listings.
- Fails if: the profile does not open, is called damaged, or reports an
  interrupted save; a message is lost or duplicated.
- Who: human; the kill scripted.

### T-CLEAN-18 Start without saving

- Start: a profile exists; the data and settings folders listed.
- Do: start Monolith and choose "Use without saving instead"; make an
  identity; exchange cards and a message with the peer; close, and confirm
  the warning.
- Expect: it works without opening the profile; the close warns that every
  identity, contact and message of the session is deleted; after exit the
  data and settings folders are as listed before, apart from a card the
  user chose to save.
- Property: the session writes none of its identities, contacts, messages
  or settings to disk. Not a forensic claim: swap, the page file,
  hibernation and Tor's own state are outside it.
- Evidence: the listings; the warning's text.
- Fails if: a file appears or changes under the data or settings folders.
- Who: human.

### T-CLEAN-19 Remove and reinstall

- Do: close Monolith. Windows: delete the program folder only, then
  extract the archive again. The README says to delete the data folder as
  well; that sentence is recorded as it reads and not followed. Linux:
  delete what `install.sh` put in place and the extracted folder, then
  install again.
- Expect: the data folder untouched; a list of what stays behind (the
  data folder, `desktop.conf`, saved cards).
- Property: removing the program never removes identities.
- Evidence: listings; the README's removal text.
- Fails if: removing the program removes data; the README leads a careful
  user to delete the only copy of the identities without a warning
  (`STATUS.md` section 5, blocker 5, confirmed or not on a clean machine).
- Who: human.

### T-CLEAN-20 Identities after reinstall

- Do: start the reinstalled program and give the passphrase; message the
  peer both ways.
- Expect: the same identities (fingerprint groups), contacts and
  conversations; "Tor connected" with the same service; messages
  delivered both ways.
- Property: identity continuity across reinstallation.
- Evidence: fingerprint groups; screenshots.
- Fails if: anything is missing. Blocker.
- Who: human.

U24 also starts once under the Xorg session ("Ubuntu on Xorg"): T-CLEAN-4
and T-CLEAN-6 only.

## 12. Evidence

For each machine and session, in a folder on the owner's machine:

- the environment record: system, edition and build, update level,
  glibc, session type, GPU and the OpenGL check, the C runtime check,
  Smart App Control, Defender, the Tor version and where it came from;
- the session log: one line per action with the UTC time, written as the
  session goes;
- the checklist: each scenario passed, failed or blocked, with the exact
  text of every prompt and error;
- screenshots named `T-CLEAN-<n>-<machine>-<k>.png`, covered as section 9
  says;
- hashes and folder listings;
- the network evidence of section 10 and its reading: a table of every
  endpoint and name not in W0, each with its explanation or the finding it
  became.

`STATUS.md` gets the matrix of scenarios by machine with the results, and
each finding with its severity and the workstream it goes to.

## 13. Blocking criteria

A finding on a primary platform blocks a controlled external beta until it
is fixed, or documented with a remedy the owner accepts:

- Monolith does not start on an intended platform.
- A runtime library it needs is missing and undocumented
  (`VCRUNTIME140.dll`, a library of section 6, OpenGL 2.1).
- Tor cannot be set up with the user README.
- A profile cannot be opened again after a restart, a forced end, or
  removal and reinstallation.
- An identity, contact or message is lost unexpectedly.
- A profile it can no longer open is called damaged when the cause is
  another (a newer format: known, `STATUS.md` blocker 7, not re-tested
  here).
- A message or a transfer breaks its documented properties: lost,
  duplicated, out of order, Delivered without the receiver having it,
  bytes moved before acceptance.
- A received file is accepted with other content.
- A transfer does not recover from a restart of either side or a forced
  end.
- The Monolith process makes a connection outside loopback, or a DNS query
  is caused by it.
- Sensitive material is written outside the profile and the places the
  documents name (a card saved on request, a received file saved on
  request).
- The installation or removal instructions can lead to the irreversible
  loss of an identity.
- The provenance of the package cannot be established well enough for the
  intended beta distribution. P9-S shows only the SHA-256 (section 8);
  this criterion stays open until P9-F and P9-B.

Recorded, not blocking: SmartScreen and antivirus prompts a tester can
pass; a slow Tor start; unclear texts; layout problems; a step missing
from a README that the run found a way around (it goes to P9-D).

The acceptance criteria are not changed to make a result pass. A
limitation is recorded as it was found.

## 14. Effort and the owner's part

| Work | Time |
| --- | --- |
| Building the archives and their records | 1 to 2 hours |
| Three clean systems, updates, snapshots, the checks of sections 4 to 7 | 4 to 6 hours, mostly waiting |
| Session A | about 4 hours, the windows of section 10 included |
| Session B | about 3 hours |
| Reading the network evidence | 3 to 4 hours |
| The write-up in `STATUS.md` | about 2 hours |
| Session C, if chosen | about 3 hours more |

About two and a half working days. The owner is at the machines for the
graphical steps and the card exchange, about a day and a half, and
decides S1 to S6 first.

## 15. Decisions before execution

- S1. The machines: physical or virtual for each system, and which
  hypervisor.
- S2. Windows graphics: a physical PC (proposed), a hypervisor with 3D
  acceleration, or GPU-PV (section 4).
- S3. Windows 10 22H2 with ESU: run session C, or leave it out (proposed
  for the first run).
- S4. The public Tor network from the owner's connection (proposed; a
  private network cannot test Tor Browser or the Debian service as users
  run them).
- S5. Building the two archives from the head of the Phase 9 branch for
  this purpose only, kept on the owner's machines.
- S6. Who runs the graphical steps: the owner, with this list.

Decided by what the run finds, not before: the remedy for a missing
`VCRUNTIME140.dll`; what to do if Smart App Control blocks the unsigned
program (with D3); whether the Linux README keeps sending users to the
distribution's `tor` while it is older than 0.4.9.13 (P9-D).
