# Mosaic tile base strip, entry canopy and parapet cap (street photo).
import random
random.seed(3)
cols = ['4a6fa8','3f9f6e','d2553a','e8d9b0','2f8f9a','f0b531','7a4aa0','8a8f98']
def strip(skip=None):
    n = int(kit.L / 0.32)
    for i in range(n):
        f = (i + 0.5) / n
        if skip and skip[0] < f < skip[1]:
            continue
        kit.box(f, 0.15, 0.03, 0.30, 0.30, 0.03, 'painted-stucco@' + random.choice(cols))
    kit.box(0.5, 7.0 - 0.07, 0.0, kit.L, 0.14, 0.42, 'painted-white@5a6590')  # parapet cap
strip()
kit.edge(3)
strip((0.03, 0.23))
kit.box(0.13, 3.2, 0.35, 0.16 * kit.L + 0.4, 0.12, 0.7, 'painted-steel@3a3f4a')  # entry canopy
# teal clerestory glazing band continues along the second street edge (hbSignBands covers edge 4 only)
kit.box(0.5, 4.4, 0.06, kit.L, 0.9, 0.1, 'glass-reflective@2f9a98')
