# Ground floor details seen in the photo: white-trimmed oval window (centre), slate siding
# ledges on the piers, and moulded panels under each bay sill.
import math
L = kit.L
WHITE, DARK, SLATE = 'painted-white@f0eee6', 'painted-steel-mid@2f2a28', 'painted-white@697e92'
# oval window: dark glass disc inside a white ring, 1.0 m x 1.4 m at y 2.2
cx, cy, a, b, ring, n = 0.5, 2.2, 0.5, 0.7, 0.09, 24
outv = B(kit.out[0], 0, kit.out[1])
for i in range(n):
    t0, t1 = 2 * math.pi * i / n, 2 * math.pi * (i + 1) / n
    pt = lambda t, k, o: kit.P(cx, cy + (b + k) * math.sin(t), o, dr=(a + k) * math.cos(t))
    kit.mb.quad([kit.P(cx, cy, 0.02), pt(t0, 0, 0.02), pt(t1, 0, 0.02)], DARK, outv)
    kit.mb.quad([pt(t0, 0, 0.04), pt(t1, 0, 0.04), pt(t1, ring, 0.04), pt(t0, ring, 0.04)], WHITE, outv)
# horizontal siding ledges on the ground piers
for f0, f1 in ((0.0, 0.10), (0.35, 0.44), (0.56, 0.645), (0.90, 1.0)):
    for k in range(17):
        kit.box((f0 + f1) / 2, 0.3 + 0.2 * k, 0.0, (f1 - f0) * L, 0.035, 0.03, SLATE)
# moulded panels under the bay sills (front facets, storeys 1-3)
for f in (0.24, 0.79):
    for lvl in (3.6, 6.9, 10.2):
        kit.box(f, lvl + 0.38, 0.7, 1.15, 0.38, 0.03, WHITE)

# Continuous flared cove / soffit under the bays across the whole frontage (photo), white trim.
for i, (d, y) in enumerate(((0.26, 3.52), (0.18, 3.38), (0.1, 3.26))):
    kit.box(0.5, y, d / 2, L, 0.14, d, WHITE)
# White frame round the recessed entry (photo): jambs and head.
for dr in (-0.1, 0.1):
    kit.box(0.225, 1.4, 0.04, 0.12, 2.8, 0.06, WHITE, dr=dr * 0 + (-1.55 if dr < 0 else 1.55))
kit.box(0.225, 2.82, 0.04, 3.4, 0.14, 0.08, WHITE)
