# -*- coding: utf-8 -*-
"""手表 GPX 轨迹「时段中断」诊断与修复（TraceReel 工具链）。

问题背景：个别运动手表在信号不良时会停止记录轨迹点，恢复后从新的 <trkseg> 继续写，
于是同一条轨迹被切成多段。段与段之间存在 5~35 秒的**时间缺口**，而空间上两端往往只差
几米（人没有走远，只是没记上）。很多软件只把 <trkpt> 拉平成一维数组，缺口处的
"时间/里程/配速/HUD 位置"就会失真或卡住。

本工具做两件事（默认只诊断，不写文件）：
  1. diagnose：逐段列出时间缺口、空间跳变，并对每个缺口判断能否安全补齐；
  2. repair  ：把「安全缺口」按 1Hz 线性插值补点，输出修复后的 GPX。

用法：
  python tools/gpx_watch_repair.py diagnose "问题(1).gpx"
  python tools/gpx_watch_repair.py repair   "问题(1).gpx" -o "问题(1)_修复.gpx"
  python tools/gpx_watch_repair.py repair   "问题(1).gpx" -o out.gpx --merge-segments

参数语义：
  --gap-threshold  判定为"中断"的时间间隔秒数，默认 4（正常采样为 1Hz，偶发 2s 抖动）
  --max-fill-dist  允许插值补齐的最大两端空间距离，默认 50 米（超过则宁缺勿补）
  --max-fill-speed 允许插值补齐的最大推算速度，默认 6 m/s（约 21.6km/h）
  --merge-segments 输出时合并所有 <trkseg>，可能影响只认分段结构的软件
"""
from __future__ import annotations

import argparse
import math
import os
import sys
from datetime import datetime, timedelta, timezone

NS_GPX = "http://www.topografix.com/GPX/1/1"
NS_ST = "urn:uuid:D0EB2ED5-49B6-44e3-B13C-CF15BE7DD7DD"

R_EARTH = 6371000.0


def haversine(lat1, lon1, lat2, lon2) -> float:
    p1, p2 = math.radians(lat1), math.radians(lat2)
    dp = p2 - p1
    dl = math.radians(lon2 - lon1)
    h = math.sin(dp / 2) ** 2 + math.cos(p1) * math.cos(p2) * math.sin(dl / 2) ** 2
    return 2 * R_EARTH * math.asin(math.sqrt(max(0.0, min(1.0, h))))


def parse_iso(s: str) -> datetime:
    s = s.strip()
    if s.endswith("Z"):
        s = s[:-1] + "+00:00"
    dt = datetime.fromisoformat(s)
    if dt.tzinfo is None:
        dt = dt.replace(tzinfo=timezone.utc)
    return dt


def iso_z(dt: datetime) -> str:
    return dt.astimezone(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


class Point:
    __slots__ = ("lat", "lon", "ele", "t")

    def __init__(self, lat, lon, ele, t):
        self.lat, self.lon, self.ele, self.t = lat, lon, ele, t


class Segment:
    __slots__ = ("points", "desc")

    def __init__(self, points=None, desc=""):
        self.points: list[Point] = points or []
        self.desc = desc

    def __len__(self):
        return len(self.points)


def parse_gpx(path: str) -> tuple[list[Segment], str]:
    import xml.etree.ElementTree as ET

    raw = open(path, "rb").read()
    head = raw[:200].decode("utf-8", "replace")
    tree = ET.parse(path)
    root = tree.getroot()
    creator = root.get("creator", "")
    segs: list[Segment] = []
    for seg_el in root.findall(f".//{{{NS_GPX}}}trkseg"):
        pts = []
        for pt in seg_el.findall(f"{{{NS_GPX}}}trkpt"):
            try:
                lat = float(pt.get("lat"))
                lon = float(pt.get("lon"))
            except (TypeError, ValueError):
                continue
            ele_el = pt.find(f"{{{NS_GPX}}}ele")
            t_el = pt.find(f"{{{NS_GPX}}}time")
            ele = float(ele_el.text) if ele_el is not None and ele_el.text else None
            if t_el is None or not t_el.text:
                continue  # 无时间戳的点无法参与按时间对齐，跳过
            pts.append(Point(lat, lon, ele, parse_iso(t_el.text)))
        desc_el = seg_el.find(f"{{{NS_GPX}}}desc")
        segs.append(Segment(pts, desc_el.text if desc_el is not None and desc_el.text else ""))
    return [s for s in segs if s.points], creator or "(未知)"


def all_points(segs: list[Segment]) -> list[Point]:
    return [p for s in segs for p in s.points]


def _gaps(segs: list[Segment], thr: float) -> list[dict]:
    """找出所有时间缺口：段间缺口（跨 trkseg）与段内缺口（同一段内跳时）。"""
    out: list[dict] = []
    for i, seg in enumerate(segs):
        for j in range(1, len(seg.points)):
            a, b = seg.points[j - 1], seg.points[j]
            dt = (b.t - a.t).total_seconds()
            if dt > thr:
                out.append({"kind": "段内", "seg": i, "idx": j, "a": a, "b": b, "dt": dt})
    for i in range(len(segs) - 1):
        a_seg, b_seg = segs[i], segs[i + 1]
        if not a_seg.points or not b_seg.points:
            continue
        a, b = a_seg.points[-1], b_seg.points[0]
        dt = (b.t - a.t).total_seconds()
        if dt > thr:
            out.append({"kind": "段间", "seg": i, "idx": None, "a": a, "b": b, "dt": dt})
    out.sort(key=lambda g: g["a"].t)
    for g in out:
        g["dist"] = haversine(g["a"].lat, g["a"].lon, g["b"].lat, g["b"].lon)
        g["speed"] = g["dist"] / g["dt"] if g["dt"] > 0 else float("inf")
    return out


def diagnose(path: str, thr: float, max_dist: float, max_speed: float) -> int:
    segs, creator = parse_gpx(path)
    pts = all_points(segs)
    if not pts:
        print("没有解析到任何带时间戳的轨迹点。")
        return 2
    t0, t1 = pts[0].t, pts[-1].t
    gaps = _gaps(segs, thr)

    print("=" * 78)
    print(f"文件        : {path}")
    print(f"记录软件    : {creator}")
    print(f"轨迹段数    : {len(segs)}   轨迹点总数: {len(pts)}")
    print(f"时间范围    : {iso_z(t0)} → {iso_z(t1)}   跨度 {(t1 - t0).total_seconds() / 60:.1f} 分钟")
    print(f"判定阈值    : 间隔 > {thr:g}s 视为中断")
    print(f"补齐约束    : 两端距离 ≤ {max_dist:g}m 且 推算速度 ≤ {max_speed:g}m/s")
    print("=" * 78)

    print("\n各段概况：")
    print(f"{'段号':>4} {'点数':>6} {'起始':>21} {'结束':>21} {'时长s':>7}")
    for i, s in enumerate(segs):
        dur = (s.points[-1].t - s.points[0].t).total_seconds()
        print(f"{i:>4} {len(s):>6} {iso_z(s.points[0].t):>21} {iso_z(s.points[-1].t):>21} {dur:>7.0f}")

    if not gaps:
        print(f"\n未发现超过 {thr:g}s 的中断。轨迹时间连续。")
        return 0

    print(f"\n发现 {len(gaps)} 处时间中断：")
    fillable = 0
    print(f"{'类型':>4} {'位置':>10} {'中断前最后点':>21} {'恢复后首点':>21} {'缺口s':>6} "
          f"{'距离m':>8} {'推算速度m/s':>11}  判定")
    for g in gaps:
        pos = f"段{g['seg']}#{g['idx']}" if g["idx"] is not None else f"段{g['seg']}→{g['seg'] + 1}"
        ok = g["dist"] <= max_dist and g["speed"] <= max_speed
        if ok:
            fillable += 1
        verdict = "可安全插值补齐" if ok else "跨度太大，保持断开（不猜）"
        print(f"{g['kind']:>4} {pos:>10} {iso_z(g['a'].t):>21} {iso_z(g['b'].t):>21} "
              f"{g['dt']:>6.0f} {g['dist']:>8.1f} {g['speed']:>11.2f}  {verdict}")

    total_dt = sum(g["dt"] for g in gaps)
    filled_pts = sum(int(round(g["dt"])) - 1 for g in gaps if g["dist"] <= max_dist and g["speed"] <= max_speed)
    print(f"\n缺口合计            : {total_dt:.0f} 秒")
    print(f"可补齐缺口          : {fillable} / {len(gaps)}")
    print(f"补齐后新增轨迹点    : 约 {filled_pts} 个（1Hz）")
    print(f"补齐后总点数(预估)  : {len(pts) + filled_pts}")
    print("\n说明：缺口两端空间距离都很小（人没走远，只是没记录），线性插值补齐后")
    print("      配速/里程/HUD 位置在缺口期间连续，不会出现里程突跳或位置卡住。")
    print("      若为真正的大跨度中断（如隧道、长信号丢失），这里只会提示、不会伪造轨迹。")
    return 0


def interpolate_gap(g: dict) -> list[Point]:
    """在缺口两端之间按 1Hz 线性插值，返回新增的中间点（不含两端）。"""
    dt = g["dt"]
    n = int(round(dt))
    if n <= 1:
        return []
    out = []
    a, b = g["a"], g["b"]
    for k in range(1, n):
        f = k / n
        t = a.t + timedelta(seconds=k)
        ele = None
        if a.ele is not None and b.ele is not None:
            ele = a.ele + (b.ele - a.ele) * f
        out.append(Point(a.lat + (b.lat - a.lat) * f, a.lon + (b.lon - a.lon) * f, ele, t))
    return out


def repair(path: str, out_path: str, thr: float, max_dist: float, max_speed: float,
           merge: bool) -> int:
    segs, creator = parse_gpx(path)
    pts = all_points(segs)
    if not pts:
        print("没有解析到任何带时间戳的轨迹点。")
        return 2

    # 记录每个缺口在所属段中的插入位置，以及新增的独立桥接段
    inserts: dict[int, list[tuple[int, list[Point]]]] = {}
    bridges: list[tuple[int, list[Point]]] = []  # (插到第几段之后, 点)
    unfilled = []
    for g in _gaps(segs, thr):
        if not (g["dist"] <= max_dist and g["speed"] <= max_speed):
            unfilled.append(g)
            continue
        mid = interpolate_gap(g)
        if not mid:
            continue
        if g["idx"] is not None:
            inserts.setdefault(g["seg"], []).append((g["idx"], mid))
        else:
            bridges.append((g["seg"], mid))

    new_segs: list[Segment] = []
    for i, s in enumerate(segs):
        if merge:
            continue
        new_segs.append(s)
        for idx, mid in sorted(inserts.get(i, []), key=lambda x: x[0]):
            s.points[idx:idx] = mid  # 段内缺口：直接插回本段
        for seg_i, mid in bridges:
            if seg_i == i:
                new_segs.append(Segment(mid, "插值补齐（原记录中断）"))

    if merge:
        merged = Segment()
        for i, s in enumerate(segs):
            merged.points.extend(s.points)
            for idx, mid in sorted(inserts.get(i, []), key=lambda x: x[0]):
                merged.points[idx:idx] = mid
            for seg_i, mid in bridges:
                if seg_i == i:
                    merged.points.extend(mid)
            merged.points.sort(key=lambda p: p.t)
        new_segs = [merged]

    # 校验：时间单调、无重复时刻
    flat = all_points(new_segs)
    for k in range(1, len(flat)):
        if flat[k].t < flat[k - 1].t:
            print(f"错误：修复后时间非单调，位置 {k}（{iso_z(flat[k - 1].t)} → {iso_z(flat[k].t)}）")
            return 3
    dup = sum(1 for k in range(1, len(flat)) if flat[k].t == flat[k - 1].t)

    write_gpx(out_path, new_segs, creator)
    added = len(flat) - len(pts)

    print("=" * 78)
    print(f"已写出修复文件  : {out_path}")
    print(f"原轨迹点/段数   : {len(pts)} 点 / {len(segs)} 段")
    print(f"新轨迹点/段数   : {len(flat)} 点 / {len(new_segs)} 段  （新增插值 {added} 点）")
    print(f"时间单调性      : 通过（重复时刻 {dup} 个）")
    if unfilled:
        print(f"未补齐的中断    : {len(unfilled)} 处（跨度超过约束，原样保留不伪造）")
        for g in unfilled:
            print(f"   {iso_z(g['a'].t)} → {iso_z(g['b'].t)}  缺口{g['dt']:.0f}s "
                  f"距离{g['dist']:.1f}m 速度{g['speed']:.2f}m/s")
    print("=" * 78)
    return 0


def write_gpx(out_path: str, segs: list[Segment], creator: str) -> None:
    lines = ['<?xml version="1.0" encoding="utf-8"?>']
    lines.append(f'<gpx xmlns:st="{NS_ST}" version="1.1" creator="TraceReel 轨迹中断修复" '
                 f'xmlns="{NS_GPX}">')
    lines.append("  <trk>")
    for s in segs:
        if not s.points:
            continue
        lines.append("    <trkseg>")
        if s.desc:
            lines.append(f"      <desc>{s.desc}</desc>")
        for p in s.points:
            lines.append(f'      <trkpt lat="{p.lat:.7f}" lon="{p.lon:.7f}">')
            if p.ele is not None:
                lines.append(f"        <ele>{p.ele:.0f}</ele>")
            lines.append(f"        <time>{iso_z(p.t)}</time>")
            lines.append("      </trkpt>")
        lines.append("    </trkseg>")
    lines.append("  </trk>")
    lines.append("</gpx>")
    with open(out_path, "w", encoding="utf-8-sig", newline="\n") as fh:
        fh.write("\n".join(lines) + "\n")


def main(argv: list[str]) -> int:
    ap = argparse.ArgumentParser(description="手表 GPX 轨迹中断诊断与修复")
    sub = ap.add_subparsers(dest="cmd", required=True)

    d = sub.add_parser("diagnose", help="只诊断，不写文件")
    d.add_argument("gpx")
    d.add_argument("--gap-threshold", type=float, default=4.0)
    d.add_argument("--max-fill-dist", type=float, default=50.0)
    d.add_argument("--max-fill-speed", type=float, default=6.0)

    r = sub.add_parser("repair", help="插值补齐安全缺口并写出新 GPX")
    r.add_argument("gpx")
    r.add_argument("-o", "--out", required=True)
    r.add_argument("--gap-threshold", type=float, default=4.0)
    r.add_argument("--max-fill-dist", type=float, default=50.0)
    r.add_argument("--max-fill-speed", type=float, default=6.0)
    r.add_argument("--merge-segments", action="store_true",
                   help="合并所有 trkseg 为一段（部分软件只认单段时使用）")

    a = ap.parse_args(argv)
    if a.cmd == "diagnose":
        return diagnose(a.gpx, a.gap_threshold, a.max_fill_dist, a.max_fill_speed)
    return repair(a.gpx, a.out, a.gap_threshold, a.max_fill_dist, a.max_fill_speed,
                  a.merge_segments)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))