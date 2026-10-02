use anyhow::{Context, bail};
use clap::Parser;
use std::{
    env,
    ffi::{CStr, CString},
    fs,
    process::{ExitCode, Stdio},
    time::Duration,
};

use tokio::{
    io::AsyncBufReadExt,
    process::Command,
    signal::{self, unix::SignalKind},
    time::Instant,
};

use tokio_util::sync::CancellationToken;
use tracing::{Instrument, Level, debug, error, info, info_span, warn};
use tracing_subscriber::fmt;

use crate::{
    config::Config,
    service::{RestartPolicy, Service},
};

mod config;
mod service;

const HEALTHY_AFTER: Duration = Duration::from_secs(10);
const TERM_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Parser, Debug)]
#[command(version = "v0.0.0", about = "Homebrew supervisor for servers", long_about = None)]
struct Cli {}

fn read_usr(user: &str) -> anyhow::Result<(u32, u32, String)> {
    let user_cstr = CString::new(user)?;

    let mut buf_len = match unsafe { libc::sysconf(libc::_SC_GETPW_R_SIZE_MAX) } {
        n if n > 0 => n as usize,

        _ => 1024,
    };

    loop {
        let mut buf: Vec<libc::c_char> = vec![0; buf_len];
        let mut passwd: libc::passwd = unsafe { std::mem::zeroed() };
        let mut result: *mut libc::passwd = std::ptr::null_mut();

        let rc = unsafe {
            libc::getpwnam_r(
                user_cstr.as_ptr(),
                &mut passwd,
                buf.as_mut_ptr(),
                buf.len(),
                &mut result,
            )
        };

        if rc == libc::ERANGE && buf_len < (1 << 20) {
            buf_len *= 2;

            continue;
        }

        if rc != 0 {
            bail!(
                "getpwnam_r failed for `{}`: {}",
                user,
                std::io::Error::from_raw_os_error(rc)
            );
        }

        if result.is_null() {
            bail!("user `{}` does not exist", user);
        }

        let home = unsafe { CStr::from_ptr(passwd.pw_dir) }
            .to_str()?
            .to_owned();

        return Ok((passwd.pw_uid, passwd.pw_gid, home));
    }
}

async fn run(service: &Service, cancel: CancellationToken) -> anyhow::Result<()> {
    let (usr_uid, usr_gid, usr_pwd) = read_usr(&service.user)?;

    let mut cmd = Command::new(&service.command);
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    cmd.uid(usr_uid);
    cmd.gid(usr_gid);
    cmd.env("HOME", &usr_pwd);
    cmd.current_dir(usr_pwd);

    cmd.process_group(0);
    cmd.args(&service.args);

    let mut fails: u32 = 0;

    loop {
        debug!("running {}", service.command);

        let start = Instant::now();
        let outcome = match cmd.spawn() {
            Ok(mut child) => {
                let child_stdout = child.stdout.take().unwrap();
                let child_stderr = child.stderr.take().unwrap();

                tokio::spawn(
                    async move {
                        let mut lines = tokio::io::BufReader::new(child_stdout).lines();

                        while let Ok(Some(line)) = lines.next_line().await {
                            info!("{}", line);
                        }
                    }
                    .in_current_span(),
                );

                tokio::spawn(
                    async move {
                        let mut lines = tokio::io::BufReader::new(child_stderr).lines();

                        while let Ok(Some(line)) = lines.next_line().await {
                            warn!("{}", line);
                        }
                    }
                    .in_current_span(),
                );

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
                                debug!("terminated proccess successfully");
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

        match service.restart_policy {
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
    let journal_stream = match env::var("JOURNAL_STREAM") {
        Ok(_) => true,
        Err(_) => false,
    };

    if journal_stream {
        fmt()
            .with_target(false)
            // .without_time()
            .with_max_level(Level::DEBUG)
            .init();
    } else {
        fmt().with_target(false).with_max_level(Level::DEBUG).init();
    }

    let _cli = Cli::parse();

    let config_path = "pidfather.toml";
    let config_text = fs::read_to_string(config_path).context("missing pidfather.toml")?;
    let config: Config = toml::from_str(&config_text)?;

    info!("loaded config from {}", config_path);
    info!("starting {} services", config.services.len());

    let cancel = CancellationToken::new();

    let mut tasks = Vec::with_capacity(config.services.len());
    for service in config.services {
        let service_span = info_span!("service", name = %service.name);
        let cancel_clone = cancel.clone();

        tasks.push(tokio::spawn(
            async move {
                if let Err(e) = run(&service, cancel_clone).await {
                    error!("service stopped: {:#}", e);
                }
            }
            .instrument(service_span),
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

    for task in tasks {
        if let Err(e) = task.await {
            error!("service task panicked: {}", e);
        }
    }

    Ok(ExitCode::SUCCESS)
}
