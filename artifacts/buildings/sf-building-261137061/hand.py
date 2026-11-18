# White balustrades at the two arched ground windows and a balustraded balconette
# on the first upper storey centre (photo). Ground 3.8 m.
WHITE = 'painted-white@f0ede6'
for f in (0.235, 0.825):
    kit.box(f, 0.8, 0.05, 1.5, 0.5, 0.06, WHITE)           # balustrade panel
    kit.box(f, 1.1, 0.07, 1.6, 0.07, 0.1, WHITE)           # top rail
kit.box(0.5, 3.9, 0.3, 1.7, 0.1, 0.6, WHITE)                # balconette slab
kit.box(0.5, 4.75, 0.58, 1.7, 0.07, 0.06, WHITE)            # top rail
for i in range(9):
    kit.column(0.5, 4.0, 4.75, 0.58, 0.018, WHITE, dr=(i - 4) * 0.19)
