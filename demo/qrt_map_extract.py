# -*- coding: utf-8 -*-
"""从 QuickRoute .qrt 工程文件里提取内嵌的地图图片。

规则：先定位候选图片起点，再**按格式自身的段结构走**到结尾。
注意不能用 data.find(b"\\xff\\xd9") 找 JPEG 结尾——文件头部含 Photoshop
缩略图，里面有 `FF FF D9`，会提前命中，只能拿到 8 KB 的缩略图。

用法：
    python qrt_extract_map.py "AM 男子短距 第一面.qrt" out.jpg
"""
import struct
import sys


def jpeg_extent(data, i):
    """从 data[i] 开始（须是 SOI）走 JPEG 段标记链，返回 (总长, 宽, 高)。"""
    if data[i:i + 2] != b"\xff\xd8":
        return None
    p = i + 2
    w = h = None
    while p + 1 < len(data):
        if data[p] != 0xFF:
            return None
        m = data[p + 1]
        if m == 0xFF:
            p += 1
            continue
        if m == 0xD8 or m == 0x01 or 0xD0 <= m <= 0xD7:
            p += 2
            continue
        if m == 0xD9:
            return (p + 2 - i, w, h)
        if m == 0xDA:                      # 熵编码区：扫到下一个真标记
            p += 2 + struct.unpack_from(">H", data, p + 2)[0]
            while p + 1 < len(data):
                if data[p] == 0xFF and data[p + 1] != 0x00 and not (0xD0 <= data[p + 1] <= 0xD7):
                    break
                p += 1
            continue
        ln = struct.unpack_from(">H", data, p + 2)[0]
        if 0xC0 <= m <= 0xCF and m not in (0xC4, 0xC8, 0xCC):
            h, w = struct.unpack_from(">HH", data, p + 5)
        p += 2 + ln
    return None


def png_extent(data, i):
    """从 data[i] 开始（须是 PNG 签名）走数据块到 IEND。"""
    if data[i:i + 8] != b"\x89PNG\r\n\x1a\n":
        return None
    p = i + 8
    w = h = None
    while p + 12 <= len(data):
        ln = struct.unpack_from(">I", data, p)[0]
        typ = data[p + 4:p + 8]
        if typ == b"IHDR":
            w, h = struct.unpack_from(">II", data, p + 8)
        p += 12 + ln
        if typ == b"IEND":
            return (p - i, w, h)
    return None


def extract(path, out, min_size=10000):
    b = open(path, "rb").read()
    best = None
    for sig, fn, kind in ((b"\xff\xd8\xff", jpeg_extent, "jpeg"),
                          (b"\x89PNG\r\n\x1a\n", png_extent, "png")):
        i = 0
        while True:
            i = b.find(sig, i)
            if i < 0:
                break
            r = fn(b, i)
            if r and r[0] >= min_size:
                print("  候选 %s @%d  长度=%d  %sx%s" % (kind, i, r[0], r[1], r[2]))
                if best is None or r[0] > best[1][0]:
                    best = (i, r, kind)
            i += 1
    if best is None:
        raise SystemExit("未找到内嵌地图图片")
    i, (size, w, h), kind = best
    data = b[i:i + size]
    open(out, "wb").write(data)
    print("已写出 %s：%d 字节，%sx%s（%s）" % (out, size, w, h, kind))
    return out


if __name__ == "__main__":
    if len(sys.argv) < 3:
        raise SystemExit(__doc__)
    extract(sys.argv[1], sys.argv[2])
