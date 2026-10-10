//! Check how the split list and view mode affect the map transform.
//!
//! `cargo run -p tracereel-core --example split_view -- <file.qrt>`

use std::env;
use std::fs;

use tracereel_core::qrt;
use tracereel_core::route_manager::{RouteManager, RouteStyle, ViewMode};

fn main() {
    let a: Vec<String> = env::args().collect();
    let data = fs::read(&a[1]).expect("read qrt");
    let proj = qrt::read_project_bytes(&data);
    let splits: Vec<i64> = proj.split_times_ms();
    let text = splits
        .iter()
        .map(|ms| {
            let s = ms / 1000;
            let (h, m, sec) = (s / 3600, (s % 3600) / 60, s % 60);
            if h > 0 {
                format!("{h}:{m:02}:{sec:02}")
            } else {
                format!("{m}:{sec:02}")
            }
        })
        .collect::<Vec<_>>()
        .join("|");
    println!("splits: {} -> {}", splits.len(), text);

    for (label, text, rel) in [
        ("no splits", String::new(), false),
        ("absolute", text.clone(), false),
        ("relative", text.clone(), true),
    ] {
        for (vm_name, vm) in [
            ("FixedSplit", ViewMode::FixedSplit),
            ("RunnerCenter", ViewMode::RunnerCentered),
            ("SplitOrient", ViewMode::RunnerCenteredSplitOriented),
        ] {
            let mut rm = RouteManager::default();
            rm.window_width = 1280.0;
            rm.window_height = 720.0;
            rm.initialize_from_points(
                proj.to_route_points(),
                &text,
                rel,
                RouteStyle::default(),
                vm,
                0.25,
            );
            let mut line = format!("{label:<10} {vm_name:<12}");
            for t in [5.0, 300.0, 1400.0] {
                rm.update(t, 16.0);
                line.push_str(&format!(
                    " | t={t:>5.0} s={:>8.3} x={:>10.1} y={:>10.1}",
                    rm.get_scale(),
                    rm.get_x(),
                    rm.get_y()
                ));
            }
            println!("{line}");
        }
    }
}
