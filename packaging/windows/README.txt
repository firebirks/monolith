Monolith desktop client for Windows
===================================

Monolith sends private messages over Tor. There are no servers and no
accounts. This folder holds the desktop client:

  monolith-desktop.exe    the program
  LICENSE, licenses\      the licence of Monolith and of the fonts and icons
  THIRD-PARTY-NOTICES.txt the licences and notices of the code of others in
                          the program

Running
-------

Run monolith-desktop.exe. It needs Windows 10 or 11 (64-bit) and a graphics
driver with OpenGL 2.1. Nothing is installed; to remove Monolith, delete
this folder and the data folder named below.

Tor
---

Monolith does not include Tor and never connects without it. It uses a Tor
that runs on this computer:

- Tor Browser, while it runs. These are the default settings of Monolith:
  SOCKS 127.0.0.1:9150, control 127.0.0.1:9151, and the cookie file in the
  Tor Browser folder, by default
  %USERPROFILE%\Desktop\Tor Browser\Browser\TorBrowser\Data\Tor\control_auth_cookie,
  or under %USERPROFILE%\OneDrive\Desktop\Tor Browser when OneDrive backs
  up the desktop and Tor Browser is there.
  If Tor Browser is somewhere else, set the path in Settings > Tor.
  Keep Tor Browser open while Monolith runs. While it is closed, messages
  wait, and Monolith sends them after Tor Browser is open again.
- The Tor Expert Bundle, run with a torrc that has
      ControlPort 9051
      CookieAuthentication 1
  In Settings > Tor choose "Tor Expert Bundle"; the cookie is in its data
  folder, by default %APPDATA%\tor\control_auth_cookie.

The settings take effect when Monolith starts again. Settings > Tor shows
whether Tor answers.

Your data
---------

- Your profile (identities, contacts, messages), encrypted under your
  passphrase: %LOCALAPPDATA%\Monolith\Profile.
- The settings of the window (size, theme, Tor endpoints):
  %LOCALAPPDATA%\Monolith\desktop.conf.

On Windows, Monolith does not check who else may read the profile folder;
it relies on the permissions Windows gives the user's own AppData folder.
Monolith keeps no log, sends no telemetry and checks for no updates. A card
you save goes to your Downloads folder.
