use serde::Deserialize;

use crate::service::Service;

#[derive(Deserialize)]
pub struct Config {
    pub services: Vec<Service>,
}
