//! Test control plane for local runs of the emulator.
//!
//! The binary requires the `test-util` feature. It writes a test certificate
//! set, serves `AgentService.Connect` over mTLS and dispatches a sample job to
//! every connected agent on a period. Epic #1 replaces it with the real control
//! plane.

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use restichub_emulator::control::TestControlPlane;
use restichub_emulator::identity::TestPki;
use restichub_emulator::job::sample_job;

struct Args {
    certs: PathBuf,
    agents: usize,
    period: Duration,
}

fn take_value(args: &[String], index: &mut usize, name: &str) -> anyhow::Result<String> {
    if let Some((_, value)) = args[*index].split_once('=') {
        return Ok(value.to_owned());
    }
    *index += 1;
    args.get(*index)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("missing value for {name}"))
}

fn parse(args: &[String]) -> anyhow::Result<Args> {
    let mut args_it = Args {
        certs: PathBuf::from("certs"),
        agents: 1,
        period: Duration::from_secs(5),
    };
    let mut index = 0;
    while index < args.len() {
        let name = args[index].split('=').next().unwrap_or_default().to_owned();
        match name.as_str() {
            "--certs" => args_it.certs = PathBuf::from(take_value(args, &mut index, &name)?),
            "--agents" => args_it.agents = take_value(args, &mut index, &name)?.parse()?,
            "--period" => {
                args_it.period =
                    Duration::from_secs_f64(take_value(args, &mut index, &name)?.parse()?);
            }
            other => anyhow::bail!("unknown argument: {other}"),
        }
        index += 1;
    }
    Ok(args_it)
}

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("restichub-emulator-test-server: {error:#}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> anyhow::Result<()> {
    let args = parse(&std::env::args().skip(1).collect::<Vec<_>>())?;
    let pki = if args.certs.join("ca.pem").exists() {
        TestPki::read(&args.certs)?
    } else {
        let pki = TestPki::generate(args.agents)?;
        pki.write(&args.certs)?;
        pki
    };
    let plane = Arc::new(TestControlPlane::start(&pki, args.agents).await?);
    println!("listening on {}", plane.url());

    let dispatcher = if args.period > Duration::ZERO {
        let plane = Arc::clone(&plane);
        Some(tokio::spawn(async move {
            let mut counter = 0u64;
            let mut ticker = tokio::time::interval(args.period);
            loop {
                ticker.tick().await;
                for id in plane.agents() {
                    let job = sample_job(&id, &format!("job-{counter:012}"));
                    plane.dispatch(&id, &job);
                    counter += 1;
                }
            }
        }))
    } else {
        None
    };

    tokio::signal::ctrl_c().await?;
    if let Some(dispatcher) = dispatcher {
        dispatcher.abort();
    }
    println!(
        "stopping: {} connections, {} results, {} rejected",
        plane.connect_count(),
        plane.result_count(),
        plane.rejected_count()
    );
    Ok(())
}
