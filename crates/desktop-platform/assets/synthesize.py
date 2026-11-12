"""Reproduce the original StickTalk PCM cues using Python's standard library.

No playback, third-party samples, network, codecs, or desktop effects.
Run from this directory. Assets are MIT OR Apache-2.0.
"""
import math
import struct
import wave
from pathlib import Path

NOTES = {
    "message": ([880, 1174], .14),
    "mention": ([660, 990, 1320], .24),
    "voice-join": ([440, 660], .20),
    "voice-leave": ([660, 440], .20),
    "mute": ([392, 294], .12),
    "unmute": ([294, 392], .12),
}
for name, (frequencies, duration) in NOTES.items():
    samples = bytearray()
    for i in range(int(24000 * duration)):
        t = i / 24000
        k = min(len(frequencies) - 1, int(t / duration * len(frequencies)))
        phase = t - k * duration / len(frequencies)
        envelope = min(1, phase / .008) * min(1, (duration / len(frequencies) - phase) / .025)
        value = int(4500 * max(0, envelope) * math.sin(2 * math.pi * frequencies[k] * phase))
        samples.extend(struct.pack("<h", value))
    with wave.open(str(Path(__file__).with_name(name + ".wav")), "wb") as output:
        output.setparams((1, 2, 24000, 0, "NONE", "not compressed"))
        output.writeframes(samples)
