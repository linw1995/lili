# Lili Terms of Service

Effective date: 2026-08-14

These terms apply to the Lili desktop application and Lili ChatGPT/Codex plugin maintained by Jade Lin. By installing or using Lili, you agree to these terms. If you do not agree, do not install or use Lili.

## Service description

Lili is a local desktop companion. On supported Codex versions, explicitly trusted plugin hooks can forward supported lifecycle events to the separately installed desktop application. On ChatGPT, the plugin provides setup and troubleshooting guidance only and does not provide automatic conversation lifecycle observation.

Lili does not install the desktop application through the plugin, approve or deny permission requests, provide a hosted synchronization service, or guarantee availability on an unreviewed Codex version or unsupported host.

## User responsibilities

You are responsible for:

- installing compatible Lili, plugin, Codex, operating-system, and architecture versions;
- reviewing and explicitly trusting the exact hook definitions before enabling them;
- protecting your operating-system account and local files;
- reviewing native action executables, argv, working directories, and explicit environment additions;
- keeping backups and following the documented migration, rollback, and deletion order;
- complying with applicable law and the terms of OpenAI, GitHub, your operating system, and any executable or service you configure.

Do not use Lili to bypass authorization, extract credentials or private data, interfere with other integrations, or run actions you are not authorized to run.

## Local actions and third-party services

Optional native actions are user-selected programs that run with the current operating-system user's authority. They are not sandboxed by Lili and may read files, use the network, or modify data. You assume responsibility for reviewing and operating them.

OpenAI plugin distribution, Codex, ChatGPT, GitHub releases, issue tracking, and operating-system services are provided by third parties under their own terms. Lili does not control their availability, review decisions, data handling, or changes.

## License

The Lili software is licensed under the Apache License, Version 2.0. The software license governs rights to copy, modify, and distribute the code. These service terms govern use of the plugin listing, documentation, support channel, and distributed application as a service offering. If these terms conflict with rights granted by the Apache License for the software itself, the Apache License controls those software rights.

## Availability and changes

Lili may change, suspend, or discontinue features and supported versions. Plugin installation or Marketplace availability does not guarantee that every surface exposes the same capabilities. Current compatibility and limitations are documented with each release.

## Warranty disclaimer and limitation of liability

To the maximum extent permitted by law, Lili is provided on an "AS IS" and "AS AVAILABLE" basis, without warranties of any kind. The Apache License warranty and liability terms also apply to the software.

To the maximum extent permitted by law, Jade Lin will not be liable for indirect, incidental, special, consequential, exemplary, or punitive damages, or for loss of data, profits, goodwill, or business interruption arising from use of Lili. Some jurisdictions do not allow certain exclusions or limitations, so these provisions apply only to the extent permitted.

## Termination and removal

You may stop using Lili at any time. Remove the plugin through supported Codex plugin controls, uninstall provenance-owned legacy integration separately, quit and remove the desktop application, and delete local data according to the [security and operations guide](security-and-operations.md#backup-reset-and-uninstall).

Plugin removal does not automatically delete the desktop application, local data, pets, actions, spool, backups, unrelated configuration, or legacy integration.

## Changes and contact

Material changes will update this file and its effective date. Continued use after a change means acceptance of the revised terms to the extent permitted by law. Questions may be submitted through [Lili Support](support.md).
