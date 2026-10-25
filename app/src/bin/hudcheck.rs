//! Offscreen HUD visual regression and repeatable rendering benchmark.
use std::time::Instant;
use tracereel::{
    composition, gui, hud, qtwrap,
    settings::{MapBox, MapBoxStyle, MapLayout, Settings, MAP_LAYOUT_VERSION},
};
use tracereel_core::{
    gpx::{GpxPoint, GpxTrack},
    route::RoutePoint,
    route_manager::{RenderMode, RouteManager, RouteStyle, ViewMode},
};
use qmetaobject::{QImage, QmlEngine};

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
    check_map_placement(&dir);
    check_map_appearance(&dir);
    check_multi_box_pixels(&dir);
    check_multi_box_cameras(&dir);
    check_multi_box_styles(&dir);
    check_single_box_equivalence(&dir);
    check_multi_box_perf(&dir);
    let track = GpxTrack::from_points(
        (0..601)
            .map(|i| GpxPoint {
                t: 1000.0 + i as f64,
                lat: 23.0 + i as f64 * 0.00002,
                lon: 113.0 + (i as f64 / 120.0).sin() * 0.0005,
                ele: 80.0 + (i as f64 / 65.0).sin() * 25.0 + i as f64 / 15.0,
                hr: Some(145.0 + (i as f64 / 30.0).sin() * 18.0),
                cadence: Some(115.0),
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
                    cadence: None,
                }
            })
            .collect(),
    )
    .unwrap();
    // 帧时刻按**视频时钟**给（预览/导出就是这么调 `hud::draw` 的）：合成轨迹从 t=1000 s 起步、
    // 「录像-出发时间」40 s，HUD 偏移 = GPX 偏移(0) − 40，所以轨迹 t=1000 对应视频 40 s。
    // 从 40.1 s 起步、每帧 100 ms —— 轨迹是 10 Hz，正好每帧前进一个采样点，60 帧都在范围内。
    for frame in 0..60 {
        let mut img = qtwrap::qimage_new_argb32(800, 450);
        {
            let mut p = qtwrap::qpainter_on_image(&mut img);
            hud::draw(
                &mut p,
                &s,
                Some(&turning_track),
                40_100.0 + frame as f64 * 100.0,
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
    // 路线时钟顶替视频时钟（地图导出的做法，见 `encode.rs`）：帧时刻就是轨迹时刻，把
    // 「录像-出发时间」设成「按表-出发时间」，HUD 偏移 = GPX 偏移(0) − 3 = −3，于是正好
    // 读 `route_time` 那一刻的轨迹；elapsed 两边都落在 147.8 s。
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

/// Exercise the shared preview/export painter, including rotated clipping and layer exclusion.
fn check_map_placement(dir: &str) {
    let mut s = Settings::default();
    s.map.image_file_path = "synthetic-map".into();
    s.hud.layout = Some(hud::Layout { version: 1, widgets: vec![] });
    s.map.frame_x = 0.2; s.map.frame_y = 0.2;
    s.map.frame_width = 0.35; s.map.frame_height = 0.55;
    let map = qtwrap::qimage_from_rgba(1000, 1000, &[220, 60, 40, 255].repeat(1000 * 1000));
    let video = qtwrap::qimage_from_rgba(320, 180, &[30, 90, 200, 255].repeat(320 * 180));
    for (mode, angle, has_map, has_video) in [
        (0, 0., true, true), (1, 0., true, true), (4, 0., true, true), (4, 33., true, true),
        (4, -45., true, false), (0, 0., true, false), (4, 33., false, true),
    ] {
        s.map.placement = mode; s.map.frame_rotation = angle;
        s.encode.include_map = has_map; s.encode.include_video = has_video;
        let mut img = qtwrap::qimage_new_argb32(320, 180);
        {
            let mut p = qtwrap::qpainter_on_image(&mut img);
            gui::draw_scene(&mut p, &RouteManager::default(), &s,
                Some(map.clone()), if has_video { Some(video.clone()) } else { None },
                320., 180., 0., 0., None);
        }
        let panels = gui::scene_panels(&s, 320., 180.);
        let r = panels.map;
        let a = panels.rotation.to_radians();
        let rgba = qtwrap::qimage_to_rgba(&img);
        let mut map_pixels = 0;
        for y in 0..180 {
            for x in 0..320 {
                let dx = x as f64 + 0.5 - r.x - r.width / 2.;
                let dy = y as f64 + 0.5 - r.y - r.height / 2.;
                let lx = (dx * a.cos() + dy * a.sin()).abs();
                let ly = (-dx * a.sin() + dy * a.cos()).abs();
                let i = (y * 320 + x) * 4;
                let red = rgba[i] > 200 && rgba[i+1] < 70;
                if has_map && lx < r.width / 2. - 1. && ly < r.height / 2. - 1. {
                    assert!(red, "map missing: mode={mode}, angle={angle}, pixel=({x},{y})");
                    map_pixels += 1;
                }
                if !has_map || lx > r.width / 2. + 1. || ly > r.height / 2. + 1. {
                    assert!(!red, "map escaped clip: mode={mode}, angle={angle}, pixel=({x},{y})");
                }
            }
        }
        if has_map { assert!(map_pixels > 1000); }
        assert!(qtwrap::qimage_save(&img, &format!("{dir}/map-{mode}-{angle}-{has_map}-{has_video}.png")));
    }
    println!("map placement pixel checks passed");
}

/// Validate alpha compositing, circular/rounded cropping, and a smooth inward feather.
fn check_map_appearance(dir: &str) {
    let mut s = Settings::default();
    s.map.image_file_path = "synthetic-map".into();
    s.hud.layout = Some(hud::Layout { version: 1, widgets: vec![] });
    s.map.placement = 4;
    s.map.frame_x = 0.1; s.map.frame_y = 0.1;
    s.map.frame_width = 0.6; s.map.frame_height = 0.75;
    let map = qtwrap::qimage_from_rgba(1000, 1000, &[220, 60, 40, 255].repeat(1000 * 1000));
    let video = qtwrap::qimage_from_rgba(400, 240, &[30, 90, 200, 255].repeat(400 * 240));
    let render = |shape, opacity, feather, corner, rotation, name: &str| {
        let mut settings = s.clone();
        settings.map.frame_shape = shape; settings.map.frame_opacity = opacity;
        settings.map.frame_feather = feather; settings.map.frame_corner = corner;
        settings.map.frame_rotation = rotation;
        let mut image = qtwrap::qimage_new_argb32(400, 240);
        {
            let mut p = qtwrap::qpainter_on_image(&mut image);
            gui::draw_scene(&mut p, &RouteManager::default(), &settings,
                Some(map.clone()), Some(video.clone()), 400., 240., 0., 0., None);
        }
        assert!(qtwrap::qimage_save(&image, &format!("{dir}/map-style-{name}.png")));
        qtwrap::qimage_to_rgba(&image)
    };
    let rgb = |pixels: &[u8], x: usize, y: usize| -> [u8; 3] {
        let i = (y * 400 + x) * 4;
        [pixels[i], pixels[i+1], pixels[i+2]]
    };
    let near = |actual: [u8; 3], expected: [u8; 3]| {
        for i in 0..3 { assert!((actual[i] as i32 - expected[i] as i32).abs() <= 3,
            "pixel {actual:?}, expected {expected:?}"); }
    };
    let hidden = render(0, 0., 0., 0., 0., "transparent");
    near(rgb(&hidden, 160, 114), [30, 90, 200]);
    let half = render(0, 0.5, 0., 0., 0., "half-opacity");
    near(rgb(&half, 160, 114), [125, 75, 120]);
    let circle = render(1, 1., 0., 0., 0., "circle");
    near(rgb(&circle, 160, 114), [220, 60, 40]);
    near(rgb(&circle, 52, 114), [30, 90, 200]); // Inscribed circle, not a stretched ellipse.
    near(rgb(&circle, 43, 27), [30, 90, 200]);
    let rounded = render(0, 1., 0., 0.25, 0., "rounded");
    near(rgb(&rounded, 43, 27), [30, 90, 200]);
    near(rgb(&rounded, 160, 30), [220, 60, 40]);
    near(rgb(&rounded, 160, 114), [220, 60, 40]);
    let feather = render(0, 1., 0.1, 0., 0., "feather");
    let edge = rgb(&feather, 42, 114)[0];
    let middle = rgb(&feather, 49, 114)[0];
    let inside = rgb(&feather, 62, 114)[0];
    assert!(edge > 30 && edge < middle && middle < inside);
    near(rgb(&feather, 62, 114), [220, 60, 40]);
    near(rgb(&feather, 38, 114), [30, 90, 200]);
    for shape in [0, 1] {
        let combined = render(shape, 0.5, 0.12, 0.25, 31., &format!("combined-{shape}"));
        near(rgb(&combined, 160, 114), [125, 75, 120]);
        near(rgb(&combined, 5, 5), [30, 90, 200]);
    }
    for mode in [0, 1] {
        let mut outputs = Vec::new();
        for styled in [false, true] {
            let mut settings = s.clone();
            settings.map.placement = mode;
            if styled {
                settings.map.frame_opacity = 0.0;
                settings.map.frame_shape = 1;
                settings.map.frame_feather = 0.2;
                settings.map.frame_corner = 0.3;
            }
            let mut image = qtwrap::qimage_new_argb32(400, 240);
            {
                let mut p = qtwrap::qpainter_on_image(&mut image);
                gui::draw_scene(&mut p, &RouteManager::default(), &settings,
                    Some(map.clone()), Some(video.clone()), 400., 240., 0., 0., None);
            }
            outputs.push(qtwrap::qimage_to_rgba(&image));
        }
        assert_eq!(outputs[0], outputs[1], "docked maps must ignore free-frame appearance");
    }
    println!("map opacity, circle, rounded corners and feather pixel checks passed");
}

// ─────────────────────────────────────────────────────────────────────────────
// 多地图框（逐框取景）回归：像素 / 相机 / 单框等价性 / 性能烟测
//
// 说明：`hudcheck` 是**独立的 bin crate**，只能调 `tracereel` 的 pub API；
// gui.rs 里 `pub(crate)` 的 effective_placements / build_frame_managers 等在本文件里
// 用等价的公开入口重建：
//   · 框列表 = `settings.map.effective_layout(&settings.route).boxes`（与 preview/export 同源）
//   · 版面矩形 = `tracereel::composition::scene_layout(...)`（draw_scene_layers 内部同一个函数）
//   · 逐框取景器 = clone 一份 RouteManager 后手工 configure（与 build_frame_managers 同语义）
// ─────────────────────────────────────────────────────────────────────────────

/// 内存里造一个多框工程：`boxes` 写进 `layout`，box0 镜像回旧字段
/// （与 QML 编辑、`.trr` 读档走同一条 `sync_legacy_from_layout` 路径）。
fn multi_box_settings(boxes: Vec<MapBox>) -> Settings {
    let mut s = Settings::default();
    s.map.image_file_path = "multi-box-synthetic".into();
    s.map.layout = MapLayout {
        version: MAP_LAYOUT_VERSION,
        boxes,
    };
    s.map.sync_legacy_from_layout(&mut s.route);
    s.hud.layout = Some(hud::Layout {
        version: 1,
        widgets: Vec::new(),
    });
    s
}

fn free_box(x: f64, y: f64, w: f64, h: f64) -> MapBox {
    MapBox {
        mode: 4,
        share: 0.25,
        x,
        y,
        w,
        h,
        rotation: 0.0,
        opacity: 1.0,
        shape: 0,
        feather: 0.0,
        corner: 0.0,
        view_mode: 0,
        view_zoom: 1.0,
        lr_margin: 10.0,
        tb_margin: 30.0,
        sensitivity: 0.5,
        style: None,
    }
}

fn docked_left_box(share: f64) -> MapBox {
    MapBox {
        mode: 0,
        share,
        ..free_box(0.03, 0.05, 0.3, 0.6)
    }
}

/// 当前有效框的版面（与 `draw_scene_layers` 内部同一个来源）。
fn current_placements(s: &Settings) -> Vec<composition::Placement> {
    s.map
        .effective_layout(&s.route)
        .boxes
        .iter()
        .map(|b| b.placement())
        .collect()
}

/// 与 `draw_scene_layers` 内部完全相同的版面计算结果（矩形 + 旋转）。
fn current_layout(s: &Settings, w: f64, h: f64) -> Vec<composition::SceneBox> {
    composition::scene_layout(
        &current_placements(s),
        w,
        h,
        gui::map_configured(s) && s.encode.include_map,
        s.encode.include_video,
    )
    .boxes
}

/// 每个框一层：第 i 层用第 i 个取景器（不够时退回第 0 个 / 跳过）。
fn layers_for<'a>(
    s: &Settings,
    rms: &'a [RouteManager],
    map: &'a QImage,
) -> Vec<gui::SceneLayer<'a>> {
    s.map
        .effective_layout(&s.route)
        .boxes
        .iter()
        .enumerate()
        .filter_map(|(i, b)| {
            let rm = rms.get(i).or_else(|| rms.first())?;
            Some(gui::SceneLayer {
                rm,
                map,
                placement: b.placement(),
                route_start_ms: 0.0,
            })
        })
        .collect()
}

/// 画进一张复用画布（性能烟测用，避免每帧新建 QImage）。
fn render_scene_into(
    s: &Settings,
    rms: &[RouteManager],
    map: &QImage,
    video: Option<&QImage>,
    img: &mut QImage,
    w: i32,
    h: i32,
    t_ms: f64,
) {
    let mut p = qtwrap::qpainter_on_image(img);
    let layers = layers_for(s, rms, map);
    gui::draw_scene_layers(&mut p, s, video, &layers, w as f64, h as f64, t_ms, None);
}

fn render_scene_image(
    s: &Settings,
    rms: &[RouteManager],
    map: &QImage,
    video: Option<&QImage>,
    w: i32,
    h: i32,
    t_ms: f64,
) -> QImage {
    let mut img = qtwrap::qimage_new_argb32(w, h);
    render_scene_into(s, rms, map, video, &mut img, w, h, t_ms);
    img
}

fn pixel(rgba: &[u8], w: i32, x: i32, y: i32) -> [u8; 3] {
    let i = ((y as usize) * (w as usize) + x as usize) * 4;
    [rgba[i], rgba[i + 1], rgba[i + 2]]
}

fn is_map_red(p: [u8; 3]) -> bool {
    p[0] > 200 && p[1] < 70 && p[2] < 70
}

fn is_video_blue(p: [u8; 3]) -> bool {
    (p[0] as i32 - 30).abs() <= 8 && (p[1] as i32 - 90).abs() <= 8 && (p[2] as i32 - 200).abs() <= 8
}

fn near_rgb(a: [u8; 3], e: [u8; 3], tol: i32) -> bool {
    (0..3).all(|i| (a[i] as i32 - e[i] as i32).abs() <= tol)
}

/// 点是否落在（绕中心旋转的）框内；`inset` = 向内收缩像素（负数 = 向外扩）。
fn inside_box(px: f64, py: f64, r: composition::Rect, rotation: f64, inset: f64) -> bool {
    let a = rotation.to_radians();
    let dx = px - (r.x + r.width / 2.0);
    let dy = py - (r.y + r.height / 2.0);
    let lx = (dx * a.cos() + dy * a.sin()).abs();
    let ly = (-dx * a.sin() + dy * a.cos()).abs();
    lx <= (r.width / 2.0 - inset).max(0.0) && ly <= (r.height / 2.0 - inset).max(0.0)
}

/// 逐框断言：每个框内（内缩 2px）必须是地图红，所有框之外不得出现地图红。
fn assert_boxes_paint_map(
    rgba: &[u8],
    w: i32,
    h: i32,
    boxes: &[composition::SceneBox],
    tag: &str,
) {
    for (i, b) in boxes.iter().enumerate() {
        let cx = (b.rect.x + b.rect.width / 2.0) as i32;
        let cy = (b.rect.y + b.rect.height / 2.0) as i32;
        let center = pixel(rgba, w, cx.clamp(0, w - 1), cy.clamp(0, h - 1));
        assert!(
            is_map_red(center),
            "{tag}: 第 {i} 框中心 ({cx},{cy}) 不是地图红，实际 {center:?}"
        );
    }
    for y in 0..h {
        for x in 0..w {
            let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
            let inside = boxes
                .iter()
                .any(|b| inside_box(px, py, b.rect, b.rotation, 2.0));
            let outside = !boxes
                .iter()
                .any(|b| inside_box(px, py, b.rect, b.rotation, -2.0));
            let p = pixel(rgba, w, x, y);
            if inside {
                assert!(
                    is_map_red(p),
                    "{tag}: 框内 ({x},{y}) 不是地图红，实际 {p:?}"
                );
            } else if outside {
                assert!(
                    !is_map_red(p),
                    "{tag}: 框外 ({x},{y}) 出现地图红 {p:?}（框没有正确裁剪/越界）"
                );
            }
        }
    }
}

/// 多地图框像素回归：贴边 + 自由（含旋转）/ 3 框 / 圆形 / 不透明度 / 纯地图导出。
fn check_multi_box_pixels(dir: &str) {
    const W: i32 = 400;
    const H: i32 = 240;
    let map = qtwrap::qimage_from_rgba(1000, 1000, &[220, 60, 40, 255].repeat(1000 * 1000));
    let video = qtwrap::qimage_from_rgba(400, 240, &[30, 90, 200, 255].repeat(400 * 240));

    // ① 2 框：第 1 框贴左边（视频区让出条带），第 2 框自由摆放 + 旋转 30°。
    let mut s = multi_box_settings(vec![
        docked_left_box(0.25),
        MapBox {
            rotation: 30.0,
            ..free_box(0.6, 0.55, 0.3, 0.3)
        },
    ]);
    s.encode.include_map = true;
    s.encode.include_video = true;
    let boxes = current_layout(&s, W as f64, H as f64);
    assert_eq!(boxes.len(), 2, "2 框工程应给出 2 个框");
    let img = render_scene_image(
        &s,
        &[RouteManager::default()],
        &map,
        Some(&video),
        W,
        H,
        0.0,
    );
    let rgba = qtwrap::qimage_to_rgba(&img);
    assert_boxes_paint_map(&rgba, W, H, &boxes, "2框");
    let blue = pixel(&rgba, W, 350, H / 2);
    assert!(is_video_blue(blue), "2框: 视频区应为蓝，实际 {blue:?}");
    assert!(qtwrap::qimage_save(&img, &format!("{dir}/multi-2box.png")));
    println!("multi-box pixels ok: 2 frames (docked left + free rotated 30°)");

    // ② 3 框：1 贴边 + 2 自由（其中一个旋转 -35°），互不重叠。
    let mut s3 = multi_box_settings(vec![
        docked_left_box(0.2),
        MapBox {
            rotation: 18.0,
            ..free_box(0.3, 0.06, 0.26, 0.34)
        },
        MapBox {
            rotation: -35.0,
            ..free_box(0.62, 0.52, 0.3, 0.4)
        },
    ]);
    s3.encode.include_map = true;
    s3.encode.include_video = true;
    let boxes3 = current_layout(&s3, W as f64, H as f64);
    assert_eq!(boxes3.len(), 3, "3 框工程应给出 3 个框");
    let img3 = render_scene_image(
        &s3,
        &[RouteManager::default()],
        &map,
        Some(&video),
        W,
        H,
        0.0,
    );
    assert_boxes_paint_map(&qtwrap::qimage_to_rgba(&img3), W, H, &boxes3, "3框");
    assert!(qtwrap::qimage_save(&img3, &format!("{dir}/multi-3box.png")));
    println!("multi-box pixels ok: 3 frames (docked + 2 free, rotated)");

    // ③ 圆形：两个自由框都是内切圆 → 圆内红、圆外（矩形四角）不红。
    let mut sc = multi_box_settings(vec![
        MapBox {
            shape: 1,
            ..free_box(0.05, 0.1, 0.4, 0.4)
        },
        MapBox {
            shape: 1,
            ..free_box(0.55, 0.1, 0.4, 0.4)
        },
    ]);
    sc.encode.include_map = true;
    sc.encode.include_video = true;
    let boxes_c = current_layout(&sc, W as f64, H as f64);
    let img_c = render_scene_image(
        &sc,
        &[RouteManager::default()],
        &map,
        Some(&video),
        W,
        H,
        0.0,
    );
    let rgba_c = qtwrap::qimage_to_rgba(&img_c);
    for (i, b) in boxes_c.iter().enumerate() {
        let r = b.rect;
        let side = r.width.min(r.height);
        let (cx, cy) = (r.x + r.width / 2.0, r.y + r.height / 2.0);
        let center = pixel(&rgba_c, W, cx as i32, cy as i32);
        assert!(is_map_red(center), "圆形第 {i} 框中心不是地图红，实际 {center:?}");
        for k in 0..8 {
            let ang = k as f64 * std::f64::consts::PI / 4.0;
            let ix = (cx + ang.cos() * side * 0.4) as i32;
            let iy = (cy + ang.sin() * side * 0.4) as i32;
            let p = pixel(&rgba_c, W, ix, iy);
            assert!(
                is_map_red(p),
                "圆形第 {i} 框内切圆内 ({ix},{iy}) 不是地图红，实际 {p:?}"
            );
            let ox = (cx + ang.cos() * side * 0.62) as i32;
            let oy = (cy + ang.sin() * side * 0.62) as i32;
            if ox >= 0 && oy >= 0 && ox < W && oy < H {
                let q = pixel(&rgba_c, W, ox, oy);
                assert!(
                    !is_map_red(q),
                    "圆形第 {i} 框圆外 ({ox},{oy}) 仍出现地图红 {q:?}（内切圆裁剪失效）"
                );
            }
        }
    }
    assert!(qtwrap::qimage_save(&img_c, &format!("{dir}/multi-circle.png")));
    println!("multi-box pixels ok: 2 circular frames (inscribed circle)");

    // ④ 不透明度：两个 0.5 透明度的自由框叠在蓝视频上 → (125,75,120)。
    let mut sh = multi_box_settings(vec![
        MapBox {
            opacity: 0.5,
            ..free_box(0.55, 0.35, 0.3, 0.3)
        },
        MapBox {
            opacity: 0.5,
            ..free_box(0.05, 0.05, 0.2, 0.2)
        },
    ]);
    sh.encode.include_map = true;
    sh.encode.include_video = true;
    let boxes_h = current_layout(&sh, W as f64, H as f64);
    let img_h = render_scene_image(
        &sh,
        &[RouteManager::default()],
        &map,
        Some(&video),
        W,
        H,
        0.0,
    );
    let rgba_h = qtwrap::qimage_to_rgba(&img_h);
    for (i, b) in boxes_h.iter().enumerate() {
        let cx = (b.rect.x + b.rect.width / 2.0) as i32;
        let cy = (b.rect.y + b.rect.height / 2.0) as i32;
        let p = pixel(&rgba_h, W, cx, cy);
        assert!(
            near_rgb(p, [125, 75, 120], 4),
            "半透明第 {i} 框中心应为 (125,75,120)，实际 {p:?}"
        );
    }
    assert!(qtwrap::qimage_save(&img_h, &format!("{dir}/multi-opacity.png")));
    println!("multi-box pixels ok: per-frame opacity (0.5 over blue video)");

    // ⑤ map_only（无视频）：贴边的第 1 框铺满整幅，四角都应是地图红。
    let mut so = multi_box_settings(vec![
        docked_left_box(0.25),
        free_box(0.6, 0.5, 0.3, 0.3),
    ]);
    so.encode.include_map = true;
    so.encode.include_video = false;
    let img_o = render_scene_image(
        &so,
        &[RouteManager::default()],
        &map,
        None,
        W,
        H,
        0.0,
    );
    let rgba_o = qtwrap::qimage_to_rgba(&img_o);
    for (x, y) in [(2, 2), (W - 3, 2), (2, H - 3), (W - 3, H - 3)] {
        let p = pixel(&rgba_o, W, x, y);
        assert!(
            is_map_red(p),
            "map_only: 贴边第 1 框应铺满整幅，四角 ({x},{y}) 不是地图红 {p:?}"
        );
    }
    assert!(qtwrap::qimage_save(&img_o, &format!("{dir}/multi-map-only.png")));
    println!("multi-box pixels ok: map-only (docked box fills the frame)");
}

// ── 相机回归：一框固定分段、一框跟跑者居中 ──────────────────────────────────

fn configure_camera(
    rm: &mut RouteManager,
    view_mode: ViewMode,
    zoom: f64,
    frame: composition::Rect,
) {
    // 与 gui::build_frame_managers → configure_frame_camera 同语义（手写避免依赖 pub(crate)）。
    rm.set_view_mode(view_mode);
    rm.centered_zoom = zoom;
    rm.left_right_margin = 10.0;
    rm.top_bottom_margin = 30.0;
    rm.use_smooth_split_transition = false;
    rm.window_resized(frame.width.max(1.0), frame.height.max(1.0));
    rm.request_full_update();
}

fn camera_route_style() -> RouteStyle {
    let mut st = RouteStyle::default();
    st.route_render_mode = RenderMode::Discreet;
    st.route_color = (0.0, 1.0, 0.0, 1.0);
    st.route_width = 8.0;
    st.tail_render_mode = RenderMode::None;
    st.show_controls = false;
    st.show_runner = true;
    st
}

/// 1 Hz 轨迹：每段约 10s、每段约 100 个地图单位宽，便于分段取景算出合理缩放。
fn camera_points() -> Vec<RoutePoint> {
    (0..=60)
        .map(|i| {
            let mut p =
                RoutePoint::new(100.0 + i as f64 * 10.0, 300.0 + (i % 10) as f64 * 4.0, i as f64);
            p.pace = 5.0 + (i % 7) as f64;
            p
        })
        .collect()
}

/// 框内绿色像素数量与重心（画布绝对坐标）。
fn green_pixels_and_centroid(
    rgba: &[u8],
    w: i32,
    h: i32,
    r: composition::Rect,
) -> (usize, f64, f64) {
    let x0 = (r.x as i32).max(0);
    let y0 = (r.y as i32).max(0);
    let x1 = ((r.x + r.width) as i32).min(w);
    let y1 = ((r.y + r.height) as i32).min(h);
    let (mut n, mut sx, mut sy) = (0usize, 0.0f64, 0.0f64);
    for y in y0..y1 {
        for x in x0..x1 {
            let p = pixel(rgba, w, x, y);
            if p[1] > 140 && p[0] < 120 && p[2] < 120 {
                n += 1;
                sx += x as f64;
                sy += y as f64;
            }
        }
    }
    if n == 0 {
        (0, 0.0, 0.0)
    } else {
        (n, sx / n as f64, sy / n as f64)
    }
}

fn check_multi_box_cameras(dir: &str) {
    const W: f64 = 640.0;
    const H: f64 = 360.0;
    let mut s = multi_box_settings(vec![
        free_box(0.02, 0.08, 0.46, 0.84),
        free_box(0.52, 0.08, 0.46, 0.84),
    ]);
    s.encode.include_map = true;
    s.encode.include_video = false;
    let boxes = current_layout(&s, W, H);
    assert_eq!(boxes.len(), 2);
    let map = qtwrap::qimage_from_rgba(1000, 1000, &[60, 60, 60, 255].repeat(1000 * 1000));

    let style = camera_route_style();
    let mut base = RouteManager::default();
    // 隐式首段 0s + 10/20/40s → 三段取景表；采样 0/10/30s 分别落在第 0/1/2 段。
    base.initialize_from_points(
        camera_points(),
        "0:10|0:20|0:40",
        false,
        style,
        ViewMode::FixedSplit,
        1.0,
    );
    let mut fs_rm = base.clone();
    configure_camera(&mut fs_rm, ViewMode::FixedSplit, 1.0, boxes[0].rect);
    let mut rc_rm = base.clone();
    configure_camera(&mut rc_rm, ViewMode::RunnerCentered, 2.0, boxes[1].rect);

    let mut frames: Vec<Vec<u8>> = Vec::new();
    for t in [0.0_f64, 10.0, 30.0] {
        for rm in [&mut fs_rm, &mut rc_rm] {
            // 跳转语义：让取景瞬间到位，便于逐值比对（帧循环里由预热保证连续）。
            rm.instant_split_transition_requested = true;
            rm.update(t, 0.0);
        }
        // ① FixedSplit 框：scale/angle/位移必须来自分段取景表。
        let sts = fs_rm.routes[0].split_transformations.clone();
        assert!(!sts.is_empty(), "FixedSplit 框应有分段取景表");
        let (sc, an, fx, fy) = (
            fs_rm.get_scale(),
            fs_rm.get_angle(),
            fs_rm.get_x(),
            fs_rm.get_y(),
        );
        assert!(
            sts.iter().any(|st| (st.scale - sc).abs() < 1e-9),
            "t={t}: FixedSplit scale {sc} 不在分段取景表 {:?}",
            sts.iter().map(|v| v.scale).collect::<Vec<_>>()
        );
        assert!(
            sts.iter().any(|st| (st.angle - an).abs() < 1e-9),
            "t={t}: FixedSplit angle {an} 不在分段取景表"
        );
        assert!(
            sts.iter()
                .any(|st| (st.x - fx).abs() < 1e-9 && (st.y - fy).abs() < 1e-9),
            "t={t}: FixedSplit 位移 ({fx},{fy}) 不在分段取景表"
        );
        // ② RunnerCentered 框：缩放 == view_zoom=2，镜头 x/y 跟着跑者。
        assert!(
            (rc_rm.get_scale() - 2.0).abs() < 1e-9,
            "t={t}: 居中框 scale 应为 2.0，实际 {}",
            rc_rm.get_scale()
        );
        let runner = rc_rm.routes[0].runner_position;
        assert!(
            (rc_rm.get_x() + runner.0).abs() < 1e-6,
            "t={t}: 居中框 x 没跟着跑者（get_x={}, runner.x={}）",
            rc_rm.get_x(),
            runner.0
        );
        assert!(
            (rc_rm.get_y() - runner.1).abs() < 1e-6,
            "t={t}: 居中框 y 没跟着跑者（get_y={}, runner.y={}）",
            rc_rm.get_y(),
            runner.1
        );
        let img = render_scene_image(
            &s,
            &[fs_rm.clone(), rc_rm.clone()],
            &map,
            None,
            W as i32,
            H as i32,
            t * 1000.0,
        );
        assert!(qtwrap::qimage_save(
            &img,
            &format!("{dir}/multi-camera-{t:.0}s.png")
        ));
        frames.push(qtwrap::qimage_to_rgba(&img));
    }

    // ③ 两框都真的画出了轨迹，且内容不同、随时间变化（重心移动）。
    for (i, b) in boxes.iter().enumerate() {
        let (n0, x0, y0) = green_pixels_and_centroid(&frames[0], W as i32, H as i32, b.rect);
        let (n2, x2, y2) = green_pixels_and_centroid(&frames[2], W as i32, H as i32, b.rect);
        assert!(n0 > 20, "第 {i} 框 t=0 没有绿色轨迹（{n0} px）");
        assert!(n2 > 20, "第 {i} 框 t=30 没有绿色轨迹（{n2} px）");
        assert!(
            (x0 - x2).abs() > 1.0 || (y0 - y2).abs() > 1.0,
            "第 {i} 框的轨迹 t=0→30 没有移动（重心 {x0:.1},{y0:.1} → {x2:.1},{y2:.1}）"
        );
    }
    let (_, fxa, fya) = green_pixels_and_centroid(&frames[0], W as i32, H as i32, boxes[0].rect);
    let (_, rxa, rya) = green_pixels_and_centroid(&frames[0], W as i32, H as i32, boxes[1].rect);
    assert!(
        (fxa - rxa).abs() > 1.0 || (fya - rya).abs() > 1.0,
        "t=0 两框取景内容应不同（重心 {fxa:.1},{fya:.1} vs {rxa:.1},{rya:.1}）"
    );
    println!("multi-box camera checks passed (FixedSplit vs RunnerCentered at t=0/10/30s)");
}

// ── 逐框轨迹样式（路线/尾巴/控制点/运动员 + 渲染范围）─────────────────────────
//
// hudcheck 是独立 bin crate，看不到 gui.rs 里 `pub(crate)` 的 `effective_route_styles` /
// `build_frame_managers` / `apply_route_style`，这里用**公开 API** 等价重建：
//   · 每框样式 = `MapBoxStyle::from_route` 回填 + `gui::route_style_from_box`（pub）
//   · 每框取景器 = clone 基础 RM 后按框的取景字段 configure（与 build_frame_managers 同语义）
// encode.rs 的测试走真正生产的 `gui::build_frame_managers`，两边互为对照。

/// 第 i 框的有效样式（与 `MapSettings::effective_layout` 同规则：None 用全局回填）。
fn box_style_of(s: &Settings, i: usize) -> MapBoxStyle {
    let global = MapBoxStyle::from_route(&s.route);
    s.map
        .effective_layout(&s.route)
        .boxes
        .get(i)
        .and_then(|b| b.style.clone())
        .unwrap_or(global)
}

/// 等价 `gui::apply_route_style`（pub(crate)）：写样式 + 校准偏移 + 自适应配速端点。
fn apply_box_style(rm: &mut RouteManager, style: &RouteStyle) {
    for r in rm.routes.iter_mut() {
        r.style = style.clone();
        r.control_time_offset = style.control_time_offset;
        r.runner_time_offset = style.runner_time_offset;
    }
    for r in rm.routes.iter_mut() {
        if !r.style.pace_auto && !r.style.tail_pace_auto {
            continue;
        }
        if let Some((low, high)) =
            tracereel_core::route_manager::adaptive_pace_range(&r.route_points)
        {
            if r.style.pace_auto {
                r.style.low_pace = low;
                r.style.high_pace = high;
            }
            if r.style.tail_pace_auto {
                r.style.tail_low_pace = low;
                r.style.tail_high_pace = high;
            }
        }
    }
}

/// 按「框自己的取景参数」（方式/缩放/边距/灵敏度）+ 框矩形配置取景器。
fn configure_camera_from_box(rm: &mut RouteManager, b: &MapBox, frame: composition::Rect) {
    let view_mode = match b.view_mode {
        1 => ViewMode::RunnerCentered,
        2 => ViewMode::RunnerCenteredSplitOriented,
        _ => ViewMode::FixedSplit,
    };
    let zoom = if b.view_zoom.is_finite() && b.view_zoom > 0.0 {
        b.view_zoom.clamp(0.01, 20.0)
    } else {
        1.0
    };
    let sens = if b.sensitivity.is_finite() && b.sensitivity > 0.0 {
        b.sensitivity.clamp(0.05, 20.0)
    } else {
        0.5
    };
    rm.set_view_mode(view_mode);
    rm.centered_zoom = zoom;
    rm.left_right_margin = if b.lr_margin.is_finite() {
        b.lr_margin.clamp(0.0, 100_000.0)
    } else {
        10.0
    };
    rm.top_bottom_margin = if b.tb_margin.is_finite() {
        b.tb_margin.clamp(0.0, 100_000.0)
    } else {
        30.0
    };
    rm.set_runner_averaging_factor(sens);
    rm.use_smooth_split_transition = false;
    rm.window_resized(frame.width.max(1.0), frame.height.max(1.0));
    rm.request_full_update();
}

/// 等价 `gui::build_frame_managers`：逐框 clone 基础取景器 → 取景参数 → 该框样式。
fn build_rms_with_styles(s: &Settings, base: &RouteManager, w: f64, h: f64) -> Vec<RouteManager> {
    let boxes = s.map.effective_layout(&s.route).boxes;
    let frames = current_layout(s, w, h);
    let full = composition::Rect {
        x: 0.0,
        y: 0.0,
        width: w.max(1.0),
        height: h.max(1.0),
    };
    let mut out = Vec::with_capacity(boxes.len());
    for (i, b) in boxes.iter().enumerate() {
        let mut rm = base.clone();
        let frame = frames.get(i).map(|f| f.rect).unwrap_or(full);
        configure_camera_from_box(&mut rm, b, frame);
        let style = b
            .style
            .clone()
            .unwrap_or_else(|| MapBoxStyle::from_route(&s.route));
        let rs = gui::route_style_from_box(s, &style);
        apply_box_style(&mut rm, &rs);
        out.push(rm);
    }
    out
}

fn styled_box(
    route_color: &str,
    width: f64,
    render_scope: i32,
    show_runner: bool,
    show_controls: bool,
) -> MapBoxStyle {
    let mut st = MapBoxStyle::default();
    st.render_mode = 1; // 简洁（单色折线，像素断言最稳）
    st.render_scope = render_scope;
    st.width = width;
    st.tail_render_mode = 0; // 尾巴关掉，避免和路线混色
    st.show_runner = show_runner;
    st.runner_radius = 10.0;
    st.runner_color = "#ffd700".into(); // 黄
    st.runner_opacity = 255.0;
    st.runner_style = 0; // 圆形
    st.show_controls = show_controls;
    st.control_radius = 12.0;
    st.control_color = "#ff00ff".into(); // 品红
    st.control_opacity = 255.0;
    st.route_color = route_color.into();
    st.route_opacity = 255.0;
    st.pace_auto = false;
    st.tail_pace_auto = false;
    st
}

#[derive(Debug, Default)]
struct ColorStats {
    n: usize,
    min_x: f64,
    min_y: f64,
    max_x: f64,
    max_y: f64,
}

fn color_stats(
    rgba: &[u8],
    w: i32,
    h: i32,
    r: composition::Rect,
    pred: impl Fn([u8; 3]) -> bool,
) -> ColorStats {
    let x0 = (r.x.max(0.0).floor() as i32).min(w);
    let y0 = (r.y.max(0.0).floor() as i32).min(h);
    let x1 = ((r.x + r.width).ceil() as i32).clamp(0, w);
    let y1 = ((r.y + r.height).ceil() as i32).clamp(0, h);
    let mut st = ColorStats {
        n: 0,
        min_x: f64::INFINITY,
        min_y: f64::INFINITY,
        max_x: f64::NEG_INFINITY,
        max_y: f64::NEG_INFINITY,
    };
    for y in y0..y1 {
        for x in x0..x1 {
            if pred(pixel(rgba, w, x, y)) {
                st.n += 1;
                st.min_x = st.min_x.min(x as f64);
                st.min_y = st.min_y.min(y as f64);
                st.max_x = st.max_x.max(x as f64);
                st.max_y = st.max_y.max(y as f64);
            }
        }
    }
    st
}

fn is_route_red(p: [u8; 3]) -> bool {
    p[0] > 150 && p[1] < 100 && p[2] < 100
}
fn is_route_green(p: [u8; 3]) -> bool {
    p[1] > 150 && p[0] < 100 && p[2] < 100
}
fn is_route_blue(p: [u8; 3]) -> bool {
    p[2] > 150 && p[0] < 100 && p[1] < 100
}
fn is_runner_yellow(p: [u8; 3]) -> bool {
    p[0] > 150 && p[1] > 150 && p[2] < 120
}
fn is_control_magenta(p: [u8; 3]) -> bool {
    p[0] > 150 && p[2] > 150 && p[1] < 120
}

/// 逐框样式回归：颜色 / 线宽 / 渲染范围（完整 vs 单段）/ 跑者控制点开关 / 校准时间全局。
fn check_multi_box_styles(dir: &str) {
    const W: i32 = 900;
    const H: i32 = 300;
    let map = qtwrap::qimage_from_rgba(1000, 1000, &[60, 60, 60, 255].repeat(1000 * 1000));
    let mut s = multi_box_settings(vec![
        // 框0：红路线、线宽 12、完整渲染、控制点开（品红）、跑者关
        MapBox {
            view_mode: 1,
            view_zoom: 0.5,
            style: Some(styled_box("#ff0000", 12.0, 0, false, true)),
            ..free_box(0.01, 0.03, 0.3, 0.94)
        },
        // 框1：绿路线、线宽 6、完整渲染、跑者/控制点都关
        MapBox {
            view_mode: 1,
            view_zoom: 0.5,
            style: Some(styled_box("#00ff00", 6.0, 0, false, false)),
            ..free_box(0.345, 0.03, 0.3, 0.94)
        },
        // 框2：蓝路线、线宽 12、单段渲染、跑者开（黄）
        MapBox {
            view_mode: 1,
            view_zoom: 0.5,
            style: Some(styled_box("#0000ff", 12.0, 1, true, false)),
            ..free_box(0.68, 0.03, 0.3, 0.94)
        },
    ]);
    s.encode.include_map = true;
    s.encode.include_video = false;
    // 全局校准时间：按表 3s、录像 1s → runner 偏移 = 2s（逐框样式不得改变它）
    s.route.control_time_offset = 3.0;
    s.video.start_time_offset_s = 1.0;

    let mut base = RouteManager::default();
    base.initialize_from_points(
        camera_points(),
        "0:10|0:20|0:40",
        false,
        RouteStyle::default(),
        ViewMode::RunnerCentered,
        1.0,
    );
    let mut rms = build_rms_with_styles(&s, &base, W as f64, H as f64);
    assert_eq!(rms.len(), 3, "3 框应有 3 套取景器");

    // ① 校准时间保持全局：三框完全一致，且等于全局换算（3s / 3s-1s=2s）。
    for (i, rm) in rms.iter().enumerate() {
        let route = &rm.routes[0];
        assert!(
            (route.style.control_time_offset - 3.0).abs() < 1e-9,
            "框{i} control_time_offset 应为 3s，实际 {}",
            route.style.control_time_offset
        );
        assert!(
            (route.style.runner_time_offset - 2.0).abs() < 1e-9,
            "框{i} runner_time_offset 应为 2s，实际 {}",
            route.style.runner_time_offset
        );
        assert!((route.control_time_offset - 3.0).abs() < 1e-9);
        assert!((route.runner_time_offset - 2.0).abs() < 1e-9);
    }

    // ② 逐框样式确实写进了各自的取景器。
    assert_eq!(rms[0].routes[0].style.render_scope, 0);
    assert_eq!(rms[1].routes[0].style.render_scope, 0);
    assert_eq!(rms[2].routes[0].style.render_scope, 1);
    assert!(
        rms[0].routes[0].style.route_color.0 > 0.9 && rms[0].routes[0].style.route_color.1 < 0.1,
        "框0 路线应红，实际 {:?}",
        rms[0].routes[0].style.route_color
    );
    assert!(
        rms[1].routes[0].style.route_color.1 > 0.9 && rms[1].routes[0].style.route_color.0 < 0.1,
        "框1 路线应绿，实际 {:?}",
        rms[1].routes[0].style.route_color
    );
    assert!(
        rms[2].routes[0].style.route_color.2 > 0.9 && rms[2].routes[0].style.route_color.0 < 0.1,
        "框2 路线应蓝，实际 {:?}",
        rms[2].routes[0].style.route_color
    );
    assert!((rms[0].routes[0].style.route_width - 12.0).abs() < 1e-9);
    assert!((rms[1].routes[0].style.route_width - 6.0).abs() < 1e-9);
    // effective_layout 回填：显式写了 style 就原样用（这里 3 框都显式写了）。
    assert_eq!(box_style_of(&s, 0).route_color, "#ff0000");
    assert_eq!(box_style_of(&s, 1).route_color, "#00ff00");
    assert_eq!(box_style_of(&s, 2).route_color, "#0000ff");

    // ③ 渲染 t=15s（第 2 段）：完整渲染 vs 单段渲染 + 各自颜色。
    for rm in rms.iter_mut() {
        rm.instant_split_transition_requested = true;
        rm.update(15.0, 0.0);
    }
    let img = render_scene_image(&s, &rms, &map, None, W, H, 15_000.0);
    let rgba = qtwrap::qimage_to_rgba(&img);
    let frames = current_layout(&s, W as f64, H as f64);
    assert_eq!(frames.len(), 3);
    let red0 = color_stats(&rgba, W, H, frames[0].rect, is_route_red);
    let green0 = color_stats(&rgba, W, H, frames[0].rect, is_route_green);
    let blue0 = color_stats(&rgba, W, H, frames[0].rect, is_route_blue);
    let magenta0 = color_stats(&rgba, W, H, frames[0].rect, is_control_magenta);
    let yellow0 = color_stats(&rgba, W, H, frames[0].rect, is_runner_yellow);
    let red1 = color_stats(&rgba, W, H, frames[1].rect, is_route_red);
    let green1 = color_stats(&rgba, W, H, frames[1].rect, is_route_green);
    let yellow1 = color_stats(&rgba, W, H, frames[1].rect, is_runner_yellow);
    let magenta1 = color_stats(&rgba, W, H, frames[1].rect, is_control_magenta);
    let red2 = color_stats(&rgba, W, H, frames[2].rect, is_route_red);
    let blue2 = color_stats(&rgba, W, H, frames[2].rect, is_route_blue);
    let yellow2 = color_stats(&rgba, W, H, frames[2].rect, is_runner_yellow);
    let magenta2 = color_stats(&rgba, W, H, frames[2].rect, is_control_magenta);

    assert!(red0.n > 100, "框0 没画出红路线（{} px）", red0.n);
    assert!(green1.n > 30, "框1 没画出绿路线（{} px）", green1.n);
    assert!(blue2.n > 100, "框2 没画出蓝路线（{} px）", blue2.n);
    // 颜色不串框：每框只有自己的路线色。
    assert_eq!(green0.n, 0, "框0 出现绿路线像素：逐框样式串了");
    assert_eq!(blue0.n, 0, "框0 出现蓝路线像素：逐框样式串了");
    assert_eq!(red1.n, 0, "框1 出现红路线像素：逐框样式串了");
    assert_eq!(red2.n, 0, "框2 出现红路线像素：逐框样式串了");
    // 跑者：只有框2 开（黄），帧 0/1 关。
    assert!(yellow2.n > 0, "框2 应画出黄色跑者（{} px）", yellow2.n);
    assert_eq!(yellow0.n, 0, "框0 没开跑者却出现黄色像素");
    assert_eq!(yellow1.n, 0, "框1 没开跑者却出现黄色像素");
    // 控制点：只有框0 开（品红）。
    assert!(magenta0.n > 0, "框0 应画出品红控制点（{} px）", magenta0.n);
    assert_eq!(magenta1.n, 0, "框1 没开控制点却出现品红像素");
    assert_eq!(magenta2.n, 0, "框2 没开控制点却出现品红像素");

    // 线宽：框0（12）与框1（6）同为完整渲染、同框尺寸 → 宽线像素明显更多。
    assert!(
        (red0.n as f64) > (green1.n as f64) * 1.3,
        "线宽 12 的框像素数应明显多于线宽 6 的框：红 {} vs 绿 {}",
        red0.n,
        green1.n
    );

    // 渲染范围：同一时刻、同框尺寸/缩放，完整渲染（红）的路线包围盒远大于单段（蓝）。
    let red_w = red0.max_x - red0.min_x;
    let blue_w = blue2.max_x - blue2.min_x;
    assert!(
        red_w > blue_w * 1.5,
        "完整渲染的路线包围盒应明显大于单段渲染：红宽 {red_w:.0} vs 蓝宽 {blue_w:.0}"
    );

    assert!(qtwrap::qimage_save(
        &img,
        &format!("{dir}/multi-box-styles.png")
    ));
    println!(
        "multi-box style checks passed (per-box colors/width/scope/controls/runner; red={} green={} blue={}; bbox red={:.0} blue={:.0})",
        red0.n, green1.n, blue2.n, red_w, blue_w
    );
}

// ── 单框等价性：旧 draw_scene 与 draw_scene_layers(1 层) 必须逐像素一致 ──────

fn check_single_box_equivalence(dir: &str) {
    const W: i32 = 320;
    const H: i32 = 180;
    let mut s = multi_box_settings(vec![MapBox {
        rotation: 27.0,
        ..free_box(0.1, 0.15, 0.45, 0.6)
    }]);
    s.encode.include_map = true;
    s.encode.include_video = true;
    let map = qtwrap::qimage_from_rgba(1000, 1000, &[220, 60, 40, 255].repeat(1000 * 1000));
    let video = qtwrap::qimage_from_rgba(320, 180, &[30, 90, 200, 255].repeat(320 * 180));
    let rm = RouteManager::default();

    let mut old_img = qtwrap::qimage_new_argb32(W, H);
    {
        let mut p = qtwrap::qpainter_on_image(&mut old_img);
        gui::draw_scene(
            &mut p,
            &rm,
            &s,
            Some(map.clone()),
            Some(video.clone()),
            W as f64,
            H as f64,
            0.0,
            0.0,
            None,
        );
    }
    let mut new_img = qtwrap::qimage_new_argb32(W, H);
    {
        let mut p = qtwrap::qpainter_on_image(&mut new_img);
        let placement = current_placements(&s)[0];
        let layer = gui::SceneLayer {
            rm: &rm,
            map: &map,
            placement,
            route_start_ms: 0.0,
        };
        gui::draw_scene_layers(
            &mut p,
            &s,
            Some(&video),
            &[layer],
            W as f64,
            H as f64,
            0.0,
            None,
        );
    }
    let old = qtwrap::qimage_to_rgba(&old_img);
    let new = qtwrap::qimage_to_rgba(&new_img);
    assert_eq!(
        old, new,
        "单框：draw_scene 与 draw_scene_layers(1 层) 必须逐像素一致（旧调用点漂移）"
    );
    assert!(qtwrap::qimage_save(&old_img, &format!("{dir}/single-box-old.png")));
    assert!(qtwrap::qimage_save(&new_img, &format!("{dir}/single-box-new.png")));
    println!("single-box equivalence passed: draw_scene == draw_scene_layers(1 layer)");
}

// ── 性能烟测：1/2/3 框，简洁路线 + 配速色带各一遍（只打印/软警告，不硬失败） ──

/// 跑 warm+30 帧（含 rm.update + 绘制，与导出帧循环同构），返回平均每帧毫秒。
fn time_render(
    s: &Settings,
    rms: &mut [RouteManager],
    map: &QImage,
    img: &mut QImage,
    w: i32,
    h: i32,
) -> f64 {
    let warm = 4usize;
    let frames = 30usize;
    for i in 0..warm {
        render_scene_into(s, rms, map, None, img, w, h, i as f64 * 1000.0 / 30.0);
    }
    let t0 = Instant::now();
    for i in 0..frames {
        let t = (warm + i) as f64 * 1000.0 / 30.0;
        for rm in rms.iter_mut() {
            rm.update(t / 1000.0, 1000.0 / 30.0);
        }
        render_scene_into(s, rms, map, None, img, w, h, t);
    }
    t0.elapsed().as_secs_f64() * 1000.0 / frames as f64
}

/// `count` 个框铺满画布，第 i 框的取景缩放由 `zoom_of(i)` 决定、样式统一用 `style`。
fn perf_case(
    base: &RouteManager,
    count: usize,
    zoom_of: impl Fn(usize) -> f64,
    w: i32,
    h: i32,
    style: &MapBoxStyle,
) -> (Settings, Vec<RouteManager>) {
    let bw = 0.96 / count as f64;
    let mut boxes = Vec::new();
    for i in 0..count {
        boxes.push(MapBox {
            view_mode: 1,
            view_zoom: zoom_of(i),
            style: Some(style.clone()),
            ..free_box(0.02 + bw * i as f64, 0.02, bw - 0.02, 0.96)
        });
    }
    let mut s = multi_box_settings(boxes);
    s.encode.include_map = true;
    s.encode.include_video = false;
    // 走与生产同语义的「逐框 clone + 取景 + 样式」：这样框上的 view_mode/view_zoom/style
    // 都会真的作用到对应取景器（hudcheck 看不到 pub(crate) 的 build_frame_managers）。
    let rms = build_rms_with_styles(&s, base, w as f64, h as f64);
    (s, rms)
}

fn check_multi_box_perf(dir: &str) {
    const W: i32 = 1280;
    const H: i32 = 720;
    let map = qtwrap::qimage_from_rgba(1000, 1000, &[60, 60, 60, 255].repeat(1000 * 1000));
    // 10 分钟 1Hz 轨迹加配速（配速色带模式要用到 pace）。
    let points: Vec<RoutePoint> = (0..=600)
        .map(|i| {
            let mut p = RoutePoint::new(
                100.0 + i as f64 * 0.8,
                500.0 + (i as f64 / 20.0).sin() * 200.0,
                i as f64,
            );
            p.pace = 5.0 + (i % 15) as f64 * 0.5;
            p
        })
        .collect();
    for (mode_name, render_mode) in [
        ("discreet", RenderMode::Discreet),
        ("pace", RenderMode::Pace),
    ] {
        let mut style = camera_route_style();
        style.route_render_mode = render_mode;
        let mut base = RouteManager::default();
        base.initialize_from_points(points.clone(), "", false, style, ViewMode::RunnerCentered, 1.0);
        // 性能场景的逐框样式必须显式写入，否则会回落到全局设置（默认路线关闭）而只测地图。
        let mut perf_style = MapBoxStyle::default();
        perf_style.render_mode = if mode_name == "pace" { 3 } else { 1 };
        perf_style.render_scope = 0;
        perf_style.width = 10.0;
        perf_style.tail_render_mode = 0;
        perf_style.route_color = "#ff0000".into();
        perf_style.route_opacity = 255.0;
        perf_style.show_controls = false;
        perf_style.show_runner = false;
        perf_style.low_pace = 15.0;
        perf_style.high_pace = 5.0;
        perf_style.pace_auto = false;
        perf_style.tail_pace_auto = false;
        for mixed in [false, true] {
            // same-zoom：所有框同一个取景缩放（框把画布切分，总地图面积不变）→ 公平衡量
            //   「框数本身」的额外开销，预算 2.2x。
            // mixed-zoom：逐框缩放不同（真实「一个固定分段 + 一个跟跑者居中」的极端）→ 每框
            //   会多一次平滑变换绘制（不是缓存问题），只记录数字，阈值放宽到 4x。
            let zoom_of = |i: usize| if mixed { 1.0 + 0.4 * i as f64 } else { 1.25 };
            let scenario = if mixed { "mixed-zoom" } else { "same-zoom" };
            let mut single = 0.0f64;
            for count in [1usize, 2, 3] {
                let (s, mut rms) = perf_case(&base, count, &zoom_of, W, H, &perf_style);
                for rm in rms.iter_mut() {
                    rm.instant_split_transition_requested = true;
                    rm.update(0.0, 0.0);
                }
                let mut img = qtwrap::qimage_new_argb32(W, H);
                let avg = time_render(&s, &mut rms, &map, &mut img, W, H);
                if count == 1 {
                    single = avg;
                }
                eprintln!(
                    "[multi-box-perf] canvas={W}x{H} route={mode_name}/{scenario} boxes={count} avg={avg:.2} ms/frame"
                );
                if count > 1 && single > 0.0 {
                    let ratio = avg / single;
                    eprintln!(
                        "[multi-box-perf] {mode_name}/{scenario} {count} 框 = {ratio:.2}x 单框"
                    );
                    let budget = if mixed { 4.0 } else { 2.2 };
                    if ratio >= budget {
                        eprintln!(
                            "[multi-box-perf][WARN] {mode_name}/{scenario} {count} 框 {ratio:.2}x 超出 {budget:.1}x 预算 —— 检查缓存命中 / 逐框固定开销"
                        );
                    }
                }
                assert!(qtwrap::qimage_save(
                    &img,
                    &format!("{dir}/multi-perf-{mode_name}-{scenario}-{count}box.png")
                ));
            }
        }
        // 附加（只打印）：两框「不同样式」（红 12 完整 / 绿 6 单段）——逐框样式对 draw 的影响。
        {
            let mode_int = if mode_name == "pace" { 3 } else { 1 };
            let mut style_a = MapBoxStyle::default();
            style_a.render_mode = mode_int;
            style_a.render_scope = 0;
            style_a.width = 12.0;
            style_a.tail_render_mode = 0;
            style_a.route_color = "#ff0000".into();
            style_a.route_opacity = 255.0;
            style_a.show_controls = false;
            style_a.show_runner = false;
            style_a.low_pace = 15.0;
            style_a.high_pace = 5.0;
            style_a.pace_auto = false;
            style_a.tail_pace_auto = false;
            let mut style_b = style_a.clone();
            style_b.render_scope = 1;
            style_b.width = 6.0;
            style_b.route_color = "#00ff00".into();
            let mut boxes = Vec::new();
            for (i, st) in [style_a, style_b].into_iter().enumerate() {
                boxes.push(MapBox {
                    view_mode: 1,
                    view_zoom: 1.25,
                    style: Some(st),
                    ..free_box(0.02 + 0.47 * i as f64, 0.02, 0.45, 0.96)
                });
            }
            let mut s = multi_box_settings(boxes);
            s.encode.include_map = true;
            s.encode.include_video = false;
            let mut rms = build_rms_with_styles(&s, &base, W as f64, H as f64);
            for rm in rms.iter_mut() {
                rm.instant_split_transition_requested = true;
                rm.update(0.0, 0.0);
            }
            let mut img = qtwrap::qimage_new_argb32(W, H);
            let avg = time_render(&s, &mut rms, &map, &mut img, W, H);
            eprintln!(
                "[multi-box-perf] canvas={W}x{H} route={mode_name} boxes=2 styles=mixed(diff width/color/scope) avg={avg:.2} ms/frame"
            );
            assert!(qtwrap::qimage_save(
                &img,
                &format!("{dir}/multi-perf-{mode_name}-mixed-style-2box.png")
            ));
        }
    }
    println!("multi-box perf smoke finished (numbers on stderr)");
}
