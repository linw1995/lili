use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use lili_session::{
    CodexAdapterDiagnostics, CodexHookSource, CodexPluginAvailability, CodexPluginIpcCompatibility,
    CodexPluginSupport, CodexPluginTrustState,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{IntegrationInspection, UninstallOutcome, inspect, preview_uninstall, uninstall};

pub const PLUGIN_MIGRATION_SCHEMA_VERSION: u16 = 2;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginMigrationState {
    Blocked,
    InstallReady,
    AwaitingHookReview,
    AwaitingVerification,
    CleanupReady,
    PluginPrimary,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginMigrationEvidence {
    pub exact_hooks_reviewed_by_user: bool,
    pub synthetic_delivery_verified: bool,
    pub overlap_deduplication_verified: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginMigrationAssessment {
    pub schema_version: u16,
    pub codex_home: PathBuf,
    pub state: PluginMigrationState,
    pub plugin_selector: String,
    pub install_command: Vec<String>,
    pub rollback_command: Vec<String>,
    pub cleanup_command: Vec<String>,
    pub blockers: Vec<String>,
    pub next_actions: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginRemovalOutcome {
    pub plugin_selector: String,
    pub legacy_configuration_changed: bool,
    pub desktop_application_changed: bool,
    pub application_data_changed: bool,
}

impl PluginMigrationAssessment {
    pub fn cleanup_allowed(&self) -> bool {
        self.state == PluginMigrationState::CleanupReady
    }
}

pub fn assess_plugin_migration(
    inspection: &IntegrationInspection,
    diagnostics: &CodexAdapterDiagnostics,
    plugin_selector: &str,
    evidence: &PluginMigrationEvidence,
) -> PluginMigrationAssessment {
    let selector_valid = valid_plugin_selector(plugin_selector);
    let legacy_active = diagnostics.plugin.hook_source == CodexHookSource::Legacy
        || diagnostics.plugin.hook_source == CodexHookSource::Overlap;
    let plugin_installed = diagnostics.plugin.installed == Some(true);
    let plugin_enabled = diagnostics.plugin.enabled == Some(true);
    let plugin_compatible =
        diagnostics.plugin.ipc_compatibility == CodexPluginIpcCompatibility::Supported;
    let real_delivery = diagnostics.plugin.last_accepted_plugin_event.is_some()
        && diagnostics.plugin.trust_state == CodexPluginTrustState::TrustedAtLastDelivery;
    let cleanup_preview = legacy_active.then(|| preview_uninstall(&inspection.codex_home));

    let mut blockers = Vec::new();
    if !selector_valid {
        blockers.push("The plugin selector is invalid.".to_owned());
    }
    if diagnostics.plugin.codex_support != CodexPluginSupport::Supported {
        blockers
            .push("The installed Codex version is not in the reviewed plugin matrix.".to_owned());
    }
    if plugin_installed && !plugin_compatible {
        blockers.push("The installed plugin and desktop IPC versions are incompatible.".to_owned());
    }
    if let Some(Err(error)) = cleanup_preview.as_ref() {
        blockers.push(format!(
            "Legacy provenance cannot be safely cleaned up: {error}"
        ));
    } else if cleanup_preview
        .as_ref()
        .and_then(|preview| preview.as_ref().ok())
        .is_some_and(|preview| !preview.complete)
    {
        blockers.push("Legacy provenance has conflicts that require manual resolution.".to_owned());
    }

    let state = if !blockers.is_empty() {
        PluginMigrationState::Blocked
    } else if !plugin_installed {
        if diagnostics.plugin.availability == CodexPluginAvailability::Available {
            PluginMigrationState::InstallReady
        } else {
            blockers
                .push("The Lili plugin is not available from a configured Marketplace.".to_owned());
            PluginMigrationState::Blocked
        }
    } else if !plugin_enabled || !evidence.exact_hooks_reviewed_by_user || !real_delivery {
        PluginMigrationState::AwaitingHookReview
    } else if !evidence.synthetic_delivery_verified || !evidence.overlap_deduplication_verified {
        PluginMigrationState::AwaitingVerification
    } else if legacy_active {
        PluginMigrationState::CleanupReady
    } else {
        PluginMigrationState::PluginPrimary
    };

    let next_actions = match state {
        PluginMigrationState::Blocked => vec![
            "Preserve the current integration and resolve every reported blocker.".to_owned(),
        ],
        PluginMigrationState::InstallReady => vec![
            "Install the plugin with the exact reviewed command, keeping legacy hooks active."
                .to_owned(),
        ],
        PluginMigrationState::AwaitingHookReview => vec![
            "Review and trust the exact Lili hook definitions in Codex, then produce one real lifecycle event."
                .to_owned(),
        ],
        PluginMigrationState::AwaitingVerification => vec![
            "Verify one synthetic event, one real plugin event, empty hook output, and overlap deduplication."
                .to_owned(),
        ],
        PluginMigrationState::CleanupReady => vec![
            "Remove only provenance-owned legacy entries with `lili integrate uninstall`."
                .to_owned(),
        ],
        PluginMigrationState::PluginPrimary => vec![
            "Keep plugin and desktop versions within the supported compatibility range.".to_owned(),
        ],
    };

    PluginMigrationAssessment {
        schema_version: PLUGIN_MIGRATION_SCHEMA_VERSION,
        codex_home: inspection.codex_home.clone(),
        state,
        plugin_selector: plugin_selector.to_owned(),
        install_command: vec![
            "codex".to_owned(),
            "plugin".to_owned(),
            "add".to_owned(),
            plugin_selector.to_owned(),
            "--json".to_owned(),
        ],
        rollback_command: vec![
            "codex".to_owned(),
            "plugin".to_owned(),
            "remove".to_owned(),
            plugin_selector.to_owned(),
            "--json".to_owned(),
        ],
        cleanup_command: vec![
            "lili".to_owned(),
            "integrate".to_owned(),
            "uninstall".to_owned(),
        ],
        blockers,
        next_actions,
    }
}

pub trait PluginLifecycleHost {
    fn install(&mut self, plugin_selector: &str) -> Result<(), PluginMigrationError>;
    fn inspect(&mut self, codex_home: &Path) -> IntegrationInspection;
    fn rollback(&mut self, plugin_selector: &str) -> Result<(), PluginMigrationError>;
}

#[derive(Default)]
pub struct CodexPluginLifecycleHost;

impl PluginLifecycleHost for CodexPluginLifecycleHost {
    fn install(&mut self, plugin_selector: &str) -> Result<(), PluginMigrationError> {
        run_codex_plugin_command("add", plugin_selector)
    }

    fn inspect(&mut self, codex_home: &Path) -> IntegrationInspection {
        inspect(codex_home)
    }

    fn rollback(&mut self, plugin_selector: &str) -> Result<(), PluginMigrationError> {
        run_codex_plugin_command("remove", plugin_selector)
    }
}

pub fn install_plugin_with_rollback<H: PluginLifecycleHost>(
    host: &mut H,
    codex_home: &Path,
    assessment: &PluginMigrationAssessment,
) -> Result<IntegrationInspection, PluginMigrationError> {
    if assessment.schema_version != PLUGIN_MIGRATION_SCHEMA_VERSION
        || assessment.state != PluginMigrationState::InstallReady
        || !assessment.blockers.is_empty()
        || !valid_plugin_selector(&assessment.plugin_selector)
    {
        return Err(PluginMigrationError::FailedPrecondition);
    }
    let before = host.inspect(codex_home);
    let plugin = &before.codex_adapter.plugin;
    if plugin.codex_support != CodexPluginSupport::Supported
        || plugin.availability != CodexPluginAvailability::Available
        || plugin.installed != Some(false)
    {
        return Err(PluginMigrationError::FailedPrecondition);
    }
    if host.install(&assessment.plugin_selector).is_err() {
        host.rollback(&assessment.plugin_selector)
            .map_err(|_| PluginMigrationError::RollbackFailed)?;
        return Err(PluginMigrationError::PluginCommandFailed);
    }
    let inspection = host.inspect(codex_home);
    let plugin = &inspection.codex_adapter.plugin;
    let postcondition_met = plugin.availability == CodexPluginAvailability::Installed
        && plugin.installed == Some(true)
        && plugin.enabled == Some(true)
        && plugin.ipc_compatibility == CodexPluginIpcCompatibility::Supported;
    if postcondition_met {
        return Ok(inspection);
    }
    host.rollback(&assessment.plugin_selector)
        .map_err(|_| PluginMigrationError::RollbackFailed)?;
    Err(PluginMigrationError::InstallVerificationFailed)
}

pub fn install_plugin(
    codex_home: &Path,
    assessment: &PluginMigrationAssessment,
) -> Result<IntegrationInspection, PluginMigrationError> {
    install_plugin_with_rollback(&mut CodexPluginLifecycleHost, codex_home, assessment)
}

pub fn rollback_plugin(plugin_selector: &str) -> Result<(), PluginMigrationError> {
    remove_plugin(plugin_selector).map(|_| ())
}

pub fn remove_plugin(plugin_selector: &str) -> Result<PluginRemovalOutcome, PluginMigrationError> {
    remove_plugin_with_host(&mut CodexPluginLifecycleHost, plugin_selector)
}

pub fn remove_plugin_with_host<H: PluginLifecycleHost>(
    host: &mut H,
    plugin_selector: &str,
) -> Result<PluginRemovalOutcome, PluginMigrationError> {
    if !valid_plugin_selector(plugin_selector) {
        return Err(PluginMigrationError::InvalidSelector);
    }
    host.rollback(plugin_selector)?;
    Ok(PluginRemovalOutcome {
        plugin_selector: plugin_selector.to_owned(),
        legacy_configuration_changed: false,
        desktop_application_changed: false,
        application_data_changed: false,
    })
}

pub fn cleanup_legacy_after_verification(
    codex_home: &Path,
    assessment: &PluginMigrationAssessment,
) -> Result<UninstallOutcome, PluginMigrationError> {
    cleanup_legacy_after_verification_with_host(
        &mut CodexPluginLifecycleHost,
        codex_home,
        assessment,
    )
}

#[doc(hidden)]
pub fn cleanup_legacy_after_verification_with_host<H: PluginLifecycleHost>(
    host: &mut H,
    codex_home: &Path,
    assessment: &PluginMigrationAssessment,
) -> Result<UninstallOutcome, PluginMigrationError> {
    if !assessment.cleanup_allowed() || !assessment.blockers.is_empty() {
        return Err(PluginMigrationError::FailedPrecondition);
    }
    let current = host.inspect(codex_home);
    let plugin = &current.codex_adapter.plugin;
    if assessment.schema_version != PLUGIN_MIGRATION_SCHEMA_VERSION
        || assessment.codex_home != codex_home
        || current.codex_home != codex_home
        || !valid_plugin_selector(&assessment.plugin_selector)
        || plugin.codex_support != CodexPluginSupport::Supported
        || plugin.availability != CodexPluginAvailability::Installed
        || plugin.installed != Some(true)
        || plugin.enabled != Some(true)
        || plugin.ipc_compatibility != CodexPluginIpcCompatibility::Supported
        || plugin.hook_source != CodexHookSource::Overlap
    {
        return Err(PluginMigrationError::FailedPrecondition);
    }
    let preview = preview_uninstall(codex_home)?;
    if !preview.complete {
        return Err(PluginMigrationError::LegacyCleanupConflict);
    }
    let outcome = uninstall(codex_home)?;
    if !outcome.complete {
        return Err(PluginMigrationError::LegacyCleanupConflict);
    }
    Ok(outcome)
}

fn valid_plugin_selector(selector: &str) -> bool {
    let Some((name, marketplace)) = selector.split_once('@') else {
        return false;
    };
    !name.is_empty()
        && !marketplace.is_empty()
        && selector.len() <= 128
        && [name, marketplace].into_iter().all(|component| {
            component
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        })
}

fn run_codex_plugin_command(
    action: &str,
    plugin_selector: &str,
) -> Result<(), PluginMigrationError> {
    if !valid_plugin_selector(plugin_selector) || !matches!(action, "add" | "remove") {
        return Err(PluginMigrationError::InvalidSelector);
    }
    let status = Command::new("codex")
        .args(["plugin", action, plugin_selector, "--json"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|_| PluginMigrationError::PluginCommandFailed)?;
    if status.success() {
        Ok(())
    } else {
        Err(PluginMigrationError::PluginCommandFailed)
    }
}

#[derive(Debug, Error)]
pub enum PluginMigrationError {
    #[error("plugin migration preconditions are not satisfied")]
    FailedPrecondition,
    #[error("plugin selector is invalid")]
    InvalidSelector,
    #[error("Codex plugin command failed")]
    PluginCommandFailed,
    #[error("installed plugin failed post-install verification")]
    InstallVerificationFailed,
    #[error("plugin rollback failed")]
    RollbackFailed,
    #[error("legacy cleanup has unresolved conflicts")]
    LegacyCleanupConflict,
    #[error("legacy cleanup failed: {0}")]
    LegacyCleanup(#[from] crate::UninstallError),
}

#[cfg(test)]
mod tests {
    use std::{
        collections::VecDeque,
        fs,
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
    };

    use lili_session::{
        CodexPluginDiagnostics, CodexPluginTrustState, DESKTOP_VERSION, LastAcceptedCodexEvent,
        SessionEventKind, TESTED_CODEX_VERSION,
    };

    use crate::{build_install_plan, inspect_with_version, install_with_verifier};

    use super::*;

    static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            let sequence = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "lili-plugin-migration-{}-{sequence}",
                std::process::id()
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn legacy_inspection(temp: &TempDir) -> IntegrationInspection {
        let inspection = inspect_with_version(&temp.0, Some(TESTED_CODEX_VERSION.to_owned()));
        let plan = build_install_plan(&inspection, &temp.0.join("bin/lili-hook"), 42);
        install_with_verifier(&plan, |_| Ok(())).unwrap();
        inspect_with_version(&temp.0, Some(TESTED_CODEX_VERSION.to_owned()))
    }

    fn plugin_diagnostics(installed: bool) -> CodexPluginDiagnostics {
        CodexPluginDiagnostics::discovered(
            Some(TESTED_CODEX_VERSION),
            if installed {
                CodexPluginAvailability::Installed
            } else {
                CodexPluginAvailability::Available
            },
            installed,
            installed,
            Some(DESKTOP_VERSION),
            true,
        )
    }

    fn accepted_plugin_diagnostics() -> CodexAdapterDiagnostics {
        let mut diagnostics = CodexAdapterDiagnostics::with_discovery(
            Some(TESTED_CODEX_VERSION),
            [lili_session::CodexIntegrationSurface::Stop],
        )
        .with_plugin(plugin_diagnostics(true));
        diagnostics.plugin.trust_state = CodexPluginTrustState::TrustedAtLastDelivery;
        diagnostics.plugin.last_accepted_plugin_event = Some(LastAcceptedCodexEvent {
            event_id: "plugin-event".to_owned(),
            event_type: SessionEventKind::TurnCompleted,
            occurred_at_ms: 42,
            surface: lili_session::CodexIntegrationSurface::Stop,
            plugin_version: Some(DESKTOP_VERSION.to_owned()),
        });
        diagnostics
    }

    #[test]
    fn migration_waits_for_user_trust_and_both_verification_classes() {
        let temp = TempDir::new();
        let inspection = legacy_inspection(&temp);
        let mut diagnostics = inspection.codex_adapter.clone();
        diagnostics.plugin = plugin_diagnostics(false);
        let evidence = PluginMigrationEvidence {
            exact_hooks_reviewed_by_user: false,
            synthetic_delivery_verified: false,
            overlap_deduplication_verified: false,
        };
        let install = assess_plugin_migration(
            &inspection,
            &diagnostics,
            "lili@test-marketplace",
            &evidence,
        );
        assert_eq!(install.state, PluginMigrationState::InstallReady);

        let diagnostics = accepted_plugin_diagnostics();
        let awaiting = assess_plugin_migration(
            &inspection,
            &diagnostics,
            "lili@test-marketplace",
            &evidence,
        );
        assert_eq!(awaiting.state, PluginMigrationState::AwaitingHookReview);

        let ready = assess_plugin_migration(
            &inspection,
            &diagnostics,
            "lili@test-marketplace",
            &PluginMigrationEvidence {
                exact_hooks_reviewed_by_user: true,
                synthetic_delivery_verified: true,
                overlap_deduplication_verified: true,
            },
        );
        assert_eq!(ready.state, PluginMigrationState::CleanupReady);
    }

    struct FakeHost {
        inspections: VecDeque<IntegrationInspection>,
        install_result: Result<(), PluginMigrationError>,
        installed: usize,
        rolled_back: usize,
    }

    impl PluginLifecycleHost for FakeHost {
        fn install(&mut self, _plugin_selector: &str) -> Result<(), PluginMigrationError> {
            self.installed += 1;
            self.install_result
                .as_ref()
                .map(|_| ())
                .map_err(|_| PluginMigrationError::PluginCommandFailed)
        }

        fn inspect(&mut self, _codex_home: &Path) -> IntegrationInspection {
            self.inspections.pop_front().unwrap()
        }

        fn rollback(&mut self, _plugin_selector: &str) -> Result<(), PluginMigrationError> {
            self.rolled_back += 1;
            Ok(())
        }
    }

    #[test]
    fn failed_install_postcondition_rolls_back_without_legacy_cleanup() {
        let temp = TempDir::new();
        let inspection = legacy_inspection(&temp);
        let mut available = inspection.codex_adapter.clone();
        available.plugin = plugin_diagnostics(false);
        let assessment = assess_plugin_migration(
            &inspection,
            &available,
            "lili@test-marketplace",
            &PluginMigrationEvidence {
                exact_hooks_reviewed_by_user: false,
                synthetic_delivery_verified: false,
                overlap_deduplication_verified: false,
            },
        );
        let mut before = inspection.clone();
        before.codex_adapter.plugin = plugin_diagnostics(false);
        let mut host = FakeHost {
            inspections: VecDeque::from([before, inspection.clone()]),
            install_result: Ok(()),
            installed: 0,
            rolled_back: 0,
        };
        assert!(matches!(
            install_plugin_with_rollback(&mut host, &temp.0, &assessment),
            Err(PluginMigrationError::InstallVerificationFailed)
        ));
        assert_eq!(host.installed, 1);
        assert_eq!(host.rolled_back, 1);
        assert!(temp.0.join(crate::CONFIG_FILE_NAME).exists());
        assert!(temp.0.join(crate::HOOKS_FILE_NAME).exists());
    }

    #[test]
    fn invalid_selector_never_reaches_a_plugin_command() {
        let temp = TempDir::new();
        let inspection = legacy_inspection(&temp);
        let mut diagnostics = inspection.codex_adapter.clone();
        diagnostics.plugin = plugin_diagnostics(false);
        let assessment = assess_plugin_migration(
            &inspection,
            &diagnostics,
            "lili;remove@marketplace",
            &PluginMigrationEvidence {
                exact_hooks_reviewed_by_user: false,
                synthetic_delivery_verified: false,
                overlap_deduplication_verified: false,
            },
        );
        assert_eq!(assessment.state, PluginMigrationState::Blocked);
        assert!(matches!(
            rollback_plugin("lili;remove@marketplace"),
            Err(PluginMigrationError::InvalidSelector)
        ));
    }

    #[test]
    fn removal_outcome_has_an_explicit_plugin_only_scope() {
        let temp = TempDir::new();
        let inspection = inspect_with_version(&temp.0, Some(TESTED_CODEX_VERSION.to_owned()));
        let mut host = FakeHost {
            inspections: VecDeque::from([inspection]),
            install_result: Ok(()),
            installed: 0,
            rolled_back: 0,
        };
        let outcome = remove_plugin_with_host(&mut host, "lili@test-marketplace").unwrap();
        assert_eq!(host.rolled_back, 1);
        assert!(!outcome.legacy_configuration_changed);
        assert!(!outcome.desktop_application_changed);
        assert!(!outcome.application_data_changed);
    }

    #[test]
    fn verified_cleanup_uses_only_provenance_owned_uninstall() {
        let temp = TempDir::new();
        let inspection = legacy_inspection(&temp);
        let assessment = assess_plugin_migration(
            &inspection,
            &accepted_plugin_diagnostics(),
            "lili@test-marketplace",
            &PluginMigrationEvidence {
                exact_hooks_reviewed_by_user: true,
                synthetic_delivery_verified: true,
                overlap_deduplication_verified: true,
            },
        );
        let mut current = inspection.clone();
        current.codex_adapter = accepted_plugin_diagnostics();
        let mut host = FakeHost {
            inspections: VecDeque::from([current]),
            install_result: Ok(()),
            installed: 0,
            rolled_back: 0,
        };
        let outcome =
            cleanup_legacy_after_verification_with_host(&mut host, &temp.0, &assessment).unwrap();
        assert!(outcome.complete);
        assert!(!temp.0.join(crate::CONFIG_FILE_NAME).exists());
        assert!(!temp.0.join(crate::HOOKS_FILE_NAME).exists());
    }

    #[test]
    fn stale_or_cross_home_cleanup_assessment_preserves_legacy_integration() {
        let source = TempDir::new();
        let source_inspection = legacy_inspection(&source);
        let assessment = assess_plugin_migration(
            &source_inspection,
            &accepted_plugin_diagnostics(),
            "lili@test-marketplace",
            &PluginMigrationEvidence {
                exact_hooks_reviewed_by_user: true,
                synthetic_delivery_verified: true,
                overlap_deduplication_verified: true,
            },
        );

        let target = TempDir::new();
        let mut current = legacy_inspection(&target);
        current.codex_adapter = accepted_plugin_diagnostics();
        let mut host = FakeHost {
            inspections: VecDeque::from([current]),
            install_result: Ok(()),
            installed: 0,
            rolled_back: 0,
        };
        assert!(matches!(
            cleanup_legacy_after_verification_with_host(&mut host, &target.0, &assessment),
            Err(PluginMigrationError::FailedPrecondition)
        ));
        assert!(target.0.join(crate::CONFIG_FILE_NAME).exists());
        assert!(target.0.join(crate::HOOKS_FILE_NAME).exists());

        for plugin in [
            CodexPluginDiagnostics::discovered(
                Some(TESTED_CODEX_VERSION),
                CodexPluginAvailability::Installed,
                true,
                false,
                Some(DESKTOP_VERSION),
                true,
            ),
            CodexPluginDiagnostics::discovered(
                Some(TESTED_CODEX_VERSION),
                CodexPluginAvailability::Available,
                false,
                false,
                Some(DESKTOP_VERSION),
                true,
            ),
        ] {
            let mut current = source_inspection.clone();
            current.codex_adapter.plugin = plugin;
            let mut host = FakeHost {
                inspections: VecDeque::from([current]),
                install_result: Ok(()),
                installed: 0,
                rolled_back: 0,
            };
            assert!(matches!(
                cleanup_legacy_after_verification_with_host(&mut host, &source.0, &assessment),
                Err(PluginMigrationError::FailedPrecondition)
            ));
            assert!(source.0.join(crate::CONFIG_FILE_NAME).exists());
            assert!(source.0.join(crate::HOOKS_FILE_NAME).exists());
        }
    }
}
