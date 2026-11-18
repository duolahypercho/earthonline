# Violet piers (sf-building-260142659): saturated violet end piers with a dark
# slot each, stepped caps above the flat parapet, a shadow line where the
# recessed white centre panel meets each pier, and a tile frieze under the windows.
V = 'painted-stucco@5c34b4'
DARK = 'painted-stucco@23164a'
H = 10.8
for f in (0.085, 0.915):
    kit.box(f, H / 2 + 0.45, 0.0, 1.5, H + 0.9, 0.14, V)       # full-height pier + 0.9 m above parapet
    kit.box(f, H + 1.0, 0.0, 1.1, 0.25, 0.12, V)               # stepped cap
    kit.box(f, 7.2, 0.0, 0.26, 2.6, 0.17, DARK)                # narrow vertical window slot
for f in (0.188, 0.812):
    kit.box(f, H / 2, 0.0, 0.07, H, 0.16, DARK)                # shadow line at the recess
# patterned tile frieze: pale tile with a violet inner field under each window strip
for f in (0.30, 0.43, 0.57, 0.70):
    kit.box(f, 4.0, 0.1, 1.0, 0.62, 0.04, 'painted-white@e8e0f4')
    kit.box(f, 4.0, 0.14, 0.7, 0.4, 0.03, V)
