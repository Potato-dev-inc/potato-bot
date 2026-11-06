# Changelog

## 1.0.1

- Fixed: on a clean PC with Windows' Linux feature on and no Linux distribution yet, the sandbox page said the feature "is on" and also "needs" it, and Set up stayed off for good. The app now treats "WSL can run" as ready even when `wsl --list` exits non-zero because no distribution is installed (checked by exit code and by text that is not translated, so it also works on Chinese and other localized Windows). Set up can then install the first distribution.
- New: when Windows needs a restart before WSL works, the page says one thing, "Restart your PC to finish, then come back.", keeps Set up off, and checks again whenever it is opened. The Windows feature button also reports a restart if WSL still cannot run after it exits 0.

## 1.0.0

- First public version.
