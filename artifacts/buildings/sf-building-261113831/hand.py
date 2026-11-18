# Patterned (diaper) brick panels under the two ground windows: a dark panel,
# a cream border, and a lighter diagonal-lattice read from crossed bars.
for f, w in ((0.17, 0.22), (0.51, 0.2)):
    W = w * kit.L
    kit.box(f, 0.55, 0.02, W, 0.62, 0.04, 'brick-common-bond@5a2418')
    kit.box(f, 0.55 + 0.33, 0.04, W + 0.1, 0.04, 0.05, 'painted-white@d9c9a8')
    kit.box(f, 0.55 - 0.33, 0.04, W + 0.1, 0.04, 0.05, 'painted-white@d9c9a8')
    n = int(W / 0.3)
    for i in range(n + 1):                      # lattice bars (vertical ticks)
        kit.box(f, 0.55, 0.04, 0.03, 0.5, 0.03, 'brick-common-bond@b0623f', dr=-W / 2 + 0.15 + i * (W - 0.3) / max(n, 1))
