# OpenAI Plugin Contract Fixtures

This directory records the public OpenAI plugin contract reviewed on 2026-08-14 and the matching Codex CLI fixture version used by Lili.

The files are a machine-readable, paraphrased snapshot of the linked official documentation. They are not a replacement for the upstream documentation or an assertion that OpenAI publishes the local JSON Schema. Update the dated fixture and `../matrix.json` whenever the reviewed contract or supported Codex version changes.

`contract.json` separates documented platform behavior from Lili policy and unresolved submission preflight questions. `hooks.schema.json` defines the exact hook subset that a Lili plugin package may use.

Local release targets are derived from `.github/workflows/Publish-Release.yaml`. Nix evaluation-only systems are intentionally excluded from the Marketplace artifact matrix until they have a matching published desktop bundle.
