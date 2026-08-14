use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    thread,
};

use lili_integration::{
    CONFIG_FILE_NAME, HOOKS_FILE_NAME, IntegrationInspection, LILI_INTEGRATION_ID,
    PluginLifecycleHost, PluginMigrationAssessment, PluginMigrationError, PluginMigrationEvidence,
    PluginMigrationState, UninstallOutcome, assess_plugin_migration,
    build_coexistence_install_plan, build_install_plan,
    cleanup_legacy_after_verification_with_host, inspect_with_version, install_with_verifier,
};
use lili_session::{
    CodexAdapterDiagnostics, CodexHookSource, CodexIntegrationSurface, CodexPluginAvailability,
    CodexPluginDiagnostics, CodexPluginTrustState, DESKTOP_VERSION, LastAcceptedCodexEvent,
    ReductionOutcome, SessionEventKind, SessionReducer, TESTED_CODEX_VERSION,
    mark_plugin_hook_event, normalize_lifecycle_json,
};

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let sequence = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "lili-migration-scenarios-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn hook_binary(&self) -> PathBuf {
        self.0.join("bin/lili-hook")
    }

    fn install_legacy(&self) {
        let inspection = inspect_with_version(&self.0, Some(TESTED_CODEX_VERSION.to_owned()));
        let plan = build_install_plan(&inspection, &self.hook_binary(), 42);
        install_with_verifier(&plan, |_| Ok(())).unwrap();
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn verified_evidence() -> PluginMigrationEvidence {
    PluginMigrationEvidence {
        exact_hooks_reviewed_by_user: true,
        synthetic_delivery_verified: true,
        overlap_deduplication_verified: true,
    }
}

fn plugin_diagnostics(legacy_active: bool, version: &str) -> CodexAdapterDiagnostics {
    let mut diagnostics = CodexAdapterDiagnostics::with_discovery(
        Some(TESTED_CODEX_VERSION),
        [CodexIntegrationSurface::Stop],
    )
    .with_plugin(
        CodexPluginDiagnostics::discovered(
            Some(TESTED_CODEX_VERSION),
            CodexPluginAvailability::Installed,
            true,
            true,
            Some(version),
            legacy_active,
        )
        .with_plugin_id(Some("lili@test-marketplace")),
    );
    diagnostics.plugin.trust_state = CodexPluginTrustState::TrustedAtLastDelivery;
    diagnostics.plugin.last_accepted_plugin_event = Some(LastAcceptedCodexEvent {
        event_id: "plugin-event".to_owned(),
        event_type: SessionEventKind::TurnCompleted,
        occurred_at_ms: 42,
        surface: CodexIntegrationSurface::Stop,
        plugin_id: Some("lili@test-marketplace".to_owned()),
        plugin_version: Some(version.to_owned()),
    });
    diagnostics
}

fn cleanup_with_diagnostics(
    temp: &TempDir,
    assessment: &PluginMigrationAssessment,
    diagnostics: CodexAdapterDiagnostics,
) -> Result<UninstallOutcome, PluginMigrationError> {
    let mut current = inspect_with_version(&temp.0, Some(TESTED_CODEX_VERSION.to_owned()));
    current.codex_adapter = diagnostics;
    let mut host = InspectionHost(Some(current));
    cleanup_legacy_after_verification_with_host(&mut host, &temp.0, assessment)
}

struct InspectionHost(Option<IntegrationInspection>);

impl PluginLifecycleHost for InspectionHost {
    fn install(
        &mut self,
        _codex_home: &Path,
        _plugin_selector: &str,
    ) -> Result<(), PluginMigrationError> {
        unreachable!("cleanup must not install a plugin")
    }

    fn inspect(&mut self, _codex_home: &Path, _plugin_selector: &str) -> IntegrationInspection {
        self.0.take().expect("cleanup inspection is single-use")
    }

    fn hooks_trusted(&mut self, _codex_home: &Path, _plugin_selector: &str) -> bool {
        self.0.as_ref().is_some_and(|inspection| {
            inspection.codex_adapter.plugin.trust_state
                == CodexPluginTrustState::TrustedAtLastDelivery
        })
    }

    fn migration_evidence_verified(
        &mut self,
        _codex_home: &Path,
        _assessment: &PluginMigrationAssessment,
    ) -> bool {
        true
    }

    fn rollback(
        &mut self,
        _codex_home: &Path,
        _plugin_selector: &str,
    ) -> Result<(), PluginMigrationError> {
        unreachable!("cleanup must not remove a plugin")
    }
}

#[test]
fn clean_plugin_adoption_reaches_plugin_primary_without_legacy_cleanup() {
    let temp = TempDir::new();
    let inspection = inspect_with_version(&temp.0, Some(TESTED_CODEX_VERSION.to_owned()));
    let assessment = assess_plugin_migration(
        &inspection,
        &plugin_diagnostics(false, DESKTOP_VERSION),
        "lili@test-marketplace",
        &verified_evidence(),
    );
    assert_eq!(assessment.state, PluginMigrationState::PluginPrimary);
    assert!(!assessment.cleanup_allowed());
    assert!(!temp.0.join(CONFIG_FILE_NAME).exists());
    assert!(!temp.0.join(HOOKS_FILE_NAME).exists());
}

#[test]
fn cleanup_preserves_unrelated_notify_and_hooks() {
    let temp = TempDir::new();
    fs::write(
        temp.0.join(CONFIG_FILE_NAME),
        "model = \"gpt-5\"\nnotify = [\"existing\", \"--channel\", \"pet\"]\n",
    )
    .unwrap();
    fs::write(
        temp.0.join(HOOKS_FILE_NAME),
        r#"{"hooks":{"SessionStart":[{"hooks":[{"type":"command","command":"existing-hook"}]}]}}"#,
    )
    .unwrap();
    let inspection = inspect_with_version(&temp.0, Some(TESTED_CODEX_VERSION.to_owned()));
    let plan = build_coexistence_install_plan(&inspection, &temp.hook_binary(), 42);
    install_with_verifier(&plan, |_| Ok(())).unwrap();

    let inspection = inspect_with_version(&temp.0, Some(TESTED_CODEX_VERSION.to_owned()));
    let assessment = assess_plugin_migration(
        &inspection,
        &plugin_diagnostics(true, DESKTOP_VERSION),
        "lili@test-marketplace",
        &verified_evidence(),
    );
    assert!(
        cleanup_with_diagnostics(
            &temp,
            &assessment,
            plugin_diagnostics(true, DESKTOP_VERSION)
        )
        .unwrap()
        .complete
    );
    let config = fs::read_to_string(temp.0.join(CONFIG_FILE_NAME)).unwrap();
    assert!(config.contains("model = \"gpt-5\""));
    assert!(config.contains("notify = [\"existing\", \"--channel\", \"pet\"]"));
    let hooks = fs::read_to_string(temp.0.join(HOOKS_FILE_NAME)).unwrap();
    assert!(hooks.contains("existing-hook"));
    assert!(!hooks.contains(LILI_INTEGRATION_ID));
}

#[test]
fn untrusted_plugin_cannot_cleanup_legacy_integration() {
    let temp = TempDir::new();
    temp.install_legacy();
    let inspection = inspect_with_version(&temp.0, Some(TESTED_CODEX_VERSION.to_owned()));
    let mut diagnostics = plugin_diagnostics(true, DESKTOP_VERSION);
    diagnostics.plugin.trust_state = CodexPluginTrustState::Unknown;
    diagnostics.plugin.last_accepted_plugin_event = None;
    let assessment = assess_plugin_migration(
        &inspection,
        &diagnostics,
        "lili@test-marketplace",
        &verified_evidence(),
    );
    assert_eq!(assessment.state, PluginMigrationState::AwaitingHookReview);
    assert!(matches!(
        cleanup_with_diagnostics(&temp, &assessment, diagnostics),
        Err(PluginMigrationError::FailedPrecondition)
    ));
    assert!(temp.0.join(CONFIG_FILE_NAME).exists());
    assert!(temp.0.join(HOOKS_FILE_NAME).exists());
}

#[test]
fn cleanup_revalidates_current_hook_trust_and_plugin_version() {
    let temp = TempDir::new();
    temp.install_legacy();
    let inspection = inspect_with_version(&temp.0, Some(TESTED_CODEX_VERSION.to_owned()));
    let trusted = plugin_diagnostics(true, DESKTOP_VERSION);
    let assessment = assess_plugin_migration(
        &inspection,
        &trusted,
        "lili@test-marketplace",
        &verified_evidence(),
    );
    assert_eq!(assessment.state, PluginMigrationState::CleanupReady);

    let mut invalidated = plugin_diagnostics(true, DESKTOP_VERSION);
    invalidated.plugin.trust_state = CodexPluginTrustState::Unknown;
    invalidated.plugin.last_accepted_plugin_event = None;
    assert!(matches!(
        cleanup_with_diagnostics(&temp, &assessment, invalidated),
        Err(PluginMigrationError::FailedPrecondition)
    ));

    let upgraded = plugin_diagnostics(true, "0.1.1");
    assert!(matches!(
        cleanup_with_diagnostics(&temp, &assessment, upgraded),
        Err(PluginMigrationError::FailedPrecondition)
    ));
    assert!(temp.0.join(CONFIG_FILE_NAME).exists());
    assert!(temp.0.join(HOOKS_FILE_NAME).exists());
}

#[test]
fn verified_delivery_allows_cleanup_on_an_unreviewed_codex_version() {
    let temp = TempDir::new();
    temp.install_legacy();
    let inspection = inspect_with_version(&temp.0, Some(TESTED_CODEX_VERSION.to_owned()));
    let mut diagnostics = plugin_diagnostics(true, DESKTOP_VERSION);
    diagnostics.plugin.codex_support = lili_session::CodexPluginSupport::Unreviewed;
    let assessment = assess_plugin_migration(
        &inspection,
        &diagnostics,
        "lili@test-marketplace",
        &verified_evidence(),
    );
    assert_eq!(assessment.state, PluginMigrationState::CleanupReady);
    assert_eq!(
        assessment.verified_plugin_version.as_deref(),
        Some(DESKTOP_VERSION)
    );
    assert!(
        cleanup_with_diagnostics(&temp, &assessment, diagnostics)
            .unwrap()
            .complete
    );
}

#[test]
fn incompatible_plugin_blocks_migration_before_cleanup() {
    let temp = TempDir::new();
    temp.install_legacy();
    let inspection = inspect_with_version(&temp.0, Some(TESTED_CODEX_VERSION.to_owned()));
    let assessment = assess_plugin_migration(
        &inspection,
        &plugin_diagnostics(true, "0.2.0"),
        "lili@test-marketplace",
        &verified_evidence(),
    );
    assert_eq!(assessment.state, PluginMigrationState::Blocked);
    assert!(
        assessment
            .blockers
            .iter()
            .any(|blocker| blocker.contains("incompatible"))
    );
    assert!(temp.0.join(CONFIG_FILE_NAME).exists());
}

#[test]
fn concurrent_legacy_and_plugin_events_deduplicate_per_session() {
    const SESSION_COUNT: usize = 32;
    let reducer = Arc::new(Mutex::new(SessionReducer::default()));
    let applied = Arc::new(AtomicUsize::new(0));
    let duplicate = Arc::new(AtomicUsize::new(0));
    let mut workers = Vec::new();
    for index in 0..SESSION_COUNT {
        let payload = format!(
            r#"{{"hook_event_name":"Stop","session_id":"session-{index}","turn_id":"turn-{index}"}}"#
        );
        let legacy = normalize_lifecycle_json(payload.as_bytes(), index as u64 + 1).unwrap();
        let mut plugin = legacy.clone();
        assert!(mark_plugin_hook_event(&mut plugin, "lili@test-marketplace"));
        for event in [legacy, plugin] {
            let reducer = reducer.clone();
            let applied = applied.clone();
            let duplicate = duplicate.clone();
            workers.push(thread::spawn(move || {
                match reducer.lock().unwrap().reduce(event) {
                    ReductionOutcome::Applied { .. } => {
                        applied.fetch_add(1, Ordering::Relaxed);
                    }
                    ReductionOutcome::Duplicate => {
                        duplicate.fetch_add(1, Ordering::Relaxed);
                    }
                    ReductionOutcome::IgnoredStale => panic!("unexpected stale event"),
                }
            }));
        }
    }
    for worker in workers {
        worker.join().unwrap();
    }
    assert_eq!(applied.load(Ordering::Relaxed), SESSION_COUNT);
    assert_eq!(duplicate.load(Ordering::Relaxed), SESSION_COUNT);
    assert_eq!(
        reducer.lock().unwrap().snapshot().notifications.len(),
        SESSION_COUNT
    );
}

#[test]
fn repeated_permission_invocations_remain_distinct_during_overlap() {
    let mut reducer = SessionReducer::with_minimum_dwell_ms(0);
    let mut applied = 0;
    let mut duplicates = 0;
    for (index, tool_use_id) in ["toolu_01", "toolu_02"].into_iter().enumerate() {
        let payload = format!(
            r#"{{"hook_event_name":"PermissionRequest","session_id":"session-1","turn_id":"turn-1","tool_use_id":"{tool_use_id}","tool_name":"Bash","tool_input":{{"command":"cargo test"}}}}"#
        );
        let legacy = normalize_lifecycle_json(payload.as_bytes(), index as u64 + 1).unwrap();
        let mut plugin = legacy.clone();
        assert!(mark_plugin_hook_event(&mut plugin, "lili@test-marketplace"));
        for event in [legacy, plugin] {
            match reducer.reduce(event) {
                ReductionOutcome::Applied { .. } => applied += 1,
                ReductionOutcome::Duplicate => duplicates += 1,
                ReductionOutcome::IgnoredStale => panic!("unexpected stale event"),
            }
        }
    }

    assert_eq!(applied, 2);
    assert_eq!(duplicates, 2);
    assert_eq!(reducer.snapshot().notifications.len(), 2);
}

#[test]
fn failed_verification_and_repeated_assessment_never_remove_legacy_early() {
    let temp = TempDir::new();
    temp.install_legacy();
    let inspection = inspect_with_version(&temp.0, Some(TESTED_CODEX_VERSION.to_owned()));
    let evidence = PluginMigrationEvidence {
        exact_hooks_reviewed_by_user: true,
        synthetic_delivery_verified: false,
        overlap_deduplication_verified: false,
    };
    for _ in 0..2 {
        let assessment = assess_plugin_migration(
            &inspection,
            &plugin_diagnostics(true, DESKTOP_VERSION),
            "lili@test-marketplace",
            &evidence,
        );
        assert_eq!(assessment.state, PluginMigrationState::AwaitingVerification);
        assert!(matches!(
            cleanup_with_diagnostics(
                &temp,
                &assessment,
                plugin_diagnostics(true, DESKTOP_VERSION)
            ),
            Err(PluginMigrationError::FailedPrecondition)
        ));
    }
    assert!(temp.0.join(CONFIG_FILE_NAME).exists());
    assert!(temp.0.join(HOOKS_FILE_NAME).exists());
}

#[test]
fn modified_legacy_provenance_blocks_cleanup_without_partial_changes() {
    let temp = TempDir::new();
    temp.install_legacy();
    let modified =
        format!("notify = [\"custom\", \"--integration-id\", \"{LILI_INTEGRATION_ID}\"]\n");
    fs::write(temp.0.join(CONFIG_FILE_NAME), &modified).unwrap();
    let hooks_before = fs::read(temp.0.join(HOOKS_FILE_NAME)).unwrap();
    let inspection = inspect_with_version(&temp.0, Some(TESTED_CODEX_VERSION.to_owned()));
    let assessment = assess_plugin_migration(
        &inspection,
        &plugin_diagnostics(true, DESKTOP_VERSION),
        "lili@test-marketplace",
        &verified_evidence(),
    );
    assert_eq!(assessment.state, PluginMigrationState::Blocked);
    assert!(matches!(
        cleanup_with_diagnostics(
            &temp,
            &assessment,
            plugin_diagnostics(true, DESKTOP_VERSION)
        ),
        Err(PluginMigrationError::FailedPrecondition)
    ));
    assert_eq!(
        fs::read_to_string(temp.0.join(CONFIG_FILE_NAME)).unwrap(),
        modified
    );
    assert_eq!(
        fs::read(temp.0.join(HOOKS_FILE_NAME)).unwrap(),
        hooks_before
    );
}

#[test]
fn modified_legacy_hook_blocks_cleanup_without_partial_changes() {
    let temp = TempDir::new();
    temp.install_legacy();
    let config_before = fs::read(temp.0.join(CONFIG_FILE_NAME)).unwrap();
    let hooks_path = temp.0.join(HOOKS_FILE_NAME);
    let mut hooks: serde_json::Value =
        serde_json::from_slice(&fs::read(&hooks_path).unwrap()).unwrap();
    let command = hooks["hooks"]["SessionStart"][0]["hooks"][0]["command"]
        .as_str()
        .unwrap()
        .to_owned();
    hooks["hooks"]["SessionStart"][0]["hooks"][0]["command"] =
        serde_json::Value::String(format!("{command} --custom"));
    let hooks_before = serde_json::to_vec_pretty(&hooks).unwrap();
    fs::write(&hooks_path, &hooks_before).unwrap();

    let inspection = inspect_with_version(&temp.0, Some(TESTED_CODEX_VERSION.to_owned()));
    let assessment = assess_plugin_migration(
        &inspection,
        &plugin_diagnostics(true, DESKTOP_VERSION),
        "lili@test-marketplace",
        &verified_evidence(),
    );
    assert_eq!(assessment.state, PluginMigrationState::Blocked);
    assert!(matches!(
        cleanup_with_diagnostics(
            &temp,
            &assessment,
            plugin_diagnostics(true, DESKTOP_VERSION)
        ),
        Err(PluginMigrationError::FailedPrecondition)
    ));
    assert_eq!(
        fs::read(temp.0.join(CONFIG_FILE_NAME)).unwrap(),
        config_before
    );
    assert_eq!(fs::read(hooks_path).unwrap(), hooks_before);
}

#[test]
fn completed_migration_is_stable_when_assessed_again() {
    let temp = TempDir::new();
    temp.install_legacy();
    let inspection = inspect_with_version(&temp.0, Some(TESTED_CODEX_VERSION.to_owned()));
    let diagnostics = plugin_diagnostics(true, DESKTOP_VERSION);
    let ready = assess_plugin_migration(
        &inspection,
        &diagnostics,
        "lili@test-marketplace",
        &verified_evidence(),
    );
    cleanup_with_diagnostics(&temp, &ready, diagnostics.clone()).unwrap();

    let inspection = inspect_with_version(&temp.0, Some(TESTED_CODEX_VERSION.to_owned()));
    let diagnostics = plugin_diagnostics(false, DESKTOP_VERSION);
    assert_eq!(diagnostics.plugin.hook_source, CodexHookSource::Plugin);
    let repeated = assess_plugin_migration(
        &inspection,
        &diagnostics,
        "lili@test-marketplace",
        &verified_evidence(),
    );
    assert_eq!(repeated.state, PluginMigrationState::PluginPrimary);
    assert!(!repeated.cleanup_allowed());
}
