# -*- coding: utf-8 -*-
"""Validate the .NET-based `.qrt` extractor output against known-good values."""
import base64
import collections
import json
import os
import sys
import tempfile

sys.stdout.reconfigure(encoding='utf-8', errors='replace')

path = sys.argv[1] if len(sys.argv) > 1 else \
    os.path.join(tempfile.gettempdir(), "qrt_out.json")
d = json.load(open(path, encoding='utf-8'))

checks = []
checks.append(("map bytes", d["mapBytes"], 4_278_305))
checks.append(("proj lon", round(d["projLon"], 7), 113.0243020))
checks.append(("proj lat", round(d["projLat"], 7), 23.1763540))
checks.append(("segments", d["segments"], 4))
checks.append(("waypoints", d["waypointCount"], 2048))
checks.append(("handles", d["handleCount"], 497))

print("=== 校验 ===")
ok = True
for name, got, want in checks:
    good = (got == want) if not isinstance(want, float) else abs(got - want) < 1e-6
    ok = ok and good
    print("  %-10s 实际=%-12s 期望=%-12s %s" % (name, got, want, "OK" if good else "**不符**"))

raw = base64.b64decode(d["mapBase64"])
print("  地图头      %s (JPEG=%s), 长度 %d"
      % (raw[:3].hex(), raw[:3] == b"\xff\xd8\xff", len(raw)))

wp = [w for w in d["waypoints"] if w["lon"] != 0.0]
print()
print("=== 轨迹 ===")
print("  非零坐标航点: %d / %d" % (len(wp), d["waypointCount"]))
print("  首点 lon=%.7f lat=%.7f time=%d alt=%.1f"
      % (wp[0]["lon"], wp[0]["lat"], wp[0]["time"], wp[0]["alt"]))
print("  末点 lon=%.7f lat=%.7f time=%d alt=%.1f"
      % (wp[-1]["lon"], wp[-1]["lat"], wp[-1]["time"], wp[-1]["alt"]))
t0, t1 = wp[0]["time"], wp[-1]["time"]
print("  时间跨度 %.1f 秒" % ((t1 - t0) / 1e7))

print()
print("=== 分段 Handle ===")
print("  首项:", d["handles"][0])
print("  末项:", d["handles"][-1])
print("  按段索引分布:", dict(sorted(collections.Counter(
    h["segmentIndex"] for h in d["handles"]).items())))
print("  按类型分布:", dict(sorted(collections.Counter(
    h["type"] for h in d["handles"]).items())))
xs = [h["x"] for h in d["handles"]]
ys = [h["y"] for h in d["handles"]]
print("  地图像素范围 x %.0f..%.0f  y %.0f..%.0f" % (min(xs), max(xs), min(ys), max(ys)))

print()
print("结论:", "全部符合" if ok else "存在不符项")
