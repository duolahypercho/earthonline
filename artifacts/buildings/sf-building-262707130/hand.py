# Hand ornament for 1908 shingle house, Hyde at Filbert (sf-building-262707130).
# Raised main floor over a brick basement: the entry sits up a straight stair
# under a gabled porch hood on two square posts; a small gabled hood over the
# corner window. Positions: (fraction along the Hyde front as the viewer reads
# it, height above the front grade, offset out of the wall).
G = 1.6                      # main floor = top of the brick basement
TRIM = 'painted-white@2a382e'
ROOF = 'asphalt-roof@3e423e'
WALL = 'wood-siding@737566'
STEP = 'concrete-precast@9c9a94'
DOOR = 'wood-siding@2f3d34'

E = 0.13                      # entry centre
PORCH = 1.0                   # porch landing depth
# entry door with a transom and a casing
kit.box(E, G + 1.05, -0.02, 0.95, 2.1, 0.08, DOOR)
kit.box(E, G + 2.32, -0.02, 0.95, 0.34, 0.07, 'glass-clear')
for dr in (-0.55, 0.55):
    kit.box(E, G + 1.25, 0.0, 0.12, 2.5, 0.06, TRIM, dr=dr)
kit.box(E, G + 2.55, 0.0, 1.3, 0.14, 0.08, TRIM)
# landing, posts and the gabled hood
kit.box(E, G - 0.06, 0.0, 1.9, 0.12, PORCH, STEP)
for dr in (-0.85, 0.85):
    kit.box(E, G + 1.4, PORCH - 0.16, 0.16, 2.8, 0.16, TRIM, dr=dr)
    kit.box(E, G + 0.06, PORCH - 0.16, 0.24, 0.12, 0.24, TRIM, dr=dr)
kit.box(E, G + 2.86, PORCH - 0.16, 1.9, 0.18, 0.16, TRIM)    # lintel beam
kit.gable(E, G + 2.95, 1.9, PORCH + 0.1, 0.95, ROOF, WALL, soffit=TRIM, eave=0.18)
kit.gable(E, G + 2.95, 2.0, PORCH + 0.14, 0.99, TRIM, TRIM, eave=0.0, off=PORCH)  # fascia trim on the pediment
# straight stair down to the sidewalk: 9 risers
n = 9
h = G / n
for k in range(n - 1):
    run = PORCH + (n - 1 - k) * 0.22
    kit.box(E, (k + 0.5) * h, 0.0, 1.5, h, run, STEP)
# cheek walls (brick) both sides of the flight
for dr in (-0.82, 0.82):
    kit.box(E, G / 2, 0.0, 0.16, G, PORCH + 0.6, 'brick-running-bond@6e3c30', dr=dr)
# small gabled hood over the corner window
kit.gable(0.925, G + 2.45, 0.7, 0.28, 0.32, ROOF, WALL, soffit=TRIM, eave=0.08)
for dr in (-0.36, 0.36):
    kit.bracket(0.925, G + 2.45, 0.0, 0.08, 0.35, 0.26, TRIM, dr=dr)
