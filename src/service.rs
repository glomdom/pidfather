pub enum RestartPolicy {
    Always,
    Never,
}

pub struct Service {
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

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn args(&self) -> &[String] {
        &self.args
    }

    pub fn command(&self) -> &str {
        &self.command
    }

    pub fn policy(&self) -> &RestartPolicy {
        &self.restart_policy
    }
}
