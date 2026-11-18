# Hand ornament for the 1910s buff-brick flats, Hyde Union-Filbert (sf-building-262685621).
# Over the garage, the parlour window has a segmental arched head: a glazed
# fan with radial bars under a buff-brick voussoir ring with a stone keystone.
# The arched porch on the left gets a real stone stair (seven risers) rising
# from the sidewalk into the recess. Positions: (fraction along the Hyde front
# as the viewer reads it, height above the front grade, offset out of the wall).
BRICK = 'brick-running-bond@c8a868'
STONE = 'cast-stone-trim@e6e0d2'
GLASS = 'glass-reflective@7d868c'
FRAME = 'painted-white@f4f0e6'
L = kit.L
# parlour window: the plaque glass runs 3.2-4.05 m, 2.6 m wide at 0.7
kit.arch_fill(0.7, 4.05, 2.6, 0.7, GLASS, off=0.01, bars=3, bar_mat=FRAME)
kit.arch_ring(0.7, 4.05, 2.6, 0.7, 0.28, 0.06, BRICK, keystone=STONE)
kit.box(0.7, 4.07, 0.0, 2.7, 0.08, 0.05, FRAME)                    # spring-line transom
# stair into the porch (layout door 0.06-0.40, stair up to ~1.2 m)
n, rise, tread = 7, 1.2 / 7, 0.27
fc, w = 0.23, (0.40 - 0.06) * L - 0.5
for k in range(n):
    kit.box(fc, (k + 0.5) * rise, -0.9, w, rise, 0.9 + (n - 1 - k) * tread * 0.45, STONE)
# cheek walls of buff brick each side of the flight with a stone cap
for dr in (-(w / 2 + 0.12), w / 2 + 0.12):
    kit.box(fc, 0.55, -0.9, 0.24, 1.1, 0.9 + 1.0, BRICK, dr=dr)
    kit.box(fc, 1.13, -0.9, 0.3, 0.07, 0.9 + 1.05, STONE, dr=dr)
