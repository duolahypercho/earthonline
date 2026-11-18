# Navy spandrel panels with a gold medallion on the front facet of each bay,
# between the window tiers (the street photo shows them on every bay).
# Bay centres f .22 and .76, projection 0.6 m; storey plates at 4.65 + 3.31 k.
NAVY = 'painted-stucco@26356e'
GOLD = 'painted-steel@d6a82c'
for f in (0.22, 0.76):
    for k in range(1, 4):
        y = 4.654 + 3.311 * k + 0.25
        kit.box(f, y, 0.58, 1.5, 0.62, 0.06, NAVY)      # panel
        kit.box(f, y, 0.63, 0.34, 0.34, 0.04, GOLD)     # medallion
