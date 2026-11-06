# Changelog

## 1.0.4

- New: live limits for the bot's computer. Settings > Potato Bot > Computer limits has a CPU limit (percent of this PC) and a Memory limit (GB). They apply at once, with no restart of the bot's computer, and are applied again at every start. The suggested limits are half the processors and a quarter of the memory (at least 2 GB). The page shows what the computer uses now. Heavy jobs run slower with lower limits. The limits are cgroup v2 settings set by a root-only script in the guest, so nothing the bot runs can change them; the control service, the secret hub and the firewall rules stay outside them.
- Lighter desktop: the screen is drawn with two threads instead of one per core, the screen stream is capped at about 15 frames a second, Chrome starts with fewer background features, and a stopped desktop no longer leaves Cinnamon's compositor running into the next start.
- New: "Give unused memory back to Windows" (Settings > Potato Bot > Sandbox) adds one line, autoMemoryReclaim=gradual, to your .wslconfig, only when you press the button. A copy is kept, a value you set yourself is never changed, and it takes effect the next time WSL starts. A quiet bot's computer also gives cached memory back after five minutes.
- Fixed: the Node.js inside the app was never used (the lookup returned its folder instead of node.exe), so a PC with no Node still could not start the bot. It is found now, and the brain starts with it first.

## 1.0.3

- Node.js is now inside the app (the official 22.20.0 win-x64 build). Nobody has to install Node any more, and the "Install Node.js" button is gone.
- Fixed: after installing Node the app still said it could not be used and asked again. A running app keeps the PATH it started with, so a Node installed later was never seen. The app now uses its own Node first and, if it ever needs a system one, reads the current PATH from Windows and the usual install folders.
- New: one-click updates. A quiet "Update available" notice (and Settings > Potato Bot > Updates > Check for updates). Update downloads the signed installer, checks its signature, stops the bots and the bot's computer cleanly, installs over the old version and restarts. Your bots, settings and the bot's computer are kept. If a bot's job is running you are warned first. Nothing installs until you press Update, and a failed update leaves the old version working.

## 1.0.2

- Fixed: Set up said "The sandbox files that ship with the app are missing." on a PC that is not a development machine. The app built its sandbox manager at start-up from the saved settings, before it had been given its app handle, so it could not find its own install folder and kept "no files" for the whole run (a development PC hid this by finding the source tree). The handle is now set first, the folder lookup waits for it, and release builds no longer look in the source tree at all.
- Fixed: the Professional persona was not in the installer. All persona packs are now bundled.
- New: a build-time check (`scripts/check-release-bundle.mjs`) fails the build if any file the app needs, such as the sandbox files, recipes, persona packs and licences, is not in the bundle.

## 1.0.1

- Fixed: on a clean PC with Windows' Linux feature on and no Linux distribution yet, the sandbox page said the feature "is on" and also "needs" it, and Set up stayed off for good. The app now treats "WSL can run" as ready even when `wsl --list` exits non-zero because no distribution is installed (checked by exit code and by text that is not translated, so it also works on Chinese and other localized Windows). Set up can then install the first distribution.
- New: when Windows needs a restart before WSL works, the page says one thing, "Restart your PC to finish, then come back.", keeps Set up off, and checks again whenever it is opened. The Windows feature button also reports a restart if WSL still cannot run after it exits 0.

## 1.0.0

- First public version.
