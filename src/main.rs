use std::{
    process::{ExitCode, ExitStatus, Stdio},
    time::Duration,
};

use crate::service::{RestartPolicy, Service};
use clap::Parser;
use tokio::{io, process::Command, time::Instant};
use tracing::{Instrument, Level, debug, error, info, info_span, warn};
use tracing_subscriber::fmt;

mod service;

const HEALTHY_AFTER: Duration = Duration::from_secs(10);

#[derive(Parser, Debug)]
#[command(version = "v0.0.0", about = "Homebrew supervisor for servers", long_about = None)]
struct Cli {}

async fn attempt(cmd: &mut Command) -> io::Result<ExitStatus> {
    let mut child = cmd.spawn()?;

    child.wait().await
}

async fn run(service: &Service) -> anyhow::Result<()> {
    let mut cmd = Command::new(service.command());
    cmd.stdout(Stdio::inherit());
    cmd.stderr(Stdio::inherit());
    cmd.args(service.args());

    let mut fails: u32 = 0;

    loop {
        debug!("running {}", service.command());

        let start = Instant::now();
        let outcome = attempt(&mut cmd).await;
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
                tokio::time::sleep(delay).await;

                continue;
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

    let service1 = Service::new("test", "sh", vec!["-c", "exit 1"]);
    let service2 = Service::new(
        "always-fail",
        "idfkskibiditoiletistgifthisisacommand",
        vec!["-c", "exit 1"],
    );

    let services = vec![service1, service2];

    info!("starting {} services", services.len());

    let mut tasks = Vec::with_capacity(services.len());
    for service in services {
        let service_span = info_span!("service", name = %service.name());

        tasks.push(tokio::spawn(
            async move { run(&service).await }.instrument(service_span),
        ));
    }

    let mut results = Vec::with_capacity(tasks.len());
    for task in tasks {
        results.push(task.await.unwrap());
    }

    Ok(ExitCode::SUCCESS)
}
