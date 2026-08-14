use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use lili_session::{
    CodexAdapterDiagnostics, CodexHookSource, CodexPluginAvailability, CodexPluginIpcCompatibility,
    CodexPluginSupport, CodexPluginTrustState,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    IntegrationInspection, UninstallOutcome, inspect_plugin, plugin_hooks_are_trusted,
    preview_uninstall, uninstall,
};

pub const PLUGIN_MIGRATION_SCHEMA_VERSION: u16 = 3;
pub const PLUGIN_MIGRATION_ASSESSMENT_FILE_NAME: &str = "lili-plugin-migration-assessment.json";
const MAX_PLUGIN_MIGRATION_ASSESSMENT_BYTES: u64 = 64 * 1024;

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
    pub verified_plugin_version: Option<String>,
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
    let plugin_identity_matches = diagnostics.plugin.plugin_id.as_deref() == Some(plugin_selector);
    let verified_plugin_version = diagnostics
        .plugin
        .last_accepted_plugin_event
        .as_ref()
        .and_then(|event| event.plugin_version.clone())
        .filter(|version| diagnostics.plugin.plugin_version.as_ref() == Some(version));
    let real_delivery = verified_plugin_version.is_some()
        && diagnostics.plugin.trust_state == CodexPluginTrustState::TrustedAtLastDelivery;
    let cleanup_preview = legacy_active.then(|| preview_uninstall(&inspection.codex_home));

    let mut blockers = Vec::new();
    if !selector_valid {
        blockers.push("The plugin selector is invalid.".to_owned());
    }
    if let Some(blocker) = codex_support_blocker(diagnostics.plugin.codex_support, real_delivery) {
        blockers.push(blocker.to_owned());
    }
    if plugin_installed && !plugin_compatible {
        blockers.push("The installed plugin and desktop IPC versions are incompatible.".to_owned());
    }
    if (plugin_installed || diagnostics.plugin.availability == CodexPluginAvailability::Available)
        && !plugin_identity_matches
    {
        blockers.push(
            "The discovered plugin does not match the selected Marketplace identity.".to_owned(),
        );
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
            format!(
                "Save this assessment as `{PLUGIN_MIGRATION_ASSESSMENT_FILE_NAME}`, then run the displayed cleanup command."
            ),
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
        verified_plugin_version,
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
            "cleanup".to_owned(),
            "--assessment".to_owned(),
            PLUGIN_MIGRATION_ASSESSMENT_FILE_NAME.to_owned(),
        ],
        blockers,
        next_actions,
    }
}

fn codex_support_blocker(support: CodexPluginSupport, real_delivery: bool) -> Option<&'static str> {
    match support {
        CodexPluginSupport::Supported => None,
        CodexPluginSupport::Unreviewed if real_delivery => None,
        CodexPluginSupport::Unreviewed => Some(
            "The unreviewed Codex version requires a verified real plugin delivery before legacy cleanup.",
        ),
        CodexPluginSupport::Unknown => Some("The installed Codex version could not be verified."),
    }
}

pub trait PluginLifecycleHost {
    fn install(
        &mut self,
        codex_home: &Path,
        plugin_selector: &str,
    ) -> Result<(), PluginMigrationError>;
    fn inspect(&mut self, codex_home: &Path, plugin_selector: &str) -> IntegrationInspection;
    fn hooks_trusted(&mut self, _codex_home: &Path, _plugin_selector: &str) -> bool {
        false
    }
    fn rollback(
        &mut self,
        codex_home: &Path,
        plugin_selector: &str,
    ) -> Result<(), PluginMigrationError>;
}

#[derive(Default)]
pub struct CodexPluginLifecycleHost;

impl PluginLifecycleHost for CodexPluginLifecycleHost {
    fn install(
        &mut self,
        codex_home: &Path,
        plugin_selector: &str,
    ) -> Result<(), PluginMigrationError> {
        run_codex_plugin_command(codex_home, "add", plugin_selector)
    }

    fn inspect(&mut self, codex_home: &Path, plugin_selector: &str) -> IntegrationInspection {
        inspect_plugin(codex_home, plugin_selector)
    }

    fn hooks_trusted(&mut self, codex_home: &Path, plugin_selector: &str) -> bool {
        plugin_hooks_are_trusted(codex_home, plugin_selector)
    }

    fn rollback(
        &mut self,
        codex_home: &Path,
        plugin_selector: &str,
    ) -> Result<(), PluginMigrationError> {
        run_codex_plugin_command(codex_home, "remove", plugin_selector)
    }
}

pub fn install_plugin_with_rollback<H: PluginLifecycleHost>(
    host: &mut H,
    codex_home: &Path,
    assessment: &PluginMigrationAssessment,
) -> Result<IntegrationInspection, PluginMigrationError> {
    if assessment.schema_version != PLUGIN_MIGRATION_SCHEMA_VERSION
        || assessment.state != PluginMigrationState::InstallReady
        || assessment.codex_home != codex_home
        || !assessment.blockers.is_empty()
        || !valid_plugin_selector(&assessment.plugin_selector)
    {
        return Err(PluginMigrationError::FailedPrecondition);
    }
    let before = host.inspect(codex_home, &assessment.plugin_selector);
    let plugin = &before.codex_adapter.plugin;
    if plugin.codex_support != CodexPluginSupport::Supported
        || plugin.availability != CodexPluginAvailability::Available
        || plugin.plugin_id.as_deref() != Some(assessment.plugin_selector.as_str())
        || plugin.installed != Some(false)
    {
        return Err(PluginMigrationError::FailedPrecondition);
    }
    if host
        .install(codex_home, &assessment.plugin_selector)
        .is_err()
    {
        host.rollback(codex_home, &assessment.plugin_selector)
            .map_err(|_| PluginMigrationError::RollbackFailed)?;
        return Err(PluginMigrationError::PluginCommandFailed);
    }
    let inspection = host.inspect(codex_home, &assessment.plugin_selector);
    let plugin = &inspection.codex_adapter.plugin;
    let postcondition_met = plugin.availability == CodexPluginAvailability::Installed
        && plugin.installed == Some(true)
        && plugin.enabled == Some(true)
        && plugin.plugin_id.as_deref() == Some(assessment.plugin_selector.as_str())
        && plugin.ipc_compatibility == CodexPluginIpcCompatibility::Supported;
    if postcondition_met {
        return Ok(inspection);
    }
    host.rollback(codex_home, &assessment.plugin_selector)
        .map_err(|_| PluginMigrationError::RollbackFailed)?;
    Err(PluginMigrationError::InstallVerificationFailed)
}

pub fn install_plugin(
    codex_home: &Path,
    assessment: &PluginMigrationAssessment,
) -> Result<IntegrationInspection, PluginMigrationError> {
    install_plugin_with_rollback(&mut CodexPluginLifecycleHost, codex_home, assessment)
}

pub fn rollback_plugin(
    codex_home: &Path,
    plugin_selector: &str,
) -> Result<(), PluginMigrationError> {
    remove_plugin(codex_home, plugin_selector).map(|_| ())
}

pub fn remove_plugin(
    codex_home: &Path,
    plugin_selector: &str,
) -> Result<PluginRemovalOutcome, PluginMigrationError> {
    remove_plugin_with_host(&mut CodexPluginLifecycleHost, codex_home, plugin_selector)
}

pub fn remove_plugin_with_host<H: PluginLifecycleHost>(
    host: &mut H,
    codex_home: &Path,
    plugin_selector: &str,
) -> Result<PluginRemovalOutcome, PluginMigrationError> {
    if !valid_plugin_selector(plugin_selector) {
        return Err(PluginMigrationError::InvalidSelector);
    }
    host.rollback(codex_home, plugin_selector)?;
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
    if assessment.schema_version != PLUGIN_MIGRATION_SCHEMA_VERSION
        || !assessment.cleanup_allowed()
        || assessment.codex_home != codex_home
        || !assessment.blockers.is_empty()
        || !valid_plugin_selector(&assessment.plugin_selector)
        || assessment.verified_plugin_version.is_none()
    {
        return Err(PluginMigrationError::FailedPrecondition);
    }
    if !host.hooks_trusted(codex_home, &assessment.plugin_selector) {
        return Err(PluginMigrationError::FailedPrecondition);
    }
    let current = host.inspect(codex_home, &assessment.plugin_selector);
    let plugin = &current.codex_adapter.plugin;
    let codex_version_allowed = matches!(
        plugin.codex_support,
        CodexPluginSupport::Supported | CodexPluginSupport::Unreviewed
    );
    if current.codex_home != codex_home
        || !codex_version_allowed
        || plugin.availability != CodexPluginAvailability::Installed
        || plugin.plugin_id.as_deref() != Some(assessment.plugin_selector.as_str())
        || plugin.installed != Some(true)
        || plugin.enabled != Some(true)
        || assessment.verified_plugin_version.as_ref() != plugin.plugin_version.as_ref()
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

pub fn load_plugin_migration_assessment(
    path: &Path,
) -> Result<PluginMigrationAssessment, PluginMigrationError> {
    let metadata =
        fs::symlink_metadata(path).map_err(|_| PluginMigrationError::AssessmentUnreadable)?;
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.len() > MAX_PLUGIN_MIGRATION_ASSESSMENT_BYTES
    {
        return Err(PluginMigrationError::AssessmentUnreadable);
    }
    let mut payload = Vec::with_capacity(metadata.len() as usize);
    fs::File::open(path)
        .map_err(|_| PluginMigrationError::AssessmentUnreadable)?
        .take(MAX_PLUGIN_MIGRATION_ASSESSMENT_BYTES + 1)
        .read_to_end(&mut payload)
        .map_err(|_| PluginMigrationError::AssessmentUnreadable)?;
    if payload.len() as u64 > MAX_PLUGIN_MIGRATION_ASSESSMENT_BYTES {
        return Err(PluginMigrationError::AssessmentUnreadable);
    }
    serde_json::from_slice(&payload).map_err(|_| PluginMigrationError::MalformedAssessment)
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
    codex_home: &Path,
    action: &str,
    plugin_selector: &str,
) -> Result<(), PluginMigrationError> {
    let status = codex_plugin_command(codex_home, action, plugin_selector)?
        .status()
        .map_err(|_| PluginMigrationError::PluginCommandFailed)?;
    if status.success() {
        Ok(())
    } else {
        Err(PluginMigrationError::PluginCommandFailed)
    }
}

fn codex_plugin_command(
    codex_home: &Path,
    action: &str,
    plugin_selector: &str,
) -> Result<Command, PluginMigrationError> {
    if !valid_plugin_selector(plugin_selector) || !matches!(action, "add" | "remove") {
        return Err(PluginMigrationError::InvalidSelector);
    }
    let mut command = Command::new("codex");
    command
        .args(["plugin", action, plugin_selector, "--json"])
        .env("CODEX_HOME", codex_home)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    Ok(command)
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
    #[error("plugin migration assessment could not be read safely")]
    AssessmentUnreadable,
    #[error("plugin migration assessment is malformed")]
    MalformedAssessment,
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
        .with_plugin_id(Some("lili@test-marketplace"))
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
        expected_home: PathBuf,
        installed: usize,
        rolled_back: usize,
    }

    impl PluginLifecycleHost for FakeHost {
        fn install(
            &mut self,
            codex_home: &Path,
            _plugin_selector: &str,
        ) -> Result<(), PluginMigrationError> {
            assert_eq!(codex_home, self.expected_home);
            self.installed += 1;
            self.install_result
                .as_ref()
                .map(|_| ())
                .map_err(|_| PluginMigrationError::PluginCommandFailed)
        }

        fn inspect(&mut self, codex_home: &Path, _plugin_selector: &str) -> IntegrationInspection {
            assert_eq!(codex_home, self.expected_home);
            self.inspections.pop_front().unwrap()
        }

        fn hooks_trusted(&mut self, codex_home: &Path, _plugin_selector: &str) -> bool {
            assert_eq!(codex_home, self.expected_home);
            self.inspections.front().is_some_and(|inspection| {
                inspection.codex_adapter.plugin.trust_state
                    == CodexPluginTrustState::TrustedAtLastDelivery
            })
        }

        fn rollback(
            &mut self,
            codex_home: &Path,
            _plugin_selector: &str,
        ) -> Result<(), PluginMigrationError> {
            assert_eq!(codex_home, self.expected_home);
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
            expected_home: temp.0.clone(),
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
            rollback_plugin(&temp.0, "lili;remove@marketplace"),
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
            expected_home: temp.0.clone(),
            installed: 0,
            rolled_back: 0,
        };
        let outcome = remove_plugin_with_host(&mut host, &temp.0, "lili@test-marketplace").unwrap();
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
            expected_home: temp.0.clone(),
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
            expected_home: target.0.clone(),
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
            )
            .with_plugin_id(Some("lili@test-marketplace")),
            CodexPluginDiagnostics::discovered(
                Some(TESTED_CODEX_VERSION),
                CodexPluginAvailability::Available,
                false,
                false,
                Some(DESKTOP_VERSION),
                true,
            )
            .with_plugin_id(Some("lili@test-marketplace")),
        ] {
            let mut current = source_inspection.clone();
            current.codex_adapter.plugin = plugin;
            let mut host = FakeHost {
                inspections: VecDeque::from([current]),
                install_result: Ok(()),
                expected_home: source.0.clone(),
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

    #[test]
    fn marketplace_identity_mismatch_blocks_assessment_and_cleanup() {
        let temp = TempDir::new();
        let inspection = legacy_inspection(&temp);
        let mut diagnostics = accepted_plugin_diagnostics();
        diagnostics.plugin.plugin_id = Some("lili@other-marketplace".to_owned());
        let assessment = assess_plugin_migration(
            &inspection,
            &diagnostics,
            "lili@test-marketplace",
            &PluginMigrationEvidence {
                exact_hooks_reviewed_by_user: true,
                synthetic_delivery_verified: true,
                overlap_deduplication_verified: true,
            },
        );
        assert_eq!(assessment.state, PluginMigrationState::Blocked);
        assert!(
            assessment
                .blockers
                .iter()
                .any(|blocker| blocker.contains("Marketplace identity"))
        );

        let mut ready_diagnostics = accepted_plugin_diagnostics();
        let ready = assess_plugin_migration(
            &inspection,
            &ready_diagnostics,
            "lili@test-marketplace",
            &PluginMigrationEvidence {
                exact_hooks_reviewed_by_user: true,
                synthetic_delivery_verified: true,
                overlap_deduplication_verified: true,
            },
        );
        ready_diagnostics.plugin.plugin_id = Some("lili@other-marketplace".to_owned());
        let mut current = inspection.clone();
        current.codex_adapter = ready_diagnostics;
        let mut host = FakeHost {
            inspections: VecDeque::from([current]),
            install_result: Ok(()),
            expected_home: temp.0.clone(),
            installed: 0,
            rolled_back: 0,
        };
        assert!(matches!(
            cleanup_legacy_after_verification_with_host(&mut host, &temp.0, &ready),
            Err(PluginMigrationError::FailedPrecondition)
        ));
        assert!(temp.0.join(crate::CONFIG_FILE_NAME).exists());
    }

    #[test]
    fn cleanup_command_loads_the_exact_bounded_assessment() {
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
        assert_eq!(
            assessment.cleanup_command,
            vec![
                "lili".to_owned(),
                "integrate".to_owned(),
                "cleanup".to_owned(),
                "--assessment".to_owned(),
                PLUGIN_MIGRATION_ASSESSMENT_FILE_NAME.to_owned(),
            ]
        );
        let path = temp.0.join(PLUGIN_MIGRATION_ASSESSMENT_FILE_NAME);
        fs::write(&path, serde_json::to_vec(&assessment).unwrap()).unwrap();
        assert_eq!(load_plugin_migration_assessment(&path).unwrap(), assessment);

        fs::write(
            &path,
            vec![b'x'; MAX_PLUGIN_MIGRATION_ASSESSMENT_BYTES as usize + 1],
        )
        .unwrap();
        assert!(matches!(
            load_plugin_migration_assessment(&path),
            Err(PluginMigrationError::AssessmentUnreadable)
        ));
    }

    #[test]
    fn plugin_commands_bind_the_selected_codex_home() {
        let codex_home = Path::new("/selected/codex-home");
        let command = codex_plugin_command(codex_home, "add", "lili@test-marketplace").unwrap();
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            ["plugin", "add", "lili@test-marketplace", "--json"]
        );
        assert_eq!(
            command
                .get_envs()
                .find(|(name, _)| *name == "CODEX_HOME")
                .and_then(|(_, value)| value),
            Some(codex_home.as_os_str())
        );
    }
}
