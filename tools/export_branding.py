"""Export all TraceReel brand variants from ONE contour geometry.

Run with Python + numpy, Pillow and PyMuPDF. Normal exports read the committed
vector master (assets/branding/contours.json) and only re-render SVG/PNG/ICO.

--trace rebuilds that master from the original design raster, which is a personal
design file and is NOT shipped with this repository: point TRACEREEL_BRAND_SOURCE
at your own copy if you need to re-trace.
"""
import argparse
import json
import os
from pathlib import Path

import numpy as np
from PIL import Image
import pymupdf

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "assets" / "branding"
SOURCE = Path(os.environ.get("TRACEREEL_BRAND_SOURCE", str(OUT / "source.png")))
ORANGE = "#ff7800"
VIEWBOX = "128 128 1000 1000"


def simplify(points, tolerance=0.65):
    if len(points) <= 2:
        return points
    p = np.asarray(points, dtype=float)
    delta = p[-1] - p[0]
    length = np.linalg.norm(delta)
    distances = (np.abs(delta[0] * (p[:, 1] - p[0, 1])
                        - delta[1] * (p[:, 0] - p[0, 0])) / length
                 if length else np.linalg.norm(p - p[0], axis=1))
    i = int(distances.argmax())
    if distances[i] <= tolerance:
        return [points[0], points[-1]]
    return simplify(points[:i + 1], tolerance)[:-1] + simplify(points[i:], tolerance)


def trace():
    if not SOURCE.is_file():
        raise SystemExit(
            f"未找到设计稿：{SOURCE}\n"
            "该文件属个人设计素材，不随仓库发布；如需重新描摹，请用 "
            "TRACEREEL_BRAND_SOURCE 指向你的副本。"
        )
    rgb = np.asarray(Image.open(SOURCE).convert("RGB"))
    mask = rgb.min(axis=2) >= 245
    # The gray presentation background never participates in the contour paths.
    mask[:152] = False
    mask[1104:] = False
    mask[:, :144] = False
    mask[:, 1112:] = False
    padded = np.pad(mask, 1)
    neighbors = [padded[:-2, 1:-1], padded[1:-1, 2:],
                 padded[2:, 1:-1], padded[1:-1, :-2]]
    edges = {}
    for side, neighbor in enumerate(neighbors):
        ys, xs = np.nonzero(mask & ~neighbor)
        for x, y in zip(xs.tolist(), ys.tolist()):
            a, b = [((x, y), (x + 1, y)),
                    ((x + 1, y), (x + 1, y + 1)),
                    ((x + 1, y + 1), (x, y + 1)),
                    ((x, y + 1), (x, y))][side]
            edges.setdefault(a, []).append(b)
    loops = []
    while edges:
        start = next(iter(edges))
        points = [start]
        current = start
        while True:
            options = edges[current]
            following = options.pop()
            if not options:
                del edges[current]
            current = following
            if current == start:
                break
            points.append(current)
        area = sum(a[0] * b[1] - b[0] * a[1]
                   for a, b in zip(points, points[1:] + points[:1])) / 2
        if abs(area) < 100:
            continue
        split = max(range(len(points)),
                    key=lambda i: (points[i][0] - start[0]) ** 2
                    + (points[i][1] - start[1]) ** 2)
        reduced = simplify(points[:split + 1])[:-1] + simplify(points[split:] + [start])[:-1]
        d = "M " + " L ".join(f"{x} {y}" for x, y in reduced) + " Z"
        loops.append((abs(area), d))
    loops.sort(reverse=True)
    assert len(loops) == 6, f"Expected three contour bands, got {len(loops)} boundaries"
    master = {"source": str(SOURCE.relative_to(ROOT)).replace("\\", "/"),
              "viewBox": VIEWBOX, "contours": " ".join(d for _, d in loops),
              "silhouette": loops[0][1]}
    (OUT / "contours.json").write_text(json.dumps(master, indent=2) + "\n", encoding="utf-8")


def export():
    master = json.loads((OUT / "contours.json").read_text(encoding="utf-8"))
    contours = master["contours"]
    for variant in ("icon", "mark", "white"):
        background = ('<rect x="144" y="152" width="966" height="952" rx="132" fill="' + ORANGE + '"/>'
                      if variant == "icon" else
                      '<rect x="128" y="128" width="1000" height="1000" fill="#fff"/>'
                      if variant == "white" else "")
        underlay = "" if variant == "icon" else f'<path d="{master["silhouette"]}" fill="#fff"/>'
        color = "#fff" if variant == "icon" else ORANGE
        svg = (f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="{master["viewBox"]}" width="1024" height="1024">\n'
               + background + "\n" + underlay + "\n"
               + f'<path id="contours" d="{contours}" fill="{color}" fill-rule="evenodd"/>\n</svg>\n')
        (OUT / f"tracereel-{variant}.svg").write_text(svg, encoding="utf-8")
        with pymupdf.open(stream=svg.encode(), filetype="svg") as vector:
            with pymupdf.open("pdf", vector.convert_to_pdf()) as doc:
                page = doc[0]
                pix = page.get_pixmap(matrix=pymupdf.Matrix(512 / page.rect.width, 512 / page.rect.height), alpha=True)
                pix.save(str(OUT / f"tracereel-{variant}.png"))
    # Windows requires a multi-size ICO container, all sizes from the same vector.
    Image.open(OUT / "tracereel-icon.png").save(
        OUT / "tracereel.ico", sizes=[(n, n) for n in (16, 20, 24, 32, 40, 48, 64, 128, 256)])
    print("Exported icon, transparent mark, white-background mark and multi-size ICO from one geometry.")


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--trace", action="store_true")
    args = parser.parse_args()
    OUT.mkdir(parents=True, exist_ok=True)
    if args.trace:
        trace()
    export()
