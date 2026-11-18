# Small balconies with ornate cream railings between the two bay stacks (photo), at the
# foot of the two bay storeys.
CREAM = 'painted-white@f0e8cc'
for y in (3.0 + 2.7, 3.0 + 2 * 2.7):
    kit.box(0.5, y - 0.06, 0.0, 1.9, 0.1, 0.5, CREAM)
    kit.box(0.5, y + 0.38, 0.45, 1.9, 0.75, 0.05, CREAM)
    for dr in (-0.9, -0.45, 0.0, 0.45, 0.9):
        kit.box(0.5, y + 0.38, 0.4, 0.05, 0.75, 0.05, CREAM, dr=dr)
