# Local Marketplace Acceptance Catalog

This catalog is a repository-owned template for testing the final Lili plugin archive with Codex `0.147.0`. The archive is not duplicated in source control. `scripts/test_local_marketplace.py` securely extracts the supplied archive into `plugins/lili` inside an isolated temporary copy of this catalog.

Run the round trip through the Nix app:

```sh
nix run .#marketplace-roundtrip -- --archive /absolute/path/to/lili-plugin-0.1.0.zip
```

The test creates an isolated `CODEX_HOME`, adds this catalog, lists the available plugin, installs and enables it, removes and reinstalls it to exercise the supported disabled/enabled boundary, installs a derived next-version catalog snapshot, restores the original archive as a rollback, removes the plugin, and removes the catalog.

Codex `0.147.0` has no separate `plugin enable`, `plugin disable`, or `plugin update` commands. The versioned mapping in `lifecycle.json` records the supported behavior used here: `plugin add` installs, enables, updates, or rolls back to the current catalog snapshot; `plugin remove` leaves the plugin not installed and not enabled. The derived next-version snapshot exists only inside the temporary acceptance directory and is never represented as a release artifact.
