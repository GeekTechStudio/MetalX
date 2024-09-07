use anyhow::Result;
use log::{error, trace};
use serde::{Deserialize, Serialize};
use std::{fs::File, io::Read};

use clap::Parser;

#[derive(Parser, Debug)]
#[command(version = "0.1.0", about = "MetalX Agent", long_about = None)]
pub(crate) struct Args {
    /// Configuration file
    #[arg(short = 'c', long = "config")]
    pub config: Option<String>,

    /// Controller Address
    #[arg(short = 'A', long = "addr")]
    pub addr: Option<String>,

    /// Controller Port
    #[arg(short = 'P', long = "port")]
    pub port: Option<u16>,

    /// Use TLS for connection to controller
    #[arg(short = 'S', long = "https")]
    pub https: Option<bool>,

    /// API base path
    #[arg(short = 'B', long = "api-base-path")]
    pub api_base_path: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub(crate) struct Config {
    /// Controller Address
    pub addr: String,

    /// Controller Port
    pub port: u16,

    /// Use TLS for connection to controller
    pub https: bool,

    /// API base path
    pub api_base_path: String,
}

impl From<Args> for Config {
    fn from(args: Args) -> Self {
        let config = if let Some(path) = args.config {
            trace!("Try loading config file: {}", &path);
            if let Ok(conf) = Config::load_toml(&path) {
                conf
            } else {
                error!("Failed to load config file, fallback to default");
                Config::default()
            }
        } else {
            trace!("No config file provided, use default");
            Config::default()
        };
        Config {
            addr: args.addr.unwrap_or(config.addr),
            port: args.port.unwrap_or(config.port),
            https: args.https.unwrap_or(config.https),
            api_base_path: args.api_base_path.unwrap_or(config.api_base_path),
        }
    }
}

impl Config {
    fn load_toml(path: &str) -> Result<Self> {
        let mut fd = File::open(path)?;
        let buf = &mut String::new();
        if fd.read_to_string(buf)? < 1 {
            return Err(anyhow::anyhow!(
                "Empty config file or failed to read file content"
            ));
        }
        let config: Config = toml::from_str(buf)?;
        Ok(config)
    }

    fn default() -> Self {
        Config {
            addr: "controller".to_string(),
            port: 1091,
            https: false,
            api_base_path: "api/v1".to_string(),
        }
    }
}
