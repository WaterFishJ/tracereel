//! TraceReel · 迹映 — orienteering video creation and route replay.
//!
//! Top-level crate. The GUI (QML + qmetaobject) lives in `ui`, the pure-Rust
//! route/map math and data-format parsers live in the `tracereel-core` crate, and the
//! Gyroflow stabilization integration lives in `gyro`.

#![recursion_limit = "256"]

pub mod branding;
pub mod composition;
pub mod data_paths;
pub mod encode;
#[macro_use]
pub mod encode_log;
pub mod export_queue;
pub mod gui;
pub mod gyro;
pub mod hud;
pub mod lens_db;
pub mod pipe;
pub mod presets;
pub mod preview_audio;
pub mod probe;
mod process;
pub mod receipt_ocr;
pub mod qtwrap;
pub mod settings;
pub mod settings_proxy;
pub mod theme;
pub mod video;
