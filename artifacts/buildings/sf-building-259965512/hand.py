# Sunburst relief over the central arched door, oval medallion in its middle (photo sv-h261).
# Sits in the base storey above the arch crown, below the belt course.
S = 'cast-stone-trim@d8d2c4'
kit.box(0.52, 4.05, 0.03, 2.6, 0.7, 0.05, 'painted-white@cdc6b6')       # relief field
for k in range(-4, 5):
    kit.box(0.52, 4.05, 0.06, 0.05, 0.55, 0.03, S, dr=k * 0.28)          # sun rays
kit.cylinder(0.52, 3.85, 4.3, 0.09, 0.2, 'cast-stone-trim@9a8a70')       # medallion (round stand-in for the oval)
# Attic panel with a raised border on the low front, under the parapet ledge (photo sv-h261).
kit.box(0.5, 5.75, 0.04, 5.2, 1.0, 0.06, 'painted-white@d8d2c4')
kit.box(0.5, 5.75, 0.08, 4.7, 0.65, 0.04, 'painted-white@cdc6b6')
