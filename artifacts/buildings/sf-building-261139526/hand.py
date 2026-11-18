# Hand ornament: projecting moulded belt cornice with a dentil course between the
# buff brick base and the first bay floor (photo: heavy cream band, small dentils).
L = kit.L
TOP = 3.75
CREAM = 'painted-white@eee6d2'
kit.box(0.5, TOP - 0.1, 0.0, L, 0.22, 0.26, CREAM)       # frieze band
kit.box(0.5, TOP + 0.06, 0.0, L, 0.1, 0.38, CREAM)       # corona
n = int(L / 0.22)
for k in range(n):
    f = (k + 0.5) / n
    kit.box(f, TOP - 0.28, 0.0, 0.11, 0.1, 0.3, CREAM)   # dentils under the band
