# Dark projecting canopy over the arched entry (door spans f .43-.53).
kit.box(0.48, 3.75, 0.6, 2.5, 0.14, 1.2, 'painted-steel@26262a')
for dr in (-1.15, 1.15):
    kit.bracket(0.48, 3.7, 0.15, 0.12, 0.45, 0.5, 'painted-steel@26262a', dr=dr)
# Cartouche panels on the front facet of every bay, in the spandrels between the
# window tiers (bay centres f .10 .24 .49 .64 .93, projection 0.6 m; plates 4.71 + 3.35 k).
PANEL = 'painted-stucco@cfc9a8'
RELIEF = 'painted-white@efe9d2'
for f, w in ((0.10, 1.2), (0.24, 0.9), (0.49, 1.5), (0.64, 1.5), (0.93, 1.5)):
    for k in range(1, 4):
        y = 4.709 + 3.35 * k + 0.25
        kit.box(f, y, 0.58, w, 0.62, 0.06, PANEL)
        kit.box(f, y, 0.63, min(0.5, w * 0.5), 0.3, 0.04, RELIEF)
