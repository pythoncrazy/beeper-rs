"""Build icons.ttf: a Lucide subset plus the Material Symbols and react-icons that Beeper Desktop draws on Linux and Windows.

Usage: python3 icons.py <lucide-static 1.57.0 font dir> <Beeper Desktop build/renderer/App-*.js> <icons.ttf>
"""
import json
import re
import sys
from fontTools import subset
from fontTools.pens.cu2quPen import Cu2QuPen
from fontTools.pens.transformPen import TransformPen
from fontTools.pens.ttGlyphPen import TTGlyphPen
from fontTools.svgLib.path import parse_path
from fontTools.ttLib import TTFont

LUCIDE = "chevron-right at-sign"
MATERIAL = "inbox low_priority_space archive schedule schedule_send email maps_ugc photo_library library_add more_horiz unfold_more filter_list search edit_square notifications_off image chevron_right expand_more close keep download mark_email_unread sms mode_comment person group check reply mood view_sidebar add"

# Desktop's react-icons by the start of their path: FontAwesome's `FaCheck` of the check column of its menus.
REACT = {"tick": "M173.898 439.404"}

lucide, bundle, out = sys.argv[1:]
codepoints = json.load(open(f"{lucide}/codepoints.json"))
font = TTFont(f"{lucide}/lucide.ttf")
opts = subset.Options()
opts.layout_features = []
sub = subset.Subsetter(opts)
sub.populate(unicodes=[codepoints[n] for n in LUCIDE.split()])
sub.subset(font)

js = open(bundle).read()
upm = font["head"].unitsPerEm
cmaps = [t.cmap for t in font["cmap"].tables if t.isUnicode()]


def glyph(code, name, view, paths):
    x0, y0, w, h = map(float, view.split())
    assert w == h, f"{name} is not square"
    pen = TTGlyphPen(None)
    k = upm / w
    for d in paths:
        parse_path(d, TransformPen(Cu2QuPen(pen, 1.0, reverse_direction=True), (k, 0, 0, -k, -x0 * k, (y0 + h) * k)))
    font["glyf"][name] = pen.glyph()
    font["hmtx"][name] = (upm, 0)
    for cmap in cmaps:
        cmap[code] = name
    print(f"pub const {name.removeprefix('material_').upper()}: char = '\\u{{{code:x}}}';")


for i, name in enumerate(MATERIAL.split()):
    m = re.search(r'viewBox:"([^"]+)",fill:"currentColor",\.\.\.e,"data-sentry-element":"svg","data-sentry-component":"%s"' % name, js)
    assert m, f"{name} is not in the bundle"
    paths = re.findall(r'\bd:"([^"]+)"', js[m.end():js.find("})", m.end()) + 2])
    assert paths, f"{name} has no path"
    glyph(0xF000 + i, f"material_{name}", m.group(1), paths)
for i, (name, start) in enumerate(REACT.items()):
    m = re.search(r'attr:\{viewBox:"([^"]+)"\},child:\[\{tag:"path",attr:\{d:"(%s[^"]*)"' % re.escape(start), js)
    assert m, f"{name} is not in the bundle"
    glyph(0xF000 + len(MATERIAL.split()) + i, name, m.group(1), [m.group(2)])
font.setGlyphOrder(font["glyf"].glyphOrder)
for name in font.getGlyphOrder():
    g = font["glyf"][name]
    g.recalcBounds(font["glyf"])
    font["hmtx"][name] = (font["hmtx"][name][0], getattr(g, "xMin", 0))
for rec in font["name"].names:
    if rec.nameID in (1, 3, 4, 6):
        rec.string = "icons"
font.save(out)
