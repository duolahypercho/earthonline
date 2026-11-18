# Dark brick base with a crisp white plinth band, a white entablature over the base,
# and white balustrade balconettes at the second-floor level near each end.
WHITE = 'painted-white@f2eee6'
G = 4.955
kit.box(0.5, 1.9, 0.0, kit.L, 0.28, 0.06, WHITE)               # plinth band
kit.box(0.5, G + 0.05, 0.0, kit.L, 0.45, 0.2, WHITE)           # entablature
for f in (0.13, 0.9):
    kit.box(f, G + 0.12, 0.0, 3.2, 0.2, 0.5, WHITE)            # slab
    kit.box(f, G + 0.95, 0.46, 3.2, 0.08, 0.1, WHITE)          # top rail
    for i in range(14):
        kit.box(f, G + 0.55, 0.46, 0.08, 0.7, 0.08, WHITE, dr=-1.5 + i * 3.0 / 13)   # balusters
