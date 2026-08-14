# Marketplace Compliance Checklist

Review date: 2026-08-14

Release: 0.1.0

Status: In preparation. This checklist records repository evidence and remaining release gates; it is not an OpenAI approval, endorsement, or certification.

## Listing and identity

- [x] The package name, display name, developer name, category, capabilities, descriptions, and starter prompts are final and consistent with the plugin manifest.
- [x] Listing copy distinguishes Codex lifecycle delivery from ChatGPT guidance-only behavior.
- [x] Listing copy states that the desktop application is a separate prerequisite.
- [x] Listing and release materials contain no claim of OpenAI endorsement, approval, verification, or guaranteed acceptance.
- [x] Availability is declared as all countries and regions supported by the Marketplace portal, with no additional exclusions.
- [ ] The Marketplace publisher identity is verified and matches `Jade Lin`.

## Privacy, permissions, and security

- [x] The published privacy policy describes local event data, purposes, recipients, retention, deletion controls, and security boundaries.
- [x] The published terms and support pages match the plugin's local-only operation.
- [x] The package declares no MCP server, authentication flow, custom UI, telemetry, or remote data endpoint.
- [x] Hook installation and enablement do not imply trust; the user must review and explicitly trust hook definitions.
- [x] Permission-request observation never returns an approval or denial decision.
- [x] The package does not request credentials or raw conversation history.

## Package and runtime evidence

- [x] The package uses confined manifest paths and rejects undeclared files, executables, path escapes, placeholders, and forbidden capabilities.
- [x] Production icons and license provenance are present; screenshots are omitted because the plugin has no custom UI.
- [x] Packaged launchers select only declared targets, preserve standard input, emit no model-visible output, and fail closed on unsupported or tampered targets.
- [x] Runtime diagnostics distinguish plugin, legacy, and overlapping hook sources and report compatibility and remediation without exposing source paths.
- [x] Migration preserves unrelated configuration and rolls back when trust, compatibility, delivery verification, or provenance checks fail.
- [x] Positive and negative reviewer cases are complete and reproducible from repository fixtures; final-archive execution remains part of packaged acceptance.
- [x] Automated consistency checks cover the manifest, skill, documentation, policies, submission metadata, release contents, and observed runtime handling.
- [ ] Clean local Marketplace install, trust, delivery, update, rollback, disable, and removal acceptance passes on every declared host.
- [ ] Final archive checksums, signing evidence where applicable, dependency and license inventory, vulnerability scan, and secret inspection are complete.

## Publication gates

- [ ] Website, support, privacy-policy, terms, repository, and desktop-release URLs are publicly reachable without authentication.
- [ ] Current Marketplace rules have been revalidated and the review date refreshed immediately before submission.
- [ ] The final archive passes strict OpenSpec validation and all automated and packaged acceptance suites.
- [ ] The submission portal accepts the final package preflight without an unresolved package or scanner restriction.

The release must remain marked not submission-ready while any publication gate is unchecked.
