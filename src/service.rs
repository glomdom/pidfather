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
    pub name: String,
    pub command: String,

    #[serde(default)]
    pub args: Vec<String>,

    #[serde(default)]
    pub restart_policy: RestartPolicy,
    pub user: String,
}
