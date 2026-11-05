"""Renders a Rust agent's whole life on Craft as a sequence plate: built once in a short-lived container, each call
run in a container of its own with its drawing sent to the Mac's GPU pool, then released.

    python3 -I Docs/README_Assets/src/lifecycle.py Docs/README_Assets

Inputs:  this folder's rr-mark.png; Playfair Display Italic from ~/Library/Fonts; Google Chrome, driven by Playwright
         over a pipe (no port is opened).
Outputs: agent-lifecycle.png in argv[1].

The design is Docs/Compute.md: the build container and a call answered inside a container are built (containers/);
the call container's sizing, the GPU pool and granite-gpu/1 are proposed. The numbers are measured (ComicAgent on an
M4 Max, 2026-11-05). The kit (header rhythm, arrows, badges, frames, lane heads) is Braid's
Docs/README_Assets/src/sequence.py, so the plates read as one set.
"""

import os
import sys
import tempfile

from playwright.sync_api import sync_playwright

HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.abspath(sys.argv[1])
PLAYFAIR = os.path.expanduser("~/Library/Fonts/PlayfairDisplay-Italic-VariableFont_wght.ttf")

# The hero's colours (Braid's build.py).
BLUE, GREEN, ORANGE, GOLD, LINE = "#5e8fda", "#28a883", "#d8773f", "#c7a659", "#cbc8c0"
CALL, REPLY, GONE = "#d6d3cb", "#8f8c84", "#d9645a"
W = 1800

# The header's rhythm, shared by Rao's plates (Braid's sequence.py): the mark's ink 70 px from the top and the left,
# every header line's ink on the mark's left edge, the title's ink 70 px under the mark's, the plate HEADER_GAP px
# under the subtitle's baseline, the page grown or shrunk to keep its foot.
INK, TITLE_INK, HEADER_GAP = 70.5, 174, 54
HEADER_JS = """<script>
document.fonts.ready.then(() => {
  const ctx = document.createElement("canvas").getContext("2d");
  const measure = (el) => {
    const cs = getComputedStyle(el), text = el.textContent.trim();
    ctx.font = `italic ${cs.fontWeight} ${cs.fontSize} Playfair`;
    const probe = document.createElement("span");
    probe.style.cssText = "display:inline-block; width:0; height:0";
    el.prepend(probe);
    const baseline = probe.getBoundingClientRect().bottom;
    probe.remove();
    return {left: parseFloat(cs.paddingLeft), top: parseFloat(cs.top), baseline,
            ink: baseline - ctx.measureText(text).actualBoundingBoxAscent, overhang: ctx.measureText(text[0]).actualBoundingBoxLeft};
  };
  const title = document.querySelector(".title"), sub = document.querySelector(".sub");
  const t = measure(title), s = measure(sub), dy = __TITLE_INK__ - t.ink;
  for (const [el, m] of [[title, t], [sub, s]]) {
    el.style.left = (__INK__ - m.left + m.overhang) + "px";
    el.style.top = (m.top + dy) + "px";
  }
  const header = ".mark, .caption, .title, .sub, script, style", rest = [...document.body.children].filter((el) => !el.matches(header));
  let first = Infinity;
  for (const el of rest) {
    const r = el.getBoundingClientRect();
    if (el instanceof SVGSVGElement) {
      const b = el.getBBox();
      if (b.width || b.height) first = Math.min(first, r.top + b.y);
    } else if (r.width && r.height) first = Math.min(first, r.top);
  }
  const shift = Math.round(s.baseline + dy + __GAP__ - first);
  for (const el of rest) el.style.translate = `0 ${shift}px`;
  const height = parseFloat(getComputedStyle(document.body).height) + shift;
  document.documentElement.style.height = document.body.style.height = height + "px";
  document.body.dataset.height = height;
});
</script>""".replace("__INK__", str(INK)).replace("__TITLE_INK__", str(TITLE_INK)).replace("__GAP__", str(HEADER_GAP))
MONO = "Menlo, 'SF Mono', monospace"
SANS = "-apple-system, 'Helvetica Neue', Helvetica, Arial, sans-serif"

# Lifelines, left to right: install happens on the left of Craft, calls on its right.
REPOS, BUILD, CRAFT, CALLS, POOL = 185, 540, 900, 1260, 1615
HEAD_TOP, HEAD_H, HEAD_W = 330, 96, 290
LIFE_TOP, LIFE_BOTTOM = HEAD_TOP + HEAD_H, 1995

# Frames: (top, bottom, left, right, label).
INSTALL = (470, 990, 40, 1225, "ONCE, WHEN IT IS INSTALLED")
EVERY_CALL = (1030, 1980, 740, 1775, "EVERY CALL, THEN RELEASED")
GPU_LOOP = (1530, 1722, 1112, 1760, "FOR EVERY GPU JOB")

# (y, kind, from, to, number, words, code)
MESSAGES = [
    (540, "call", CRAFT, BUILD, 1, "builds the agent away from the Mac", "agents add · --memory 8g --cpus 8"),
    (620, "call", BUILD, REPOS, 2, "clones it at its pinned commit", "git clone · cargo metadata"),
    (692, "reply", REPOS, BUILD, None, "the agent; its lock pins GraniteRust", "ComicAgent@baea334 · granite@7ef00cd"),
    (764, "self", BUILD, BUILD, 3, "compiles the runner, no GPU code", "cargo build --release"),
    (850, "reply", BUILD, CRAFT, None, "a call image, and what it carries", "--describe → entries · image digest"),
    (922, "self", CRAFT, CRAFT, 4, "keeps it on the shelf", "ComicAgent/installation.json"),
    (1100, "call", CRAFT, CALLS, 5, "a container for this call alone", "--rm -i · 512 MB · 1 CPU · /work"),
    (1172, "reply", CALLS, CRAFT, None, "it speaks the agent standard", "hello · granite-agent/1"),
    (1250, "call", CRAFT, CALLS, 6, "the call the session's model wrote", "perform {word: comic, input}"),
    (1322, "call", CALLS, CRAFT, 7, "asks Craft's model for a script", "model {kind: object, schema}"),
    (1394, "reply", CRAFT, CALLS, None, "the strip's script", "answer · craft-comic/1"),
    (1466, "self", CALLS, CALLS, 8, "lays out and letters the strip", "compose · measure · revise"),
    (1600, "call", CALLS, POOL, 9, "the strokes, with the call's token", "job {kernel, buffers} · 1.5 MB"),
    (1672, "reply", POOL, CALLS, None, "the pixels, and the GPU time", "done {buffers, gpuMs: 8} · 14 MB"),
    (1790, "call", CALLS, POOL, 10, "done with the GPU", "bye · its buffers freed"),
    (1862, "call", CALLS, CRAFT, 11, "the answer: the strip it wrote", "output {paths: comics/*.png}"),
    (1934, "self", CRAFT, CRAFT, 12, "reaps it; the container is gone", "released · peak 30 MB · GPU 8 ms"),
]

# Who is alive when: (lifeline, first y, last y). A container's bar ends where it is removed (✕).
ALIVE = [(BUILD, 540, 850), (CALLS, 1100, 1862), (POOL, 1600, 1790)]
REMOVED = [(BUILD, 902), (CALLS, 1896)]


def arrow(x0, x1, y, kind):
    colour = CALL if kind == "call" else REPLY
    d = 1 if x1 > x0 else -1
    tip = x1 - d * 4
    dash = '' if kind == "call" else ' stroke-dasharray="7 6"'
    out = f'<line x1="{x0 + d * 6}" y1="{y}" x2="{tip}" y2="{y}" stroke="{colour}" stroke-width="2"{dash} stroke-linecap="round"/>'
    if kind == "call":
        out += f'<path d="M{tip},{y} L{tip - d * 13},{y - 6.5} L{tip - d * 13},{y + 6.5} Z" fill="{colour}"/>'
    else:
        out += f'<path d="M{tip - d * 12},{y - 7} L{tip},{y} L{tip - d * 12},{y + 7}" fill="none" stroke="{colour}" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>'
    return out


def self_call(x, y):
    w, h = 54, 40
    return (f'<path d="M{x + 6},{y} H{x + w} V{y + h} H{x + 10}" fill="none" stroke="{CALL}" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>'
            f'<path d="M{x + 10},{y + h} L{x + 23},{y + h - 6.5} L{x + 23},{y + h + 6.5} Z" fill="{CALL}"/>')


def badge(x, y, n):
    return (f'<circle cx="{x}" cy="{y}" r="12" fill="#191917" stroke="{GOLD}" stroke-width="1.6"/>'
            f'<text x="{x}" y="{y + 4.8}" text-anchor="middle" font-family="{SANS}" font-size="13.5" font-weight="600" fill="{GOLD}">{n}</text>')


def frame(top, bottom, left, right, label_width):
    stroke = "rgba(199,166,89,.45)"
    tab = f"M{left + 1},{top + 12} Q{left + 1},{top + 1} {left + 12},{top + 1} H{left + label_width + 16} V{top + 14} L{left + label_width},{top + 30} H{left + 1} Z"
    return (f'<rect x="{left}" y="{top}" width="{right - left}" height="{bottom - top}" rx="12" fill="rgba(255,255,255,.018)" stroke="{stroke}" stroke-width="1.4"/>'
            f'<path d="{tab}" fill="#1c1c19"/>'
            f'<path d="M{left},{top + 30} H{left + label_width} L{left + label_width + 16},{top + 14} V{top}" fill="none" stroke="{stroke}" stroke-width="1.4"/>')


def removed(x, y):
    r = 11
    return (f'<path d="M{x - r},{y - r} L{x + r},{y + r} M{x + r},{y - r} L{x - r},{y + r}" stroke="{GONE}" stroke-width="2.6" stroke-linecap="round"/>')


def svg(height):
    p = []
    for x in (REPOS, BUILD, CRAFT, CALLS, POOL):
        p.append(f'<line x1="{x}" y1="{LIFE_TOP}" x2="{x}" y2="{LIFE_BOTTOM}" stroke="#55534d" stroke-width="1.6" stroke-dasharray="3 7" stroke-linecap="round"/>')
    # Busy bars before the frames, so a frame's label tab covers what passes behind it.
    for x, first, last in ALIVE:
        p.append(f'<rect x="{x - 6}" y="{first - 4}" width="12" height="{last - first + 8}" rx="3" fill="#2a2925" stroke="#4d4b45" stroke-width="1"/>')
    for top, bottom, left, right, label in (INSTALL, EVERY_CALL, GPU_LOOP):
        p.append(frame(top, bottom, left, right, 34 + len(label) * 10.6))
    for x, y in REMOVED:
        p.append(removed(x, y))
    for y, kind, x0, x1, n, _, _ in MESSAGES:
        if kind == "self":
            p.append(self_call(x0, y - 20))
        else:
            p.append(arrow(x0, x1, y, kind))
        if n:
            bx = (x0 - 34) if kind == "self" else (x0 + (28 if x1 > x0 else -28))
            p.append(badge(bx, y, n))
    return f'<svg width="{W}" height="{height}">{"".join(p)}</svg>'


def labels():
    out = []
    for y, kind, x0, x1, n, words, code in MESSAGES:
        if kind == "self":
            out.append(f'<div class="label self" style="left:{x0 + 72}px; top:{y - 26}px"><span>{words}</span><code>{code}</code></div>')
        else:
            left, right = min(x0, x1), max(x0, x1)
            out.append(f'<div class="label {kind}" style="left:{left + 20}px; width:{right - left - 40}px; top:{y - 30}px"><span>{words}</span></div>')
            out.append(f'<div class="label {kind}" style="left:{left + 20}px; width:{right - left - 40}px; top:{y + 7}px"><code>{code}</code></div>')
    for top, _, left, _, label in (INSTALL, EVERY_CALL, GPU_LOOP):
        out.append(f'<div class="looplabel" style="left:{left + 14}px; top:{top + 7}px">{label}</div>')
    for x, y in REMOVED:
        out.append(f'<div class="gone" style="left:{x + 20}px; top:{y - 11}px">removed</div>')
    return "".join(out)


def heads():
    def box(x, inner, extra=""):
        return f'<div class="lifehead{extra}" style="left:{x - HEAD_W / 2}px; top:{HEAD_TOP}px; width:{HEAD_W}px; height:{HEAD_H}px">{inner}</div>'

    def dot(colour):
        return f'<i class="dot" style="background:{colour}"></i>'
    stack = "".join(f'<div class="lifehead ghost" style="left:{CALLS - HEAD_W / 2 + d}px; top:{HEAD_TOP - d}px; width:{HEAD_W}px; height:{HEAD_H}px"></div>' for d in (16, 8))
    return (box(REPOS, f'<b>{dot(LINE)}The repositories</b><p>GitHub · the agent, GraniteRust</p>')
            + box(BUILD, f'<b>{dot(ORANGE)}Build container</b><p>short-lived · 8 GB · once an install</p>')
            + box(CRAFT, f'<b>{dot(GOLD)}Craft</b><p>the host · its model, its shelf</p>')
            + stack
            + box(CALLS, f'<b>{dot(GREEN)}Call containers</b><p>short-lived · 512 MB · 1 CPU · a call</p>')
            + box(POOL, f'<b>{dot(BLUE)}GPU pool</b><p>the Mac’s GPU · Metal · shared</p>'))


FOOT_TOP = 2040
TAG_TOP = FOOT_TOP + 116
H = TAG_TOP + 54


def page():
    return f"""<!doctype html><html><head><meta charset="utf-8"><style>
@font-face {{ font-family: "Playfair"; font-style: italic; font-weight: 400 900; src: url("file://{PLAYFAIR}") format("truetype"), local("Playfair Display Italic"); }}
html, body {{ margin: 0; width: {W}px; height: {H}px; overflow: hidden; }}
body {{ position: relative; font-family: "Playfair", Georgia, serif; font-style: italic; color: #a29f97;
       background: radial-gradient(ellipse 90% 100% at 22% 20%, #20201c 0%, #191917 45%, #121211 100%); }}
.mark {{ position: absolute; left: 62.5px; top: 60px; width: 64px; }}
.caption {{ position: absolute; left: 150px; top: 78px; letter-spacing: .3em; font-size: 15px; color: #8b8a84; }}
.title {{ position: absolute; left: 25px; padding-left: 40px; top: 162px; font-size: 104px; line-height: 1; font-weight: 500; letter-spacing: -.005em; padding-right: 30px; padding-bottom: 32px;
         background: linear-gradient(100deg, #8d9097 0%, #c8cad0 35%, #f2f3f6 58%, #b6b9c1 100%);
         -webkit-background-clip: text; background-clip: text; color: transparent; }}
.sub {{ position: absolute; left: 69px; top: 280px; font-size: 32px; color: #9b9991; }}
svg {{ position: absolute; left: 0; top: 0; }}
.lifehead {{ position: absolute; box-sizing: border-box; border: 1px solid rgba(255,255,255,.13); border-radius: 14px; background: #1d1d1a;
            padding: 16px 16px; text-align: center; }}
.lifehead.ghost {{ background: #1a1a18; border-color: rgba(255,255,255,.08); }}
.lifehead b {{ display: block; font-weight: 400; font-size: 25px; color: #ecebe5; white-space: nowrap; }}
.lifehead p {{ margin: 6px 0 0; font-size: 15px; color: #8f8c84; white-space: nowrap; }}
.dot {{ display: inline-block; width: 12px; height: 12px; border-radius: 50%; margin-right: 10px; transform: translateY(-3px); }}
.label {{ position: absolute; display: flex; justify-content: center; white-space: nowrap; }}
.label span {{ font-size: 18px; color: #d9d6ce; }}
.label.reply span {{ color: #a29f97; }}
.label code {{ font-family: {MONO}; font-style: normal; font-size: 13px; color: #8f8c84; }}
.label.self {{ display: flex; flex-direction: column; gap: 6px; justify-content: flex-start; }}
.looplabel {{ position: absolute; font-size: 12px; letter-spacing: .24em; color: {GOLD}; white-space: nowrap; }}
.gone {{ position: absolute; font-size: 15px; color: {GONE}; }}
.foot {{ position: absolute; left: 70px; top: {FOOT_TOP}px; width: {W - 140}px; display: grid; grid-template-columns: repeat(3, 1fr); column-gap: 40px; }}
.foot div {{ border-top: 1px solid rgba(255,255,255,.1); padding-top: 14px; }}
.foot h4 {{ margin: 0; font-size: 12px; font-weight: 400; letter-spacing: .24em; color: #77756f; }}
.foot p {{ margin: 8px 0 0; font-size: 18px; line-height: 1.4; color: #b4b1a9; }}
.tag {{ position: absolute; right: 70px; top: {TAG_TOP}px; font-size: 18px; color: #8b8a84; }}
</style></head><body>
<img class="mark" src="rr-mark.png">
<div class="caption">RAO STUDIOS · GRANITERUST · A CRAFT AGENT IN RUST</div>
<div class="title">Built once, called, released</div>
<div class="sub">a Rust agent on Craft: built in one container, each call run in another, its drawing on the Mac’s GPU</div>
{svg(H)}
{heads()}
{labels()}
<div class="foot">
  <div><h4>WHAT A CALL CONTAINER HOLDS</h4><p>the agent's logic: the call, its asks of Craft's model, the files it writes; a call peaks at 30 MB</p></div>
  <div><h4>WHAT THE GPU POOL HOLDS</h4><p>every drawing operation from every container, on the Mac's one GPU; a strip is 8 ms of it</p></div>
  <div><h4>WHAT IS LEFT AFTER A CALL</h4><p>the strip in the project and its answer; the container, its memory and its GPU buffers are gone</p></div>
</div>
<div class="tag">built in GraniteRust’s check: the build container, and a call answered inside one · proposed: Craft running them, the GPU pool</div>
{HEADER_JS}
</body></html>"""


def main():
    with tempfile.NamedTemporaryFile("w", suffix=".html", dir=HERE, delete=False, encoding="utf-8") as f:
        f.write(page())
        name = f.name
    try:
        with sync_playwright() as p:
            browser = p.chromium.launch(channel="chrome")
            tab = browser.new_page(device_scale_factor=2, viewport={"width": W, "height": H})
            tab.goto("file://" + name)
            tab.wait_for_selector("body[data-height]", state="attached")
            tab.set_viewport_size({"width": W, "height": round(float(tab.evaluate("document.body.dataset.height")))})
            tab.wait_for_timeout(100)
            tab.screenshot(path=os.path.join(OUT, "agent-lifecycle.png"))
            browser.close()
    finally:
        os.unlink(name)


if __name__ == "__main__":
    main()
