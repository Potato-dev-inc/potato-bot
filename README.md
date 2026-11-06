# Potato Bot

Potato Bot is a Windows desktop app: a chat friend with its own small brain, a memory, a mood and a
voice, and a small Linux computer it can use when you let it. You can make several bots. Each has
its own name, face, persona, memory and settings.

**Status: first public version (1.0.0).** It is a young app. Expect rough edges, and please report them.

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

Not in this version: anything that is not in the list above.

## Requirements

- Windows 10 or 11, 64-bit.
- **Node.js 20 or newer** on your PC. The bot's brain is a Node program; the app does not ship Node.
  If Node is missing the app shows an "Install Node.js" button, which runs `winget` in a visible window.
- **WSL 2**, only for the bot's computer. The app can run without it, but the bot then has no computer.
  The first set-up installs an Ubuntu 24.04 distro for the bot with `wsl --install`, then builds the
  desktop inside it from recipes. This downloads Ubuntu and many packages, so it needs an internet
  connection and a good amount of time and disk space. Windows may ask for a restart the first time WSL is turned on.
- **Your own model API key.** The brain can use DeepSeek, OpenCode Go, a terminal or ACP agent you
  already run, or a custom OpenAI-compatible provider. Model usage is billed by that provider, not by this app.

## Install

1. Download `Potato Bot_1.0.0_x64_en-US.msi` from the Releases page of this repository.
2. Double-click it and follow the installer.
3. The installer is **not code-signed**, so Windows SmartScreen will warn you. Choose
   **More info**, then **Run anyway**. If you would rather check the file first, compare its SHA-256
   (shown on the release page) with `Get-FileHash "Potato Bot_1.0.0_x64_en-US.msi"` in PowerShell.
4. Start Potato Bot, open Settings, add your model key, and make your first bot.

The app keeps its data under `%APPDATA%\com.puppetmaster.app` (the folder name is from an earlier
name of the project). Bot chats, settings, memory, artifacts and the sandbox live under `potato/`
there. Deleting a bot moves its files to `potato/.removed/`.

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
npm install
npm run dev                      # run the app in development
```

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
