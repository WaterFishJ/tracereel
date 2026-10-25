#pragma once
#include <QtGui/QImage>
#include <QtGui/QPainter>
#include <algorithm>
#include <array>
#include <cmath>

// ─────────────────────────────────────────────────────────────────────────────
// 地图框遮罩（覆盖率）缓存：多地图框版本
//
// 只缓存「覆盖率遮罩」，地图位图、路线、运动员每帧照常重绘；遮罩只跟框的
// **局部几何**有关，所以：
//   - key = (w, h, shape, feather, corner) —— 与旧版完全一致；
//   - 位置(x,y)、旋转、不透明度、绘制层级都**不进 key**：拖动/旋转地图框不会
//     让缓存失效（遮罩在框的局部坐标系里生成，中心对称，跟框摆在哪无关）；
//   - 不透明度在最后整层合成时用 `setOpacity` 应用，同样与遮罩无关；
//   - 只有拖动「羽化/圆角/形状」滑杆时 key 才会逐帧变化，那是编辑交互本身
//     的代价（预览时每帧重建一次，可接受）。
//
// 为什么必须多槽：多地图框时同一帧里会连续画 N 个不同尺寸/外观的框；单槽缓存
// 会被 A→B→A 轮流挤掉，导致**每个框每帧都重建**（1080p 半屏框 2~6 ms/框，
// 4K 全屏框 30~100 ms/框，直接形成性能断崖）。改成定长多槽 LRU 后，每个框
// 的遮罩只在其几何/外观变化时才重建，稳态下全部命中。
//
// 槽位与预算（可配置）：
//   - `kMapMaskCacheSlots = 8`：同帧不同 key 的数量 ≤ 地图框数上限 4，8 槽够用
//     （多出的槽留给编辑时新出现的临时 key 与余量）；
//   - `kMapMaskCacheBudgetBytes = 64 MB`：一张遮罩占 w*h*4 字节，4K 全屏框约
//     33 MB/张；超出预算时按「最久未使用」淘汰其它条目；
//   - 若单个条目自己就超过预算（或淘汰到只剩它仍放不下），允许它**独占缓存**
//     —— 即清掉其它条目后仍然插入，宁可暂时超出预算，也不要退化成每帧重建。
//
// 缓存仍是 `thread_local`：预览线程与导出线程各一份，互不干扰；导出线程那份
// 在整个导出过程中 key 不变，建一次后全程命中。
// ─────────────────────────────────────────────────────────────────────────────

/// 遮罩缓存槽数（可配置）：多地图框同帧并存的不同遮罩数量 ≤ 框数。
static constexpr int kMapMaskCacheSlots = 8;
/// 遮罩缓存总字节预算（可配置）：按 QImage::sizeInBytes() 统计。
static constexpr qint64 kMapMaskCacheBudgetBytes = 64LL * 1024 * 1024;

/// 一张缓存下来的遮罩：key + 像素 + LRU 时间戳。
struct OvMaskCacheEntry {
    int w = 0;
    int h = 0;
    int shape = -1;
    double feather = -1.0;
    double corner = -1.0;
    QImage image;
    quint64 last_use = 0;

    bool empty() const { return image.isNull(); }
    /// 该条目占用的字节数（空条目为 0）。
    qint64 bytes() const { return image.isNull() ? 0 : static_cast<qint64>(image.sizeInBytes()); }
};

/// 每个线程一份的遮罩缓存。
struct OvMaskCache {
    std::array<OvMaskCacheEntry, kMapMaskCacheSlots> entries;
    /// 单调递增的 LRU 时钟；每命中/插入一次 +1。
    quint64 tick = 0;
    /// 当前所有条目像素字节数之和（用于 64 MB 预算判断）。
    qint64 bytes = 0;
};

/// 取当前线程的遮罩缓存（函数内 static thread_local：与旧实现同样按线程懒初始化）。
static OvMaskCache &ovMaskCache() {
    static thread_local OvMaskCache cache;
    return cache;
}

// Cache only the coverage mask; the map and runner are rendered anew each frame.
// Distances are measured inward, so feathering never paints outside the frame.
// 函数签名保持不变：调用方（qtwrap::qpainter_draw_map_layer）无需改动。
static void ovDrawMapLayer(QPainter *p, QImage *layer, double width, double height,
                          int shape, double feather, double corner, double opacity) {
    const int w = layer->width(), h = layer->height();
    if (w <= 0 || h <= 0 || opacity <= 0) return;

    OvMaskCache &cache = ovMaskCache();

    // 1) 精确命中：key 与旧版一致，与框位置/旋转/不透明度无关。
    OvMaskCacheEntry *slot = nullptr;
    for (auto &e : cache.entries) {
        if (e.w == w && e.h == h && e.shape == shape &&
            e.feather == feather && e.corner == corner) {
            slot = &e;
            break;
        }
    }

    if (slot) {
        // 命中：只刷新 LRU 时间戳，不重建像素。
        slot->last_use = ++cache.tick;
    } else {
        // 2) 未命中：先找空槽，槽满则选「最久未使用」的那个作为淘汰目标。
        for (auto &e : cache.entries) {
            if (e.empty()) {
                slot = &e;
                break;
            }
        }
        if (!slot) {
            slot = &cache.entries[0];
            for (auto &e : cache.entries) {
                if (e.last_use < slot->last_use) slot = &e;
            }
        }

        // 3) 预算：覆盖目标槽之后若仍超 64 MB，就从其它条目里按 LRU 淘汰。
        //    只剩目标槽一个条目还放不下时停止淘汰 —— 允许单张超大遮罩独占缓存
        //    （例如 4K 全屏框 ≈ 33 MB；硬性拒缓存会退化成每帧重建，更糟）。
        const qint64 need = static_cast<qint64>(w) * static_cast<qint64>(h) * 4;
        qint64 projected = (cache.bytes - slot->bytes()) + need;
        while (projected > kMapMaskCacheBudgetBytes) {
            OvMaskCacheEntry *victim = nullptr;
            for (auto &e : cache.entries) {
                if (&e != slot && !e.empty() && (!victim || e.last_use < victim->last_use)) {
                    victim = &e;
                }
            }
            if (!victim) break;  // 只剩目标槽：单条目独占（预算暂时超出）
            cache.bytes -= victim->bytes();
            *victim = OvMaskCacheEntry();  // 释放像素并把该槽恢复为空
            projected = (cache.bytes - slot->bytes()) + need;
        }

        // 4) 重建遮罩：像素算法与旧版逐字节一致（hudcheck / test_map_layout 的
        //    像素级断言依赖同一套公式，改这里会破坏那些回归）。
        cache.bytes -= slot->bytes();
        slot->w = w;
        slot->h = h;
        slot->shape = shape;
        slot->feather = feather;
        slot->corner = corner;
        slot->image = QImage(w, h, QImage::Format_ARGB32_Premultiplied);
        const double shorter = (std::min)(w, h);
        const double radius = shorter * (shape == 1 ? 0.5 : corner);
        const double fade = shorter * feather;
        for (int y = 0; y < h; ++y) {
            auto row = reinterpret_cast<QRgb *>(slot->image.scanLine(y));
            for (int x = 0; x < w; ++x) {
                const double dx = std::abs(x + 0.5 - w / 2.0);
                const double dy = std::abs(y + 0.5 - h / 2.0);
                double distance;
                if (shape == 1) {
                    distance = radius - std::hypot(dx, dy);
                } else {
                    const double qx = dx - (w / 2.0 - radius);
                    const double qy = dy - (h / 2.0 - radius);
                    distance = radius - std::hypot((std::max)(qx, 0.0), (std::max)(qy, 0.0))
                        - (std::min)((std::max)(qx, qy), 0.0);
                }
                double coverage;
                if (fade > 0) {
                    const double t = std::clamp(distance / fade, 0.0, 1.0);
                    coverage = t * t * (3 - 2 * t);
                } else {
                    coverage = std::clamp(distance + 0.5, 0.0, 1.0);
                }
                row[x] = qRgba(0, 0, 0, int(std::round(coverage * 255)));
            }
        }
        slot->last_use = ++cache.tick;
        cache.bytes += slot->bytes();  // 用 sizeInBytes() 统计，避免行对齐误差
    }

    // 5) 应用遮罩 + 整层合成：与旧版一致（遮罩只作用于本帧新建的离屏 layer）。
    {
        QPainter mask(layer);
        mask.setCompositionMode(QPainter::CompositionMode_DestinationIn);
        mask.drawImage(0, 0, slot->image);
    }
    p->save();
    p->setOpacity(p->opacity() * opacity);
    p->drawImage(QRectF(-width / 2, -height / 2, width, height), *layer);
    p->restore();
}
