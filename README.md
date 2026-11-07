# Potato Bot

Potato Bot is a Windows desktop app: a chat friend with its own small brain, a memory, a mood and a
voice, and a small Linux computer it can use when you let it. You can make several bots. Each has
its own name, face, persona, memory and settings.

**Status: an early public version.** It is a young app. Expect rough edges, and please report them.

## What it does

- Make several bots, each with its own name, face, persona, brain, memory, mood and settings.
- Chat with a bot. Threads are kept; messages can be saved as files; the bot keeps artifacts,
  routines and a list of open work.
- Give a bot its own Linux computer (see below): a screen you can watch, a mouse and keyboard, a
  browser, a terminal and files.
- Let a bot ask before it starts a helper, then run that helper in a real terminal pane. A bot can
  also use a coding agent you already have on this PC ("My TUI agent", as a terminal or as an ACP chat).
- Add tool servers (MCP) from Settings > Connectors.
- Speak replies aloud and send desktop notifications.
- Set a daily spending limit shared by all bots, or turn the limit off.
- Group chats with several of your bots are available as an opt-in switch in Settings > Potato Bot (off by default).
  A group holds up to 10 bots.
- Use your bots from your phone: a small web app (no store, no account) that you pair by scanning a QR code in
  Settings > Phone. It lists your bots, opens a chat, answers cards (Allow, Deny, choices), and can stop a job.
  See "The phone app" below for what it needs and its limits.
- Update in one click: the app shows "Update available" and an Update button. Your bots, settings and the bot's
  computer are kept.

Limits you should know about:

- **Up to 10 bots** (and up to 50 groups).
- **Languages:** the app is in English and Traditional Chinese (it follows your system language, or you pick one
  in Settings). Everything else is English. The phone app is in English and Traditional Chinese too. A bot
  answers in whatever language you and the model use; that does not depend on the app's language.

Not in this version: anything that is not in the list above.

## Requirements

- Windows 10 or 11, 64-bit.
- Nothing else to install for the bot's brain: the installer includes its own Node.js. (Tools you connect yourself, such as a coding agent you already run, may need their own Node.js or npm.)
- **WSL 2**, only for the bot's computer. The app can run without it, but the bot then has no computer.
  The first set-up installs an Ubuntu 24.04 distro for the bot with `wsl --install`, then builds the
  desktop inside it from recipes. This downloads Ubuntu and many packages, so it needs an internet
  connection and a good amount of time and disk space. Windows may ask for a restart the first time WSL is turned on.
- **Your own model API key.** The brain can use DeepSeek, OpenCode Go, a terminal or ACP agent you
  already run, or a custom OpenAI-compatible provider. Model usage is billed by that provider, not by this app.

## Install

1. Download the `Potato-Bot_<version>_x64_en-US.msi` from the Releases page of this repository.
2. Double-click it and follow the installer.
3. The installer is **not code-signed**, so Windows SmartScreen will warn you. Choose
   **More info**, then **Run anyway**. If you would rather check the file first, compare its SHA-256
   (shown on the release page) with `Get-FileHash "Potato-Bot_<version>_x64_en-US.msi"`  in PowerShell.
4. Start Potato Bot, open Settings, add your model key, and make your first bot.

Updates: when a new version is out, the app shows a small "Update available" notice. Press Update; it downloads the signed installer, checks it, installs over the old version and restarts. Your bots, settings and the bot's computer are kept, and nothing installs until you press the button.

The app keeps its data under `%APPDATA%\com.puppetmaster.app` (the folder name is from an earlier
name of the project). Bot chats, settings, memory, artifacts and the sandbox live under `potato/`
there. Deleting a bot moves its files to `potato/.removed/`.

## The phone app

Settings > Phone shows a QR code. Scanning it with a phone pairs that phone (one pairing code, used once; each phone
gets its own token, and Settings lists phones and signs one out at once). The phone then opens a web page served
by your PC. There is no hosted service and no account.

- **Same Wi-Fi:** the phone reaches your PC directly at its home-network address (plain `http`). Some Wi-Fi networks
  (guest, mesh, hotspot) stop phones from reaching each other; then it cannot connect.
- **Installing it to the home screen and push notifications need `https`.** A plain `http` address works for
  chatting, but the phone will not offer to install the page or allow notifications on it. For `https`, turn on
  **Reach from outside your Wi-Fi** in Settings > Phone, or use your own `https` address.
- **Reach from outside your Wi-Fi** uses a Cloudflare "quick tunnel" (the `cloudflared` program inside the app).
  Cloudflare says quick tunnels are for testing and may stop at any time. The tunnel's address stays the same while
  it keeps running, including across app restarts and updates, but **it changes when the tunnel truly restarts** (for
  example after you restart Windows). A phone paired with the old address has to scan the QR code again. The tunnel
  is off by default, and uninstalling the app ends it.
- Windows Firewall is read, never changed without your press.
- The sign-in card never takes a password on the phone; it says to sign in on your PC. Bot screens are not on the phone yet.

## Safety notes

- **The bot's computer is a separate Linux system** (a WSL 2 distro of its own). Windows drives are not
  mounted inside it and it cannot start Windows programs. The bot does what it likes in there, and it
  can reach the internet, so treat it like a real computer someone else is using.
- **When to ask** has three levels per bot: *Ask first*, *Ask when risky* and *Full*. At **Full** the
  bot works on its own inside its computer and asks only with questions and when a login is missing.
  Pick Full only for bots you trust with that.
- **No limit** turns off the daily spending limit. Spending then will not stop by itself. The default
  limit is $0.50 a day per bot.
- **Passwords and logins are yours to type at every level.** The app's own wording is that
  passwords and logins are always yours to type, so the bot does not ask for them. But
  anything you sign in to on the bot's computer (for example in its browser) lives on that computer,
  and the bot can use it. Use accounts you are happy for the bot to use, and keep your main
  passwords out of there. Tokens you give to Connectors are held in a separate process in the
  computer that the bot's own user cannot read.
- You are responsible for what you let a bot do, and for the cost of the model you connect.

## Build from source

You need Node.js 22+, Rust 1.91+ with Cargo, the Tauri 2 build prerequisites for Windows (Microsoft
C++ Build Tools and WebView2), and WSL 2 if you want to try the bot's computer.

```bash
npm ci                                   # or npm install
npm run build -w @puppet-master/shared   # the shared types the app imports
npm run build -w @puppet-master/app      # the web UI, the brain, the sandbox files, the licences, Node.js, cloudflared
npm run dev                              # run the app in development
```

The two `build` lines matter on a clean clone: the Rust crate needs the folders they create
(`packages/app/dist`, `resources/potato-brain`, `resources/potato-sandbox`, `resources/legal`, `resources/node`,
`resources/bin`, `resources/phone`), and `cargo test` or `cargo check` fail with "glob pattern ... path not found"
without them. The app build downloads the official Node.js and cloudflared once (about 85 MB, checked against
pinned checksums, cached under `%LOCALAPPDATA%\potato-bot-build-cache`), so it needs network access the first time.

Development builds turn every Labs switch on; release builds do not (see below).

Build the installer (MSI). The build bundles the brain and the sandbox files first:

```bash
npm install
npm run tauri -- build --bundles msi
```

The MSI is written to `packages/app/src-tauri/target/release/bundle/msi/`. Tauri downloads WiX
on first use.

Tests:

```bash
npm test -w @puppet-master/potato-brain
npm test -w @puppet-master/app
npm run check:warnings
cargo test --lib --manifest-path packages/app/src-tauri/Cargo.toml
```

## Parts of the repository

| Part | What it does |
|---|---|
| `packages/app` | The Tauri 2 desktop app (React UI, terminal panes, local bridge, bot engine) |
| `packages/potato-brain` | The Node program that thinks and replies |
| `packages/potato-sandbox` | The WSL 2 guest files, desktop recipes and the noVNC viewer for the bot's computer |
| `packages/shared` | Shared TypeScript types |

## Licence

MIT, see `LICENSE`. Parts are ported from other projects under their own licences (Apache-2.0, MIT,
MPL-2.0); see `THIRD-PARTY-NOTICES.md` and `LICENSES/`. The installer puts these in its `legal` folder.
