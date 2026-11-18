# Hand ornament for the stucco flats over shops at Hyde and Union (sf-building-262685625).
# The corner restaurant front (layout 0.61-0.955) sits in a chunky white
# rubble-stone surround: a segmental arch of alternating deep and shallow
# voussoirs springing from stacked block jambs either side. Positions:
# (fraction along the Hyde front as the viewer reads it, height above the
# front grade, offset out of the wall).
STONE = 'cast-stone-trim@ecebe6'
STONE2 = 'cast-stone-trim@dcdad2'
L = kit.L
f0, f1 = 0.61, 0.955
fc, span = (f0 + f1) / 2, (f1 - f0) * L
SPRING, RISE = 2.35, 0.85
kit.arch_ring(fc, SPRING, span, RISE, 0.34, 0.1, STONE, n=17, keystone=STONE2)
kit.arch_ring(fc, SPRING, span + 0.36, RISE + 0.12, 0.12, 0.06, STONE2, n=9)   # stepped outer course
# block jambs: alternating long and short stones up both sides
for side in (-1, 1):
    for k in range(6):
        long = k % 2 == 0
        w = 0.5 if long else 0.32
        dr = side * (span / 2 + w / 2 - 0.05)
        kit.box(fc, 0.2 + k * 0.4, 0.0, w, 0.36, 0.1 if long else 0.08, STONE if long else STONE2, dr=dr)
