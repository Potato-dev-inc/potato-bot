# Third-party notices

Potato Bot is MIT licensed (`LICENSE`). It includes or builds on the following. The licence texts are in `LICENSES/` and are installed with the app in its `legal` folder.

## Code ported into this repository

| Part | Source | Licence | Where |
|---|---|---|---|
| Parts of the sandbox guest scripts, recipes and viewer; parts of the brain's `src/lifted/`; `potato-speech.ts` | Rakazo, https://github.com/elie222/rakazo, commit f48fa5c7, Copyright 2026 Rakazo contributors | Apache-2.0 (`LICENSES/Apache-2.0.txt`) | Each ported file keeps its header. Tables: `packages/potato-sandbox/NOTICE.md`, `packages/potato-brain/NOTICE.md`, `packages/app/NOTICE.md`, the `PORTED.md` files |
| `desk-core` and `desk-proto` crates, and the `src/desk/` and `src/p2p/` ports | desktoplab, https://github.com/v7ren/desktoplab, commit 6e5360e | MIT (`LICENSES/MIT-desktoplab.txt`) | `packages/app/src-tauri/crates/PORTED.md` |
| The brain (`packages/potato-brain`) | fishd, by the same maintainer | MIT (`packages/potato-brain/LICENSE`) | `packages/potato-brain/NOTICE.md` |

## Vendored libraries

| Library | Version | Licence | Where |
|---|---|---|---|
| noVNC (unmodified) | 1.7.0, npm `@novnc/novnc`, https://github.com/novnc/noVNC | MPL-2.0 core (`LICENSES/MPL-2.0.txt`), other parts under their own licences, see `packages/potato-sandbox/viewer/novnc/LICENSE.txt` | `packages/potato-sandbox/viewer/novnc/` |
| pako | bundled with noVNC | MIT | `packages/potato-sandbox/viewer/novnc/vendor/pako/LICENSE` |
| Node.js (official win-x64 build, unmodified `node.exe`) | 22.20.0, https://nodejs.org | MIT and the licences of its bundled parts (`LICENSES/Node.txt`) | `resources/node/node.exe` in the installed app; fetched and checked against SHASUMS256.txt at release build time (`scripts/fetch-node.mjs`) |

The MPL-2.0 files are unmodified. Their source is the noVNC 1.7.0 package linked above.

## Credit, no code

The loading steps in `packages/app/src/components/cockpit/potato/WorkSteps.tsx` follow the design idea of KokonutUI "AI State Loading" (https://kokonutui.com, MIT, Copyright 2025 kokonutUI). It is a rewrite; none of its source is copied.

## Dependencies

The app is built from npm and Rust packages. Of the Rust crates in `Cargo.lock`, almost all are MIT, Apache-2.0, ISC, BSD, Zlib or Unicode licensed. Five are MPL-2.0 and used unmodified (`cssparser`, `cssparser-macros`, `dtoa-short`, `option-ext`, `selectors`); their source is on crates.io. No crate is GPL-only. Each crate's licence is in its package.

## Persona

The "Fish" persona pack (`packages/potato-brain/persona/packs/fish/`) is the maintainer's own character. See `packages/potato-brain/NOTICE.md`.
