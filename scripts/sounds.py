#!/usr/bin/env python3
"""Renders Mostik's notification sounds and ringtone into app/src-tauri/sounds/*.wav.

The sounds are synthesised here from scratch (FM bells, modal marimba, swept sines, filtered
noise), so they are the project's own work under its license, not samples of other apps.
Standard library only; the output is deterministic.

    python3 scripts/sounds.py
"""

import math
import random
import struct
import wave
from pathlib import Path

RATE = 44100
PEAK = 0.45  # Notifications should not be loud: about -7 dBFS.
OUT = Path(__file__).resolve().parent.parent / "app" / "src-tauri" / "sounds"


def bell(f, decay, index=1.8, ratio=1.4):
    """Chowning-style FM bell: the modulation index fades with the tone, the tail turns pure."""

    def voice(t):
        env = math.exp(-t / decay)
        mod = index * env * math.sin(2 * math.pi * f * ratio * t)
        return env * math.sin(2 * math.pi * f * t + mod)

    return voice


def marimba(f):
    """Struck bar: the fundamental plus the bar's 4th and 9th partials, which die out fast."""
    partials = [(1.0, 1.0, 0.20), (3.93, 0.28, 0.045), (9.2, 0.08, 0.018)]

    def voice(t):
        return sum(a * math.exp(-t / d) * math.sin(2 * math.pi * f * r * t) for r, a, d in partials)

    return voice


def sweep(f0, f1, glide, decay):
    """Sine whose pitch glides exponentially from f0 to f1 over `glide` seconds."""

    def voice(t):
        k = math.log(f1 / f0) / glide
        if t < glide:
            phase = f0 * (math.exp(k * t) - 1) / k
        else:
            phase = f0 * (math.exp(k * glide) - 1) / k + f1 * (t - glide)
        return math.exp(-t / decay) * math.sin(2 * math.pi * phase)

    return voice


def knock(seed):
    """Soft knock on wood: two damped low modes and a short low-passed noise burst."""
    rng = random.Random(seed)
    noise = [rng.uniform(-1, 1) for _ in range(int(RATE * 0.03))]
    smooth, state = [], 0.0
    for x in noise:
        state += 0.25 * (x - state)
        smooth.append(state)

    def voice(t):
        body = 0.9 * math.exp(-t / 0.035) * math.sin(2 * math.pi * 185 * t)
        body += 0.4 * math.exp(-t / 0.02) * math.sin(2 * math.pi * 420 * t)
        i = int(t * RATE)
        click = 0.8 * math.exp(-t / 0.006) * smooth[i] if i < len(smooth) else 0.0
        return body + click

    return voice


def glass(f, decay):
    """Glassy tone: a sine with a quiet inharmonic partial, like a tapped wine glass."""

    def voice(t):
        env = math.exp(-t / decay)
        return env * (math.sin(2 * math.pi * f * t) + 0.18 * math.sin(2 * math.pi * f * 2.76 * t))

    return voice


# Each sound: (start in seconds, gain, voice) notes and the total length.
SOUNDS = {
    # Two soft bells a fifth apart, rising.
    "chime": ([(0.0, 1.0, bell(880.0, 0.32)), (0.11, 0.8, bell(1318.5, 0.38))], 1.0),
    # One bright desk bell with a long ring.
    "bell": ([(0.0, 1.0, bell(1568.0, 0.55, index=2.4, ratio=3.5))], 1.3),
    # Two marimba notes, a falling third.
    "marimba": ([(0.0, 1.0, marimba(1318.5)), (0.12, 0.9, marimba(1046.5))], 0.8),
    # A water drop and its smaller echo.
    "drop": ([(0.0, 1.0, sweep(520.0, 1450.0, 0.07, 0.09)), (0.13, 0.35, sweep(700.0, 1900.0, 0.05, 0.06))], 0.5),
    # A bubble popping.
    "pop": ([(0.0, 1.0, sweep(1100.0, 340.0, 0.045, 0.06))], 0.3),
    # Two knocks.
    "knock": ([(0.0, 1.0, knock(1)), (0.14, 0.8, knock(2))], 0.45),
    # A quick major arpeggio on glass.
    "glass": ([(0.0, 0.9, glass(1318.5, 0.22)), (0.07, 0.8, glass(1661.2, 0.22)), (0.14, 0.75, glass(1975.5, 0.3))], 0.9),
    # The ring of an incoming call, played in a loop: two rising marimba-and-bell pairs.
    "ringtone": (
        [
            (0.0, 0.9, marimba(784.0)),
            (0.0, 0.35, bell(1568.0, 0.18)),
            (0.16, 1.0, marimba(1046.5)),
            (0.16, 0.35, bell(2093.0, 0.2)),
            (0.5, 0.9, marimba(784.0)),
            (0.5, 0.35, bell(1568.0, 0.18)),
            (0.66, 1.0, marimba(1046.5)),
            (0.66, 0.35, bell(2093.0, 0.25)),
        ],
        1.5,
    ),
}


def render(notes, length):
    n = int(RATE * length)
    out = [0.0] * n
    for start, gain, voice in notes:
        first = int(start * RATE)
        for i in range(first, n):
            t = (i - first) / RATE
            # 2 ms attack: no click at the start of a note.
            attack = min(1.0, t / 0.002)
            out[i] += gain * attack * voice(t)
    # 30 ms fade at the end, then normalise.
    fade = int(RATE * 0.03)
    for i in range(fade):
        out[n - 1 - i] *= i / fade
    peak = max(abs(x) for x in out) or 1.0
    return [x * PEAK / peak for x in out]


def write(path, samples):
    with wave.open(str(path), "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(RATE)
        w.writeframes(b"".join(struct.pack("<h", int(round(x * 32767))) for x in samples))


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    for name, (notes, length) in SOUNDS.items():
        path = OUT / f"{name}.wav"
        write(path, render(notes, length))
        print(f"{path.relative_to(OUT.parent.parent.parent)}: {path.stat().st_size // 1024} KB")


if __name__ == "__main__":
    main()
