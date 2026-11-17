#!/usr/bin/env python3
"""Synthesize T1K's notification sounds into resources/sounds/.

Each theme is a pair in one timbre: "alert" (brighter, for a tab that needs you or an
error) and "soft" (for done, reminders and program banners). Short and quiet on purpose:
peak about -14 dBFS, under two seconds, bells ring out rather than stop. Pure stdlib,
adapted from Coo's gen-sounds.py. Rerun after tweaking a recipe (`gen-sounds.py bell`
redoes only that theme).
"""
import math, os, random, struct, sys, wave

RATE = 44100
OUT = os.path.join(os.path.dirname(__file__), "..", "resources", "sounds")
PEAK = 10 ** (-14 / 20)


def env(t, attack, decay):
    a = min(1.0, t / attack) if attack > 0 else 1.0
    return a * math.exp(-t / decay)


def bell(freq, dur, decay, partials=((1, 1.0), (2.76, 0.18), (5.4, 0.06)), attack=0.004, start=0.0):
    def f(t):
        t -= start
        if t < 0 or t > dur:
            return 0.0
        return env(t, attack, decay) * sum(a * math.sin(2 * math.pi * freq * m * t) for m, a in partials)
    return f


def drip(u, f0=700.0, f1=1700.0, decay=0.045):
    """A drop: a resonance whose pitch rises fast, with a short tail."""
    if u < 0 or u > 0.25:
        return 0.0
    k = 55.0
    phase = 2 * math.pi * (f1 * u - (f1 - f0) * (1 - math.exp(-k * u)) / k)
    return env(u, 0.0015, decay) * math.sin(phase)


# Mercury: a drop of liquid metal. A drip whose landing rings like a small metal bead.
METAL = ((1, 1.0), (2.41, 0.22), (3.9, 0.08))


def mercury_alert(t):
    return drip(t, 800.0, 1900.0) + 0.55 * bell(1975.5, 0.9, 0.16, partials=METAL, start=0.05)(t)


def mercury_soft(t):
    return 0.8 * drip(t, 600.0, 1400.0, 0.05) + 0.35 * bell(1480.0, 0.8, 0.14, partials=METAL, start=0.05)(t)


# Bell: the terminal bell, softened. A rounded bell, two notes for alert.
def bell_alert(t):
    return bell(1046.5, 1.4, 0.3)(t) + 0.75 * bell(1568.0, 1.4, 0.32, start=0.12)(t)


def bell_soft(t):
    return bell(880.0, 1.4, 0.32, partials=((1, 1.0), (2.76, 0.1), (5.4, 0.03)))(t)


# Tick: two wooden key clicks.
_NOISE = [random.Random(3).uniform(-1, 1) for _ in range(int(RATE * 0.02))]


def click(u, f, gain=1.0):
    if u < 0 or u > 0.2:
        return 0.0
    i = int(u * RATE)
    noise = 0.35 * math.exp(-u / 0.002) * (_NOISE[i] if i < len(_NOISE) else 0.0)
    body = sum(a * math.exp(-u / d) * math.sin(2 * math.pi * f * m * u) for m, a, d in ((1, 1.0, 0.03), (3.9, 0.15, 0.012)))
    return gain * (min(1.0, u / 0.001) * body + noise)


def tick_alert(t):
    return click(t, 1318.5) + click(t - 0.11, 1760.0, 0.85)


def tick_soft(t):
    return click(t, 988.0, 0.9) + click(t - 0.13, 988.0, 0.6)


# Pulse: two quiet sine blips.
def blip(u, f, dur=0.09):
    if u < 0 or u > dur:
        return 0.0
    x = u / dur
    return math.sin(math.pi * x) ** 2 * math.sin(2 * math.pi * f * u)


def pulse_alert(t):
    return blip(t, 1174.7) + blip(t - 0.13, 1568.0)


def pulse_soft(t):
    return blip(t, 880.0, 0.11) + 0.7 * blip(t - 0.16, 880.0, 0.11)


# Chord: a soft open fifth.
SOFT = ((1, 1.0), (2.0, 0.06), (3.0, 0.03))


def chord_alert(t):
    return bell(659.3, 1.8, 0.5, partials=SOFT, attack=0.01)(t) + 0.8 * bell(987.8, 1.8, 0.5, partials=SOFT, attack=0.01, start=0.06)(t)


def chord_soft(t):
    return bell(523.3, 1.8, 0.45, partials=SOFT, attack=0.015)(t) + 0.7 * bell(784.0, 1.8, 0.45, partials=SOFT, attack=0.015)(t)


RING_OUT = {"mercury", "bell", "chord"}
SOUNDS = {
    "mercury": ((mercury_alert, 0.9), (mercury_soft, 0.8)),
    "bell": ((bell_alert, 1.5), (bell_soft, 1.3)),
    "tick": ((tick_alert, 0.35), (tick_soft, 0.35)),
    "pulse": ((pulse_alert, 0.3), (pulse_soft, 0.35)),
    "chord": ((chord_alert, 1.8), (chord_soft, 1.7)),
}


def render(fn, dur, ring_out=False):
    n = int(RATE * dur)
    xs = [fn(i / RATE) for i in range(n)]
    if ring_out:
        f = int(n * 0.45)
        for i in range(f):
            xs[n - f + i] *= 0.5 * (1 + math.cos(math.pi * i / f))
    f = int(RATE * 0.003)
    for i in range(f):
        xs[i] *= i / f
        xs[-1 - i] *= i / f
    m = max(abs(x) for x in xs) or 1.0
    return [x / m * PEAK for x in xs]


def main():
    os.makedirs(OUT, exist_ok=True)
    only = set(sys.argv[1:])
    for name, pair in SOUNDS.items():
        if only and name not in only:
            continue
        for kind, (fn, dur) in zip(("alert", "soft"), pair):
            xs = render(fn, dur, name in RING_OUT)
            path = os.path.join(OUT, f"{name}-{kind}.wav")
            with wave.open(path, "wb") as w:
                w.setnchannels(1)
                w.setsampwidth(2)
                w.setframerate(RATE)
                w.writeframes(b"".join(struct.pack("<h", int(x * 32767)) for x in xs))
            print(path, os.path.getsize(path))


if __name__ == "__main__":
    main()
