Monolith desktop client for Linux
=================================

Monolith sends private messages over Tor. There are no servers and no
accounts. This folder holds the desktop client:

  monolith-desktop    the program
  install.sh          installs it for you, without root
  monolith.desktop    its entry for the application menu
  monolith.png        its icon
  LICENSE, licenses/  the licence of Monolith and of the fonts and icons
  THIRD-PARTY-NOTICES.txt
                      the licences and notices of the code of others in
                      the program

Running
-------

Run ./monolith-desktop from this folder, or run ./install.sh once to put it
in ~/.local/bin and in the application menu.

It needs a graphical session (X11 or Wayland) with OpenGL 2.1 or OpenGL ES
2.0, as Mesa provides, and libxkbcommon. Nothing else is needed: the fonts
are part of the program. For Chinese, Japanese, Korean and some other
scripts it also uses DejaVu Sans and Noto Sans CJK if they are installed.

Tor
---

Monolith does not include Tor and never connects without it. It uses the
Tor your system runs:

- The Tor service of Debian, Ubuntu and similar systems (package "tor"):
  SOCKS on 127.0.0.1:9050, control on the socket /run/tor/control, cookie
  /run/tor/control.authcookie. Your user must be in the group debian-tor to
  read the cookie and use the socket: sudo adduser "$USER" debian-tor, then
  log in again. These are the default settings of Monolith.
- Tor Browser, while it runs: in Settings > Tor choose "Tor Browser" and
  check the path of the cookie file, which is in the Tor Browser folder,
  under Browser/TorBrowser/Data/Tor/control_auth_cookie.

The settings take effect when Monolith starts again. Settings > Tor shows
whether Tor answers.

Tails and Whonix need a filter for the control port of Tor; see the folder
integrations/ of the Monolith source.

Your data
---------

- Your profile (identities, contacts, messages), encrypted under your
  passphrase: ~/.local/share/monolith (or $XDG_DATA_HOME/monolith).
- The settings of the window (size, theme, Tor endpoints):
  ~/.config/monolith/desktop.conf.

Monolith keeps no log, sends no telemetry and checks for no updates. A
card you save goes to your Downloads folder.
