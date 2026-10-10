//! Print everything the `.qrt` reader recovered, with optional ground-truth
//! comparison against `demo/qrt_dump.ps1`'s JSON output.
//!
//! ```text
//! cargo run -p tracereel-core --example qrt_check -- <file.qrt> [dump.json]
//! ```

use std::env;
use std::fs;

fn fmt_seconds(ms: i64) -> String {
    let s = ms / 1000;
    let (h, m, sec) = (s / 3600, (s % 3600) / 60, s % 60);
    if h > 0 {
        format!("{h}:{m:02}:{sec:02}")
    } else {
        format!("{m}:{sec:02}")
    }
}

fn main() {
    let a: Vec<String> = env::args().collect();
    if a.len() < 2 {
        eprintln!("usage: qrt_check <file.qrt> [dump.json]");
        return;
    }
    let data = fs::read(&a[1]).expect("read qrt");
    let proj = tracereel_core::qrt::read_project_bytes(&data);

    match proj.map.as_ref() {
        Some(m) => println!(
            "map: {} {}x{} ({} bytes)",
            m.kind.as_str(),
            m.width,
            m.height,
            m.bytes.len()
        ),
        None => println!("map: none"),
    }
    println!("projection_origin: {:?}", proj.projection_origin);
    println!(
        "waypoints: {} (times recovered: {})",
        proj.waypoints.len(),
        proj.waypoints.iter().filter(|w| w.time_ticks > 0).count()
    );
    if let (Some(f), Some(l)) = (proj.waypoints.first(), proj.waypoints.last()) {
        println!("  first idx={} lon={:.7} lat={:.7}", f.index, f.longitude, f.latitude);
        println!("  last  idx={} lon={:.7} lat={:.7}", l.index, l.longitude, l.latitude);
        if f.time_ticks > 0 && l.time_ticks > 0 {
            let span = (l.time_ticks - f.time_ticks) as f64 / 1e7;
            println!("  time span: {span:.1} s");
        }
    }
    println!(
        "handles: {} (structural handle data present: {})",
        proj.handles.len(),
        proj.has_split_data
    );
    println!("splits to import: {}", proj.split_count);
    if let Some(m) = proj.map.as_ref() {
        let projected = proj.projected_positions();
        if let Some(t) = tracereel_core::qrt::initial_pixel_transform(
            &projected,
            m.width as f64,
            m.height as f64,
        ) {
            println!("  initial fit scale: {:.4} px/m", t.scale());
        }
        let pos = proj.map_positions();
        if !pos.is_empty() {
            let xs = pos.iter().map(|p| p.0);
            let ys = pos.iter().map(|p| p.1);
            let (min_x, max_x) = xs.fold((f64::MAX, f64::MIN), |(a, b), v| (a.min(v), b.max(v)));
            let (min_y, max_y) = ys.fold((f64::MAX, f64::MIN), |(a, b), v| (a.min(v), b.max(v)));
            println!(
                "  route bbox (map-centred px): x {min_x:.0}..{max_x:.0}  y {min_y:.0}..{max_y:.0}"
            );
        }
    }
    println!(
        "laps: {} (start={} split={} stop={})",
        proj.laps.len(),
        proj.laps.iter().filter(|l| l.kind == tracereel_core::qrt::LAP_START).count(),
        proj.laps.iter().filter(|l| l.kind == tracereel_core::qrt::LAP_SPLIT).count(),
        proj.laps.iter().filter(|l| l.kind == tracereel_core::qrt::LAP_STOP).count()
    );
    let splits = proj.split_times_ms();
    println!("split times: {}", splits.len());
    if !splits.is_empty() {
        println!("  first: {}", fmt_seconds(splits[0]));
        println!("  last : {}", fmt_seconds(*splits.last().unwrap()));
        println!(
            "SPLIT_TIMES={}",
            splits.iter().map(|m| fmt_seconds(*m)).collect::<Vec<_>>().join("|")
        );
    }

    if let Some(dump) = a.get(2) {
        // Pull the counters out of the JSON without a JSON dependency.
        let text = fs::read_to_string(dump).expect("read json");
        let grab = |key: &str| -> Option<u64> {
            let pat = format!("\"{key}\"");
            let i = text.find(&pat)?;
            let rest = &text[i + pat.len()..];
            let c = rest.find(':')?;
            let rest = rest[c + 1..].trim_start();
            let end = rest.find(|ch: char| !ch.is_ascii_digit()).unwrap_or(rest.len());
            rest[..end].parse().ok()
        };
        let want_map = grab("mapBytes").unwrap_or(0);
        let want_wp = grab("waypointCount").unwrap_or(0);
        let want_handles = grab("handleCount").unwrap_or(0);
        let got_map = proj.map.as_ref().map(|m| m.bytes.len() as u64).unwrap_or(0);
        println!("--- ground truth (demo/qrt_dump.ps1) ---");
        println!("map bytes:      {got_map} vs {want_map} -> {}", got_map == want_map);
        // The dump walks the graph recursively and so visits each Handle several
        // times; the file itself holds far fewer distinct handles.
        println!("waypoints:      {} vs {want_wp}", proj.waypoints.len());
        println!("handles (ours): {}", proj.split_count);
        println!("handles (dump): {want_handles} raw entries (traversal duplicates)");
    }
}
