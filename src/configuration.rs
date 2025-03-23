use std::{fs::File, path::Path};

use log::error;
use serde::{Deserialize, Serialize};

const DEFAULT_PORT: u16 = 3030;
const DEFAULT_REPLICATION_PORT: u16 = 3040;
const DEFAULT_CACHE_SIZE_MB: usize = 500;

#[derive(Serialize, Deserialize)]
pub struct Configuration {
    max_range_response: Option<usize>,
    fsync: Option<FSyncStrategy>,
    port: Option<u16>,
    replication_port: Option<u16>,
    cache_size: Option<usize>,
    replicas: Option<Vec<String>>,
    is_replica: Option<bool>,
}

#[derive(Serialize, Deserialize, Copy, Clone, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum FSyncStrategy {
    Default,
    Strict,
}

impl Configuration {
    pub fn read(configuration_path: &str) -> Result<Configuration, ()> {
        let path = Path::new(configuration_path);
        if !path.exists() {
            return Err(());
        }

        let file = match File::open(path) {
            Ok(f) => f,
            Err(_) => return Err(()),
        };

        match serde_json::from_reader::<_, Configuration>(file) {
            Ok(r) => Ok(r),
            Err(e) => {
                error!("{e:#?}");

                Err(())
            }
        }
    }

    pub fn max_range_response(&self) -> Option<usize> {
        self.max_range_response
    }

    pub fn fsync(&self) -> FSyncStrategy {
        self.fsync.unwrap_or(FSyncStrategy::Default)
    }

    pub fn port(&self) -> u16 {
        self.port.unwrap_or(DEFAULT_PORT)
    }

    pub fn replication_port(&self) -> u16 {
        self.replication_port.unwrap_or(DEFAULT_REPLICATION_PORT)
    }

    pub fn cache_size(&self) -> usize {
        self.cache_size.unwrap_or(DEFAULT_CACHE_SIZE_MB) * 1024 * 1024
    }

    pub fn replicas(&self) -> Vec<String> {
        self.replicas.clone().unwrap_or(vec![])
    }

    pub fn is_replica(&self) -> bool {
        self.is_replica.unwrap_or(false)
    }
}
