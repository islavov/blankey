mod app;
pub mod config;
pub mod core;
pub mod crypto;
pub mod fieldtypes;
pub mod i18n;
pub mod keychain;
pub mod library;
pub mod links;
pub mod mcp;
pub mod relay;
pub mod render;
pub mod telemetry;
pub mod templates;
pub mod vault;

pub use app::run;
