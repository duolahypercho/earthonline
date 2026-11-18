# Hand ornament for the 1926 black-base tower (sf-building-259974371): three
# small square-pane windows in the black base above the grey garage door,
# left of the arched entry (photo shows them in the stone base).
GLASS = 'glass-clear'
FRAME = 'painted-white@e8e4da'
for f in (0.075, 0.155, 0.235):
    kit.box(f, 2.95, 0.0, 0.82, 1.1, 0.05, GLASS)
    kit.box(f, 3.52, 0.02, 0.94, 0.07, 0.08, FRAME)       # head
    kit.box(f, 2.38, 0.02, 0.94, 0.07, 0.1, FRAME)        # sill
    for dr in (-0.44, 0.44):
        kit.box(f, 2.95, 0.02, 0.07, 1.2, 0.08, FRAME, dr=dr)
    kit.box(f, 2.95, 0.02, 0.05, 1.1, 0.07, FRAME)        # mullion
