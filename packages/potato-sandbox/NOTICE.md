# Notice

The sandbox guest and viewer scripts in this folder are partly ported from Rakazo (https://github.com/elie222/rakazo), commit f48fa5c7, licensed under the Apache License, Version 2.0. The licence text is in `LICENSE-APACHE-2.0` in this folder. Copyright 2026 Rakazo contributors. Each ported file keeps the Rakazo header, which names the source file and lists what Puppet Master changed. The per-folder `PORTED.md` files have the tables of files, changes and line counts.

Ported from Rakazo:

- `guest/control/`: `pm-sbx-control`, `pm-sbx-focus-or-launch`, `test_control.py`
- `guest/desktop/`: `pm-sbx-browser`, `pm-sbx-browser.desktop`, `pm-sbx-start-desktop.sh`, `pm-no-wslg.sh`, `pm-sbx-chrome-policy.json`, and the recipe data in `recipes/` (`base.json`, `light.json`, `coding.json`; `xfce.json`, `daily.json` and `zorin-look.json` are not Rakazo)
- `guest/egress/`: `pm-sbx-egress.sh`, `pm-sbx-egress-probe.sh`
- `guest/page-browser/`: `pm-sbx-page-browser`, `test_page_browser.py`
- `viewer/`: `embed.html`, `clipboard-bridge.js`, `mobile-keyboard.js`

Written by Puppet Master, not Rakazo: `guest/desktop/pm-sbx-boot.sh`, `pm-sbx-zorin-look.sh`, `pm-sbx-zorin-look.desktop`, `apply-recipe.py`, `profile-pm.sh`, `wsl.conf`, the MCP hub files, and `recipes/zorin-look.json`.

The viewer loads noVNC (MPL-2.0 and other licences in `viewer/novnc/`) and pako (`viewer/novnc/vendor/pako/`). Those keep their own licence files.
