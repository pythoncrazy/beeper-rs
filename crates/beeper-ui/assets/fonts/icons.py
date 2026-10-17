"""Build icons.ttf: a Lucide subset plus the Material Symbols that Beeper Desktop draws on Linux and Windows.

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

LUCIDE = "bell cloud circle-plus circle-check arrow-up-right link plus smile audio-lines arrow-up at-sign settings paintbrush keyboard smartphone sparkles puzzle database circle-help info pencil trash-2 reply file-text play history bookmark message-square tablet monitor"
MATERIAL = "inbox low_priority_space archive schedule schedule_send email maps_ugc photo_library library_add more_horiz unfold_more filter_list search edit_square notifications_off image chevron_right expand_more close keep download mark_email_unread sms mode_comment person group check"

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
for i, name in enumerate(MATERIAL.split()):
    m = re.search(r'viewBox:"([^"]+)",fill:"currentColor",\.\.\.e,"data-sentry-element":"svg","data-sentry-component":"%s"' % name, js)
    assert m, f"{name} is not in the bundle"
    x0, y0, w, h = map(float, m.group(1).split())
    assert w == h, f"{name} is not square"
    paths = re.findall(r'\bd:"([^"]+)"', js[m.end():js.find("})", m.end()) + 2])
    assert paths, f"{name} has no path"
    pen = TTGlyphPen(None)
    k = upm / w
    for d in paths:
        parse_path(d, TransformPen(Cu2QuPen(pen, 1.0, reverse_direction=True), (k, 0, 0, -k, -x0 * k, (y0 + h) * k)))
    glyph = f"material_{name}"
    font["glyf"][glyph] = pen.glyph()
    font["hmtx"][glyph] = (upm, 0)
    for cmap in cmaps:
        cmap[0xF000 + i] = glyph
    print(f"pub const {name.upper()}: char = '\\u{{{0xF000 + i:x}}}';")
font.setGlyphOrder(font["glyf"].glyphOrder)
for name in font.getGlyphOrder():
    g = font["glyf"][name]
    g.recalcBounds(font["glyf"])
    font["hmtx"][name] = (font["hmtx"][name][0], getattr(g, "xMin", 0))
for rec in font["name"].names:
    if rec.nameID in (1, 3, 4, 6):
        rec.string = "icons"
font.save(out)
