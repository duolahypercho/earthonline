# Cream stone lintels with keystones over every upper window (3 columns x 7
# storeys; floor lines: ground 3.944, uppers 2.904) and the entry canopy at the
# right-hand end of the front.
STONE = 'cast-stone-trim@e6dcc0'
for k in range(1, 8):
    head = 3.944 + 2.904 * (k - 1) + 2.3
    for f, w in ((0.14, 1.4), (0.34, 1.4), (0.54, 1.4), (0.74, 1.4), (0.93, 1.4)):
        kit.box(f, head + 0.14, 0.05, w, 0.28, 0.1, STONE)          # lintel
        kit.box(f, head + 0.3, 0.07, 0.3, 0.42, 0.1, STONE)          # keystone / ear
# entry canopy over the door at the right end
kit.box(0.91, 3.4, 1.0, 2.2, 0.15, 2.0, 'painted-steel@e8e2d0')
