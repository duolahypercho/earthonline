# Columned portico with a steep stair (sf-building-259218965): the entry door sits at
# storey-1 level (3.0 m) under the left round bay, three white columns on the porch
# edge, a straight stair down to the pavement flanked by white cheek walls.
G = 3.0
E = 0.215
PORCH = 1.3
W = 'painted-white@f4f0e6'
STEP = 'cast-stone-trim@e8e4da'
kit.box(E, G + 1.1, -0.02, 1.0, 2.2, 0.08, 'wood-siding@2a2420')      # door
kit.box(E, G + 2.3, -0.02, 1.0, 0.3, 0.07, 'glass-clear')              # transom
for dr in (-0.58, 0.58):
    kit.box(E, G + 1.3, 0.0, 0.1, 2.6, 0.06, W, dr=dr)                 # casing
kit.box(E, G - 0.06, 0.0, 3.5, 0.12, PORCH, STEP)                      # landing
kit.box(E, G + 2.5, 0.0, 3.5, 0.18, PORCH, W)                          # porch ceiling / bay belly
for dr in (-1.54, 0.87, 1.37):
    kit.cylinder(E, G, G + 2.4, PORCH - 0.15, 0.11, W, dr=dr)          # columns
n = 11
h = G / n
for k in range(n - 1):                                                 # stair
    kit.box(E, (k + 0.5) * h, 0.0, 1.5, h, PORCH + (n - 1 - k) * 0.22, STEP)
for dr in (-0.85, 0.85):
    kit.box(E, G * 0.45, 0.0, 0.18, G * 0.9, PORCH + 1.0, W, dr=dr)    # cheek walls
