"""Generate Reflow's original 100 ms start/stop cues (stdlib only)."""
import math
from pathlib import Path
import struct
import wave

directory = Path(__file__).resolve().parents[1] / "public" / "assets"
directory.mkdir(parents=True, exist_ok=True)
for name, frequency in [("start", 660), ("stop", 440)]:
    with wave.open(str(directory / f"{name}.wav"), "wb") as output:
        output.setnchannels(1)
        output.setsampwidth(2)
        output.setframerate(16000)
        output.writeframes(b"".join(
            struct.pack("<h", int(5000 * math.sin(2 * math.pi * frequency * i / 16000)
                                  * math.sin(math.pi * i / 1600)))
            for i in range(1600)
        ))
