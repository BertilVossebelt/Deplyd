"""Lays the drawn notification over the tail of the GIF.

agg renders the terminal; this adds the toast on the frames after the watcher
reports the deploy, which is when a hook would really have fired. It slides in
from the right into the bottom-right corner, the way Windows shows one.

A single shared palette keeps the file small - per-frame palettes triple it.
"""
from PIL import Image
import os, sys

HERE = os.path.dirname(os.path.abspath(__file__))
src, dst = sys.argv[1], sys.argv[2]

SLIDE_STEPS = 9         # frames of travel
SLIDE_MS = 45           # per travel frame
HOLD_MS = 2600          # how long it sits there once arrived
MARGIN = 18

toast = Image.open(os.path.join(HERE, "notify.png")).convert("RGBA")
gif = Image.open(src)

frames, durations = [], []
for i in range(gif.n_frames):
    gif.seek(i)
    frames.append(gif.convert("RGBA"))
    durations.append(gif.info.get("duration", 60))

W, H = frames[0].size
rest_x = W - toast.width - MARGIN
y = H - toast.height - MARGIN

# The last frame is the deploy already reported; animate on top of that picture.
base = frames[-1].copy()
tail_ms = durations[-1]
frames.pop(); durations.pop()

for s in range(1, SLIDE_STEPS + 1):
    p = s / float(SLIDE_STEPS)
    ease = 1 - (1 - p) ** 3                       # ease-out, fast then settling
    x = int(W - (W - rest_x) * ease)
    f = base.copy()
    f.alpha_composite(toast, (x, y))
    frames.append(f)
    durations.append(SLIDE_MS)

settled = base.copy()
settled.alpha_composite(toast, (rest_x, y))
frames.append(settled)
durations.append(max(HOLD_MS, tail_ms - SLIDE_STEPS * SLIDE_MS))

# One palette for every frame, built from the colours that actually occur
# across the whole animation. Sampling only a frame or two loses the oranges
# and reds of the report, which then quantise to grey.
seen = set()
for f in frames:
    for _count, colour in (f.convert("RGB").getcolors(maxcolors=1 << 20) or []):
        seen.add(colour)
palette = sorted(seen)
if len(palette) <= 256:
    flat = []
    for c in palette:
        flat.extend(c)
    flat.extend([0, 0, 0] * (256 - len(palette)))
    pal = Image.new("P", (1, 1))
    pal.putpalette(flat)
else:                                    # too many: quantise over every frame
    strip = Image.new("RGB", (frames[0].width, frames[0].height * len(frames)))
    for i, f in enumerate(frames):
        strip.paste(f.convert("RGB"), (0, i * frames[0].height))
    pal = strip.quantize(colors=256, method=Image.MEDIANCUT)
print("distinct colours: %d (%s palette)"
      % (len(palette), "exact" if len(palette) <= 256 else "quantised"))

out = [f.convert("RGB").quantize(palette=pal, dither=Image.Dither.NONE) for f in frames]
out[0].save(dst, save_all=True, append_images=out[1:], duration=durations,
            loop=0, optimize=True, disposal=1)
print("frames: %d  slide: %d  total: %.1fs" % (len(out), SLIDE_STEPS, sum(durations) / 1000.0))
