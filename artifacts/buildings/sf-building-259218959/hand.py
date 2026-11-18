# Porch over the gated entry (sf-building-259218959): two white columns carry a
# deck with a balustrade; the photo shows this over the right half of the ground storey.
W = 'painted-white@fbfaf4'
C = 0.74            # porch centre (fraction along the front)
G = 3.3             # ground storey height = deck level
for dr in (-1.05, 1.05):
    kit.cylinder(C, 0.15, G, 0.75, 0.11, W, dr=dr)        # columns
    kit.box(C, 0.08, 0.75, 0.34, 0.16, 0.34, W, dr=dr)     # plinths
kit.box(C, G + 0.06, 0.4, 2.6, 0.18, 0.8, W)                # deck slab / entablature
kit.box(C, G + 0.7, 0.75, 2.5, 0.07, 0.07, W)               # top rail
kit.box(C, G + 0.28, 0.75, 2.5, 0.07, 0.07, W)              # bottom rail
for k in range(9):                                          # balusters
    kit.cylinder(C, G + 0.28, G + 0.7, 0.75, 0.025, W, dr=-1.1 + k * 0.275)
