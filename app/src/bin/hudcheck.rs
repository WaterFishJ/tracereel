//! Offscreen HUD visual regression and repeatable rendering benchmark.
use tracereel::{gui, hud, qtwrap, settings::Settings};
use tracereel_core::{
    gpx::{GpxPoint, GpxTrack},
    route_manager::RouteManager,
};
use qmetaobject::QmlEngine;

fn main() {
    qtwrap::asserts_silent_continue();
    let uri = std::ffi::CString::new("TraceReel").unwrap();
    qmetaobject::qml_register_type::<tracereel::receipt_ocr::ReceiptOcr>(
        &uri, 1, 0, &std::ffi::CString::new("ReceiptOcr").unwrap(),
    );
    qmetaobject::qml_register_type::<tracereel::gui::TraceReelItem>(
        &uri,
        1,
        0,
        &std::ffi::CString::new("TraceReelItem").unwrap(),
    );
    qmetaobject::qml_register_type::<tracereel::settings_proxy::SettingsProxy>(
        &uri,
        1,
        0,
        &std::ffi::CString::new("SettingsProxy").unwrap(),
    );
    let mut engine = QmlEngine::new();
    let dir = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "target/hud-check".into());
    std::fs::create_dir_all(&dir).unwrap();
    let track = GpxTrack::from_points(
        (0..601)
            .map(|i| GpxPoint {
                t: 1000.0 + i as f64,
                lat: 23.0 + i as f64 * 0.00002,
                lon: 113.0 + (i as f64 / 120.0).sin() * 0.0005,
                ele: 80.0 + (i as f64 / 65.0).sin() * 25.0 + i as f64 / 15.0,
                hr: Some(145.0 + (i as f64 / 30.0).sin() * 18.0),
            })
            .collect(),
    )
    .unwrap();
    let mut s = Settings::default();
    s.video.start_time_offset_s = 40.0;
    s.splits.split_times = "120|150|210".into();
    s.splits.is_relative = true;
    let rm = RouteManager::default();
    // Pixel-check that overlays never paint over the map or video letterbox bars.
    s.map.image_file_path = "viewport-test-map".into();
    s.map.relative_width = 0.25;
    let video = qtwrap::qimage_from_rgba(640, 360, &[30, 48, 58, 255].repeat(640 * 360));
    for theme in ["campus", "park", "simple"] {
        let mut baseline = qtwrap::qimage_new_argb32(1920, 1080);
        s.hud.layout = Some(hud::Layout {
            version: 1,
            widgets: vec![],
        });
        {
            let mut p = qtwrap::qpainter_on_image(&mut baseline);
            gui::draw_scene(
                &mut p,
                &rm,
                &s,
                None,
                Some(video.clone()),
                1920.0,
                1080.0,
                187_800.0,
                0.0,
                Some(&track),
            );
        }
        let mut rendered = qtwrap::qimage_new_argb32(1920, 1080);
        s.hud.layout = Some(hud::Layout::theme(theme));
        {
            let mut p = qtwrap::qpainter_on_image(&mut rendered);
            gui::draw_scene(
                &mut p,
                &rm,
                &s,
                None,
                Some(video.clone()),
                1920.0,
                1080.0,
                187_800.0,
                0.0,
                Some(&track),
            );
        }
        let before = qtwrap::qimage_to_rgba(&baseline);
        let after = qtwrap::qimage_to_rgba(&rendered);
        let mut changed = 0;
        for y in 0..1080 {
            for x in 0..1920 {
                let i = (y * 1920 + x) * 4;
                if x < 480 || y < 135 || y >= 945 {
                    assert_eq!(&before[i..i + 4], &after[i..i + 4]);
                } else if before[i..i + 4] != after[i..i + 4] {
                    changed += 1;
                }
            }
        }
        assert!(changed > 0);
        assert!(qtwrap::qimage_save(
            &rendered,
            &format!("{dir}/{theme}-video-bounds.png")
        ));
    }
    s.map.image_file_path.clear();
    for theme in ["campus", "park", "simple"] {
        s.hud.layout = Some(hud::Layout::theme(theme));
        for (width, height) in [(1920, 1080), (3840, 2160), (1080, 1920)] {
            let mut img = qtwrap::qimage_new_argb32(width, height);
            {
                let mut p = qtwrap::qpainter_on_image(&mut img);
                gui::draw_scene(
                    &mut p,
                    &rm,
                    &s,
                    None,
                    None,
                    width as f64,
                    height as f64,
                    187_800.0,
                    0.0,
                    Some(&track),
                );
            }
            assert!(qtwrap::qimage_save(
                &img,
                &format!("{dir}/{theme}-{width}x{height}.png")
            ));
        }
    }
    for (name, layout, available) in [
        ("missing", Some(hud::Layout::theme("modern")), false),
        ("legacy", None, true),
        (
            "empty",
            Some(hud::Layout {
                version: 1,
                widgets: vec![],
            }),
            true,
        ),
    ] {
        s.hud.layout = layout;
        let mut img = qtwrap::qimage_new_argb32(1920, 1080);
        {
            let mut p = qtwrap::qpainter_on_image(&mut img);
            gui::draw_scene(
                &mut p,
                &rm,
                &s,
                None,
                None,
                1920.0,
                1080.0,
                187_800.0,
                0.0,
                if available { Some(&track) } else { None },
            );
        }
        assert!(qtwrap::qimage_save(&img, &format!("{dir}/{name}.png")));
    }
    // Compare deterministic re-render after seeking away to another time.
    s.hud.layout = Some(hud::Layout::theme("modern"));
    for (i, time) in [187_800.0, 250_000.0, 187_800.0].iter().enumerate() {
        let mut img = qtwrap::qimage_new_argb32(1920, 1080);
        {
            let mut p = qtwrap::qpainter_on_image(&mut img);
            gui::draw_scene(
                &mut p,
                &rm,
                &s,
                None,
                None,
                1920.0,
                1080.0,
                *time,
                0.0,
                Some(&track),
            );
        }
        qtwrap::qimage_save(&img, &format!("{dir}/seek-{i}.png"));
    }
    assert_eq!(
        std::fs::read(format!("{dir}/seek-0.png")).unwrap(),
        std::fs::read(format!("{dir}/seek-2.png")).unwrap()
    );
    // Render actual animation frames through the same painter as preview/export.
    let mut animated = hud::Layout::theme("outdoor");
    animated
        .widgets
        .retain(|w| w.metric == "heading" || w.metric == "heart_rate");
    for (i, w) in animated.widgets.iter_mut().enumerate() {
        w.x = 0.05 + i as f64 * 0.48;
        w.y = 0.08;
        w.w = 0.42;
        w.h = 0.84;
        w.font_size = 64.0;
    }
    s.hud.layout = Some(animated);
    let turning_track = GpxTrack::from_points(
        (0..=61)
            .map(|i| {
                let angle = i as f64 * std::f64::consts::TAU / 60.0;
                GpxPoint {
                    t: 1000.0 + i as f64 * 0.1,
                    lat: 23.0 + angle.cos() * 0.0001,
                    lon: 113.0 + angle.sin() * 0.0001,
                    ele: 100.0,
                    hr: Some(90.0 + i as f64 * 1.2),
                }
            })
            .collect(),
    )
    .unwrap();
    for frame in 0..60 {
        let mut img = qtwrap::qimage_new_argb32(800, 450);
        {
            let mut p = qtwrap::qpainter_on_image(&mut img);
            hud::draw(
                &mut p,
                &s,
                Some(&turning_track),
                100.0 + frame as f64 * 100.0,
                800.0,
                450.0,
            );
        }
        assert!(qtwrap::qimage_save(
            &img,
            &format!("{dir}/animation-{frame:03}.png")
        ));
    }
    assert_ne!(
        std::fs::read(format!("{dir}/animation-000.png")).unwrap(),
        std::fs::read(format!("{dir}/animation-001.png")).unwrap()
    );
    s.hud.layout = Some(hud::Layout::theme("modern"));
    // Same activity instant through the map-only route clock and the video clock.
    let video_time = 187_800.0;
    s.route.control_time_offset = 3.0;
    let route_time = video_time - 40_000.0 + 3_000.0;
    s.hud.gpx_time_offset += s.video.start_time_offset_s - s.route.control_time_offset;
    s.video.start_time_offset_s = s.route.control_time_offset;
    let mut img = qtwrap::qimage_new_argb32(1920, 1080);
    {
        let mut p = qtwrap::qpainter_on_image(&mut img);
        gui::draw_scene(
            &mut p,
            &rm,
            &s,
            None,
            None,
            1920.0,
            1080.0,
            route_time,
            0.0,
            Some(&track),
        );
    }
    qtwrap::qimage_save(&img, &format!("{dir}/route-clock.png"));
    assert_eq!(
        std::fs::read(format!("{dir}/seek-0.png")).unwrap(),
        std::fs::read(format!("{dir}/route-clock.png")).unwrap()
    );
    for (label, layout) in [
        (
            "empty",
            Some(hud::Layout {
                version: 1,
                widgets: vec![],
            }),
        ),
        ("legacy", None),
        ("modern", Some(hud::Layout::theme("modern"))),
    ] {
        s.hud.layout = layout;
        let start = std::time::Instant::now();
        for frame in 0..120 {
            let mut img = qtwrap::qimage_new_argb32(1920, 1080);
            let mut p = qtwrap::qpainter_on_image(&mut img);
            gui::draw_scene(
                &mut p,
                &rm,
                &s,
                None,
                None,
                1920.0,
                1080.0,
                150_000.0 + frame as f64 * 1000.0 / 30.0,
                0.0,
                Some(&track),
            );
        }
        println!(
            "{label}: {:.3} ms/frame",
            start.elapsed().as_secs_f64() * 1000.0 / 120.0
        );
    }
    println!("HUD snapshots and deterministic time tests passed: {dir}");
    if std::env::args().any(|a| a == "--render-only") {
        return;
    }
    engine.set_property("orvPath".into(), qmetaobject::QString::from("").into());
    engine.load_file(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src/bin/gui.qml")
            .to_string_lossy()
            .as_ref()
            .into(),
    );
    let result = engine.invoke_method("fmtTime".into(), &[0.into()]);
    assert!(
        !result.to_qstring().to_string().is_empty(),
        "Main GUI failed to load"
    );
    for _ in 0..40 {
        qtwrap::process_events();
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    println!(
        "Main GUI load passed; screenshot: {}",
        qtwrap::save_first_window(&format!("{dir}/gui.png"))
    );
}
