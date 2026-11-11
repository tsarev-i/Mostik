#!/usr/bin/env python3
"""Renders Mostik's notification sounds and ringtone into app/src-tauri/sounds/*.wav.

The sounds are synthesised here from scratch (FM bells and electric piano, modal marimba, swept
sines, filtered noise, a small reverb), so they are the project's own work under its license, not
samples of other apps.
Standard library only; the output is deterministic.

    python3 scripts/sounds.py
"""

import cmath
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


def partial(out, t0, f, amp, tau):
    """Exponentially decaying sine added into `out` from `t0`, computed by complex rotation."""
    start = int(t0 * RATE)
    n = min(len(out) - start, int(tau * RATE * 9.3))
    w = cmath.exp(complex(-1.0 / (tau * RATE), 2 * math.pi * f / RATE))
    z = complex(amp, 0.0)
    attack = int(0.0015 * RATE)
    for i in range(n):
        v = z.imag
        if i < attack:
            v *= i / attack
        out[start + i] += v
        z *= w


def strike_marimba(out, t0, f, gain):
    """Struck bar with the tuned partials of a marimba: the fundamental, two octaves and a third above."""
    for ratio, amp, tau in [(1, 1.0, 0.38), (4.0, 0.25, 0.07), (10.0, 0.05, 0.02)]:
        partial(out, t0, f * ratio, gain * amp, tau)


def rhodes(out, t0, f, gain, tau=1.1):
    """Electric piano tine: a 1:1 FM tone whose brightness fades fast, with a short metallic click."""
    start = int(t0 * RATE)
    n = min(len(out) - start, int(tau * 7 * RATE))
    for i in range(n):
        t = i / RATE
        index = 1.1 * math.exp(-t / 0.22) + 0.15
        env = math.exp(-t / tau) * min(1.0, t / 0.002)
        v = math.sin(2 * math.pi * f * t + index * math.sin(2 * math.pi * f * t))
        v += 0.12 * math.exp(-t / 0.03) * math.sin(2 * math.pi * f * 7 * t)
        out[start + i] += gain * env * v


def square(out, t0, dur, f, release, gain):
    """Soft square wave of a game console: odd harmonics up to the ninth, a quick attack and fade."""
    start = int(t0 * RATE)
    n = min(len(out) - start, int((dur + release) * RATE))
    for i in range(n):
        t = i / RATE
        env = min(1.0, t / 0.003) if t < dur else max(0.0, 1 - (t - dur) / release)
        phase = 2 * math.pi * f * t
        out[start + i] += gain * env * sum(math.sin(k * phase) / k for k in (1, 3, 5, 7, 9))


def reverb(x, amount):
    """Small Freeverb-style room, mono; `amount` is the wet peak relative to the dry peak."""
    n = len(x)
    wet = [0.0] * n
    for d in (1116, 1188, 1277, 1356, 1422, 1491):
        line, idx, store = [0.0] * d, 0, 0.0
        for i in range(n):
            y = line[idx]
            store = y * 0.7 + store * 0.3
            line[idx] = x[i] + store * 0.8
            idx = idx + 1 if idx + 1 < d else 0
            wet[i] += y
    for d in (556, 441, 341):
        line, idx = [0.0] * d, 0
        for i in range(n):
            b = line[idx]
            v = wet[i]
            wet[i] = b - v
            line[idx] = v + b * 0.5
            idx = idx + 1 if idx + 1 < d else 0
    k = amount * max(map(abs, x)) / (max(map(abs, wet)) or 1.0)
    return [a + k * b for a, b in zip(x, wet)]


def finish(x, room):
    """Reverb, a 30 ms fade at the end, then normalise; the loudest 300 ms are kept from getting loud."""
    x = reverb(x, room)
    n, fade = len(x), int(RATE * 0.03)
    for i in range(fade):
        x[n - 1 - i] *= i / fade
    peak = max(map(abs, x)) or 1.0
    x = [v * PEAK / peak for v in x]
    w = int(0.3 * RATE)
    acc = sum(v * v for v in x[:w])
    best = acc
    for i in range(w, n):
        acc += x[i] * x[i] - x[i - w] * x[i - w]
        best = max(best, acc)
    rms = math.sqrt(best / min(w, n))
    return [v * 0.1 / rms for v in x] if rms > 0.1 else x


def electric_piano():
    """Two dyads of an electric piano, Cmaj7 spread: C5+E5, then G5+B5."""
    x = [0.0] * int(1.5 * RATE)
    for f in (523.25, 659.26):
        rhodes(x, 0.0, f, 0.6)
    for f in (783.99, 987.77):
        rhodes(x, 0.13, f, 0.6)
    return finish(x, 0.2)


def retro():
    """A coin in an old game console: B5, then E6."""
    x = [0.0] * int(0.6 * RATE)
    square(x, 0.0, 0.07, 987.77, 0.01, 0.6)
    square(x, 0.075, 0.08, 1318.51, 0.3, 0.6)
    return finish(x, 0.1)


def ringtone():
    """The ring of an incoming call: eight marimba notes, made to loop seamlessly.

    The phrase is rendered twice and only the second pass is kept: its start carries the tail of
    the first, as when the loop plays round.
    """
    period = 2.6
    melody = [783.99, 1046.5, 1318.51, 1174.66, 1046.5, 783.99, 880.0, 1046.5]  # G5 C6 E6 D6 C6 G5 A5 C6
    x = [0.0] * int(2 * period * RATE)
    for t0 in (0.0, period):
        for i, f in enumerate(melody):
            strike_marimba(x, t0 + i * 0.16, f, 0.95 if i % 2 == 0 else 0.8)
    x = reverb(x, 0.18)[int(period * RATE):]
    peak = max(map(abs, x)) or 1.0
    return [v * PEAK / peak for v in x]


# Each sound: (start in seconds, gain, voice) notes and the total length, or a function that renders it.
SOUNDS = {
    # Two soft bells a fifth apart, rising.
    "chime": ([(0.0, 1.0, bell(880.0, 0.32)), (0.11, 0.8, bell(1318.5, 0.38))], 1.0),
    # One bright desk bell with a long ring.
    "bell": ([(0.0, 1.0, bell(1568.0, 0.55, index=2.4, ratio=3.5))], 1.3),
    # Two marimba notes, a falling third.
    "marimba": ([(0.0, 1.0, marimba(1318.5)), (0.12, 0.9, marimba(1046.5))], 0.8),
    # A water drop and its smaller echo.
    "drop": ([(0.0, 1.0, sweep(520.0, 1450.0, 0.07, 0.09)), (0.13, 0.35, sweep(700.0, 1900.0, 0.05, 0.06))], 0.5),
    # Two knocks.
    "knock": ([(0.0, 1.0, knock(1)), (0.14, 0.8, knock(2))], 0.45),
    # A quick major arpeggio on glass.
    "glass": ([(0.0, 0.9, glass(1318.5, 0.22)), (0.07, 0.8, glass(1661.2, 0.22)), (0.14, 0.75, glass(1975.5, 0.3))], 0.9),
    # Electric piano: two dyads, rising.
    "electric_piano": electric_piano,
    # A coin in an old game console.
    "retro": retro,
    # The incoming call ring, played in a loop.
    "ringtone": ringtone,
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
    for name, sound in SOUNDS.items():
        path = OUT / f"{name}.wav"
        write(path, sound() if callable(sound) else render(*sound))
        print(f"{path.relative_to(OUT.parent.parent.parent)}: {path.stat().st_size // 1024} KB")


if __name__ == "__main__":
    main()
