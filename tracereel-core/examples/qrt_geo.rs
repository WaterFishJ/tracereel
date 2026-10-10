//! Diagnostic: how well does the `.qrt` route sit on the map?
//!
//! QuickRoute records, for every handle, both its pixel position and the route
//! point it belongs to, so the recovered route must pass exactly through every
//! marker. This prints the worst offenders.
//!
//! Usage: `cargo run -p tracereel-core --example qrt_geo -- <file.qrt>`

use tracereel_core::qrt::read_project;

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args
        .next()
        .unwrap_or_else(|| "demo/AM 男子短距 第一面.qrt".to_string());
    let proj = match read_project(&path) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("read {path}: {e}");
            std::process::exit(1);
        }
    };
    let map = proj.map.as_ref().expect("embedded map");
    let (w, h) = (map.width as f64, map.height as f64);
    let (cx, cy) = (w / 2.0, h / 2.0);
    let pos = proj.map_positions();
    println!(
        "map {w}x{h}  waypoints {}  positions {}  handles {}  origin {:?}",
        proj.waypoints.len(),
        pos.len(),
        proj.handles.len(),
        proj.projection_origin
    );
    if pos.is_empty() {
        println!("no positions (missing projection origin or map)");
        return;
    }

    let xs: Vec<f64> = pos.iter().map(|p| p.0).collect();
    let ys: Vec<f64> = pos.iter().map(|p| p.1).collect();
    let (min_x, max_x) = (
        xs.iter().cloned().fold(f64::MAX, f64::min),
        xs.iter().cloned().fold(f64::MIN, f64::max),
    );
    let (min_y, max_y) = (
        ys.iter().cloned().fold(f64::MAX, f64::min),
        ys.iter().cloned().fold(f64::MIN, f64::max),
    );
    println!("route bbox (centred px): x {min_x:.1}..{max_x:.1}  y {min_y:.1}..{max_y:.1}");
    println!("map bbox   (centred px): x {:.1}..{:.1}  y {:.1}..{:.1}", -cx, cx, -cy, cy);

    let handles = proj.ordered_handles();
    let projected = proj.projected_positions();
    let transforms = proj.handle_transforms(&projected);
    let indices = proj.transform_indices(projected.len());
    println!(
        "handles {} transforms {} indices {}",
        handles.len(),
        transforms.len(),
        indices.len()
    );

    // Pair invariant: transform m must map handle m *and* handle m+1 onto their
    // recorded pixel positions. This is `handle_transforms`' own contract.
    let mut pair_rows: Vec<(f64, f64, f64, f64)> = Vec::new();
    for m in 0..handles.len().saturating_sub(1) {
        let Some(t) = transforms.get(m) else { continue };
        let a = &handles[m];
        let b = &handles[m + 1];
        let qa = t.apply(tracereel_core::qrt::project_at(&projected, a.value));
        let qb = t.apply(tracereel_core::qrt::project_at(&projected, b.value));
        let da = ((qa.0 - a.x).powi(2) + (qa.1 - a.y).powi(2)).sqrt();
        let db = ((qb.0 - b.x).powi(2) + (qb.1 - b.y).powi(2)).sqrt();
        pair_rows.push((da.max(db), a.value, da, db));
    }
    pair_rows.sort_by(|x, y| y.0.partial_cmp(&x.0).unwrap_or(std::cmp::Ordering::Equal));
    println!(
        "pair invariant: worst {:.3} px, {} of {} pairs over 1 px",
        pair_rows.first().map(|r| r.0).unwrap_or(0.0),
        pair_rows.iter().filter(|r| r.0 > 1.0).count(),
        pair_rows.len()
    );
    for (w, v, da, db) in pair_rows.iter().take(8) {
        println!("  worst {w:9.3} px from handle {v:9.3}  (self {da:.3}, next {db:.3})");
    }

    if std::env::var_os("QRT_GEO_TRACE").is_some() {
        let want: usize = std::env::var("QRT_GEO_TRACE")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(119);
        for i in want.saturating_sub(3)..=(want + 3).min(indices.len() - 1) {
            let hv = handles.get(indices[i]).map(|h| h.value).unwrap_or(-1.0);
            println!(
                "  route point {i}: transform {} (handle at route point {hv:.3})",
                indices[i]
            );
        }
    }

    // The marker invariant: QuickRoute derives each handle's matrix so that it
    // maps that handle's own route point onto its recorded pixel position, and
    // the reader applies the *preceding* handle's matrix there.
    let mut rows: Vec<(f64, f64, f64, f64)> = Vec::new();
    for h in &handles {
        let node = h.value.floor();
        if node < 0.0 || node >= projected.len() as f64 {
            continue;
        }
        let Some(&ti) = indices.get(node as usize) else {
            continue;
        };
        let Some(t) = transforms.get(ti) else { continue };
        let p = tracereel_core::qrt::project_at(&projected, h.value);
        let q = t.apply(p); // raw map pixels, origin top-left
        let d = ((q.0 - h.x).powi(2) + (q.1 - h.y).powi(2)).sqrt();
        rows.push((d, h.value, q.0 - h.x, q.1 - h.y));
    }
    rows.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    let worst = rows.first().map(|r| r.0).unwrap_or(0.0);
    let over1 = rows.iter().filter(|r| r.0 > 1.0).count();
    println!(
        "handle-marker error: worst {worst:.2} px, {} of {} over 1 px",
        over1,
        rows.len()
    );
    for (d, v, dx, dy) in rows.iter().take(12) {
        println!("  err {d:9.2} px  routePoint {v:9.3}  d=({dx:8.2},{dy:8.2})");
    }
    if args.next().as_deref() == Some("--handles") {
        for h in &handles {
            println!("H {:.9}\t{:.9}\t{:.9}", h.value, h.x, h.y);
        }
    }
    if std::env::var_os("QRT_GEO_WAYPOINTS").is_some() {
        for (i, w) in proj.waypoints.iter().enumerate() {
            println!(
                "W {i}\t{}\t{:.9}\t{:.9}\t{}",
                w.index, w.longitude, w.latitude, w.time_ticks
            );
        }
    }
}
