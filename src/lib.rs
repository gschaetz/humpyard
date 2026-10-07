//! Unified LLM gateway: protocol translation in front of Switchyard-routed providers.

pub mod config;
pub mod error;
pub mod policy;
pub mod pool;
pub mod routing;
pub mod server;
