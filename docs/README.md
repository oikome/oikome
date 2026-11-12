# Oikome docs

| File | What it is | Language |
|------|------------|----------|
| [`architecture.md`](architecture.md) | The architecture: what is built, how, and why (Decision Log in §16). The canonical specification | English |
| [`prd.md`](prd.md) | Product requirements for the user-facing apps: who they are for, the pages, plugins, devices, permissions, milestones | English |
| [`content-viewing.md`](content-viewing.md) | The agreed content-viewing direction and delivery plan: light core viewers, safe editing, local opening and optional content plugins | English |
| [`prototype/`](prototype/prototype.html) | The design folder: the clickable prototype of the apps (`prototype.html`, nine languages in `i18n/`, photos in `lisbon/`) and the design references (`references/`) | — |

`AGENT.md` at the repository root is the development protocol for contributors and agents.

When a decision changes one of these, update the others in the same change: the architecture for what the node does, the PRD for what people get, the prototype for how it looks. `node docs/prototype/i18n/check.cjs <lang>` checks a translation against `en.js`.
