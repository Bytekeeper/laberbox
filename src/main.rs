use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

mod server;

/// This is the request from a client
#[derive(Deserialize, Debug)]
pub struct Post {
    path: String,
    message: String,
    name: String,
    #[serde(default)]
    url: String,
    redirect_url: String,
}

/// This will be serialized into a comment file on GitHub
#[derive(Serialize, Debug)]
struct Comment<'a> {
    id: &'a str,
    message: &'a str,
    name: &'a str,
    url: &'a str,
    date: u64,
}

#[derive(Deserialize)]
pub struct Committer {
    pub name: String,
    pub email: String,
}

#[derive(Deserialize)]
pub struct Config {
    pub listen: std::net::SocketAddr,
    pub token: String,
    pub owner: String,
    pub repo: String,
    pub committer: Committer,
    /// Root content directory in the repo, e.g. "content" for Hugo/Zola or "_posts" for Jekyll.
    pub content_dir: String,
    /// GitHub API base URL. Defaults to https://api.github.com; override in tests to point at a mock.
    #[serde(default = "default_github_api_url")]
    pub github_api_url: String,
    /// Minimum seconds between accepted requests. Defaults to 10; set lower in tests.
    #[serde(default = "default_rate_limit_secs")]
    pub rate_limit_secs: u64,
}

fn default_github_api_url() -> String {
    "https://api.github.com".to_string()
}

fn default_rate_limit_secs() -> u64 {
    10
}

pub static CONFIG: OnceLock<Config> = OnceLock::new();

fn main() -> anyhow::Result<()> {
    env_logger::Builder::new()
        .filter_level(log::LevelFilter::Info)
        .try_init()
        .expect("Failed to initialize logging");
    let config_path = std::env::var("LABERBOX_CONFIG").unwrap_or_else(|_| "config.yaml".into());
    let Ok(_) = CONFIG.set(
        serde_yaml::from_slice(&std::fs::read(&config_path).context("Loading config file")?)
            .context("Parsing config file")?,
    ) else {
        panic!("Could not set config")
    };
    server::main()?;
    Ok(())
}
