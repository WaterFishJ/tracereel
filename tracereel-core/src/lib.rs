//! tracereel-core — pure-Rust TraceReel data model and math.
//!
//! This crate holds the path/rendering math and the QuickRoute/GPX parsers, kept free
//! of any GUI or ffmpeg dependency so it can be unit-tested in isolation.

pub mod gpx;
pub mod math;
pub mod quickroute;
pub mod qrt;
pub mod route;
pub mod route_manager;
pub mod splits;
