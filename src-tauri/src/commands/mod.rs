//! Thin layer of Tauri commands over the scanner and query modules.
//!
//! Commands are `async` so that heavier queries (filters, treemap) do not block the window's
//! main thread.

pub mod actions;
pub mod persist;
pub mod query;
pub mod scan;
mod state;

pub use state::AppState;

type Res<T> = Result<T, String>;
