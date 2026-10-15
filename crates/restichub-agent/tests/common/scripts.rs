//! Fixtures for the script tests: a host with the setting file next to the work directory and the
//! step builders.

#![allow(dead_code)]
#![allow(clippy::expect_used)]

use std::path::{Path, PathBuf};

use restichub_agent::hooks::Trust;
use restichub_agent::jobs::Jobs;
use restichub_agent::scripts::ScriptStore;
use restichub_proto::agent::v1::{
    HookPhase, RejectReason, ScriptEnv, ScriptFailurePolicy, ScriptInterpreter, ScriptStep, Step,
    StepStatus, agent_message, script_env, step,
};
use tokio::sync::mpsc;

use super::hooks::{HookFixture, set_mode};
use super::{AGENT, Collected, TENANT, ring, signed};

/// The fixture of a host, with the setting file next to the work directory.
pub struct Host {
    pub fx: HookFixture,
}

impl Host {
    pub fn new(name: &str) -> Self {
        Self {
            fx: HookFixture::new(name),
        }
    }

    pub fn setting(&self) -> PathBuf {
        self.fx.work.join("server-scripts")
    }

    pub fn write_setting(&self, text: &str, mode: u32) {
        let path = self.setting();
        let _ = std::fs::remove_file(&path);
        std::fs::write(&path, text).expect("cannot write the setting");
        set_mode(&path, mode);
    }

    pub fn enable(&self) {
        self.write_setting("server_scripts = enabled\n", 0o644);
    }

    pub fn cache(&self) -> PathBuf {
        self.fx.dir.join("cache")
    }

    pub fn jobs(&self) -> Jobs {
        self.fx
            .jobs()
            .with_script_store(ScriptStore::new(
                self.setting(),
                Trust::CurrentUser,
                &self.cache(),
            ))
            .with_stop_grace(std::time::Duration::from_millis(300))
    }

    /// The jobs of the host, with a chosen search path for the scripts.
    pub fn jobs_with_path(&self, path: std::ffi::OsString) -> Jobs {
        self.fx
            .jobs()
            .with_script_store(
                ScriptStore::new(self.setting(), Trust::CurrentUser, &self.cache())
                    .with_search_path(path),
            )
            .with_stop_grace(std::time::Duration::from_millis(300))
    }

    /// The directory where the scripts of the test write their marks.
    pub fn marks(&self) -> PathBuf {
        let dir = self.fx.work.join("marks");
        std::fs::create_dir_all(&dir).expect("cannot create the marks directory");
        dir
    }

    pub fn mark(&self, name: &str) -> String {
        std::fs::read_to_string(self.marks().join(name)).unwrap_or_default()
    }

    /// The scripts directory of the cache: it must hold nothing after a run (AC-HK-09).
    pub fn leftovers(&self) -> Vec<PathBuf> {
        std::fs::read_dir(self.cache().join("scripts"))
            .map(|entries| {
                entries
                    .filter_map(|entry| Some(entry.ok()?.path()))
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn job(&self, job_id: &str, steps: Vec<Step>) -> restichub_proto::agent::v1::Job {
        self.fx.job(job_id, steps)
    }
}

pub fn variable(name: &str, value: &str) -> ScriptEnv {
    ScriptEnv {
        name: name.to_owned(),
        value: Some(script_env::Value::StringValue(value.to_owned())),
    }
}

pub fn secret(name: &str, value: &str) -> ScriptEnv {
    ScriptEnv {
        name: name.to_owned(),
        value: Some(script_env::Value::SecretValue(value.to_owned())),
    }
}

pub fn script_step(
    step_id: &str,
    phase: HookPhase,
    policy: ScriptFailurePolicy,
    body: &str,
    env: Vec<ScriptEnv>,
) -> Step {
    interpreted(step_id, ScriptInterpreter::Bash, phase, policy, body, env)
}

pub fn interpreted(
    step_id: &str,
    interpreter: ScriptInterpreter,
    phase: HookPhase,
    policy: ScriptFailurePolicy,
    body: &str,
    env: Vec<ScriptEnv>,
) -> Step {
    let mut built = super::steps::make_step(
        step_id,
        step::Kind::Script(ScriptStep {
            name: format!("name-{step_id}"),
            interpreter: interpreter as i32,
            phase: phase as i32,
            body: body.to_owned(),
            failure_policy: policy as i32,
            env,
        }),
    );
    built.timeout_seconds = 30;
    built
}

pub fn simple(step_id: &str, phase: HookPhase, body: &str, env: Vec<ScriptEnv>) -> Step {
    script_step(step_id, phase, ScriptFailurePolicy::AbortBackup, body, env)
}

pub fn status_of(collected: &Collected, step_id: &str) -> StepStatus {
    StepStatus::try_from(collected.step(step_id).status).expect("a known status")
}

pub async fn rejection(jobs: &Jobs, job: &restichub_proto::agent::v1::Job) -> Option<RejectReason> {
    let (tx, mut rx) = mpsc::channel(8);
    jobs.attach(&tx);
    jobs.handle(signed(job), &ring(), AGENT, TENANT).await;
    let message = rx.recv().await.expect("the agent must answer");
    match message.payload {
        Some(agent_message::Payload::JobRejected(rejected)) => {
            Some(RejectReason::try_from(rejected.reason).expect("a known reason"))
        }
        _ => None,
    }
}

pub fn mark_body(host: &Host, name: &str) -> String {
    format!(
        "echo {name} >> '{}'\n",
        host.marks().join("order").display()
    )
}

pub fn contains_bytes(dir: &Path, needle: &str) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    entries.filter_map(Result::ok).any(|entry| {
        let path = entry.path();
        if path.is_dir() {
            contains_bytes(&path, needle)
        } else {
            std::fs::read(&path).is_ok_and(|bytes| {
                bytes
                    .windows(needle.len())
                    .any(|window| window == needle.as_bytes())
            })
        }
    })
}
