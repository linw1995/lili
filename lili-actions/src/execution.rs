use std::{collections::BTreeMap, ffi::OsString, io, process::Stdio};

use thiserror::Error;
use tokio::{
    io::AsyncWriteExt,
    process::{Child, Command},
};

use crate::{InteractionContextV1, LoadedAction, MAX_INTERACTION_CONTEXT_BYTES};

#[derive(Debug, Error)]
pub enum ActionSpawnError {
    #[error("interaction context could not be encoded")]
    Encode(#[source] serde_json::Error),
    #[error("interaction context exceeds its byte limit")]
    InputTooLarge,
    #[error("action process could not be spawned")]
    Spawn(#[source] io::Error),
    #[error("action process did not expose piped stdin")]
    StdinUnavailable,
    #[error("interaction context could not be written to the action process")]
    StdinWrite(#[source] io::Error),
}

#[derive(Debug)]
pub struct SpawnedAction {
    child: Child,
}

impl SpawnedAction {
    pub fn process_id(&self) -> Option<u32> {
        self.child.id()
    }

    pub fn child_mut(&mut self) -> &mut Child {
        &mut self.child
    }

    pub fn into_child(self) -> Child {
        self.child
    }
}

pub async fn spawn_action(
    action: &LoadedAction,
    context: &InteractionContextV1,
) -> Result<SpawnedAction, ActionSpawnError> {
    let input = serde_json::to_vec(context).map_err(ActionSpawnError::Encode)?;
    if input.len() > MAX_INTERACTION_CONTEXT_BYTES {
        return Err(ActionSpawnError::InputTooLarge);
    }

    let mut command = Command::new(action.executable());
    command
        .args(action.arguments())
        .current_dir(action.working_directory())
        .env_clear()
        .envs(minimal_environment())
        .envs(action.environment())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    let mut child = command.spawn().map_err(ActionSpawnError::Spawn)?;
    let Some(mut stdin) = child.stdin.take() else {
        terminate_failed_spawn(&mut child).await;
        return Err(ActionSpawnError::StdinUnavailable);
    };
    if let Err(error) = stdin.write_all(&input).await {
        terminate_failed_spawn(&mut child).await;
        return Err(ActionSpawnError::StdinWrite(error));
    }
    if let Err(error) = stdin.shutdown().await {
        terminate_failed_spawn(&mut child).await;
        return Err(ActionSpawnError::StdinWrite(error));
    }

    Ok(SpawnedAction { child })
}

async fn terminate_failed_spawn(child: &mut Child) {
    let _ = child.kill().await;
    let _ = child.wait().await;
}

#[cfg(unix)]
fn minimal_environment() -> BTreeMap<OsString, OsString> {
    BTreeMap::from([
        (OsString::from("LANG"), OsString::from("C.UTF-8")),
        (OsString::from("PATH"), OsString::from("/usr/bin:/bin")),
    ])
}

#[cfg(windows)]
fn minimal_environment() -> BTreeMap<OsString, OsString> {
    let mut environment = BTreeMap::new();
    if let Some(system_root) = std::env::var_os("SystemRoot") {
        let mut path = system_root.clone();
        path.push("\\System32");
        environment.insert(OsString::from("PATH"), path);
        environment.insert(OsString::from("SystemRoot"), system_root);
    }
    environment
}

#[cfg(not(any(unix, windows)))]
fn minimal_environment() -> BTreeMap<OsString, OsString> {
    BTreeMap::new()
}

#[cfg(all(test, unix))]
mod tests {
    use std::path::Path;

    use uuid::Uuid;

    use super::*;
    use crate::{
        ActionLoadContext, InteractionTrigger, PetLifecycleSnapshotV1, PetSnapshotV1,
        load_actions_str,
    };

    fn load_action(command: &str, environment: &str) -> crate::LoadedAction {
        let source = format!(
            r#"
version = 1

[[action]]
id = "test"
trigger = "pet_click"
command = [{command}]

[action.environment.allow]
{environment}
"#,
        );
        let context = ActionLoadContext::new("/", "/", Vec::new());
        load_actions_str(&source, &context).enabled()[0].clone()
    }

    fn interaction(label: &str) -> InteractionContextV1 {
        InteractionContextV1::for_pet(
            Uuid::nil(),
            1,
            InteractionTrigger::PetClick,
            PetSnapshotV1 {
                pet_id: "lili".to_owned(),
                label: label.to_owned(),
                lifecycle: PetLifecycleSnapshotV1::Idle,
            },
        )
        .unwrap()
    }

    #[tokio::test]
    async fn child_receives_context_only_on_standard_input() {
        let cat = if Path::new("/bin/cat").is_file() {
            "/bin/cat"
        } else {
            "/usr/bin/cat"
        };
        let action = load_action(&format!("{cat:?}"), "VISIBLE = \"allowed\"");
        let context = interaction("$(touch /tmp/not-an-action); ' quoted");
        let output = spawn_action(&action, &context)
            .await
            .unwrap()
            .into_child()
            .wait_with_output()
            .await
            .unwrap();
        assert!(output.status.success());
        let received: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(received["pet"]["label"], context.pet.label);
        assert_eq!(action.arguments().len(), 0);
    }

    #[tokio::test]
    async fn child_environment_is_cleared_before_allowlisted_values_are_added() {
        let env = if Path::new("/usr/bin/env").is_file() {
            "/usr/bin/env"
        } else {
            "/bin/env"
        };
        let action = load_action(&format!("{env:?}"), "VISIBLE = \"allowed\"");
        let output = spawn_action(&action, &interaction("Lili"))
            .await
            .unwrap()
            .into_child()
            .wait_with_output()
            .await
            .unwrap();
        assert!(output.status.success());
        let environment = String::from_utf8(output.stdout).unwrap();
        assert!(environment.lines().any(|line| line == "VISIBLE=allowed"));
        assert!(!environment.lines().any(|line| line.starts_with("HOME=")));
        assert!(
            !environment
                .lines()
                .any(|line| line.starts_with("SSH_AUTH_SOCK="))
        );
        assert!(
            !environment
                .lines()
                .any(|line| line.starts_with("CODEX_HOME="))
        );
    }

    #[tokio::test]
    async fn oversized_context_is_rejected_before_spawn() {
        let cat = if Path::new("/bin/cat").is_file() {
            "/bin/cat"
        } else {
            "/usr/bin/cat"
        };
        let action = load_action(&format!("{cat:?}"), "");
        let error = spawn_action(
            &action,
            &interaction(&"x".repeat(MAX_INTERACTION_CONTEXT_BYTES)),
        )
        .await
        .unwrap_err();
        assert!(matches!(error, ActionSpawnError::InputTooLarge));
    }
}
