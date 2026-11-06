# Changelog

## 1.0.2

- Fixed: Set up said "The sandbox files that ship with the app are missing." on a PC that is not a development machine. The app built its sandbox manager at start-up from the saved settings, before it had been given its app handle, so it could not find its own install folder and kept "no files" for the whole run (a development PC hid this by finding the source tree). The handle is now set first, the folder lookup waits for it, and release builds no longer look in the source tree at all.
- Fixed: the Professional persona was not in the installer. All persona packs are now bundled.
- New: a build-time check (`scripts/check-release-bundle.mjs`) fails the build if any file the app needs, such as the sandbox files, recipes, persona packs and licences, is not in the bundle.

## 1.0.1

- Fixed: on a clean PC with Windows' Linux feature on and no Linux distribution yet, the sandbox page said the feature "is on" and also "needs" it, and Set up stayed off for good. The app now treats "WSL can run" as ready even when `wsl --list` exits non-zero because no distribution is installed (checked by exit code and by text that is not translated, so it also works on Chinese and other localized Windows). Set up can then install the first distribution.
- New: when Windows needs a restart before WSL works, the page says one thing, "Restart your PC to finish, then come back.", keeps Set up off, and checks again whenever it is opened. The Windows feature button also reports a restart if WSL still cannot run after it exits 0.

## 1.0.0

- First public version.
