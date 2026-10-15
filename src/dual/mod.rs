//! Experimental Linux/WSL entry point. Real-client authentication is not certified.
pub mod cli;
pub mod doctor;
pub mod launch;
pub mod security;
pub mod session;

pub const PARENT_MODEL: &str = "claude-opus-5-5";
pub const WORKER_MODEL: &str = "gpt-6-astra[1m]";
pub const CLIENT_VERSION: &str = "2.1.300";
