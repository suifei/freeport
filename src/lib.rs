pub mod batch;
pub mod catalog;
pub mod cli;
pub mod elevation;
pub mod engine;
pub mod handles;
pub mod i18n;
pub mod inventory;
pub mod platform;
pub mod rules;
pub mod scan;
pub mod selection;

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
pub fn fail<T>(message: impl Into<String>) -> Result<T> {
    Err(message.into().into())
}

pub fn human(bytes: u64) -> String {
    if bytes >= 1 << 30 {
        format!("{:.2} GiB", bytes as f64 / (1u64 << 30) as f64)
    } else if bytes >= 1 << 20 {
        format!("{:.1} MiB", bytes as f64 / (1u64 << 20) as f64)
    } else {
        format!("{:.1} KiB", bytes as f64 / 1024.)
    }
}
