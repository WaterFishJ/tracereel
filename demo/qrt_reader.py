# -*- coding: utf-8 -*-
"""QuickRoute .qrt 工程文件读取器（读地图 + 轨迹；分段解析持续完善中）。

用法：
    python qrt_reader.py <file.qrt>                      # 打印摘要
    python qrt_reader.py <file.qrt> --map out.jpg        # 导出内嵌地图
    python qrt_reader.py <file.qrt> --gpx out.gpx        # 导出轨迹为 GPX

已确认的格式（详见 docs/qrt格式逆向笔记.md）：

* 容器是 .NET BinaryFormatter 流，头部 17 字节，随后是类表，再是对象图。
* `Map.rawData` 里内嵌整张地图图片（JPEG/PNG），必须按图片自身的段结构
  走到结尾——用 find(b"\\xff\\xd9") 会命中文件头的 Photoshop 缩略图。
* 轨迹点以定长 25 字节记录存在：
      +0  8B  经度 double LE
      +8  8B  纬度 double LE
      +16 1B  常量 0x01
      +17 4B  点索引 int32 LE
      +21 4B  常量 0
"""
import math
import struct
import sys

sys.stdout.reconfigure(encoding='utf-8', errors='replace')


# ---------------------------------------------------------------- 图片
def _jpeg_extent(d, i):
    if d[i:i + 2] != b"\xff\xd8":
        return None
    p = i + 2
    w = h = None
    while p + 1 < len(d):
        if d[p] != 0xFF:
            return None
        m = d[p + 1]
        if m == 0xFF:
            p += 1
            continue
        if m == 0xD8 or m == 0x01 or 0xD0 <= m <= 0xD7:
            p += 2
            continue
        if m == 0xD9:
            return (p + 2 - i, w, h)
        if m == 0xDA:
            p += 2 + struct.unpack_from(">H", d, p + 2)[0]
            while p + 1 < len(d):
                if d[p] == 0xFF and d[p + 1] != 0x00 and not (0xD0 <= d[p + 1] <= 0xD7):
                    break
                p += 1
            continue
        ln = struct.unpack_from(">H", d, p + 2)[0]
        if 0xC0 <= m <= 0xCF and m not in (0xC4, 0xC8, 0xCC):
            h, w = struct.unpack_from(">HH", d, p + 5)
        p += 2 + ln
    return None


def _png_extent(d, i):
    if d[i:i + 8] != b"\x89PNG\r\n\x1a\n":
        return None
    p = i + 8
    w = h = None
    while p + 12 <= len(d):
        ln = struct.unpack_from(">I", d, p)[0]
        typ = d[p + 4:p + 8]
        if typ == b"IHDR":
            w, h = struct.unpack_from(">II", d, p + 8)
        p += 12 + ln
        if typ == b"IEND":
            return (p - i, w, h)
    return None


def find_map_image(data, min_size=10000):
    """返回 (bytes, kind, width, height)；找不到返回 None。"""
    best = None
    for sig, fn, kind in ((b"\xff\xd8\xff", _jpeg_extent, "jpeg"),
                          (b"\x89PNG\r\n\x1a\n", _png_extent, "png")):
        i = 0
        while True:
            i = data.find(sig, i)
            if i < 0:
                break
            ext = fn(data, i)
            if ext and ext[0] >= min_size:
                if best is None or ext[0] > best[0]:
                    best = (ext[0], i, kind, ext[1], ext[2])
            i += 1
    if best is None:
        return None
    size, off, kind, w, h = best
    return (data[off:off + size], kind, w, h)


# ---------------------------------------------------------------- 轨迹
def find_waypoints(data, lon_range=None, lat_range=None):
    """扫描定长 25 字节的 waypoint 记录（可能分散成多段）。

    记录形状：|经度 double|纬度 double|0x01|索引 int32|4 字节常量|。
    要求经度、纬度都非退化（|v| > 1e-6）且标记字节为 0x01。
    第一遍用宽范围收集，用于推断地图所在的经纬度窗口。
    """
    lo_lo, lo_hi = (-180.0, 180.0) if lon_range is None else lon_range
    la_lo, la_hi = (-90.0, 90.0) if lat_range is None else lat_range

    hits = []
    off = 0
    n = len(data)
    while off + 25 <= n:
        if data[off + 16] == 0x01:
            lon = struct.unpack_from('<d', data, off)[0]
            if lo_lo < lon < lo_hi and abs(lon) > 1e-6:
                lat = struct.unpack_from('<d', data, off + 8)[0]
                if la_lo < lat < la_hi and abs(lat) > 1e-6:
                    idx = struct.unpack_from('<i', data, off + 17)[0]
                    if 0 <= idx < 1000000:
                        hits.append((off, lon, lat, idx))
                        off += 25
                        continue
        off += 1
    return hits


# ---------------------------------------------------------------- 投影原点
def find_projection_origin(data):
    """定位 `Document.projectionOrigin`（LongLat：lon、lat 两个 double）。

    对象图里两个成员按声明顺序写（先 longitude 后 latitude），紧接着就是内嵌
    地图图片，所以「紧靠图片起点之前的那组经纬度 double」就是投影原点。
    返回 (lon, lat) 或 None。
    """
    img = find_map_image(data)
    if img is None:
        return None
    img_at = data.find(img[0][:4])
    if img_at < 16:
        return None
    best = None
    for off in range(max(0, img_at - 64), img_at - 15):
        lon = struct.unpack_from('<d', data, off)[0]
        if not (-180.0 < lon < 180.0) or abs(lon) < 1e-6:
            continue
        lat = struct.unpack_from('<d', data, off + 8)[0]
        if not (-90.0 < lat < 90.0) or abs(lat) < 1e-6:
            continue
        best = (lon, lat)          # 取最靠近图片的那一组
    return best


def to_map_pixels(waypoints, origin, map_w, map_h):
    """经纬度 → 地图像素坐标（正交投影 + 按轨迹包围盒归一化）。

    与 `tracereel-core/src/qrt.rs` 的 `QuickRouteProject::map_positions` 同一算法。
    """
    if not waypoints or origin is None or map_w <= 0 or map_h <= 0:
        return []
    olon, olat = origin
    m_lat = 110574.0
    m_lon = 111320.0 * math.cos(math.radians(olat))
    east = [(w[1] - olon) * m_lon for w in waypoints]
    north = [(w[2] - olat) * m_lat for w in waypoints]
    e_min, e_max = min(east), max(east)
    n_min, n_max = min(north), max(north)
    sx = map_w / max(e_max - e_min, 1e-9)
    sy = map_h / max(n_max - n_min, 1e-9)
    return [((east[i] - e_min) * sx, (n_max - north[i]) * sy, waypoints[i][3])
            for i in range(len(waypoints))]


# ---------------------------------------------------------------- 主读取器
class QrtReader:
    def __init__(self, path):
        self.path = path
        self.data = open(path, 'rb').read()
        self._parse()

    def _parse(self):
        d = self.data
        self.map_image = None
        self.map_kind = None
        self.map_size = None
        img = find_map_image(d)
        if img:
            self.map_bytes, self.map_kind, self.map_w, self.map_h = img
            self.map_image = True
            self.map_size = self.map_w, self.map_h

        # 轨迹：先用宽松范围扫一遍，拿到经纬度中位数后收窄
        loose = find_waypoints(d)
        if loose:
            lons = sorted(h[1] for h in loose)
            lats = sorted(h[2] for h in loose)
            clon = lons[len(lons) // 2]
            clat = lats[len(lats) // 2]
            self.waypoints = find_waypoints(
                d, (clon - 0.5, clon + 0.5), (clat - 0.5, clat + 0.5))
        else:
            self.waypoints = []
        self.waypoints.sort(key=lambda w: w[3])
        self.projection_origin = find_projection_origin(d)
        self.map_positions = to_map_pixels(
            self.waypoints, self.projection_origin,
            self.map_w if self.map_image else 0,
            self.map_h if self.map_image else 0)

    # ------------------------------------------------------------ 输出
    def summary(self):
        out = ["文件: %s (%d 字节)" % (self.path, len(self.data))]
        if self.map_image:
            out.append("地图: 内嵌 %s，%dx%d，%d 字节"
                       % (self.map_kind.upper(), self.map_w, self.map_h,
                          len(self.map_bytes)))
        else:
            out.append("地图: 未找到内嵌图片")
        if self.waypoints:
            i0 = self.waypoints[0][3]
            i1 = self.waypoints[-1][3]
            out.append("轨迹: %d 个点（索引 %d..%d）" % (len(self.waypoints), i0, i1))
            out.append("       首点 %.7f, %.7f   末点 %.7f, %.7f"
                       % (self.waypoints[0][1], self.waypoints[0][2],
                          self.waypoints[-1][1], self.waypoints[-1][2]))
            out.append("       首个索引非 0：前 %d 个点未以该记录格式存储" % i0
                       if i0 > 0 else "       索引从 0 开始（完整轨迹）")
        else:
            out.append("轨迹: 未找到 waypoint 记录")
        if self.projection_origin:
            out.append("投影原点: %.7f, %.7f" % self.projection_origin)
        if self.map_positions:
            xs = [p[0] for p in self.map_positions]
            ys = [p[1] for p in self.map_positions]
            out.append("地图像素: x %.0f..%.0f, y %.0f..%.0f（共 %d 点）"
                       % (min(xs), max(xs), min(ys), max(ys), len(self.map_positions)))
        return "\n".join(out)

    def write_map(self, out_path):
        if not self.map_image:
            raise SystemExit("没有内嵌地图可导出")
        open(out_path, 'wb').write(self.map_bytes)
        return out_path

    def write_gpx(self, out_path):
        pts = "\n".join(
            '      <trkpt lat="%.7f" lon="%.7f"><name>%d</name></trkpt>'
            % (w[2], w[1], w[3]) for w in self.waypoints)
        gpx = ('<?xml version="1.0" encoding="UTF-8"?>\n'
               '<gpx version="1.1" creator="tracereel qrt_reader">\n'
               '  <trk>\n    <trkseg>\n%s\n    </trkseg>\n  </trk>\n</gpx>\n' % pts)
        open(out_path, 'w', encoding='utf-8').write(gpx)
        return out_path


if __name__ == '__main__':
    if len(sys.argv) < 2:
        raise SystemExit(__doc__)
    r = QrtReader(sys.argv[1])
    print(r.summary())
    args = sys.argv[2:]
    for k in range(0, len(args) - 1, 2):
        if args[k] == '--map':
            print("已写出地图:", r.write_map(args[k + 1]))
        elif args[k] == '--gpx':
            print("已写出轨迹:", r.write_gpx(args[k + 1]))
