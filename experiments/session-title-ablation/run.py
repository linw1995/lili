#!/usr/bin/env python3
"""Run independent title-runtime ablations against an archived revision."""
import argparse
import difflib
import hashlib
import io
import json
import os
from pathlib import Path
import re
import subprocess
import tarfile
import tempfile
import time

BASE = "8bbac24964cd3f3e3ba4a71c29396968b1442fb6"
SUPERVISOR = "lili-actions/src/supervisor.rs"
RUNTIME = "lili-actions/src/title_runtime.rs"
APP = "lili-app-state/src/title.rs"
REDUCER = "lili-session/src/reducer.rs"
FILES = [SUPERVISOR, RUNTIME, APP, REDUCER]

RUNTIME_PROBES = r'''
    #[cfg(unix)]
    #[tokio::test]
    async fn ablation_coalesced_null_notifies_every_waiter() {
        let supervisor = supervisor("cat >/dev/null; sleep 0.02; printf '%s' '{\"version\":1,\"title\":null}'", 0);
        let (a, b) = tokio::time::timeout(Duration::from_secs(2), async {
            tokio::join!(supervisor.resolve_title("example", "one"), supervisor.resolve_title("example", "one"))
        }).await.unwrap();
        assert_eq!((a, b), (None, None));
        assert_eq!(supervisor.audit_snapshot().await.len(), 1);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn ablation_failed_results_obey_per_session_debounce() {
        let supervisor = supervisor("cat >/dev/null; printf invalid", 60000);
        assert!(supervisor.resolve_title("example", "one").await.is_none());
        assert!(supervisor.resolve_title("example", "one").await.is_none());
        assert_eq!(supervisor.audit_snapshot().await.len(), 1);
        assert!(supervisor.resolve_title("example", "two").await.is_none());
        assert_eq!(supervisor.audit_snapshot().await.len(), 2);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn ablation_cache_keeps_providers_separate() {
        let mut source = String::from("version = 1\n");
        for (id, provider, title) in [("z-first", "first", "First"), ("a-second", "second", "Second")] {
            let script = format!("cat >/dev/null; printf '%s' '{}'", serde_json::json!({"version":1,"title":title}));
            source.push_str(&format!("\n[[action]]\nid = \"{id}\"\ntrigger = \"session_title\"\ncommand = [\"/bin/sh\", \"-c\", {}]\n[action.filters]\nproviders = [\"{provider}\"]\n", toml::Value::String(script)));
        }
        let loaded = crate::load_actions_str(&source, &crate::ActionLoadContext::new("/", "/", vec![]));
        let supervisor = ActionSupervisor::new(loaded, 1).unwrap();
        assert_eq!(supervisor.resolve_title("first", "shared").await.as_deref(), Some("First"));
        assert_eq!(supervisor.resolve_title("second", "shared").await.as_deref(), Some("Second"));
        assert_eq!(supervisor.resolve_title("first", "shared").await.as_deref(), Some("First"));
        assert_eq!(supervisor.audit_snapshot().await.len(), 2);
    }
'''

APP_PROBES = r'''
    #[tokio::test]
    async fn ablation_unmatched_notification_schedules_no_worker() {
        let state = AppState::default();
        let loaded = load_actions_str(r#"version = 1
[[action]]
id = "unmatched"
trigger = "session_title"
command = ["/bin/sh", "-c", "exit 1"]
[action.filters]
providers = ["another-provider"]
"#, &ActionLoadContext::new("/", "/", vec![]));
        assert!(state.configure_actions(loaded, 1).await);
        state.apply_session_event(event("unmatched", "one")).await;
        assert!(state.title_dispatch.lock().await.pending.is_empty());
        assert!(state.action_audit().await.is_empty());
    }
'''


def replace(files, path, old, new, count=1):
    actual = files[path].count(old)
    if actual != count:
        raise ValueError(f"{path}: expected {count} matches, found {actual}: {old[:80]!r}")
    files[path] = files[path].replace(old, new)


def smaller_key(files):
    replace(files, RUNTIME, "type Key = (String, String, String);", "type Key = (String, String);")
    replace(files, RUNTIME, "let key = (action_id, provider.to_owned(), session_id.to_owned());", "let key = (provider.to_owned(), session_id.to_owned());")
    replace(files, RUNTIME, '("title".into(), "example".into(), index.to_string())', '("example".into(), index.to_string())')


def single_option(files):
    replace(files, RUNTIME, "type Reply = Option<Option<String>>;", "type Reply = Option<String>;")
    replace(files, RUNTIME, "sender.send_replace(Some(title));", "sender.send_replace(title);")
    replace(files, RUNTIME, '''        loop {
            if let Some(reply) = receiver.borrow().clone() {
                return reply;
            }
            if receiver.changed().await.is_err() {
                return None;
            }
        }''', '''        receiver.changed().await.ok()?;
        receiver.borrow_and_update().clone()''')


def single_index(files):
    replace(files, SUPERVISOR, "pub(crate) actions: Arc<BTreeMap<String, Arc<ActionRuntime>>>,", "pub(crate) actions: Arc<Vec<Arc<ActionRuntime>>>,")
    replace(files, SUPERVISOR, "    pub(crate) title_order: Arc<Vec<String>>,\n", "")
    start = files[SUPERVISOR].index("        let title_order = Arc::new(")
    end = files[SUPERVISOR].index("        let (shutdown, _)", start)
    files[SUPERVISOR] = files[SUPERVISOR][:start] + files[SUPERVISOR][end:]
    replace(files, SUPERVISOR, '''            .map(|action| {
                let id = action.id().to_owned();
                (id, Arc::new(ActionRuntime::new(action)))
            })''', '''            .map(|action| Arc::new(ActionRuntime::new(action)))''')
    replace(files, SUPERVISOR, "            title_order,\n", "")
    replace(files, SUPERVISOR, '''        self.actions
            .values()
            .filter(|runtime| matches_context(&runtime.action, context))
            .map(|runtime| runtime.action.id().to_owned())
            .collect()''', '''        let mut ids: Vec<_> = self.actions
            .iter()
            .filter(|runtime| matches_context(&runtime.action, context))
            .map(|runtime| runtime.action.id().to_owned())
            .collect();
        ids.sort();
        ids''')
    replace(files, SUPERVISOR, "self.actions.get(action_id).cloned()", "self.actions.iter().find(|runtime| runtime.action.id() == action_id).cloned()")
    # BTreeMap is no longer used anywhere in this module.
    replace(files, SUPERVISOR, "collections::{BTreeMap, VecDeque}", "collections::VecDeque")
    start = files[RUNTIME].index("        self.title_order")
    end = files[RUNTIME].index("\n    }", start)
    files[RUNTIME] = files[RUNTIME][:start] + '''        self.actions.iter().find(|runtime| {
            let filters = runtime.action.filters();
            runtime.action.trigger() == crate::ActionTrigger::SessionTitle
                && (filters.providers.is_empty() || filters.providers.iter().any(|value| value == provider))
        }).map(|runtime| runtime.action.id())''' + files[RUNTIME][end:]
    replace(files, RUNTIME, "self.actions.get(&action_id)?.clone()", "self.actions.iter().find(|runtime| runtime.action.id() == action_id)?.clone()")


def no_debounce(files):
    replace(files, RUNTIME, '''            if !state.accept(&key, runtime.action.debounce_ms()) {
                return None;
            }
''', "")


def no_coalescing(files):
    replace(files, RUNTIME, "if let Some(pending) = state.pending.get(&key)", "if let Some(pending) = None::<&Pending>")


def no_incarnation(files):
    replace(files, REDUCER, "            || !std::sync::Arc::ptr_eq(&notification.incarnation, &original.incarnation)\n", "")


def no_publication_join(files):
    replace(files, APP, '''        for task in pending.into_values() {
            let _ = task.worker.await;
        }''', "        drop(pending);")


def no_prefilter(files):
    replace(files, APP, '''                || supervisor
                    .title_action_id(notification.provider.as_str())
                    .is_none()
''', "")


def combined(files):
    smaller_key(files)
    single_option(files)
    single_index(files)


VARIANTS = {
    "baseline": lambda files: None,
    "smaller_key": smaller_key,
    "single_option": single_option,
    "single_index": single_index,
    "combined": combined,
    "no_debounce": no_debounce,
    "no_coalescing": no_coalescing,
    "no_incarnation": no_incarnation,
    "no_publication_join": no_publication_join,
    "no_prefilter": no_prefilter,
}


def patch(before, after):
    return "".join("".join(difflib.unified_diff(before[path].splitlines(True), after[path].splitlines(True), fromfile=f"a/{path}", tofile=f"b/{path}")) for path in FILES)


def production_lines(source):
    source = re.split(r"#\[cfg\([^\n]*test", source, maxsplit=1)[0]
    return sum(bool(line.strip()) for line in source.splitlines())


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("variants", nargs="*", choices=list(VARIANTS))
    args = parser.parse_args()
    variants = args.variants or list(VARIANTS)
    repo = Path(subprocess.check_output(["git", "rev-parse", "--show-toplevel"], text=True).strip())
    output = Path(__file__).resolve().parent / "results"
    output.mkdir(exist_ok=True)
    revision = subprocess.check_output(["git", "rev-parse", BASE], cwd=repo, text=True).strip()
    archive = subprocess.check_output(["git", "archive", revision], cwd=repo)
    with tempfile.TemporaryDirectory(prefix="lili-title-ablation-") as directory:
        work = Path(directory)
        with tarfile.open(fileobj=io.BytesIO(archive)) as source:
            source.extractall(work, filter="data")
        original = {path: (work / path).read_text() for path in FILES}
        baseline = dict(original)
        baseline[RUNTIME] = baseline[RUNTIME].replace("    use super::*;", "    use super::*;\n" + RUNTIME_PROBES, 1)
        baseline[APP] = baseline[APP].replace("    use super::*;", "    use super::*;\n" + APP_PROBES, 1)
        environment = os.environ.copy()
        environment["CARGO_TARGET_DIR"] = str(repo / "target")
        for path, content in baseline.items():
            (work / path).write_text(content)
        subprocess.run(["cargo", "fmt", "--all"], cwd=work, env=environment, check=True, capture_output=True)
        baseline = {path: (work / path).read_text() for path in FILES}
        (output / "probes.patch").write_text(patch(original, baseline))
        prior = json.loads((output / "summary.json").read_text()) if (output / "summary.json").exists() else {}
        if prior and prior["base_revision"] != revision:
            raise ValueError("existing results use a different baseline")
        results = prior.get("results", [])
        for result in results:
            result.setdefault("harness_sha256", prior.get("harness_sha256"))
        for name in VARIANTS:
            files = dict(baseline)
            VARIANTS[name](files)
            for path, content in files.items():
                (work / path).write_text(content)
            # Normalize each variant before comparing source size.
            subprocess.run(["cargo", "fmt", "--all"], cwd=work, env=environment, check=True, capture_output=True)
            files = {path: (work / path).read_text() for path in FILES}
            (output / f"{name}.patch").write_text(patch(baseline, files))
            if name not in variants:
                continue
            command = ["cargo", "test", "--locked", "--offline", "--no-fail-fast", "-p", "lili-actions", "-p", "lili-app-state", "-p", "lili-session", "--lib"]
            started = time.monotonic()
            print(f"START {name}", flush=True)
            try:
                run = subprocess.run(command, cwd=work, env=environment, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=300)
                log, code = run.stdout, run.returncode
            except subprocess.TimeoutExpired as error:
                log = (error.stdout or b"").decode(errors="replace") if isinstance(error.stdout, bytes) else (error.stdout or "")
                code = 124
            (output / f"{name}.log").write_text(log)
            result = {
                "variant": name, "exit_code": code,
                "harness_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
                "seconds": round(time.monotonic() - started, 3),
                "passed": sum(map(int, re.findall(r"test result: (?:ok|FAILED)\. (\d+) passed", log))),
                "failed_tests": re.findall(r"^test (.+) \.\.\. FAILED$", log, re.M),
                "production_line_delta": sum(production_lines(files[path]) - production_lines(original[path]) for path in FILES),
                "compile_error": "error[E" in log or "could not compile" in log,
            }
            if code == 0 and name in {"baseline", "combined"}:
                check = subprocess.run(["cargo", "clippy", "--locked", "--offline", "-p", "lili-actions", "-p", "lili-app-state", "-p", "lili-session", "--all-targets", "--", "-D", "warnings"], cwd=work, env=environment, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=300)
                (output / f"{name}-clippy.log").write_text(check.stdout)
                result["clippy_exit_code"] = check.returncode
            results = [previous for previous in results if previous["variant"] != name]
            results.append(result)
            results.sort(key=lambda item: list(VARIANTS).index(item["variant"]))
            document = {"base_revision": revision, "platform": os.uname().sysname + " " + os.uname().machine, "command": command, "harness_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(), "results": results}
            (output / "summary.json").write_text(json.dumps(document, indent=2) + "\n")
            print(json.dumps(result), flush=True)


if __name__ == "__main__":
    main()
