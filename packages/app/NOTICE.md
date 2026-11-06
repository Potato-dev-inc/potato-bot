# Notice

The Puppet Master app is partly ported from Rakazo (https://github.com/elie222/rakazo), commit f48fa5c7, licensed under the Apache License, Version 2.0. Copyright 2026 Rakazo contributors. The licence text is `packages/potato-sandbox/LICENSE-APACHE-2.0`. The build copies the sandbox files into `src-tauri/resources/potato-sandbox/` (gitignored), with the same licence file and PORTED.md files.

Ported from Rakazo in `src/`:

- `lib/potato-speech.ts` (from Rakazo `packages/core/src/speech-text.ts`). Its header names the changes.

The Rust crates in `src-tauri/crates/` (`desk-core`, `desk-proto`) are ported from desktoplab under the MIT licence, not Rakazo. The MIT text is in `LICENSES/MIT-desktoplab.txt` at the repository root. The desktoplab source has no licence file; its Cargo.toml only says `license = "MIT"`, so that file is the standard MIT text with the project named as the holder.
