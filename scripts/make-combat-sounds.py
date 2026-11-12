#!/usr/bin/env python3
"""Synthesize Tells' combat sound layers (our own, CC0) into assets/audio/*.ogg.

  python3 -I scripts/make-combat-sounds.py [--tmp DIR]

- bullet_crack_<i>.ogg: a supersonic bullet passing close by. An N-wave (the shock front:
  a near-instant rise, a linear fall through zero, a near-instant return; ~0.4 ms long) with
  a short bright whiz tail and a faint downward-swept hiss (the bullet's wake).
- shot_tail_indoor.ogg: what a room adds to a gunshot: early reflections at 6-35 ms off walls
  and ceiling, then a dense, fast-decaying (RT60 ~0.45 s) darkened tail.
- shot_tail_outdoor.ogg: the open-air tail: a low rumble rolling away over ~1.6 s with two
  slap-back echoes from distant buildings.

Standard library only (wave + math + random), seeded so the files are reproducible; the WAVs are
encoded to Vorbis with oggenc (or ffmpeg). Nothing is downloaded.
"""
import math, os, random, shutil, subprocess, sys, tempfile, wave

RATE = 48000
ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OUT = os.path.join(ROOT, "assets", "audio")


def lowpass(x, hz):
    a = 1.0 - math.exp(-2.0 * math.pi * hz / RATE)
    y, s = [], 0.0
    for v in x:
        s += a * (v - s)
        y.append(s)
    return y


def highpass(x, hz):
    lp = lowpass(x, hz)
    return [a - b for a, b in zip(x, lp)]


def normalise(x, peak=0.89):
    m = max(1e-9, max(abs(v) for v in x))
    return [v * peak / m for v in x]


def write(name, x, tmp):
    wav = os.path.join(tmp, name + ".wav")
    with wave.open(wav, "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(RATE)
        w.writeframes(b"".join(int(max(-1.0, min(1.0, v)) * 32767).to_bytes(2, "little", signed=True) for v in x))
    out = os.path.join(OUT, name + ".ogg")
    if shutil.which("oggenc"):
        subprocess.run(["oggenc", "-Q", "-q", "5", "-o", out, wav], check=True)
    else:  # ffmpeg's built-in (experimental, stereo-only) Vorbis encoder
        subprocess.run(["ffmpeg", "-y", "-loglevel", "error", "-i", wav, "-c:a", "vorbis", "-strict", "-2", "-ac", "2", "-q:a", "5", out], check=True)
    print(out, f"{len(x) / RATE:.2f} s")


def crack(seed):
    rng = random.Random(seed)
    n = int(RATE * 0.28)
    x = [0.0] * n
    # N-wave: duration grows a little with distance / calibre.
    dur = int(RATE * rng.uniform(0.00035, 0.0005))
    start = int(RATE * 0.004)
    for i in range(dur):
        x[start + i] = 1.0 - 2.0 * i / dur
    # Whiz: band-limited noise, fast decay, pitch falling as the bullet passes.
    noise = [rng.uniform(-1, 1) for _ in range(n)]
    bright = highpass(noise, 2500)
    for i in range(n):
        t = i / RATE
        env = math.exp(-t / 0.018) * 0.22 + math.exp(-t / 0.09) * 0.05
        x[i] += bright[i] * env
    # Wake: a hiss swept down (amplitude-modulated noise through a lowering low-pass).
    wake, s = [], 0.0
    for i in range(n):
        t = i / RATE
        hz = 5000 * math.exp(-t / 0.06) + 600
        a = 1.0 - math.exp(-2.0 * math.pi * hz / RATE)
        s += a * (noise[(i * 7) % n] - s)
        wake.append(s * math.exp(-t / 0.07) * 0.12)
    x = [a + b for a, b in zip(x, wake)]
    return normalise(x, 0.95)


def tail_indoor():
    rng = random.Random(7)
    n = int(RATE * 0.7)
    x = [0.0] * n
    # Early reflections: discrete, each a little darker and quieter.
    for ms, g in [(6, 0.55), (9, 0.45), (14, 0.4), (19, 0.33), (24, 0.3), (31, 0.24), (35, 0.2)]:
        i = int(RATE * ms / 1000)
        for k in range(int(RATE * 0.003)):
            x[i + k] += g * rng.uniform(-1, 1) * math.exp(-k / (RATE * 0.0008))
    # Dense late tail.
    rt60 = 0.45
    noise = lowpass([rng.uniform(-1, 1) for _ in range(n)], 3200)
    for i in range(int(RATE * 0.02), n):
        t = i / RATE - 0.02
        x[i] += noise[i] * 0.7 * math.exp(-6.91 * t / rt60) * min(1.0, t / 0.015)
    x = highpass(x, 120)
    return normalise(x, 0.8)


def tail_outdoor():
    rng = random.Random(11)
    n = int(RATE * 1.9)
    noise = [rng.uniform(-1, 1) for _ in range(n)]
    rumble = lowpass(lowpass(noise, 420), 420)
    x = [0.0] * n
    for i in range(n):
        t = i / RATE
        x[i] = rumble[i] * math.exp(-t / 0.5) * min(1.0, t / 0.04) * 1.6
    # Slap-back echoes off far walls: a darkened copy of a short burst.
    burst = lowpass([rng.uniform(-1, 1) * math.exp(-k / (RATE * 0.01)) for k in range(int(RATE * 0.06))], 1500)
    for delay, g in [(0.23, 0.35), (0.61, 0.18)]:
        i0 = int(RATE * delay)
        for k, v in enumerate(burst):
            if i0 + k < n:
                x[i0 + k] += v * g * 3.0
    return normalise(x, 0.8)


def main():
    tmp = sys.argv[sys.argv.index("--tmp") + 1] if "--tmp" in sys.argv else tempfile.mkdtemp(prefix="combat-sounds-")
    os.makedirs(tmp, exist_ok=True)
    for i in range(3):
        write(f"bullet_crack_{i}", crack(100 + i), tmp)
    write("shot_tail_indoor", tail_indoor(), tmp)
    write("shot_tail_outdoor", tail_outdoor(), tmp)


if __name__ == "__main__":
    main()
