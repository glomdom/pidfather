use std::{
    process::{Command, ExitCode, Stdio},
    thread,
    time::{Duration, Instant},
    vec,
};

use clap::Parser;

#[allow(dead_code)]
struct Service {
    name: String,
    command: String,
    args: Vec<String>,
    restart_policy: RestartPolicy,
}

impl Service {
    pub fn new(
        name: impl Into<String>,
        command: impl Into<String>,
        args: Vec<impl Into<String>>,
    ) -> Self {
        Self {
            name: name.into(),
            command: command.into(),
            args: args.into_iter().map(Into::into).collect(),
            restart_policy: RestartPolicy::Always,
        }
    }
}

#[allow(dead_code)]
enum RestartPolicy {
    Always,
    Never,
}

#[derive(Parser, Debug)]
#[command(version = "v0.0.0", about = "Homebrew supervisor for servers", long_about = None)]
struct Cli {}

fn run(service: &Service) -> anyhow::Result<()> {
    let mut cmd = Command::new(&service.command);
    cmd.stdout(Stdio::inherit());
    cmd.stderr(Stdio::inherit());
    cmd.args(&service.args);

    let mut fails = 0;

    loop {
        println!("running {:?}", cmd);

        let start = Instant::now();
        let mut res = cmd.spawn()?;
        res.wait()?;

        let elapsed = start.elapsed();
        let delay = Duration::from_secs(1 * 2u64.pow(fails));

        if elapsed >= delay {
            fails = 0;

            println!(
                "service `{}` lasted longer than {}s, marking as healthy",
                service.name,
                delay.as_secs()
            );
        } else {
            fails = (fails + 1).min(5);

            println!(
                "service `{}` failed to last longer than {}s, increasing restart delay",
                service.name,
                delay.as_secs()
            );
        }

        match service.restart_policy {
            RestartPolicy::Always => {
                thread::sleep(delay);

                continue;
            }

            RestartPolicy::Never => break,
        }
    }

    Ok(())
}

fn main() -> anyhow::Result<ExitCode> {
    let _cli = Cli::parse();

    // let service = Service::new(
    //     "test",
    //     "sh",
    //     vec![
    //         "-c",
    //         "for i in 1 2 3; do echo tick $i; echo oops $i >&2; sleep 1; done; exit 1",
    //     ],
    // );

    let service = Service::new("test", "sh", vec!["-c", "exit 1"]);

    let services = vec![service];

    run(&services[0])?;

    Ok(ExitCode::SUCCESS)
}
