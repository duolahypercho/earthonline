# 531-style flats: continuous wrought-iron balconies across the front on each upper floor
# (photo shows them running the full width) and a heavy string course over the base.
IRON = 'painted-steel@2a2a2a'
STONE = 'painted-stucco@c4b48c'
L = kit.L
LV = [0.0, 4.771, 8.074, 11.377]
kit.box(0.5, LV[1] + 0.08, 0.0, L, 0.2, 0.3, STONE)            # string course
for y in LV[1:]:
    kit.box(0.5, y + 0.02, 0.0, 0.86 * L, 0.1, 0.55, IRON)      # balcony slab
    kit.box(0.5, y + 1.0, 0.5, 0.86 * L, 0.05, 0.04, IRON)      # top rail
    kit.box(0.5, y + 0.15, 0.5, 0.86 * L, 0.03, 0.03, IRON)     # bottom rail
    n = int(0.86 * L / 0.4)
    for i in range(n + 1):
        kit.box(0.07 + 0.86 * i / n, y + 0.58, 0.5, 0.02, 0.85, 0.02, IRON)   # balusters
