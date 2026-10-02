use serde::Deserialize;

#[derive(Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RestartPolicy {
    #[default]
    Always,
    Never,
}

#[derive(Deserialize)]
pub struct Service {
    name: String,
    command: String,

    #[serde(default)]
    args: Vec<String>,

    #[serde(default)]
    restart_policy: RestartPolicy,
}

impl Service {
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
