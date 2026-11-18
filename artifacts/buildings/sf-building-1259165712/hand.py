# Brick front (sf-building-1259165712). The cornice and trim are brick; the
# street photo shows white stone lintels and sills, a white dentil course
# (style hbCorniceDentils) and a low white pediment at the middle of the parapet.
W = 'painted-white@e6dfd0'
for f in (0.22, 0.7):
    for sill, head in ((5.30, 6.75), (8.17, 9.62)):
        kit.box(f, head + 0.14, 0.0, 2.35, 0.22, 0.12, W)    # lintel
        kit.box(f, sill - 0.05, 0.0, 2.25, 0.10, 0.14, W)    # sill
kit.gable(0.5, 10.8, 2.6, 0.15, 0.3, W, W)                   # shallow pediment
