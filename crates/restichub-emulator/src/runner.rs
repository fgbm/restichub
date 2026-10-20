//! Load runner of bench S2 (NFR-PERF-04).
//!
//! The runner starts the agents of a profile with a ramp-up, applies a
//! reconnect storm and serves the metrics. It drives the agent side only: the
//! control plane owns the schedule and the job rate (ADR-0002). The bench
//! collects the control plane CPU, RAM and dispatch delay on the control plane
//! side (#37, #38).

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use restichub_proto::signing::KeyRing;
use tokio::sync::watch;
use tokio::task::JoinHandle;

use crate::agent::{AgentConfig, AgentLoad, Backoff, run_agent};
use crate::executor::Executor;
use crate::hostile::{HostileLoad, SeenJobs};
use crate::identity::AgentIdentity;
use crate::logs::LogLoad;
use crate::metrics::{Metrics, Summary, bind, serve};
use crate::profile::{LoadProfile, ramp_delay, storm_selects};
use crate::random_fraction;

/// Everything one load run needs.
pub struct RunConfig {
    pub profile: LoadProfile,
    pub server_url: String,
    pub server_name: String,
    pub ca_pem: String,
    pub tenant_id: String,
    pub key_ring: KeyRing,
    pub identities: Vec<AgentIdentity>,
    pub executor: Arc<dyn Executor>,
    pub backoff: Backoff,
    pub metrics_addr: Option<SocketAddr>,
}

/// A running load profile. `stop` ends it and returns the summary.
pub struct LoadRun {
    profile: LoadProfile,
    metrics: Arc<Metrics>,
    shutdown: watch::Sender<bool>,
    tasks: Vec<JoinHandle<()>>,
    metrics_server: Option<JoinHandle<()>>,
    metrics_addr: Option<SocketAddr>,
    started: Instant,
}

impl LoadRun {
    /// Launches the agents and the metrics endpoint.
    ///
    /// # Errors
    ///
    /// Returns an error when the metrics endpoint cannot bind.
    pub async fn start(config: RunConfig) -> anyhow::Result<Self> {
        let started = Instant::now();
        let metrics = Arc::new(Metrics::default());
        let (shutdown, shutdown_rx) = watch::channel(false);

        let (metrics_server, metrics_addr) = match config.metrics_addr {
            Some(addr) => {
                let listener = bind(addr).await?;
                let bound = listener.local_addr()?;
                let metrics = Arc::clone(&metrics);
                let rx = shutdown_rx.clone();
                let task = tokio::spawn(async move {
                    if let Err(error) = serve(listener, metrics, rx).await {
                        eprintln!("metrics endpoint stopped: {error:#}");
                    }
                });
                (Some(task), Some(bound))
            }
            None => (None, None),
        };

        let agents = config.profile.agents;
        let storm = config.profile.reconnect_storm;
        let hostile = config.profile.hostile;
        let seen_jobs = (hostile.agents > 0 && hostile.forged_result_share > 0.0)
            .then(|| Arc::new(SeenJobs::default()));
        let log_load = (config.profile.log_lines_per_second > 0.0).then_some(LogLoad {
            lines_per_second: config.profile.log_lines_per_second,
            line_bytes: config.profile.log_line_bytes,
        });
        let mut tasks = Vec::with_capacity(agents);
        for (index, identity) in config.identities.iter().take(agents).cloned().enumerate() {
            let mut load = AgentLoad::new(Arc::clone(&metrics));
            load.run_start = started;
            if storm_selects(index, agents, storm.percent) {
                load.drop_at = Some(Duration::from_secs_f64(storm.at_seconds));
            }
            load.clock_skew_micros = (config.profile.clock_skew_seconds * 1e6) as i64;
            load.ack_delay = Duration::from_millis(config.profile.ack_delay_ms);
            load.os_label = config.profile.os_label(random_fraction());
            load.log_load = log_load;
            load.seen_jobs = seen_jobs.clone();
            if index < hostile.agents {
                load.hostile = Some(HostileLoad::new(&hostile));
            }
            let agent = Arc::new(AgentConfig {
                tenant_id: config.tenant_id.clone(),
                server_url: config.server_url.clone(),
                server_name: config.server_name.clone(),
                ca_pem: config.ca_pem.clone(),
                identity,
                key_ring: config.key_ring.clone(),
                executor: Arc::clone(&config.executor),
                backoff: config.backoff,
                load,
            });
            let delay = ramp_delay(index, config.profile.ramp_up_per_second);
            let mut rx = shutdown_rx.clone();
            tasks.push(tokio::spawn(async move {
                if !delay.is_zero() {
                    tokio::select! {
                        () = tokio::time::sleep(delay) => {}
                        _ = rx.changed() => return,
                    }
                }
                run_agent(agent, rx).await;
            }));
        }

        Ok(Self {
            profile: config.profile,
            metrics,
            shutdown,
            tasks,
            metrics_server,
            metrics_addr,
            started,
        })
    }

    /// Live counters of the run.
    #[must_use]
    pub fn metrics(&self) -> Arc<Metrics> {
        Arc::clone(&self.metrics)
    }

    /// The bound metrics endpoint address, when the endpoint is on.
    #[must_use]
    pub fn metrics_addr(&self) -> Option<SocketAddr> {
        self.metrics_addr
    }

    /// Configured length of the run.
    #[must_use]
    pub fn duration(&self) -> Duration {
        self.profile.duration()
    }

    /// Blocks until the profile duration passes, then stops.
    pub async fn wait(self) -> Summary {
        let duration = self.duration();
        tokio::time::sleep(duration).await;
        self.stop().await
    }

    /// Ends the run and returns the summary.
    pub async fn stop(self) -> Summary {
        let _ = self.shutdown.send(true);
        for task in self.tasks {
            let _ = task.await;
        }
        if let Some(server) = self.metrics_server {
            server.abort();
            let _ = server.await;
        }
        self.metrics
            .summary(self.profile.agents, self.started.elapsed().as_secs_f64())
    }
}
