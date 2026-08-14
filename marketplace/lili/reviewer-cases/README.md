# Lili Marketplace Reviewer Cases

These fixtures make the Marketplace review workflow reproducible without credentials, private conversations, or access to a maintainer environment.

Use `positive.json` for supported behavior and `negative.json` for refusal and fail-closed behavior. Each case declares its own safe fixture data, ordered workflow, expected result shape, assertions, and repository automation evidence. Paths are relative to the repository root.

The cases do not authorize the setup skill to mutate plugin, trust, integration, or application state. A reviewer performs every state-changing Plugin Directory or Codex action explicitly.
