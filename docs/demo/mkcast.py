"""Turns captured deplyd output into the asciicast the GIF is rendered from.

The output is real - each scene is what the binary actually printed. Only the
timing is invented: the typing, and the beats between the progress line, the
report, and (in the watch scene) the deploy landing.
"""
import io, json, re, sys

E = chr(27)
OSC8 = re.compile(re.escape(E + ']8;;') + '[^' + re.escape(E) + ']*' + re.escape(E + chr(92)))
DEMO = sys.argv[1]
WIDTH, HEIGHT = 100, 26

# command, capture, delay before each blank-line-separated stage, hold at the end
SCENES = [
    ('deplyd status',        'sc1.ansi', [0.30, 1.15],       3.6),
    ('deplyd status pr 412', 'sc2.ansi', [0.30, 1.10],       3.4),
    ('deplyd hooks add ./hooks/notify.ps1',
                             'sc3.ansi', [0.35],             2.6),
    ('deplyd watch -E production -A Bertil --every 60s',
                             'sc4.ansi', [0.30, 0.85, 2.9],  4.4),
]

def read(name):
    s = io.open(DEMO + '/' + name, encoding='utf-8', errors='replace').read()
    return OSC8.sub('', s)

events, t = [], 0.0
def emit(data, dt=0.0):
    global t
    t += dt
    events.append([round(t, 3), 'o', data])

PROMPT = (E + '[38;2;126;186;244m' + '~/Deplyd/widgets' + E + '[0m '
          + E + '[38;2;130;136;148m$' + E + '[0m ')

overlay_at = None
for i, (cmd, out, stages, hold) in enumerate(SCENES):
    if i:
        emit(E + '[2J' + E + '[H', 0.9)
    emit(PROMPT, 0.45)
    for ch in cmd:
        emit(ch, 0.042)
    emit('\r\n', 0.5)

    parts = read(out).split('\n\n')
    if len(parts) > len(stages):                 # fold the tail into the last stage
        parts = parts[:len(stages) - 1] + ['\n\n'.join(parts[len(stages) - 1:])]
    for n, (part, dt) in enumerate(zip(parts, stages)):
        chunk = part if n == 0 else '\n\n' + part
        emit(chunk.replace('\n', '\r\n'), dt)
        if i == len(SCENES) - 1 and n == len(stages) - 1:
            overlay_at = t                       # the deploy has just landed
    emit('', hold)

header = {
    'version': 2, 'width': WIDTH, 'height': HEIGHT,
    'env': {'TERM': 'xterm-256color', 'SHELL': '/bin/bash'},
    'theme': {
        'bg': '#15181d', 'fg': '#d4d7dd',
        'palette': ':'.join([
            '#15181d', '#e05561', '#8cc265', '#d18f52', '#4aa5f0', '#c162de',
            '#42b3c2', '#d4d7dd', '#3b4250', '#ff616e', '#a5e075', '#f0a45d',
            '#4dc4ff', '#de73ff', '#4cd1e0', '#e6e6e6',
        ]),
    },
}
with io.open(DEMO + '/demo.cast', 'w', encoding='utf-8', newline='\n') as f:
    f.write(json.dumps(header) + '\n')
    for e in events:
        f.write(json.dumps(e, ensure_ascii=False) + '\n')
io.open(DEMO + '/overlay_at.txt', 'w').write(str(round(overlay_at, 3)))
print('events:', len(events), 'duration:', round(t, 1), 's', 'overlay at:', round(overlay_at, 2), 's')
