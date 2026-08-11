use std::{collections::BTreeMap, ffi::OsString, io, process::Stdio};

use thiserror::Error;
use tokio::{
    io::AsyncWriteExt,
    process::{Child, Command},
    task::JoinHandle,
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
    #[error("action process tree could not be isolated")]
    ProcessTree(#[source] io::Error),
    #[error("action process did not expose piped stdin")]
    StdinUnavailable,
}

#[derive(Debug)]
pub struct SpawnedAction {
    child: Option<Child>,
    process_tree: process_tree::ProcessTree,
    stdin_task: Option<JoinHandle<io::Result<()>>>,
}

impl SpawnedAction {
    pub fn process_id(&self) -> Option<u32> {
        self.child.as_ref().and_then(Child::id)
    }

    pub(crate) fn child_mut(&mut self) -> &mut Child {
        self.child
            .as_mut()
            .expect("spawned action retains its child")
    }

    #[cfg(all(test, unix))]
    fn into_child(mut self) -> Child {
        self.process_tree.disarm();
        drop(self.stdin_task.take());
        self.child.take().expect("spawned action retains its child")
    }

    pub(crate) fn take_stdin_task(&mut self) -> Option<JoinHandle<io::Result<()>>> {
        self.stdin_task.take()
    }

    pub(crate) async fn terminate_tree(&mut self) -> io::Result<()> {
        let tree_error = self.process_tree.terminate().err();
        if tree_error.is_some() {
            let _ = self.child_mut().kill().await;
        }
        let wait_result = self.child_mut().wait().await.map(|_| ());
        match tree_error {
            Some(error) => Err(error),
            None => wait_result,
        }
    }

    pub(crate) fn mark_finished(&mut self) {
        self.process_tree.disarm();
    }
}

impl Drop for SpawnedAction {
    fn drop(&mut self) {
        if self.child.is_some() {
            if let Some(stdin_task) = &self.stdin_task {
                stdin_task.abort();
            }
            let _ = self.process_tree.terminate();
        }
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
    process_tree::configure(&mut command);

    let mut child = command.spawn().map_err(ActionSpawnError::Spawn)?;
    let process_tree = match process_tree::ProcessTree::attach(&child) {
        Ok(process_tree) => process_tree,
        Err(error) => {
            terminate_failed_spawn(&mut child).await;
            return Err(ActionSpawnError::ProcessTree(error));
        }
    };
    let mut spawned = SpawnedAction {
        child: Some(child),
        process_tree,
        stdin_task: None,
    };
    let Some(mut stdin) = spawned.child_mut().stdin.take() else {
        let _ = spawned.terminate_tree().await;
        return Err(ActionSpawnError::StdinUnavailable);
    };
    spawned.stdin_task = Some(tokio::spawn(async move {
        stdin.write_all(&input).await?;
        stdin.shutdown().await
    }));

    Ok(spawned)
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

#[cfg(unix)]
mod process_tree {
    use std::{io, os::unix::process::CommandExt};

    use tokio::process::{Child, Command};

    pub fn configure(command: &mut Command) {
        command.as_std_mut().process_group(0);
    }

    #[derive(Debug)]
    pub struct ProcessTree {
        process_group: i32,
        active: bool,
    }

    impl ProcessTree {
        pub fn attach(child: &Child) -> io::Result<Self> {
            let process_group = child
                .id()
                .and_then(|id| i32::try_from(id).ok())
                .ok_or_else(|| io::Error::other("child process identity is unavailable"))?;
            Ok(Self {
                process_group,
                active: true,
            })
        }

        pub fn terminate(&mut self) -> io::Result<()> {
            if !self.active {
                return Ok(());
            }
            // The child starts a fresh process group whose id is the validated child pid.
            let result = unsafe { libc::kill(-self.process_group, libc::SIGKILL) };
            if result == 0 {
                self.active = false;
                Ok(())
            } else {
                let error = io::Error::last_os_error();
                if matches!(error.raw_os_error(), Some(libc::ESRCH)) {
                    self.active = false;
                    Ok(())
                } else {
                    Err(error)
                }
            }
        }

        pub const fn disarm(&mut self) {
            self.active = false;
        }
    }
}

#[cfg(windows)]
mod process_tree {
    use std::{ffi::c_void, io, mem, ptr};

    use tokio::process::{Child, Command};
    use windows_sys::Win32::{
        Foundation::{CloseHandle, HANDLE},
        System::JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
            SetInformationJobObject, TerminateJobObject,
        },
    };

    pub fn configure(_command: &mut Command) {}

    #[derive(Debug)]
    pub struct ProcessTree {
        job: HANDLE,
        active: bool,
    }

    // SAFETY: the owned job handle is only accessed through exclusive methods and CloseHandle.
    unsafe impl Send for ProcessTree {}

    impl ProcessTree {
        pub fn attach(child: &Child) -> io::Result<Self> {
            let process = child
                .raw_handle()
                .map(|handle| handle as HANDLE)
                .ok_or_else(|| io::Error::other("child process handle is unavailable"))?;
            let job = unsafe { CreateJobObjectW(ptr::null(), ptr::null()) };
            if job.is_null() {
                return Err(io::Error::last_os_error());
            }
            let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let configured = unsafe {
                SetInformationJobObject(
                    job,
                    JobObjectExtendedLimitInformation,
                    (&raw const limits).cast::<c_void>(),
                    u32::try_from(mem::size_of_val(&limits))
                        .expect("job limit structure fits in u32"),
                )
            };
            if configured == 0 || unsafe { AssignProcessToJobObject(job, process) } == 0 {
                let error = io::Error::last_os_error();
                unsafe {
                    CloseHandle(job);
                }
                return Err(error);
            }
            Ok(Self { job, active: true })
        }

        pub fn terminate(&mut self) -> io::Result<()> {
            if !self.active {
                return Ok(());
            }
            if unsafe { TerminateJobObject(self.job, 1) } == 0 {
                Err(io::Error::last_os_error())
            } else {
                self.active = false;
                Ok(())
            }
        }

        pub const fn disarm(&mut self) {
            self.active = false;
        }
    }

    impl Drop for ProcessTree {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.job);
            }
        }
    }
}

#[cfg(not(any(unix, windows)))]
mod process_tree {
    use std::io;

    use tokio::process::{Child, Command};

    pub fn configure(_command: &mut Command) {}

    #[derive(Debug)]
    pub struct ProcessTree {
        active: bool,
    }

    impl ProcessTree {
        pub fn attach(_child: &Child) -> io::Result<Self> {
            Ok(Self { active: true })
        }

        pub fn terminate(&mut self) -> io::Result<()> {
            self.active = false;
            Ok(())
        }

        pub const fn disarm(&mut self) {
            self.active = false;
        }
    }
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
