# Columned entry porch with a balustraded balcony on top (sf-building-261111927):
# the door is raised to storey-1 level (2.6 m); two white columns carry an
# entablature and the storey-2 balcony deck with a balustrade; a steep stair
# runs down to the pavement.
G = 2.6
C = 0.69
PORCH = 1.2
W = 'painted-white@f0eee6'
STEP = 'cast-stone-trim@e0ddd2'
kit.box(C, G + 1.1, -0.02, 1.1, 2.2, 0.08, 'wood-siding@3a2a24')       # door
kit.box(C, G - 0.06, 0.0, 3.3, 0.12, PORCH, STEP)                       # landing
for dr in (-1.3, 1.3):
    kit.cylinder(C, G, G + 3.1, PORCH - 0.12, 0.13, W, dr=dr)           # columns
kit.box(C, G + 3.3, 0.0, 3.5, 0.4, PORCH + 0.05, W)                     # entablature
kit.box(C, G + 3.5, 0.0, 3.5, 0.1, PORCH + 0.3, W)                      # balcony deck
kit.box(C, G + 4.15, PORCH + 0.2, 3.4, 0.07, 0.07, W)                   # top rail
for k in range(13):
    kit.cylinder(C, G + 3.55, G + 4.15, PORCH + 0.2, 0.025, W, dr=-1.6 + k * 0.267)
n = 8
h = G / n
for k in range(n - 1):
    kit.box(C, (k + 0.5) * h, 0.0, 1.6, h, PORCH + (n - 1 - k) * 0.24, STEP)
for dr in (-0.9, 0.9):
    kit.box(C, G * 0.45, 0.0, 0.18, G * 0.9, PORCH + 0.9, W, dr=dr)     # cheek walls
