# Lili Support

Lili is maintained by Jade Lin. Support is provided through the public [Lili issue tracker](https://github.com/linw1995/lili/issues).

Before opening an issue:

1. Read the [configuration guide](configuration.md) and [security and operations guide](security-and-operations.md).
2. Run `codex --version` and the release binary's `lili integrate inspect` command.
3. Record the Lili desktop version, plugin version, operating system and architecture, plugin installed/enabled status, hook source, IPC compatibility, and the event identifier and timestamp from the most recent accepted plugin event.
4. Reproduce the problem without private prompts or credentials. When reporting storage behavior, include only the platform and whether the Lili application directory, SQLite database, runtime credentials, or Hook diagnostics are available.

Do not include API keys, tokens, forwarding credentials, raw prompts, complete assistant messages, approval arguments, conversation exports, private database files, rollout logs, spool files, process dumps, or inherited environment values in a support request. Redact local usernames and paths when they are not required to reproduce the issue.

Security-sensitive reports that should not be public may use GitHub's private vulnerability reporting feature when it is available for the repository. Do not attach live credentials; rotate any credential that may have been exposed.

## Supported scope

Support covers the published Lili desktop releases, the declared plugin/desktop compatibility range, Codex `0.147.0`, and the operating-system and architecture targets listed in the release. Other versions and hosts may be inspected, but are not represented as reviewed or supported.

The plugin does not provide ChatGPT lifecycle notifications. On ChatGPT it provides setup and troubleshooting guidance only.
