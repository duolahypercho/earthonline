# Hand ornament: vertical pilasters between the upper window bays and between the ground panel bays.
C='painted-white@e6e3da'
G=4.463; H=10.9
for f in (0.02,0.23,0.42,0.605,0.79,0.975):
    kit.box(f, (G+H-0.6)/2, 0.0, 0.7, H-0.6-G, 0.22, C)
for f in (0.15,0.28,0.41,0.525):
    kit.box(f, G/2, 0.0, 0.5, G, 0.2, C)
kit.box(0.5, G+0.05, 0.0, kit.L, 0.22, 0.28, C)    # ground cornice band
