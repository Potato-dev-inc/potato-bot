# Notice

Based on fishd by the maintainer (2026). It is vendored here, with changes, and relicensed under the MIT licence in `LICENSE`.

## The Fish persona

The Fish persona pack (`persona/packs/fish/`) is the maintainer's own character, published with this project.

## What is not shipped

No art, pet assets, meme-pack images or gifs are included. The desktop pet and the web page of the original fishd are not part of this package. There is no chat-app adapter in phase 1.

## Code ported from Rakazo (Apache-2.0)

Parts of `src/lifted/` are ported from Rakazo (https://github.com/elie222/rakazo), commit f48fa5c7, licensed under the Apache License, Version 2.0. The licence text is in `LICENSE-APACHE-2.0` in this folder. Copyright 2026 Rakazo contributors. Each ported file keeps its header, which names the source file and lists what Puppet Master changed. The table of files, changes and line counts is in `src/lifted/PORTED.md`. Thanks to the Rakazo authors for the code.

Ported files: `src/lifted/context-pruning.ts`, `tool-loop.ts`, `silent-reply.ts`, `prompt-data.ts`, `redact-secrets.ts`, `screen-lease.ts`, `tool-schemas.ts`, and the matching tests `test/lifted-*.test.ts`.

Rakazo has no NOTICE file of its own, so there is no upstream attribution text to carry over. The other brain files that look similar to Rakazo (`src/core/schedule-tools.ts`, `memory-tools.ts`, `skill-tools.ts`, `history-tools.ts`, `web-text.ts`, `question-tools.ts`, `message-user-tools.ts`, `mcp-tools.ts`) were written from ideas and copy no code, so they carry no Apache header.
