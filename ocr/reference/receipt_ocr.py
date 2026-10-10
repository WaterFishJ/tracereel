"""Offline PP-OCRv4 mobile (RapidOCR/ONNX Runtime). JSON stdin/stdout protocol."""
import json
import math
import re
import sys
from pathlib import Path


def rectify(image, corners, rotation=0):
    import cv2
    import numpy as np
    points = np.asarray(corners, dtype=np.float32)
    if points.shape != (4, 2) or not np.isfinite(points).all():
        raise ValueError("请按左上、右上、右下、左下设置成绩表四角")
    h, w = image.shape[:2]
    if (points < 0).any() or (points > 1).any():
        raise ValueError("裁剪点超出图片范围")
    points *= [w - 1, h - 1]
    if not cv2.isContourConvex(points.astype(np.int32)):
        raise ValueError("四角必须构成不交叉的凸四边形")
    width = round(max(np.linalg.norm(points[1] - points[0]), np.linalg.norm(points[2] - points[3])))
    height = round(max(np.linalg.norm(points[3] - points[0]), np.linalg.norm(points[2] - points[1])))
    if min(width, height) < 30:
        raise ValueError("裁剪区域过小")
    scale = min(2.0, 2200 / max(width, height))
    width, height = round(width * scale), round(height * scale)
    target = np.float32([[0, 0], [width - 1, 0], [width - 1, height - 1], [0, height - 1]])
    corrected = cv2.warpPerspective(image, cv2.getPerspectiveTransform(points, target), (width, height))
    turns = int(rotation) % 4
    if turns:
        corrected = np.rot90(corrected, -turns).copy()
    return corrected


def grid_from_boxes(items, boundaries=None):
    """Recover physical columns; retain headers, empty cells and original order.

    OCR sometimes joins the ordinal and control code. Split whitespace tokens
    using estimated positions rather than guessing semantic column roles.
    """
    tokens = []
    for box, text, score in items:
        left = min(p[0] for p in box)
        right = max(p[0] for p in box)
        top = min(p[1] for p in box)
        bottom = max(p[1] for p in box)
        text = re.sub(r"(?<=\d)\s*[:：]\s*(?=\d)", ":", text)
        text = re.sub(r"(?<=\d)\s*([.,])\s*(?=\d)", r"\1", text)
        matches = list(re.finditer(r"\S+", text))
        for match in matches:
            x = left + (right - left) * match.start() / max(1, len(text))
            tokens.append(dict(text=match.group(), x=x, y=(top + bottom) / 2,
                               height=bottom - top, score=float(score)))
    if not tokens:
        return {"rows": [], "columns": 0}
    tokens.sort(key=lambda t: t["y"])
    rows = []
    for token in tokens:
        if not rows or abs(token["y"] - rows[-1][0]["y"]) > max(5, token["height"] * .55):
            rows.append([])
        rows[-1].append(token)
    # Infer column anchors from the densest rows, not spanning title lines.
    max_cells = max(len(row) for row in rows)
    anchors = []
    for row in rows:
        if len(row) >= max(2, max_cells - 1):
            for token in row:
                nearest = min(range(len(anchors)), key=lambda i: abs(anchors[i][0] - token["x"]), default=None)
                tolerance = max(12, token["height"] * 1.1)
                if nearest is not None and abs(anchors[nearest][0] - token["x"]) < tolerance:
                    x, count = anchors[nearest]
                    anchors[nearest] = ((x * count + token["x"]) / (count + 1), count + 1)
                else:
                    anchors.append((token["x"], 1))
    if boundaries:
        anchors = [((a + b) / 2, 1) for a, b in zip(boundaries, boundaries[1:])]
    if not anchors:
        anchors = [(t["x"], 1) for t in max(rows, key=len)]
    anchors.sort()
    output = []
    for row in rows:
        cells = [""] * len(anchors)
        confidence = [1.0] * len(anchors)
        for t in sorted(row, key=lambda t: t["x"]):
            if boundaries:
                col = min(len(anchors) - 1, max(0, sum(t["x"] >= b for b in boundaries) - 1))
            else:
                col = min(range(len(anchors)), key=lambda i: abs(anchors[i][0] - t["x"]))
            cells[col] = (cells[col] + " " + t["text"]).strip()
            confidence[col] = min(confidence[col], t["score"])
        output.append({"cells": cells, "scores": confidence})
    return {"rows": output, "columns": len(anchors)}


def recognize(request):
    import cv2
    import numpy as np
    from rapidocr_onnxruntime import RapidOCR
    image = cv2.imdecode(np.fromfile(request["path"], dtype=np.uint8), cv2.IMREAD_COLOR)
    if image is None:
        raise ValueError("无法读取图片")
    corrected = rectify(image, request["corners"], request.get("rotation", 0))
    # This pinned package bundles all three Paddle mobile models: no downloads
    # or cloud calls occur during recognition.
    engine = RapidOCR(det_limit_side_len=1600, det_limit_type="max")
    items, _ = engine(corrected)
    items = items or []
    # Recover stable white gutters across the central data rows. Detecting each
    # strip separately prevents OCR joining tightly spaced ordinal/control/time.
    heights = [max(p[1] for p in box) - min(p[1] for p in box) for box, _, _ in items]
    boundaries = None
    if heights:
        gray = cv2.cvtColor(corrected, cv2.COLOR_BGR2GRAY)
        _, ink = cv2.threshold(gray, 0, 255, cv2.THRESH_BINARY_INV + cv2.THRESH_OTSU)
        h, w = ink.shape
        density = (ink[round(h * .13):round(h * .88)] > 0).mean(axis=0)
        blank = np.where(density < .015)[0]
        runs = np.split(blank, np.where(np.diff(blank) > 1)[0] + 1)
        left = min(min(p[0] for p in box) for box, _, _ in items)
        right = max(max(p[0] for p in box) for box, _, _ in items)
        min_gap = max(10, float(np.median(heights)) * .3)
        gutters = [(int(run[0]) + int(run[-1])) // 2 for run in runs
                   if len(run) >= min_gap and run[0] > left + min_gap and run[-1] < right - min_gap]
        if 1 <= len(gutters) <= 7:
            boundaries = [0] + gutters + [w]
    grid = grid_from_boxes(items)
    if boundaries:
        # Detect rows once on the whole table, then recognize cell crops in a
        # single batch. Detection on narrow columns tends to join vertical digits.
        bands = []
        for box, text, score in sorted(items, key=lambda t: sum(p[1] for p in t[0]) / 4):
            top, bottom = min(p[1] for p in box), max(p[1] for p in box)
            cy = (top + bottom) / 2
            if not bands or abs(cy - bands[-1][2]) > (bottom - top) * .55:
                bands.append([top, bottom, cy, []])
            else:
                bands[-1][0] = min(bands[-1][0], top)
                bands[-1][1] = max(bands[-1][1], bottom)
            bands[-1][3].append((box, text, score))
        crops = []
        for top, bottom, _, row_items in bands:
            for a, b in zip(boundaries, boundaries[1:]):
                # Use each detected line's slanted edges, not a horizontal band:
                # the curved receipt can otherwise leak adjacent rows into cells.
                overlaps = [(max(0, min(b, max(p[0] for p in box)) - max(a, min(p[0] for p in box))), box)
                            for box, _, _ in row_items]
                overlap, box = max(overlaps, key=lambda pair: pair[0])
                if overlap < 5:
                    crop = np.full((32, 32, 3), 255, dtype=np.uint8)
                else:
                    x1 = max(a, min(p[0] for p in box))
                    x2 = min(b, max(p[0] for p in box))
                    def edge_y(p, q, x):
                        return p[1] + (q[1]-p[1]) * (x-p[0]) / max(1, q[0]-p[0])
                    quad = np.float32([[x1, edge_y(box[0],box[1],x1)-2],
                                       [x2, edge_y(box[0],box[1],x2)-2],
                                       [x2, edge_y(box[3],box[2],x2)+2],
                                       [x1, edge_y(box[3],box[2],x1)+2]])
                    cw = max(8, round(x2-x1))
                    ch = max(8, round(max(quad[3][1]-quad[0][1], quad[2][1]-quad[1][1])))
                    target_quad = np.float32([[0,0],[cw-1,0],[cw-1,ch-1],[0,ch-1]])
                    crop = cv2.warpPerspective(corrected, cv2.getPerspectiveTransform(quad,target_quad), (cw,ch), borderValue=(255,255,255))
                crops.append(cv2.copyMakeBorder(crop, 3, 3, 3, 3, cv2.BORDER_CONSTANT, value=(255,255,255)))
        recognized, _ = engine.text_rec(crops)
        count = len(boundaries) - 1
        grid = {"columns": count, "rows": []}
        for start in range(0, len(recognized), count):
            cells = recognized[start:start+count]
            grid["rows"].append({"cells": [text for text, _ in cells], "scores": [float(score) for _, score in cells]})
    import tempfile
    preview = Path(tempfile.gettempdir()) / "tracereel-ocr"
    preview.mkdir(exist_ok=True)
    import uuid
    target = preview / (uuid.uuid4().hex + ".png")
    cv2.imencode(".png", corrected)[1].tofile(str(target))
    # Only our previews, kept bounded across runs.
    for old in sorted(preview.glob("*.png"), key=lambda p: p.stat().st_mtime)[:-10]:
        try:
            old.unlink()
        except OSError:
            pass
    grid["preview"] = target.as_uri()
    return grid


if __name__ == "__main__":
    try:
        sys.stderr.reconfigure(encoding="utf-8")
        request = json.loads(sys.stdin.buffer.read().decode("utf-8"))
        # Third-party logs must never corrupt the JSON protocol.
        from contextlib import redirect_stdout
        with redirect_stdout(sys.stderr):
            result = recognize(request)
        print(json.dumps(result, ensure_ascii=True))
    except Exception as exc:
        print(str(exc), file=sys.stderr)
        sys.exit(1)
