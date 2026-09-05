#!/usr/bin/env python3
"""Quantise Natural Earth 110m outer rings into the YTTA v1 binary."""

import json
import struct
import sys


def quantise(value):
    return round(value * 100)


def unwrap(points):
    """Remove +-360 jumps so consecutive lons differ by <= 180."""
    out = [points[0]]
    shift = 0
    for lon, lat in points[1:]:
        prev = out[-1][0]
        delta = (lon + shift) - prev
        if delta > 18000:
            shift -= 36000
        elif delta < -18000:
            shift += 36000
        out.append((lon + shift, lat))
    return out


def signed_area_and_centroid(points):
    area2 = 0.0
    cx = 0.0
    cy = 0.0
    for (x0, y0), (x1, y1) in zip(points, points[1:] + points[:1]):
        cross = x0 * y1 - x1 * y0
        area2 += cross
        cx += (x0 + x1) * cross
        cy += (y0 + y1) * cross
    area = area2 / 2.0
    if area2 == 0:
        return 0.0, points[0]
    return area, (cx / (3.0 * area2), cy / (3.0 * area2))


def zigzag_varint(value):
    value = (value << 1) ^ (value >> 31)
    out = bytearray()
    while True:
        byte = value & 0x7F
        value >>= 7
        if value:
            out.append(byte | 0x80)
        else:
            out.append(byte)
            return bytes(out)


def encode_name(name):
    raw = name.encode("utf-8")[:60]
    while True:
        try:
            raw.decode("utf-8")
            return raw
        except UnicodeDecodeError:
            raw = raw[:-1]


def main():
    src, dst = sys.argv[1], sys.argv[2]
    features = json.load(open(src, encoding="utf-8"))["features"]
    out = bytearray(b"YTTA\x01" + struct.pack("<H", len(features)))
    total_points = 0
    for feature in features:
        code = feature["properties"]["code"]
        code = b"--" if code == "-99" else code.encode("ascii")[:2].ljust(2, b" ")
        name = encode_name(feature["properties"]["name"])
        geometry = feature["geometry"]
        polys = (
            [geometry["coordinates"]]
            if geometry["type"] == "Polygon"
            else geometry["coordinates"]
        )
        rings = []
        for poly in polys:
            outer = poly[0]
            raw = [(quantise(pt[0]), quantise(pt[1])) for pt in outer]
            deduped = [raw[0]]
            for pt in raw[1:]:
                if pt != deduped[-1]:
                    deduped.append(pt)
            if len(deduped) > 1 and deduped[-1] == deduped[0]:
                deduped.pop()
            if len(deduped) >= 3:
                rings.append(deduped)
        bbox = (
            min(min(p[0] for p in ring) for ring in rings),
            min(min(p[1] for p in ring) for ring in rings),
            max(max(p[0] for p in ring) for ring in rings),
            max(max(p[1] for p in ring) for ring in rings),
        )
        largest = max(rings, key=lambda ring: abs(signed_area_and_centroid(unwrap(ring))[0]))
        _, (cx, cy) = signed_area_and_centroid(unwrap(largest))
        cx = (cx + 18000) % 36000 - 18000
        cx = 18000 if cx == -18000 else cx
        out += code + bytes([len(name)]) + name
        out += struct.pack("<hhhhhh", round(cy), round(cx), *bbox)
        out += struct.pack("<H", len(rings))
        for ring in rings:
            out += struct.pack("<H", len(ring))
            out += struct.pack("<hh", ring[0][0], ring[0][1])
            prev = ring[0]
            for pt in ring[1:]:
                out += zigzag_varint(pt[0] - prev[0])
                out += zigzag_varint(pt[1] - prev[1])
                prev = pt
            total_points += len(ring)
    open(dst, "wb").write(bytes(out))
    print(f"features={len(features)} points={total_points} bytes={len(out)}")


if __name__ == "__main__":
    main()
