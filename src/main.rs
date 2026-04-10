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
}

pub static CONFIG: OnceLock<Config> = OnceLock::new();

fn main() -> anyhow::Result<()> {
    env_logger::Builder::new()
        .filter_level(log::LevelFilter::Info)
        .try_init()
        .expect("Failed to initialize logging");
    let Ok(_) = CONFIG.set(
        serde_yaml::from_slice(&std::fs::read("config.yaml").context("Loading config file")?)
            .context("Parsing config file")?,
    ) else {
        panic!("Could not set config")
    };
    server::main()?;
    Ok(())
}
