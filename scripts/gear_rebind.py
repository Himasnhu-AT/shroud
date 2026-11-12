#!/usr/bin/env python3
"""Rewrite a gear GLB's inverse bind matrices from the character's own rest skeleton.

    python3 -I scripts/gear_rebind.py <character.glb> <gear.glb>

Blender re-rolls bones on export, so the gear's joint frames differ from the character's, and
the import/export round trip leaves the gear in a turned copy of the character's mesh space
(the axis swap plus the root's re-imported tilt). The rigid transform between the two is solved
from matching joint positions (residual checked) and applied to the gear's positions, normals,
tangents and bounds first. The game skins gear with the character's joints (by
name), so each gear joint's inverse bind must be the character's: the inverse of that joint's
rest world transform (from the node hierarchy). Checked against the character's own skins.
Standard library only; the gear file is rewritten in place.
"""
import json
import struct
import sys


def read_glb(path):
    b = open(path, "rb").read()
    n = struct.unpack_from("<I", b, 12)[0]
    j = json.loads(b[20:20 + n])
    off = 20 + n
    ln = struct.unpack_from("<I", b, off)[0]
    return j, bytearray(b[off + 8:off + 8 + ln])


def write_glb(path, j, binary):
    js = json.dumps(j, separators=(",", ":")).encode()
    js += b" " * ((4 - len(js) % 4) % 4)
    binary = bytes(binary) + b"\0" * ((4 - len(binary) % 4) % 4)
    total = 12 + 8 + len(js) + 8 + len(binary)
    out = struct.pack("<III", 0x46546C67, 2, total) + struct.pack("<II", len(js), 0x4E4F534A) + js + struct.pack("<II", len(binary), 0x004E4942) + binary
    open(path, "wb").write(out)


def mat_mul(a, b):  # column-major 4x4
    return [sum(a[k * 4 + r] * b[c * 4 + k] for k in range(4)) for c in range(4) for r in range(4)]


def trs(node):
    if "matrix" in node:
        return list(node["matrix"])
    t = node.get("translation", [0, 0, 0])
    x, y, z, w = node.get("rotation", [0, 0, 0, 1])
    s = node.get("scale", [1, 1, 1])
    r = [
        1 - 2 * (y * y + z * z), 2 * (x * y + z * w), 2 * (x * z - y * w),
        2 * (x * y - z * w), 1 - 2 * (x * x + z * z), 2 * (y * z + x * w),
        2 * (x * z + y * w), 2 * (y * z - x * w), 1 - 2 * (x * x + y * y),
    ]
    return [r[0] * s[0], r[1] * s[0], r[2] * s[0], 0, r[3] * s[1], r[4] * s[1], r[5] * s[1], 0, r[6] * s[2], r[7] * s[2], r[8] * s[2], 0, t[0], t[1], t[2], 1]


def invert_affine(m):
    # Rotation-scale block inverse via the adjugate (column-major).
    a = [[m[c * 4 + r] for c in range(3)] for r in range(3)]
    det = a[0][0] * (a[1][1] * a[2][2] - a[1][2] * a[2][1]) - a[0][1] * (a[1][0] * a[2][2] - a[1][2] * a[2][0]) + a[0][2] * (a[1][0] * a[2][1] - a[1][1] * a[2][0])
    inv = [[0.0] * 3 for _ in range(3)]
    for r in range(3):
        for c in range(3):
            rr = [i for i in range(3) if i != c]
            cc = [i for i in range(3) if i != r]
            minor = a[rr[0]][cc[0]] * a[rr[1]][cc[1]] - a[rr[0]][cc[1]] * a[rr[1]][cc[0]]
            inv[r][c] = (-1) ** (r + c) * minor / det
    t = [m[12], m[13], m[14]]
    it = [-sum(inv[r][k] * t[k] for k in range(3)) for r in range(3)]
    out = [0.0] * 16
    for c in range(3):
        for r in range(3):
            out[c * 4 + r] = inv[r][c]
    out[12:15] = it
    out[15] = 1.0
    return out


def rest_worlds(j):
    """Name -> rest world matrix of every node (scene roots at identity)."""
    nodes = j["nodes"]
    parent = {}
    for i, n in enumerate(nodes):
        for c in n.get("children", []):
            parent[c] = i
    cache = {}

    def world(i):
        if i not in cache:
            local = trs(nodes[i])
            cache[i] = mat_mul(world(parent[i]), local) if i in parent else local
        return cache[i]

    return {n.get("name"): world(i) for i, n in enumerate(nodes)}, cache


def sub(a, b):
    return [x - y for x, y in zip(a, b)]


def cross(a, b):
    return [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]


def unit(a):
    n = sum(x * x for x in a) ** 0.5
    return [x / n for x in a]


def frame(p0, p1, p2):
    """Orthonormal frame (rows) from three points."""
    x = unit(sub(p1, p0))
    z = unit(cross(x, sub(p2, p0)))
    return [x, cross(z, x), z]


def solve_rigid(char_w, gear_w):
    """R, t with char = R * gear + t, from the joints both files have (pelvis, head, a hand)."""
    pos = lambda w, n: [w[n][12], w[n][13], w[n][14]]
    keys = ("pelvis", "head", "hand_l")
    fc = frame(*(pos(char_w, k) for k in keys))
    fg = frame(*(pos(gear_w, k) for k in keys))
    # R = Fc^T * Fg (frames as rows).
    R = [[sum(fc[k][r] * fg[k][c] for k in range(3)) for c in range(3)] for r in range(3)]
    apply = lambda v: [sum(R[r][c] * v[c] for c in range(3)) for r in range(3)]
    t = sub(pos(char_w, "pelvis"), apply(pos(gear_w, "pelvis")))
    worst = max(max(abs(x) for x in sub(pos(char_w, n), [a + b for a, b in zip(apply(pos(gear_w, n)), t)])) for n in gear_w if n in char_w and n)
    return R, t, worst


def transform_meshes(j, binary, R, t):
    """Positions by R, t; normals and tangents by R; once per accessor; bounds updated."""
    done = set()
    for m in j["meshes"]:
        for pr in m["primitives"]:
            for key in ("POSITION", "NORMAL", "TANGENT"):
                ai = pr["attributes"].get(key)
                if ai is None or ai in done:
                    continue
                done.add(ai)
                a = j["accessors"][ai]
                assert a["componentType"] == 5126, f"{key} is not float"
                width = 4 if a["type"] == "VEC4" else 3
                bv = j["bufferViews"][a["bufferView"]]
                stride = bv.get("byteStride", 4 * width)
                o = bv.get("byteOffset", 0) + a.get("byteOffset", 0)
                lo, hi = [1e30] * 3, [-1e30] * 3
                for i in range(a["count"]):
                    at = o + i * stride
                    v = struct.unpack_from("<3f", binary, at)
                    w = [sum(R[r][c] * v[c] for c in range(3)) for r in range(3)]
                    if key == "POSITION":
                        w = [x + y for x, y in zip(w, t)]
                    struct.pack_into("<3f", binary, at, *w)
                    lo = [min(p, q) for p, q in zip(lo, w)]
                    hi = [max(p, q) for p, q in zip(hi, w)]
                if "min" in a:
                    a["min"][:3], a["max"][:3] = lo, hi


def check_bounds(cj, gj):
    """Every gear mesh sits within the character's body bounds (plus a pack's depth)."""
    body = next(m for m in cj["meshes"] if m["name"].startswith("body"))
    a = cj["accessors"][body["primitives"][0]["attributes"]["POSITION"]]
    blo, bhi = a["min"], a["max"]
    for m in gj["meshes"]:
        g = gj["accessors"][m["primitives"][0]["attributes"]["POSITION"]]
        for k in range(3):
            if g["min"][k] < blo[k] - 0.45 or g["max"][k] > bhi[k] + 0.45:
                sys.exit(f"gear mesh {m['name']} is outside the body ({g['min']}..{g['max']} vs {blo}..{bhi})")


def main():
    char_path, gear_path = sys.argv[1], sys.argv[2]
    cj, cbin = read_glb(char_path)
    gj, gbin = read_glb(gear_path)
    worlds, _ = rest_worlds(cj)
    # The character's meshes sit at the scene root with no transform: mesh space = world.
    for n in cj["nodes"]:
        if "mesh" in n:
            assert not any(k in n for k in ("matrix", "rotation", "translation", "scale")), "character mesh node has a transform"
    # Sanity: our rest-pose inverse binds reproduce the character's own.
    worst = 0.0
    for sk in cj.get("skins", []):
        a = cj["accessors"][sk["inverseBindMatrices"]]
        bv = cj["bufferViews"][a["bufferView"]]
        o = bv.get("byteOffset", 0) + a.get("byteOffset", 0)
        for k, ji in enumerate(sk["joints"]):
            have = struct.unpack_from("<16f", cbin, o + 64 * k)
            want = invert_affine(worlds[cj["nodes"][ji]["name"]])
            worst = max(worst, max(abs(x - y) for x, y in zip(have, want)))
    if worst > 1e-3:
        sys.exit(f"rest skeleton does not reproduce the character's inverse binds (off by {worst})")
    gworlds, _ = rest_worlds(gj)
    R, t, resid = solve_rigid(worlds, gworlds)
    if resid > 2e-3:
        sys.exit(f"gear skeleton does not match the character's (joint residual {resid:.4f} m)")
    transform_meshes(gj, gbin, R, t)
    check_bounds(cj, gj)
    n = 0
    for sk in gj["skins"]:
        a = gj["accessors"][sk["inverseBindMatrices"]]
        bv = gj["bufferViews"][a["bufferView"]]
        o = bv.get("byteOffset", 0) + a.get("byteOffset", 0)
        for k, ji in enumerate(sk["joints"]):
            name = gj["nodes"][ji]["name"]
            if name not in worlds:
                sys.exit(f"gear joint {name} is not in the character")
            struct.pack_into("<16f", gbin, o + 64 * k, *invert_affine(worlds[name]))
            n += 1
    write_glb(gear_path, gj, gbin)
    print(f"rebind: {n} joint binds from {char_path} (check {worst:.1e}, joint residual {resid * 1000:.2f} mm)")


main()
