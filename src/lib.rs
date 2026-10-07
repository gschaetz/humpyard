//! Unified LLM gateway: protocol translation in front of Switchyard-routed providers.

pub mod auth;
pub mod clock;
pub mod config;
pub mod error;
pub mod ledger;
pub mod metering;
pub mod policy;
pub mod pool;
pub mod pricing;
pub mod routing;
pub mod server;
