# Hand ornament for the 1912 round-bay flats, Hyde Union-Filbert (sf-building-262685651).
# The covered porch between the garages: two cream Tuscan columns carrying a
# moulded entablature across the porch mouth and a stone threshold step. Positions: (fraction along
# the Hyde front as the viewer reads it, height above the front grade, offset).
CREAM = 'painted-white@ece6d4'
STONE = 'cast-stone-trim@d8d0bc'
L = kit.L
TOP = 3.1                       # porch head (layout headM)
f0, f1 = 0.36, 0.55
for f in (f0 + 0.35 / L, f1 - 0.35 / L):
    kit.column(f, 0.0, TOP, 0.05, 0.15, CREAM)
fc, w = (f0 + f1) / 2, (f1 - f0) * L
kit.box(fc, TOP + 0.12, 0.05, w + 0.3, 0.24, 0.4, CREAM)            # architrave
kit.box(fc, TOP + 0.33, 0.05, w + 0.3, 0.18, 0.34, CREAM)           # frieze
kit.box(fc, TOP + 0.47, 0.05, w + 0.55, 0.1, 0.52, CREAM)           # cornice
kit.box(fc, 0.07, 0.0, w + 0.2, 0.14, 0.55, STONE)                  # threshold step
