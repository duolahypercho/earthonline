# Open carport railing (photo: steel picket rail on the dark low wall, 1.5 m up).
# Top and mid rail along the whole front plus pickets between the piers.
W = kit.L
kit.box(0.5, 2.45, 0.05, W, 0.05, 0.05, 'painted-steel@2a2e30')
kit.box(0.5, 1.55, 0.05, W, 0.05, 0.05, 'painted-steel@2a2e30')
n = int(W / 0.2)
for i in range(1, n):
    kit.box(i / n, 2.0, 0.05, 0.02, 0.9, 0.02, 'painted-steel@2a2e30')

# Right-end stacked balconies with solid parapets (photo): deep slab + solid front/side
# parapets at each upper floor line (ground 3.9 m, uppers ~2.85 m).
for k in range(1, 5):
    y = 3.9 + 2.85 * (k - 1)
    kit.box(0.925, y, 0.55, 2.6, 0.15, 1.1, 'concrete-precast@cfc4b4')
    kit.box(0.925, y + 0.6, 1.08, 2.6, 1.0, 0.08, 'painted-stucco@d4bc9c')
    kit.box(0.925, y + 0.6, 0.55, 0.08, 1.0, 1.1, 'painted-stucco@d4bc9c', dr=-1.26)
