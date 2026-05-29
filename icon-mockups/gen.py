#!/usr/bin/env python3
"""Generate Andon app-icon concept mockups."""
import cairosvg, os

OUT = os.path.dirname(__file__)
BG = "#15151b"        # dark squircle background (matches current icon)
PANEL = "#1f1f29"     # slightly lighter housing
RED = "#ef4444"
AMBER = "#f59e0b"
GREEN = "#22c55e"
WARM = "#fcd34d"
DIM = "#3a3a46"

def squircle(extra):
    """A 512x512 tile: dark rounded square + the given inner markup."""
    return f'''
    <rect x="6" y="6" width="500" height="500" rx="104" ry="104" fill="{BG}"/>
    {extra}'''

# ---- Concept 1: Andon stack light (signal tower) -------------------------
def c1_stacklight():
    cx = 256
    # capsule x 186..326 (w140), y 96..372, 3 segments of ~92
    x, w = 186, 140
    top, seg = 96, 92
    glow = lambda c: f'filter="url(#g1)"'
    return f'''
    <defs><filter id="g1" x="-50%" y="-50%" width="200%" height="200%">
      <feGaussianBlur stdDeviation="6"/></filter></defs>
    <!-- red dome top -->
    <path d="M {x} {top+40} a 70 70 0 0 1 140 0 v 52 h -140 z" fill="{RED}"/>
    <rect x="{x}" y="{top+seg}" width="{w}" height="{seg}" fill="{AMBER}"/>
    <path d="M {x} {top+2*seg} h {w} v 12 a 70 30 0 0 1 -140 0 z" fill="{GREEN}"/>
    <!-- pole + base -->
    <rect x="{cx-14}" y="{top+3*seg-22}" width="28" height="60" rx="6" fill="{PANEL}"/>
    <rect x="{cx-70}" y="{top+3*seg+34}" width="140" height="26" rx="13" fill="{PANEL}"/>
    <!-- highlights -->
    <rect x="{x+18}" y="{top+seg+14}" width="14" height="{seg-28}" rx="7" fill="#ffffff" opacity="0.30"/>
    '''

# ---- Concept 2: Signal dots (vertical traffic light) ---------------------
def c2_dots():
    cx = 256
    return f'''
    <rect x="186" y="92" width="140" height="328" rx="70" fill="{PANEL}"/>
    <circle cx="{cx}" cy="160" r="44" fill="{RED}"/>
    <circle cx="{cx}" cy="256" r="44" fill="{AMBER}"/>
    <circle cx="{cx}" cy="352" r="44" fill="{GREEN}"/>
    <circle cx="{cx-14}" cy="146" r="12" fill="#ffffff" opacity="0.35"/>
    '''

# ---- Concept 3: Andon lantern (paper lamp) ------------------------------
def c3_lantern():
    cx = 256
    return f'''
    <defs>
      <radialGradient id="lg" cx="50%" cy="45%" r="60%">
        <stop offset="0%" stop-color="{WARM}"/>
        <stop offset="100%" stop-color="{AMBER}"/>
      </radialGradient>
    </defs>
    <!-- top + bottom caps -->
    <rect x="{cx-46}" y="96" width="92" height="22" rx="8" fill="{DIM}"/>
    <rect x="{cx-58}" y="386" width="116" height="26" rx="10" fill="{DIM}"/>
    <!-- body -->
    <rect x="150" y="120" width="212" height="266" rx="92" fill="url(#lg)"/>
    <!-- ribs -->
    <g stroke="#000000" stroke-opacity="0.18" stroke-width="7">
      <line x1="160" y1="186" x2="352" y2="186"/>
      <line x1="150" y1="253" x2="362" y2="253"/>
      <line x1="160" y1="320" x2="352" y2="320"/>
    </g>
    <line x1="{cx}" y1="74" x2="{cx}" y2="96" stroke="{DIM}" stroke-width="8"/>
    '''

# ---- Concept 4: Pulse / telemetry wave ----------------------------------
def c4_pulse():
    return f'''
    <defs><filter id="g4" x="-50%" y="-50%" width="200%" height="200%">
      <feGaussianBlur stdDeviation="5"/></filter></defs>
    <circle cx="256" cy="256" r="150" fill="none" stroke="{PANEL}" stroke-width="20"/>
    <polyline points="118,256 188,256 224,168 270,344 308,256 394,256"
      fill="none" stroke="{AMBER}" stroke-width="22"
      stroke-linecap="round" stroke-linejoin="round"/>
    <circle cx="394" cy="256" r="20" fill="{GREEN}" filter="url(#g4)"/>
    <circle cx="394" cy="256" r="14" fill="{GREEN}"/>
    '''

# ---- Concept 5: Gauge / dashboard dial ----------------------------------
def c5_gauge():
    import math
    cx, cy, r = 256, 270, 150
    def pt(deg):
        a = math.radians(deg)
        return cx + r*math.cos(a), cy - r*math.sin(a)
    # arc from 210deg to -30deg (sweep across top)
    x1,y1 = pt(210); x2,y2 = pt(-30)
    # needle to 60deg
    nx = cx + 120*math.cos(math.radians(70)); ny = cy - 120*math.sin(math.radians(70))
    return f'''
    <path d="M {x1:.1f} {y1:.1f} A {r} {r} 0 1 1 {x2:.1f} {y2:.1f}"
      fill="none" stroke="{PANEL}" stroke-width="26" stroke-linecap="round"/>
    <path d="M {x1:.1f} {y1:.1f} A {r} {r} 0 0 1 {nx-cx+cx:.1f} {ny:.1f}"
      fill="none" stroke="{AMBER}" stroke-width="26" stroke-linecap="round" opacity="0"/>
    <!-- colored arc segments -->
    <path d="M {pt(210)[0]:.1f} {pt(210)[1]:.1f} A {r} {r} 0 0 1 {pt(130)[0]:.1f} {pt(130)[1]:.1f}" fill="none" stroke="{GREEN}" stroke-width="26" stroke-linecap="round"/>
    <path d="M {pt(120)[0]:.1f} {pt(120)[1]:.1f} A {r} {r} 0 0 1 {pt(60)[0]:.1f} {pt(60)[1]:.1f}" fill="none" stroke="{AMBER}" stroke-width="26"/>
    <path d="M {pt(50)[0]:.1f} {pt(50)[1]:.1f} A {r} {r} 0 0 1 {pt(-30)[0]:.1f} {pt(-30)[1]:.1f}" fill="none" stroke="{RED}" stroke-width="26" stroke-linecap="round"/>
    <!-- needle -->
    <line x1="{cx}" y1="{cy}" x2="{nx:.1f}" y2="{ny:.1f}" stroke="{WARM}" stroke-width="14" stroke-linecap="round"/>
    <circle cx="{cx}" cy="{cy}" r="22" fill="{WARM}"/>
    <circle cx="{cx}" cy="{cy}" r="9" fill="{BG}"/>
    '''

# ---- Concept 6: Beacon / signal pulse rings -----------------------------
def c6_beacon():
    cx, cy = 256, 256
    return f'''
    <path d="M {cx} {cy} m -150 0 a 150 150 0 0 1 300 0" fill="none"
      stroke="{AMBER}" stroke-width="20" stroke-linecap="round" opacity="0.30"/>
    <path d="M {cx} {cy} m -96 0 a 96 96 0 0 1 192 0" fill="none"
      stroke="{AMBER}" stroke-width="22" stroke-linecap="round" opacity="0.6"/>
    <path d="M {cx} {cy} m -46 0 a 46 46 0 0 1 92 0" fill="none"
      stroke="{AMBER}" stroke-width="24" stroke-linecap="round"/>
    <circle cx="{cx}" cy="{cy+8}" r="30" fill="{GREEN}"/>
    '''

CONCEPTS = [
    ("1 · Stack light", c1_stacklight),
    ("2 · Signal dots", c2_dots),
    ("3 · Lantern",     c3_lantern),
    ("4 · Pulse",       c4_pulse),
    ("5 · Gauge",       c5_gauge),
    ("6 · Beacon",      c6_beacon),
]

# render each standalone at 256
for i,(name,fn) in enumerate(CONCEPTS,1):
    svg = f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 512 512" width="512" height="512">{squircle(fn())}</svg>'
    open(f"{OUT}/concept{i}.svg","w").write(svg)
    cairosvg.svg2png(bytestring=svg.encode(), write_to=f"{OUT}/concept{i}.png", output_width=256, output_height=256)

# contact sheet: 3 cols x 2 rows, with labels
TW, TH = 300, 330
cols, rows = 3, 2
W = cols*TW + 40
H = rows*TH + 60
tiles = ""
for idx,(name,fn) in enumerate(CONCEPTS):
    r,c = divmod(idx, cols)
    ox = 20 + c*TW + (TW-256)/2
    oy = 40 + r*TH
    tiles += f'<svg x="{ox}" y="{oy}" width="256" height="256" viewBox="0 0 512 512">{squircle(fn())}</svg>'
    tiles += f'<text x="{ox+128}" y="{oy+292}" fill="#e5e7eb" font-family="sans-serif" font-size="26" text-anchor="middle">{name}</text>'
sheet = f'''<svg xmlns="http://www.w3.org/2000/svg" width="{W}" height="{H}" viewBox="0 0 {W} {H}">
<rect width="{W}" height="{H}" fill="#0b0b0f"/>
<text x="{W/2}" y="28" fill="#9ca3af" font-family="sans-serif" font-size="22" text-anchor="middle">Andon icon concepts</text>
{tiles}</svg>'''
open(f"{OUT}/contact-sheet.svg","w").write(sheet)
cairosvg.svg2png(bytestring=sheet.encode(), write_to=f"{OUT}/contact-sheet.png", output_width=W, output_height=H)
print("done", W, H)
