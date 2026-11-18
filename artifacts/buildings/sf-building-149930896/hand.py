# Wrought-iron balconies under the three wide storey-1 windows (street still: viewer left to right).
IRON = 'painted-steel@2a2a2a'
for f, w in ((0.30, 1.7), (0.69, 1.5), (0.93, 1.3)):
    y = 5.5 + 0.45                       # slab just under the window sill
    kit.box(f, y, 0.36, w, 0.1, 0.72, IRON)
    kit.box(f, y + 1.0, 0.70, w, 0.04, 0.04, IRON)      # top rail
    kit.box(f, y + 0.15, 0.70, w, 0.03, 0.03, IRON)     # bottom rail
    n = int(w / 0.13)
    for j in range(n + 1):
        kit.box(f, y + 0.55, 0.70, 0.02, 0.9, 0.02, IRON, dr=-w / 2 + w * j / n)
