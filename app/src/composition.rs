//! Geometry shared by preview, export, HUD and the map editor.
//! Free placement overlays the video; docked placement reserves a strip.

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, Copy)]
pub struct Placement {
    /// 0: left, 1: right, 4: free. Removed top/bottom IDs fall back to left.
    pub mode: i32,
    pub share: f64,
    /// Normalized, unrotated rectangle; rotation is about its centre.
    pub frame: Rect,
    pub rotation: f64,
    pub opacity: f64,
    pub shape: i32, // 0 rectangle, 1 inscribed circle.
    pub feather: f64, // Fraction of the shorter edge, fading inward.
    pub corner: f64, // Rectangle corner radius / shorter edge.
}

impl Default for Placement {
    fn default() -> Self {
        Self {
            mode: 0,
            share: 0.25,
            frame: Rect {
                x: 0.03,
                y: 0.05,
                width: 0.3,
                height: 0.6,
            },
            rotation: 0.0, opacity: 1.0, shape: 0, feather: 0.0, corner: 0.0,
        }
    }
}

impl Placement {
    pub fn sanitized(mut self) -> Self {
        fn finite(v: f64, fallback: f64, lo: f64, hi: f64) -> f64 {
            if v.is_finite() {
                v.clamp(lo, hi)
            } else {
                fallback
            }
        }
        let d = Self::default();
        self.mode = if [0, 1, 4].contains(&self.mode) { self.mode } else { 0 };
        self.opacity = finite(self.opacity, 1.0, 0.0, 1.0);
        self.shape = if self.shape == 1 { 1 } else { 0 };
        self.feather = finite(self.feather, 0.0, 0.0, 0.5);
        self.corner = finite(self.corner, 0.0, 0.0, 0.5);
        self.share = finite(self.share, d.share, 0.0, 1.0);
        self.frame.x = finite(self.frame.x, d.frame.x, -1.0, 1.0);
        self.frame.y = finite(self.frame.y, d.frame.y, -1.0, 1.0);
        self.frame.width = finite(self.frame.width, d.frame.width, 0.02, 1.0);
        self.frame.height = finite(self.frame.height, d.frame.height, 0.02, 1.0);
        self.rotation = finite(self.rotation, 0.0, -360.0, 360.0);
        self
    }

    pub fn panels(self, width: f64, height: f64, map: bool, video: bool) -> Panels {
        let p = self.sanitized();
        let full = Rect {
            x: 0.0,
            y: 0.0,
            width: width.max(0.0),
            height: height.max(0.0),
        };
        let mut out = Panels {
            map: Rect {
                width: 0.0,
                height: 0.0,
                ..full
            },
            video: full,
            rotation: 0.0,
        };
        if !map {
            return out;
        }
        if p.mode == 4 {
            out.map = Rect {
                x: p.frame.x * width,
                y: p.frame.y * height,
                width: p.frame.width * width,
                height: p.frame.height * height,
            };
            out.rotation = p.rotation;
        } else if !video {
            out.map = full;
            out.video.width = 0.0;
        } else {
            out.map = Rect { width: width * p.share, ..full };
            out.video.width = width - out.map.width;
            if p.mode == 0 { out.video.x = out.map.width; }
            else { out.map.x = out.video.width; }
        }
        out
    }
}

pub struct Panels {
    pub map: Rect,
    pub video: Rect,
    pub rotation: f64,
}

/// 一个地图框在画布上的落位：未旋转矩形 + 绕矩形中心的旋转角（度）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SceneBox {
    pub rect: Rect,
    pub rotation: f64,
}

/// 整幅画面的版面：视频区 + 依次叠加的地图框（数组序 = 绘制顺序，后面的盖住前面的）。
/// 预览、导出、HUD 视口、导出解码尺寸共用这一个真相。
#[derive(Debug, Clone, PartialEq)]
pub struct SceneLayout {
    pub video: Rect,
    pub boxes: Vec<SceneBox>,
}

/// 多地图框版面。规则：
/// - 视频区默认占满整幅；**只有第 1 个框**可以贴边（mode 0/1），此时视频区让出条带；
/// - 第 2 个及以后的框一律自由叠加（`sanitize` 已把它们的 mode 归成 4，这里再兜一层，
///   把残留的贴边值按等效自由矩形放置，绝不第二个框去挤视频区）；
/// - 没有视频层时，贴边的第 1 个框铺满整幅（沿用纯地图导出的旧行为），自由框保持原矩形；
/// - 没有地图层时没有框，视频区仍是整幅（画面只有视频/黑底）。
pub fn scene_layout(
    boxes: &[Placement],
    width: f64,
    height: f64,
    map: bool,
    video: bool,
) -> SceneLayout {
    let full = Rect {
        x: 0.0,
        y: 0.0,
        width: width.max(0.0),
        height: height.max(0.0),
    };
    let mut out = SceneLayout {
        video: full,
        boxes: Vec::new(),
    };
    if !map || boxes.is_empty() {
        return out;
    }
    let first = boxes[0].sanitized();
    let first_rect = if first.mode == 4 {
        free_rect(first, width, height)
    } else if video {
        let strip = (width * first.share).clamp(0.0, full.width);
        let rect = Rect {
            x: if first.mode == 0 { 0.0 } else { full.width - strip },
            y: 0.0,
            width: strip,
            height: full.height,
        };
        out.video = if first.mode == 0 {
            Rect {
                x: strip,
                y: 0.0,
                width: (full.width - strip).max(0.0),
                height: full.height,
            }
        } else {
            Rect {
                x: 0.0,
                y: 0.0,
                width: (full.width - strip).max(0.0),
                height: full.height,
            }
        };
        rect
    } else {
        // 纯地图导出：贴边框铺满整幅（与旧的 Placement::panels 一致，视频区宽度归零）。
        out.video = Rect {
            width: 0.0,
            ..full
        };
        full
    };
    out.boxes.push(SceneBox {
        rect: first_rect,
        // 贴边条带不旋转（与旧的 `Placement::panels` 一致：rotation 只对自由摆放生效）。
        rotation: if first.mode == 4 { first.rotation } else { 0.0 },
    });
    for p in &boxes[1..] {
        let p = p.sanitized();
        let free = p.mode == 4;
        let rect = if free {
            free_rect(p, width, height)
        } else {
            // 兜底：多框时贴边值没有意义，按等效自由矩形摆放。
            let strip = (width * p.share).clamp(0.0, full.width);
            Rect {
                x: if p.mode == 0 { 0.0 } else { full.width - strip },
                y: 0.0,
                width: strip,
                height: full.height,
            }
        };
        out.boxes.push(SceneBox {
            rect,
            rotation: if free { p.rotation } else { 0.0 },
        });
    }
    out
}

/// 自由摆放框在画布上的像素矩形（归一化位置 × 画布宽高）。
fn free_rect(p: Placement, width: f64, height: f64) -> Rect {
    Rect {
        x: p.frame.x * width,
        y: p.frame.y * height,
        width: p.frame.width * width,
        height: p.frame.height * height,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn docked_strips_reserve_the_correct_video_region() {
        let left = Placement::default().panels(1920., 1080., true, true);
        assert_eq!(left.map, Rect { x: 0., y: 0., width: 480., height: 1080. });
        assert_eq!(left.video, Rect { x: 480., y: 0., width: 1440., height: 1080. });
        let right = Placement { mode: 1, ..Placement::default() }.panels(1920., 1080., true, true);
        assert_eq!(right.map.x, 1440.);
        assert_eq!(right.video.x, 0.);
        for removed in [2, 3, 99] {
            assert_eq!(Placement { mode: removed, ..Placement::default() }.sanitized().mode, 0);
        }
    }

    #[test]
    fn free_frame_scales_with_canvas_and_preserves_video() {
        let placement = Placement {
            mode: 4,
            rotation: 35.,
            ..Placement::default()
        };
        let a = placement.panels(1000., 500., true, true);
        let b = placement.panels(2000., 1000., true, true);
        assert_eq!(
            a.map,
            Rect {
                x: 30.,
                y: 25.,
                width: 300.,
                height: 300.
            }
        );
        assert_eq!(
            a.video,
            Rect {
                x: 0.,
                y: 0.,
                width: 1000.,
                height: 500.
            }
        );
        assert_eq!(b.map.width, a.map.width * 2.);
        assert_eq!(b.rotation, 35.);
        assert_eq!(placement.panels(1000., 500., true, false).map, a.map);
    }

    #[test]
    fn absent_layers_and_invalid_geometry_are_safe() {
        for mode in [0, 1, 4] {
            let p = Placement {
                mode,
                ..Placement::default()
            };
            assert_eq!(p.panels(800., 600., false, true).video.width, 800.);
            assert_eq!(p.panels(800., 600., false, false).map.width, 0.);
            if mode != 4 {
                assert_eq!(p.panels(800., 600., true, false).map.width, 800.);
            }
        }
        let p = Placement {
            share: f64::NAN,
            rotation: f64::INFINITY,
            frame: Rect {
                x: 5.,
                y: -8.,
                width: -1.,
                height: 4.,
            },
            ..Placement::default()
        }
        .sanitized();
        assert_eq!(p.share, 0.25);
        assert_eq!(p.rotation, 0.);
        assert_eq!(
            p.frame,
            Rect {
                x: 1.,
                y: -1.,
                width: 0.02,
                height: 1.
            }
        );
    }

    /// 单框时多框版面必须与旧的 `Placement::panels` 完全一致（预览/导出/HUD 的现状不能变）。
    #[test]
    fn single_box_layout_matches_legacy_panels() {
        for mode in [0, 1, 4, 2, 99] {
            for video in [true, false] {
                let p = Placement {
                    mode,
                    ..Placement::default()
                };
                let legacy = p.panels(1920., 1080., true, video);
                let modern = scene_layout(&[p], 1920., 1080., true, video);
                assert_eq!(modern.video, legacy.video, "mode={mode} video={video}");
                assert_eq!(modern.boxes.len(), 1);
                assert_eq!(modern.boxes[0].rect, legacy.map, "mode={mode} video={video}");
                assert_eq!(modern.boxes[0].rotation, legacy.rotation);
            }
        }
    }

    /// 两个自由框：视频区保持整幅，每个框按自己的归一化矩形落位，数组序 = 绘制序。
    #[test]
    fn multiple_free_boxes_overlay_the_full_video() {
        let first = Placement {
            mode: 4,
            frame: Rect {
                x: 0.02,
                y: 0.05,
                width: 0.3,
                height: 0.4,
            },
            ..Placement::default()
        };
        let second = Placement {
            mode: 4,
            frame: Rect {
                x: 0.6,
                y: 0.5,
                width: 0.35,
                height: 0.45,
            },
            rotation: 30.0,
            ..Placement::default()
        };
        let layout = scene_layout(&[first, second], 1000., 500., true, true);
        assert_eq!(
            layout.video,
            Rect {
                x: 0.,
                y: 0.,
                width: 1000.,
                height: 500.
            }
        );
        assert_eq!(
            layout.boxes[0].rect,
            Rect {
                x: 20.,
                y: 25.,
                width: 300.,
                height: 200.
            }
        );
        assert_eq!(
            layout.boxes[1].rect,
            Rect {
                x: 600.,
                y: 250.,
                width: 350.,
                height: 225.
            }
        );
        assert_eq!(layout.boxes[1].rotation, 30.0);
    }

    /// 只有第 1 个框能贴边：视频区只让出一次条带，后面的框叠在整幅画布上（不会二次挤压视频）。
    #[test]
    fn only_the_first_box_may_dock() {
        let docked_left = Placement {
            mode: 1, // 固定右侧
            share: 0.25,
            ..Placement::default()
        };
        let free = Placement {
            mode: 4,
            frame: Rect {
                x: 0.6,
                y: 0.1,
                width: 0.3,
                height: 0.3,
            },
            ..Placement::default()
        };
        let layout = scene_layout(&[docked_left, free], 800., 600., true, true);
        assert_eq!(
            layout.video,
            Rect {
                x: 0.,
                y: 0.,
                width: 600.,
                height: 600.
            }
        );
        assert_eq!(
            layout.boxes[0].rect,
            Rect {
                x: 600.,
                y: 0.,
                width: 200.,
                height: 600.
            }
        );
        assert_eq!(
            layout.boxes[1].rect,
            Rect {
                x: 480.,
                y: 60.,
                width: 240.,
                height: 180.
            }
        );
    }

    /// 纯地图（无视频层）：贴边的第 1 框铺满整幅，自由框仍按自己的矩形画在黑底上。
    #[test]
    fn docked_first_box_fills_the_canvas_without_video() {
        let docked = Placement::default(); // mode 0
        let free = Placement {
            mode: 4,
            frame: Rect {
                x: 0.5,
                y: 0.5,
                width: 0.4,
                height: 0.4,
            },
            ..Placement::default()
        };
        let layout = scene_layout(&[docked, free], 400., 300., true, false);
        assert_eq!(
            layout.boxes[0].rect,
            Rect {
                x: 0.,
                y: 0.,
                width: 400.,
                height: 300.
            }
        );
        assert_eq!(
            layout.boxes[1].rect,
            Rect {
                x: 200.,
                y: 150.,
                width: 160.,
                height: 120.
            }
        );
    }

    /// 没有地图层/空列表时没有框、视频区整幅。
    #[test]
    fn absent_map_layer_yields_no_boxes() {
        let free = Placement {
            mode: 4,
            ..Placement::default()
        };
        for map in [true, false] {
            for video in [true, false] {
                let out = scene_layout(&[free], 640., 360., map, video);
                assert_eq!(out.boxes.len(), usize::from(map), "map={map} video={video}");
                assert_eq!(out.video.width, 640.);
                assert_eq!(out.video.height, 360.);
            }
        }
        let empty = scene_layout(&[], 640., 360., true, true);
        assert!(empty.boxes.is_empty());
        assert_eq!(empty.video.width, 640.);
    }
}
