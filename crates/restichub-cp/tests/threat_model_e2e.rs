//! End-to-end checks of the threat model rows that the protocol epic owns (design/threat-model.md,
//! NFR-SEC-02). Each test name starts with the threat IDs and the matching rows name the test.
//!
//! The tests run the real control plane, the real listener and the emulator or the real agent
//! check. The attacker has no job signing key and no certificate of the victim.

#![allow(clippy::expect_used)]

mod common;
mod pki_support;
mod queue_support;
mod rotation_support;

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use async_trait::async_trait;
use ed25519_dalek::SigningKey;
use prost_types::Timestamp;
use restichub_agent::rotation as agent_rotation;
use restichub_canary_scan::{Canary, Scanner};
use restichub_cp::rotation::{self, PresentedCertificate};
use restichub_emulator::agent::ProgressSink;
use restichub_emulator::executor::{Executor, StepOutcome};
use restichub_proto::agent::v1::{
    ControlMessage, Job, KeyUpdate, SignedJob, SigningKeyUpdate, Step, control_message,
};
use restichub_proto::signing::{sign_job, sign_key_update};

use queue_support::{Env, eventually, test_queue};

/// Counts the steps that the emulated agent runs.
struct Counting(Arc<AtomicU32>);

#[async_trait]
impl Executor for Counting {
    async fn run_step(&self, _step: &Step, _progress: &ProgressSink) -> StepOutcome {
        self.0.fetch_add(1, Ordering::SeqCst);
        StepOutcome::success()
    }
}

fn attacker_key() -> SigningKey {
    SigningKey::from_bytes(&[0x42; 32])
}

fn timestamp(offset_seconds: i64) -> Timestamp {
    Timestamp {
        seconds: time::OffsetDateTime::now_utc().unix_timestamp() + offset_seconds,
        nanos: 0,
    }
}

/// A job for the given agent that only the signature can make acceptable.
fn job_for(job_id: &str, identity: &restichub_emulator::enroll::EnrolledIdentity) -> Job {
    Job {
        job_id: job_id.to_owned(),
        tenant_id: identity.tenant_id.clone(),
        agent_id: identity.agent_id.clone(),
        issued_at: Some(timestamp(-5)),
        expires_at: Some(timestamp(300)),
        ..Job::default()
    }
}

fn send_signed(env: &Env, agent: restichub_db::AgentId, signed: SignedJob) {
    let sent = env.state.agent_registry().send_to_agent(
        agent,
        ControlMessage {
            payload: Some(control_message::Payload::SignedJob(signed)),
        },
    );
    assert!(sent, "the stream of the agent must be open");
}

/// Waits until a log line holds both texts.
async fn log_holds(logs: &Arc<Mutex<Vec<u8>>>, first: &str, second: &str) {
    log_count(logs, first, second, 1).await;
}

/// Waits until two log lines hold both texts.
async fn log_holds_twice(logs: &Arc<Mutex<Vec<u8>>>, first: &str, second: &str) {
    log_count(logs, first, second, 2).await;
}

async fn log_count(logs: &Arc<Mutex<Vec<u8>>>, first: &str, second: &str, wanted: usize) {
    eventually(|| async {
        let text = String::from_utf8_lossy(&logs.lock().expect("the log buffer")).into_owned();
        text.lines()
            .filter(|line| line.contains(first) && line.contains(second))
            .count()
            >= wanted
    })
    .await;
}

/// T3, T13, AC-PR-01, AC-PR-09: a job that an attacker signed and a key update that an attacker
/// signed do not move the agent. The control plane learns the typed reason of each refusal, and a
/// genuine job still runs afterwards.
#[tokio::test(flavor = "multi_thread")]
async fn t3_t13_a_forged_job_and_a_forged_key_update_are_refused_by_the_agent() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let logs = captured_logs();
    let steps = Arc::new(AtomicU32::new(0));
    let identity = env.enroll_agent("forgery-host").await;
    let current = identity.signing_keys[0].key_id.clone();
    let emulator = env
        .start_emulator_for(identity.clone(), Arc::new(Counting(steps.clone())))
        .await;
    let repository = env.create_repository(emulator.id, "pw").await;

    // A genuine key id with a signature of the attacker: the signature does not match. The agent
    // cannot trust the identifier of such a job, so the control plane gets the reason without a
    // job and logs it.
    send_signed(
        &env,
        emulator.id,
        sign_job(
            &job_for(&uuid::Uuid::now_v7().to_string(), &identity),
            current.clone(),
            &attacker_key(),
        ),
    );
    log_holds(logs, &emulator.id.to_string(), "bad_signature").await;

    // A key that the agent never received.
    send_signed(
        &env,
        emulator.id,
        sign_job(
            &job_for(&uuid::Uuid::now_v7().to_string(), &identity),
            "attacker-key",
            &attacker_key(),
        ),
    );
    log_holds(logs, &emulator.id.to_string(), "unknown_key").await;

    // A key update that the attacker signs and attributes to the genuine key: the signature check
    // fails, so the agent does not trust the key of the attacker.
    let update = KeyUpdate {
        key_id: "attacker-key".to_owned(),
        public_key: attacker_key().verifying_key().to_bytes().to_vec(),
        signer_key_id: current.clone(),
        issued_at: Some(timestamp(0)),
        revoked_key_ids: vec![current.clone()],
    };
    let forged: SigningKeyUpdate = sign_key_update(&update, &attacker_key());
    let sent = env.state.agent_registry().send_to_agent(
        emulator.id,
        ControlMessage {
            payload: Some(control_message::Payload::SigningKeyUpdate(forged)),
        },
    );
    assert!(sent);
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    send_signed(
        &env,
        emulator.id,
        sign_job(
            &job_for(&uuid::Uuid::now_v7().to_string(), &identity),
            "attacker-key",
            &attacker_key(),
        ),
    );
    log_holds_twice(logs, &emulator.id.to_string(), "unknown_key").await;
    assert_eq!(steps.load(Ordering::SeqCst), 0, "no forged job ran a step");

    // The genuine job still runs: the forged update did not revoke the genuine key.
    let genuine = env.run_fresh_plan(repository, emulator.id).await;
    eventually(|| async {
        env.pass().await;
        env.job_status(genuine).await == "succeeded"
    })
    .await;
    assert!(steps.load(Ordering::SeqCst) > 0);

    emulator.stop().await;
    env.db.cleanup().await;
}

/// T4, AC-PR-01, ADR-0020: a job that reaches the agent a second time does not run again. The
/// agent answers with the stored result.
#[tokio::test(flavor = "multi_thread")]
async fn t4_a_replayed_job_does_not_run_twice() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    // A stream plays the first delivery and keeps the signed job; the emulator takes the identity
    // over afterwards and gets the same bytes twice.
    let mut first = env.connect_agent("replay-host").await;
    let identity = first.identity.clone();
    let id = first.id;
    let repository = env.create_repository(id, "pw").await;
    let backup = env.run_fresh_plan(repository, id).await;
    env.pass().await;
    let signed = first.job().await;
    drop(first);
    // The emulator takes the identity over only after the first stream is gone.
    eventually(|| async { !env.state.agent_registry().is_online(id) }).await;

    let steps = Arc::new(AtomicU32::new(0));
    let emulator = env
        .start_emulator_for(identity, Arc::new(Counting(steps.clone())))
        .await;
    send_signed(&env, id, signed.clone());
    eventually(|| async { env.job_status(backup).await == "succeeded" }).await;
    let ran = steps.load(Ordering::SeqCst);
    assert!(ran > 0);

    send_signed(&env, id, signed);
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    assert_eq!(steps.load(Ordering::SeqCst), ran, "the replay ran no step");
    assert_eq!(env.job_status(backup).await, "succeeded");
    emulator.stop().await;
    env.db.cleanup().await;
}

/// T6, AC-TN-02, AC-PR-05, FR-PR-08: an agent acts only for its own identity. The messages of
/// agent A about a job of agent B change nothing, and a request for a certificate names the
/// identity of the presented certificate, not the one in the request.
#[tokio::test(flavor = "multi_thread")]
async fn t6_an_agent_cannot_act_for_another_agent() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let attacker = env.connect_agent("attacker-host").await;
    let mut victim = env.connect_agent("victim-host").await;
    let repository = env.create_repository(victim.id, "pw").await;
    let backup = env.run_fresh_plan(repository, victim.id).await;
    env.pass().await;
    let signed = victim.job().await;
    let job = queue_support::decode_job_id(&signed);
    assert_eq!(job, backup.to_string());

    // The attacker acknowledges, runs and finishes the job of the victim.
    attacker.ack(&job).await;
    attacker
        .result(
            &job,
            restichub_proto::agent::v1::JobStatus::Succeeded,
            restichub_proto::agent::v1::StepStatus::Success,
        )
        .await;
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    assert_eq!(
        env.job_status(backup).await,
        "sent",
        "the job of the victim is unchanged"
    );

    // The attacker asks for a certificate that names the victim: the control plane signs the
    // identity of the presented certificate, and refuses a request for another one.
    let attacker_cert =
        PresentedCertificate::from_der(&attacker.identity.certificate_der).expect("a cert");
    let due = agent_rotation::rotation_due(attacker_cert.not_before, attacker_cert.not_after);
    let victim_uri = format!(
        "restichub://tenant/{}/agent/{}",
        victim.identity.tenant_id, victim.identity.agent_id
    );
    let refusal = rotation::issue(
        &env.state,
        env.tenant,
        attacker.id,
        &attacker_cert,
        &rotation_support::csr_with_uri(&victim_uri),
        due,
    )
    .await
    .expect_err("a certificate for the victim is refused");
    assert_eq!(
        refusal,
        restichub_proto::agent::v1::CertificateRejectReason::InvalidRequest
    );

    // The genuine victim finishes its own job.
    victim.ack(&job).await;
    eventually(|| async { env.job_status(backup).await == "running" }).await;
    env.db.cleanup().await;
}

/// The log lines of the tests of this binary.
fn captured_logs() -> &'static Arc<Mutex<Vec<u8>>> {
    static LOGS: OnceLock<Arc<Mutex<Vec<u8>>>> = OnceLock::new();
    LOGS.get_or_init(|| {
        let buffer = Arc::new(Mutex::new(Vec::new()));
        let writer = buffer.clone();
        let _ = tracing_subscriber::fmt()
            .with_env_filter("debug,sqlx=warn,hyper=warn,h2=warn,rustls=warn,tonic=warn")
            .with_ansi(false)
            .with_writer(move || LogWriter(writer.clone()))
            .try_init();
        buffer
    })
}

struct LogWriter(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for LogWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .map_err(|_| std::io::Error::other("the log buffer is poisoned"))?
            .extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// The base64 body of a PEM private key, without the armor lines.
fn pem_body(pem: &str) -> String {
    pem.lines()
        .filter(|line| !line.starts_with("-----"))
        .collect::<String>()
}

/// T9, T21, AC-ID-02: after an enrollment and a rotation, a dump of the database and the log
/// lines hold no private key of the agent and no value of the enrollment token.
#[tokio::test(flavor = "multi_thread")]
async fn t9_t21_the_enrollment_and_the_rotation_leave_no_key_or_token_in_the_database_or_the_logs()
{
    let logs = captured_logs();
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let (token, _) = common::seed_enrollment_token_value(
        env.db.database(),
        env.tenant,
        env.creator_id(),
        &[],
        time::OffsetDateTime::now_utc() + time::Duration::hours(1),
    )
    .await;
    let enrolled = env.enroll_with_token(&token).await;
    let agent = rotation_support::agent_of(&enrolled);
    let presented = PresentedCertificate::from_der(&enrolled.certificate_der).expect("a cert");
    let due = agent_rotation::rotation_due(presented.not_before, presented.not_after);
    let (pending, csr) = agent_rotation::begin().expect("a key pair");
    let (issued, next) = rotation::issue(&env.state, env.tenant, agent, &presented, &csr, due)
        .await
        .expect("the rotation is signed");
    let rotated = agent_rotation::accept(
        &rotation_support::stored_identity(&enrolled),
        &pending,
        &issued,
        due,
    )
    .expect("the agent accepts it");
    rotation::confirm(&env.state, env.tenant, agent, &next.serial, due)
        .await
        .expect("the confirmation");

    let canaries = [
        Canary::parse(&pem_body(&enrolled.key_pem)),
        Canary::parse(&pem_body(&rotated.key_pem)),
        Canary::parse(&token),
    ];
    let scanner = Scanner::new(&canaries);
    let mut dump = pki_support::dump_all(&env.db, env.tenant).await;
    dump.push_str(&pki_support::dump_all(&env.db, restichub_db::PLATFORM_TENANT_ID).await);
    assert!(
        scanner.scan("database", dump.as_bytes()).is_empty(),
        "the database dump holds a private key or a token"
    );
    let log = logs.lock().expect("the log buffer").clone();
    assert!(!log.is_empty(), "the test captured the log lines");
    assert!(
        scanner.scan("log", &log).is_empty(),
        "the log holds a private key or a token"
    );
    env.db.cleanup().await;
}
