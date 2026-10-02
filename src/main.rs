use std::{
    fs,
    process::{ExitCode, ExitStatus, Stdio},
    time::Duration,
};

use crate::{
    config::Config,
    service::{RestartPolicy, Service},
};
use anyhow::Context;
use clap::Parser;
use tokio::{
    process::Command,
    signal::{self, unix::SignalKind},
    time::Instant,
};
use tokio_util::sync::CancellationToken;
use tracing::{Instrument, Level, debug, error, info, info_span, warn};
use tracing_subscriber::fmt;

mod config;
mod service;

const HEALTHY_AFTER: Duration = Duration::from_secs(10);
const TERM_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Parser, Debug)]
#[command(version = "v0.0.0", about = "Homebrew supervisor for servers", long_about = None)]
struct Cli {}

async fn run(service: &Service, cancel: CancellationToken) -> anyhow::Result<()> {
    let mut cmd = Command::new(service.command());
    cmd.stdout(Stdio::inherit());
    cmd.stderr(Stdio::inherit());
    cmd.args(service.args());

    let mut fails: u32 = 0;

    loop {
        debug!("running {}", service.command());

        let start = Instant::now();

        let outcome = match cmd.spawn() {
            Ok(mut child) => {
                tokio::select! {
                    r = child.wait() => {
                        r
                    }

                    _ = cancel.cancelled() => {
                        let pid = match child.id() {
                            Some(x) => x as libc::pid_t,
                            None => break,
                        };

                        let term_ok = unsafe { libc::kill(pid, libc::SIGTERM) } != -1;
                        if !term_ok {
                            warn!("failed to terminate process, killing");

                            child.kill().await?;

                            break;
                        }

                        let timeout = tokio::time::timeout(TERM_TIMEOUT, child.wait()).await;
                        match timeout {
                            Ok(_) => {
                                info!("terminated proccess successfully");
                            },

                            Err(_) => {
                                warn!("failed to terminate process in {}s, killing", TERM_TIMEOUT.as_secs());

                                child.kill().await?;
                            }
                        }

                        break;
                    }
                }
            }

            Err(e) => Err(e),
        };

        let elapsed = start.elapsed();

        match outcome {
            Ok(status) if elapsed >= HEALTHY_AFTER => {
                fails = 0;

                info!(
                    "exited ({}) after {}s, marking as healthy",
                    status,
                    elapsed.as_secs()
                );
            }

            Ok(status) => {
                fails = (fails + 1).min(5);

                warn!(
                    "exited ({}) before {}s, increasing restart delay",
                    status,
                    HEALTHY_AFTER.as_secs()
                );
            }

            Err(e) => {
                fails = (fails + 1).min(5);

                error!("failed to start: {}, increasing restart delay", e);
            }
        }

        let delay = Duration::from_secs(2u64.pow(fails));

        match service.policy() {
            RestartPolicy::Always => {
                tokio::select! {
                    _ = tokio::time::sleep(delay) => {
                        continue;
                    }

                    _ = cancel.cancelled() => {
                        debug!("terminated proccess successfully");

                        break;
                    }
                }
            }

            RestartPolicy::Never => {
                break;
            }
        }
    }

    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<ExitCode> {
    let fmt = fmt()
        .with_target(false)
        .with_max_level(Level::DEBUG)
        .finish();

    tracing::subscriber::set_global_default(fmt)?;

    let _cli = Cli::parse();

    let config_path = "pidfather.toml";
    let config_text = fs::read_to_string(config_path).context("missing pidfather.toml")?;
    let config: Config = toml::from_str(&config_text)?;

    info!("loaded config from {}", config_path);
    info!("starting {} services", config.services.len());

    let cancel = CancellationToken::new();

    let mut tasks = Vec::with_capacity(config.services.len());
    for service in config.services {
        let service_span = info_span!("service", name = %service.name());
        let cancel_clone = cancel.clone();

        tasks.push(tokio::spawn(
            async move { run(&service, cancel_clone).await }.instrument(service_span),
        ));
    }

    let mut term = signal::unix::signal(SignalKind::terminate())?;

    tokio::select! {
        _ = term.recv() => {
            info!("received SIGTERM, shutting down");
        }

        _ = signal::ctrl_c() => {
            info!("received SIGINT, shutting down");
        }
    }

    cancel.cancel();

    let mut results = Vec::with_capacity(tasks.len());
    for task in tasks {
        results.push(task.await.unwrap());
    }

    Ok(ExitCode::SUCCESS)
}
