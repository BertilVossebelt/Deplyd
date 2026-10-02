"""Draws the desktop notification the GIF composites in.

A real toast cannot be screenshotted cleanly - Windows rounds the corners and the
desktop shows through them - so this redraws one. Every measurement below was
taken off a capture of a real toast (notify-capture.png): card and border colour,
text rows, text brightness, and the positions of the overflow dots and close
cross. The wording is what example-hooks/notify.ps1 produces for a changeLive
event; the app icon is the real one, cropped from that capture.
"""
from PIL import Image, ImageDraw, ImageFont
import os

HERE = os.path.dirname(os.path.abspath(__file__))
S = 3                                    # draw at 3x, downscale at the end
W, H = 460 * S, 144 * S

CARD = (36, 35, 32)                      # sampled from the card interior
BORDER = (70, 70, 72)                    # the 1px ring around it
APP_NAME = (255, 255, 255)               # brightest pixel in the app row
HEADLINE = (255, 255, 255)
BODY = (205, 205, 205)                   # brightest pixel in the body row
CHROME = (168, 168, 168)

HEAD_TEXT = "Your change is live on WEB"
BODY_TEXT = "feat(web): show the deployed sha in the footer"


def font(name, size):
    p = r"C:\Windows\Fonts\%s" % name
    if os.path.exists(p):
        return ImageFont.truetype(p, size)
    for alt in ("/usr/share/fonts/truetype/dejavu/DejaVuSans%s.ttf"
                % ("-Bold" if "b" in name else ""),):
        if os.path.exists(alt):
            return ImageFont.truetype(alt, size)
    return ImageFont.load_default()


card = Image.new("RGBA", (W, H), (0, 0, 0, 0))
d = ImageDraw.Draw(card)
d.rounded_rectangle([0, 0, W - 1, H - 1], radius=8 * S,
                    fill=CARD + (255,), outline=BORDER + (255,), width=S)

cap = os.path.join(HERE, "notify-capture.png")
if os.path.exists(cap):
    icon = Image.open(cap).convert("RGBA").crop((16, 20, 42, 46))
    card.paste(icon.resize((26 * S, 26 * S), Image.LANCZOS), (18 * S, 19 * S),
               icon.resize((26 * S, 26 * S), Image.LANCZOS))

# Text rows measured on the real toast: app 26-38, headline 74-90, body 98-114.
d.text((52 * S, 21 * S), "Windows PowerShell", font=font("segoeui.ttf", 16 * S), fill=APP_NAME)
d.text((24 * S, 69 * S), HEAD_TEXT, font=font("segoeuib.ttf", 16 * S), fill=HEADLINE)
d.text((24 * S, 93 * S), BODY_TEXT, font=font("segoeui.ttf", 16 * S), fill=BODY)

# Chrome, drawn rather than typed: Segoe UI has no glyph for either codepoint.
# Real dot centres sit at x 367.5/373/378.5, the cross centre at 423.
cy = 32 * S
for cx in (W - 92 * S, W - 86.5 * S, W - 81 * S):
    r = 1.3 * S
    d.ellipse([cx - r, cy - r, cx + r, cy + r], fill=CHROME)
cx, r = W - 37 * S, 5.5 * S
d.line([cx - r, cy - r, cx + r, cy + r], fill=CHROME, width=max(1, S - 1))
d.line([cx - r, cy + r, cx + r, cy - r], fill=CHROME, width=max(1, S - 1))

card.resize((W // S, H // S), Image.LANCZOS).save(os.path.join(HERE, "notify.png"))
print("wrote notify.png")
