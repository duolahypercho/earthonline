# Stepped fluted fins of the blue-grey 1930 deco building (sf-building-260146980).
H = 11.8
F = 'painted-stucco@7a8ea8'
D = 'painted-stucco@3f4f66'
for f, top in ((0.04, H + 1.3), (0.35, H + 0.6), (0.63, H + 0.6), (0.95, H + 1.3)):
    kit.box(f, (3.0 + top) / 2, 0.0, 0.42, top - 3.0, 0.32, F)
    kit.box(f, (3.0 + top) / 2, 0.17, 0.12, top - 3.0, 0.05, F)      # flute rib
    kit.box(f, top + 0.05, 0.0, 0.5, 0.1, 0.38, D)                   # dark cap
    kit.box(f, top - 0.35, 0.0, 0.3, 0.3, 0.36, F)                   # step
kit.box(0.5, 4.4, 0.0, 1.5, 1.0, 0.06, 'cast-stone-trim@a8a090')   # relief panel over entry
for i in range(5):
    kit.box(0.5, 4.0 + i * 0.2, 0.04, 1.3, 0.04, 0.03, 'cast-stone-trim@7d7468')  # texture bands
