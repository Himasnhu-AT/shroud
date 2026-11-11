#!/usr/bin/env python3
"""Shrink KTX2 textures to a maximum side by dropping their top mip levels.

    python3 -I scripts/ktx2_trim.py --max 2048 DIR_OR_FILE...      # rewrite in place
    python3 -I scripts/ktx2_trim.py --max 2048 IN.ktx2 --out OUT.ktx2
    python3 -I scripts/ktx2_trim.py --check DIR_OR_FILE...         # validate only

Used by scripts/stage-assets.sh on the *staged* copy of assets/ (originals stay 4K). No re-encode:
the kept levels are copied byte for byte, so a 4096² file with 13 levels becomes 2048² with 12.
Rewrites the header (pixelWidth/Height, levelCount), the level index and the section offsets;
DFD, KVD and SGD are kept. Level data is laid out smallest first with the alignment the spec asks
for (1 when supercompressed, else lcm(texel block size, 4)). Files already within the limit, with
no mip chain, 3D, or BasisLZ (its SGD indexes every level) are left alone.
"""
import argparse
import math
import os
import struct
import sys
from pathlib import Path

MAGIC = bytes([0xAB, 0x4B, 0x54, 0x58, 0x20, 0x32, 0x30, 0xBB, 0x0D, 0x0A, 0x1A, 0x0A])
BASIS_LZ, ZSTD = 1, 2


class Ktx2:
    def __init__(self, data: bytes):
        if data[:12] != MAGIC:
            raise ValueError("not a KTX2 file")
        self.data = data
        (self.vk, self.type_size, self.w, self.h, self.depth, self.layers, self.faces, self.n,
         self.scheme) = struct.unpack_from("<9I", data, 12)
        self.dfd_off, self.dfd_len, self.kvd_off, self.kvd_len = struct.unpack_from("<4I", data, 48)
        self.sgd_off, self.sgd_len = struct.unpack_from("<2Q", data, 64)
        self.levels = [struct.unpack_from("<3Q", data, 80 + 24 * i) for i in range(max(self.n, 1))]
        dfd = data[self.dfd_off:self.dfd_off + self.dfd_len]
        # Basic DFD: texelBlockDimension at +12 (each minus one), bytesPlane0 at +16 (after dfdTotalSize).
        self.block_w, self.block_h = dfd[16] + 1, dfd[17] + 1
        self.block_bytes = dfd[20]

    def side(self, s, level):
        return max(1, s >> level)

    def level_bytes(self, level):
        """Uncompressed size of one level: every layer and face (bytesPlane0 0 = unsized: unknown)."""
        if not self.block_bytes:
            return None
        bw = -(-self.side(self.w, level) // self.block_w)
        bh = -(-self.side(self.h, level) // self.block_h)
        return bw * bh * self.block_bytes * max(self.layers, 1) * self.faces

    def align(self):
        return 1 if self.scheme else math.lcm(self.block_bytes or 4, 4)


def check(k: Ktx2):
    """Structural checks; raises ValueError with what is wrong."""
    d = k.data
    if k.n == 0:
        return
    if k.w >> (k.n - 1) == 0 and k.h >> (k.n - 1) == 0:
        raise ValueError(f"{k.n} levels is too many for {k.w}x{k.h}")
    spans = []
    for i, (off, length, raw) in enumerate(k.levels):
        if off + length > len(d):
            raise ValueError(f"level {i} runs past the end of the file")
        if off % k.align():
            raise ValueError(f"level {i} offset {off} not aligned to {k.align()}")
        want = k.level_bytes(i)
        if want is not None and k.scheme != BASIS_LZ and raw != want:
            raise ValueError(f"level {i} uncompressed length {raw}, expected {want}")
        if not k.scheme and length != raw:
            raise ValueError(f"level {i} byteLength {length} != uncompressed {raw}")
        spans.append((off, off + length))
    spans.sort()
    for (a0, a1), (b0, _) in zip(spans, spans[1:]):
        if b0 < a1:
            raise ValueError("level data overlaps")
    if [o for o, _, _ in k.levels] != sorted((o for o, _, _ in k.levels), reverse=True):
        raise ValueError("levels not stored smallest first")
    if spans[-1][1] != len(d):
        raise ValueError(f"{len(d) - spans[-1][1]} trailing bytes after the level data")
    header_end = 80 + 24 * k.n
    if k.dfd_off != header_end:
        raise ValueError(f"DFD at {k.dfd_off}, expected right after the level index ({header_end})")
    if k.scheme == ZSTD:
        try:
            from compression import zstd  # Python 3.14+
        except ImportError:
            return
        for i, (off, length, raw) in enumerate(k.levels):
            if len(zstd.decompress(d[off:off + length])) != raw:
                raise ValueError(f"level {i} does not decompress to {raw} bytes")


def trim(k: Ktx2, max_side: int):
    """Returns the trimmed file bytes, or None when nothing should change."""
    if k.n < 2 or max(k.w, k.h) <= max_side or k.depth > 1 or k.scheme == BASIS_LZ:
        return None
    drop = 0
    while max(k.side(k.w, drop), k.side(k.h, drop)) > max_side and drop + 1 < k.n:
        drop += 1
    # Keep the new base level whole blocks (what the engine's cap_texture_size does too).
    while drop and (k.side(k.w, drop) % k.block_w or k.side(k.h, drop) % k.block_h):
        drop -= 1
    if not drop:
        return None
    d, n = k.data, k.n - drop
    dfd = d[k.dfd_off:k.dfd_off + k.dfd_len]
    kvd = d[k.kvd_off:k.kvd_off + k.kvd_len] if k.kvd_len else b""
    sgd = d[k.sgd_off:k.sgd_off + k.sgd_len] if k.sgd_len else b""

    out = bytearray(80 + 24 * n)
    pad = lambda m: out.extend(b"\0" * (-len(out) % m))
    dfd_off = len(out)
    out += dfd
    kvd_off = 0
    if kvd:
        pad(4)
        kvd_off = len(out)
        out += kvd
    sgd_off = 0
    if sgd:
        pad(8)
        sgd_off = len(out)
        out += sgd
    index = [None] * n
    for new in reversed(range(n)):  # smallest level first
        off, length, raw = k.levels[new + drop]
        pad(k.align())
        index[new] = (len(out), length, raw)
        out += d[off:off + length]

    out[:12] = MAGIC
    struct.pack_into("<9I", out, 12, k.vk, k.type_size, k.side(k.w, drop), k.side(k.h, drop) if k.h else 0,
                     k.depth, k.layers, k.faces, n, k.scheme)
    struct.pack_into("<4I", out, 48, dfd_off, len(dfd), kvd_off, len(kvd))
    struct.pack_into("<2Q", out, 64, sgd_off, len(sgd))
    for i, entry in enumerate(index):
        struct.pack_into("<3Q", out, 80 + 24 * i, *entry)
    return bytes(out)


def files(paths):
    for p in map(Path, paths):
        if p.is_dir():
            yield from sorted(p.rglob("*.ktx2"))
        else:
            yield p


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("paths", nargs="+", help="KTX2 files or folders (searched recursively)")
    ap.add_argument("--max", type=int, help="largest side to keep, e.g. 2048")
    ap.add_argument("--out", help="write here instead of in place (one input file only)")
    ap.add_argument("--check", action="store_true", help="validate only, change nothing")
    ap.add_argument("-q", "--quiet", action="store_true")
    a = ap.parse_args()
    if not a.check and not a.max:
        ap.error("--max or --check")
    todo = list(files(a.paths))
    if a.out and len(todo) != 1:
        ap.error("--out takes exactly one input file")
    failed = saved = 0
    for f in todo:
        try:
            k = Ktx2(f.read_bytes())
            check(k)
            if a.check:
                not a.quiet and print(f"ok {f}: {k.w}x{k.h}, {k.n} levels")
                continue
            new = trim(k, a.max)
            dest = Path(a.out) if a.out else f
            if new is None:
                if a.out:
                    dest.write_bytes(k.data)
                continue
            t = Ktx2(new)
            check(t)
            tmp = dest.with_name(dest.name + ".tmp")
            tmp.write_bytes(new)
            os.replace(tmp, dest)
            saved += len(k.data) - len(new)
            not a.quiet and print(f"{f}: {k.w}x{k.h} {k.n} levels -> {t.w}x{t.h} {t.n} levels, "
                                  f"{len(k.data) / 1e6:.1f} -> {len(new) / 1e6:.1f} MB")
        except (ValueError, struct.error, IndexError) as e:
            failed += 1
            print(f"error: {f}: {e}", file=sys.stderr)
    if a.max and not a.check:
        print(f"ktx2_trim: {saved / 1e6:.1f} MB saved")
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
