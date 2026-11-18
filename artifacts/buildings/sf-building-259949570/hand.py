# Hand ornament: pilasters with capitals between the three ground arches, and the projecting
# balcony slab with a white railing over the arcade, running the width of the facade.
C='painted-white@ece6d6'
L=kit.L
for f in (0.085,0.33,0.695,0.945):
    kit.box(f, 1.2, 0.0, 0.32, 2.4, 0.2, C)
    kit.box(f, 2.5, 0.0, 0.46, 0.18, 0.3, C)       # capital
    kit.box(f, 0.1, 0.0, 0.42, 0.2, 0.26, C)       # base
kit.box(0.5, 3.55, 0.0, L, 0.2, 1.0, C)           # balcony slab
kit.box(0.5, 4.55, 0.95, L, 0.06, 0.06, C)
kit.box(0.5, 3.75, 0.95, L, 0.05, 0.05, C)
for n in range(int(L/0.14)+1):
    kit.box((n*0.14)/L, 4.15, 0.95, 0.03, 0.8, 0.03, C)
