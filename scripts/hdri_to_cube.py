"""Radiance .hdr (equirect) -> KTX2 cubemap (RGBA16F, zstd), in Shroud's engine cube convention.

Needs Python 3.14+ (compression.zstd). The shipped skies were made with (from the repo root):
  python3 -I scripts/hdri_to_cube.py assets/hdri/kloofendal_48d_partly_cloudy_puresky_2k.hdr assets/sky/sirocco_kloofendal_48d.ktx2 --size 512 --sun-az 205 --sky-mean 0.55 --ground 0.30,0.26,0.20
  python3 -I scripts/hdri_to_cube.py assets/hdri/overcast_soil_puresky_2k.hdr assets/sky/gantry_overcast_soil.ktx2 --size 512 --sun-az 105 --sky-mean 0.6 --ground 0.24,0.25,0.26 --clamp 4
  python3 -I scripts/hdri_to_cube.py assets/hdri/abandoned_factory_canteen_01_2k.hdr assets/sky/office_factory_canteen.ktx2 --size 256 --sky-mean 0.5 --clamp 16

usage: python3 -I scripts/hdri_to_cube.py IN.hdr OUT.ktx2 --size 512 [--sun-az DEG] [--sky-mean 0.55]
        [--ground R,G,B] [--clamp 12] [--preview OUT.ppm]

- Normalises so the mean luminance of the upper hemisphere (after clamping) is --sky-mean.
- Clamps texels to --clamp x that mean (removes the sun hot spot: the scene's DirectionalLight
  is the sun; a second one in the env map would double it).
- --sun-az rotates the sky about Y so its brightest point sits at that engine azimuth
  (SunLook::direction convention), so the visible sun glow matches the shadows.
- --ground replaces everything below the horizon with that linear colour (relative units),
  blended over a few degrees (puresky HDRIs have no real ground).
"""
import math, struct, sys
from compression import zstd

def read_hdr(path):
    f = open(path, 'rb')
    data = f.read()
    # header
    i = 0
    w = h = None
    while True:
        j = data.index(b'\n', i)
        line = data[i:j]
        i = j + 1
        if line.startswith(b'-Y '):
            p = line.split()
            h, w = int(p[1]), int(p[3])
            break
    px = [None] * (w * h)
    pos = i
    out = bytearray(w * h * 4)
    for y in range(h):
        if data[pos] == 2 and data[pos + 1] == 2 and (data[pos + 2] << 8 | data[pos + 3]) == w:
            pos += 4
            row = [bytearray(w) for _ in range(4)]
            for c in range(4):
                x = 0
                r = row[c]
                while x < w:
                    n = data[pos]; pos += 1
                    if n > 128:
                        n -= 128
                        v = data[pos]; pos += 1
                        r[x:x + n] = bytes([v]) * n
                        x += n
                    else:
                        r[x:x + n] = data[pos:pos + n]
                        pos += n
                        x += n
            base = y * w * 4
            for x in range(w):
                out[base + x * 4:base + x * 4 + 4] = bytes((row[0][x], row[1][x], row[2][x], row[3][x]))
        else:
            # flat (uncompressed) scanline
            out[y * w * 4:(y + 1) * w * 4] = data[pos:pos + w * 4]
            pos += w * 4
    # decode RGBE to float triplets
    fl = [0.0] * (w * h * 3)
    for k in range(w * h):
        e = out[k * 4 + 3]
        if e:
            s = math.ldexp(1.0, e - 136)
            fl[k * 3] = out[k * 4] * s
            fl[k * 3 + 1] = out[k * 4 + 1] * s
            fl[k * 3 + 2] = out[k * 4 + 2] * s
    return w, h, fl

def lum(r, g, b):
    return 0.2126 * r + 0.7152 * g + 0.0722 * b

def main():
    a = sys.argv[1:]
    src, dst = a[0], a[1]
    opt = lambda k, d: a[a.index(k) + 1] if k in a else d
    size = int(opt('--size', '512'))
    sun_az = opt('--sun-az', None)
    sky_mean = float(opt('--sky-mean', '0.55'))
    clamp_k = float(opt('--clamp', '12'))
    ground = opt('--ground', None)
    ground = tuple(float(x) for x in ground.split(',')) if ground else None
    preview = opt('--preview', None)
    w, h, fl = read_hdr(src)
    print('read', w, h)
    # Equirect: u -> lon in [-pi, pi), v -> lat; direction in the engine's frame:
    # (cos lat sin lon, sin lat, -cos lat cos lon)  (lon = azimuth like SunLook).
    # Brightest point (smoothed by taking upper-hemisphere max luminance).
    best, bu, bv = -1, 0, 0
    tot, cnt = 0.0, 0
    for v in range(h // 2):
        for u in range(0, w, 2):
            k = (v * w + u) * 3
            L = lum(fl[k], fl[k + 1], fl[k + 2])
            if L > best:
                best, bu, bv = L, u, v
    sun_lon = ((bu + 0.5) / w) * 2 * math.pi - math.pi
    sun_lat = math.pi / 2 - (bv + 0.5) / h * math.pi
    print('sun lon %.1f lat %.1f peak %.1f' % (math.degrees(sun_lon), math.degrees(sun_lat), best))
    rot = 0.0
    if sun_az is not None:
        rot = math.radians(float(sun_az)) - sun_lon
    # Mean of the upper hemisphere, robust: clamp at 4x the median first.
    ls = []
    for v in range(0, h // 2, 4):
        for u in range(0, w, 4):
            k = (v * w + u) * 3
            ls.append(lum(fl[k], fl[k + 1], fl[k + 2]))
    ls.sort()
    med = ls[len(ls) // 2]
    cap0 = med * clamp_k
    m = sum(min(x, cap0) for x in ls) / len(ls)
    scale = sky_mean / max(m, 1e-6)
    cap = cap0 * scale
    print('median %.3f mean %.3f scale %.4f cap %.2f' % (med, m, scale, cap))

    def sample(dx, dy, dz):
        lat = math.asin(max(-1.0, min(1.0, dy)))
        lon = math.atan2(dx, -dz) - rot
        uf = ((lon + math.pi) / (2 * math.pi)) % 1.0 * w - 0.5
        vf = (math.pi / 2 - lat) / math.pi * h - 0.5
        u0 = math.floor(uf); v0 = math.floor(vf)
        fu = uf - u0; fv = vf - v0
        r = g = b = 0.0
        for (du, dv, wt) in ((0, 0, (1 - fu) * (1 - fv)), (1, 0, fu * (1 - fv)), (0, 1, (1 - fu) * fv), (1, 1, fu * fv)):
            uu = (u0 + du) % w
            vv = min(max(v0 + dv, 0), h - 1)
            k = (vv * w + uu) * 3
            r += fl[k] * wt; g += fl[k + 1] * wt; b += fl[k + 2] * wt
        r *= scale; g *= scale; b *= scale
        L = lum(r, g, b)
        if L > cap:
            s = cap / L
            r *= s; g *= s; b *= s
        if ground is not None:
            # Below the horizon: the ground colour, blended over ~3 degrees.
            t = max(0.0, min(1.0, (-dy) / 0.05 + 0.5))
            if t > 0:
                r = r * (1 - t) + ground[0] * t
                g = g * (1 - t) + ground[1] * t
                b = b * (1 - t) + ground[2] * t
        return r, g, b

    def texel_dir(face, x, y):
        uu = 2.0 * (x + 0.5) / size - 1.0
        vv = 2.0 * (y + 0.5) / size - 1.0
        c = [(1.0, -vv, -uu), (-1.0, -vv, uu), (uu, 1.0, vv), (uu, -1.0, -vv), (uu, -vv, 1.0), (-uu, -vv, -1.0)][face]
        d = (c[0], c[1], -c[2])
        n = math.sqrt(d[0] ** 2 + d[1] ** 2 + d[2] ** 2)
        return d[0] / n, d[1] / n, d[2] / n

    raw = bytearray()
    pack = struct.Struct('<4e')
    one = 1.0
    for face in range(6):
        for y in range(size):
            for x in range(size):
                r, g, b = sample(*texel_dir(face, x, y))
                raw += pack.pack(min(r, 65000.0), min(g, 65000.0), min(b, 65000.0), one)
        print('face', face)
    if preview:
        # Equirect LDR preview of what was written (Reinhard-ish, sRGB-ish).
        pw, ph = 512, 256
        pv = bytearray()
        for v in range(ph):
            lat = math.pi / 2 - (v + 0.5) / ph * math.pi
            for u in range(pw):
                lon = (u + 0.5) / pw * 2 * math.pi - math.pi
                d = (math.cos(lat) * math.sin(lon), math.sin(lat), -math.cos(lat) * math.cos(lon))
                r, g, b = sample(*d)
                pv += bytes(int(255 * min(1.0, (c / (1 + c)) ** (1 / 2.2))) for c in (r, g, b))
        open(preview, 'wb').write(b'P6 %d %d 255\n' % (pw, ph) + bytes(pv))
    write_ktx2(dst, size, bytes(raw))

def dfd_rgba16f():
    samples = []
    for i, ch in enumerate((0, 1, 2, 15)):
        samples.append((i * 16, 15, ch | 0x80 | 0x40))  # float + signed
    block = 24 + 16 * len(samples)
    d = bytearray()
    d += struct.pack('<I', 4 + block)
    d += struct.pack('<I', 0)
    d += struct.pack('<I', 2 | (block << 16))
    d += bytes([1, 1, 1, 0])  # RGBSDA, BT709, linear
    d += bytes([0, 0, 0, 0])
    d += bytes([8, 0, 0, 0, 0, 0, 0, 0])
    for off, ln, ch in samples:
        d += struct.pack('<H', off) + bytes([ln, ch]) + bytes(4)
        d += struct.pack('<I', 0xBF800000) + struct.pack('<I', 0x3F800000)
    return bytes(d)

def write_ktx2(path, size, raw):
    VK_R16G16B16A16_SFLOAT = 97
    packed = zstd.compress(raw, 19)
    dfd = dfd_rgba16f()
    n = 1
    header_len = 80 + 24 * n
    dfd_off = header_len
    data_off = dfd_off + len(dfd)
    out = bytearray(bytes([0xAB, 0x4B, 0x54, 0x58, 0x20, 0x32, 0x30, 0xBB, 0x0D, 0x0A, 0x1A, 0x0A]))
    for v in (VK_R16G16B16A16_SFLOAT, 2, size, size, 0, 0, 6, n, 2):
        out += struct.pack('<I', v)
    out += struct.pack('<IIII', dfd_off, len(dfd), 0, 0)
    out += struct.pack('<QQ', 0, 0)
    out += struct.pack('<QQQ', data_off, len(packed), len(raw))
    out += dfd
    out += packed
    open(path, 'wb').write(out)
    print('wrote', path, len(out))

main()
