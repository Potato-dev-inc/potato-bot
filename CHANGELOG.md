# Changelog

## 1.0.11

- The phone app is redone: a header on every screen, the message box stays just above the keyboard, and a new black-and-white look in light and dark.
- Watch a bot's computer live from the phone. To use its mouse and keyboard from the phone, first turn on "Can control the bot's computer" for that phone in Settings > Phone on your PC (off by default).
- Spaces on the phone, when Spaces is on: read spaces and channels, and send messages. Inviting, removing people and adding bots stay on the PC.
- Questions and approvals now sit in the chat instead of covering half the screen.
- Not tried yet on real phones: the keyboard behaviour, and a real sandbox screen over the phone.

## 1.0.10

- Fix: Settings > Phone was blank. It shows the pairing QR, your phones and the other phone settings again.

## 1.0.9

- Spaces screens redone on the Potato look: the Bots panel, the sheets, the space list, the channel list and the empty screen. Sheets fit narrow windows.
- Opening Invite no longer makes a new invite each time. You can see open invites and cancel them, so you are never locked out at three.
- The Spaces page no longer stays empty if it opened before Spaces was ready; it tries again on its own.
- Messages from the same person sit together, the composer has a file button and an @ picker for bots, and sending, failed and read-only states say what is going on.
- Still not tried: two real PCs talking to each other, and a real model answering inside a space.

## 1.0.8

- New, early and off by default: Spaces, small-group chat between people's own computers, with no server keeping your messages. Turn it on in Settings > Potato Bot > Labs > Spaces. You make a private space, invite friends with a link or QR text, talk in channels, share files, and add your own bots to a channel. Bots can join a channel and answer when someone calls them.
- Spaces works while people are online at the same time. If nobody else is online, your messages wait until someone is.
- Not tried yet: two real PCs talking to each other, and a real model answering inside a space. Some Spaces screens are still being reworked.

## 1.0.7

- The bot's computer screen is closed to every other user: the display now checks a key (X cookie) that only the sandbox account can read, a new one at every start. The secret hub and the tool hub were adjusted to match.
- A custom agent brain gets a private view of its folder, a kept 1 GB home and its own secrets.
- An existing sandbox gets the new guest files the next time it starts (no reset). The screen key applies at that start; a sandbox that is running keeps its old screen until it stops or idles out.

## 1.0.6

- New: the OpenCode Go key can be set in Settings, and each bot can pick a Go model.
- Two comments no longer name a folder on the maintainer's PC.

## 1.0.5

- New: use your bots from your phone. Settings > Phone shows a QR; scanning it pairs one phone (no password, no account) and the phone web app can be added to the home screen. It lists your bots with a status dot and Needs you, opens a chat with photos, live steps and Queued, answers cards (Allow once, Allow for this job, Deny, choices, Keep going or Stop), stops a job, pauses everything, and follows the phone language (English or Traditional Chinese). The sign-in card never takes a password on the phone: it says Sign in on your PC. Every phone has its own token; Settings lists phones and signs one out at once. Phones on the same Wi-Fi reach the PC directly; if the network blocks that, turn on Reach from outside your Wi-Fi (off by default) and scan again. Windows Firewall is read, never changed without your press. Notifications use Web Push sent from your PC (no hosted service). Bot screens are not on the phone yet.
- New: Reach from outside your Wi-Fi (Settings > Phone). Runs the bundled Cloudflare quick tunnel against the phone server only, as a separate process that survives app restarts and updates, so its address stays the same until it truly restarts. Cloudflare says quick tunnels are for testing and may stop at any time.
- The official cloudflared 2026.10.7 (win-x64, Apache-2.0) is now inside the app for the phone's away-from-home link. It is fetched and checked against a pinned sha256 at build time.
- Fixed: uninstalling the app (MSI) now ends the phone tunnel. The installer runs the app's own `--phone-cleanup` just before the files are removed, and only on a real uninstall: an update (one-click or a newer MSI) never stops the tunnel, so its address stays. The earlier hook only covered an NSIS installer, which is not what we ship.
- The web UI loads in smaller pieces (Settings and the Terminal window are their own chunks), so the app chunk is under 500 kB again without raising the warning limit.
- README: build-from-source steps that work on a clean clone, the phone app and its limits (https for installing and notifications, the tunnel address changes when the tunnel truly restarts), up to 10 bots, and the languages.
- The release check also requires the phone web app files, `cloudflared.exe`, `node.exe`, both languages and the uninstall action.

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
