# Stepped deco crest of the tan 1931 office (sf-building-260146977): tall centre block
# with a relief panel, two lower steps, small edge blocks. Seen in the street photo.
H = 8.9
T = 'painted-stucco@d6c1a0'
R = 'cast-stone-trim@c8b090'
kit.box(0.5, H + 0.7, 0.0, 3.4, 1.4, 0.30, T)        # centre crest
kit.box(0.5, H + 0.75, 0.16, 2.0, 0.8, 0.04, R)      # relief panel
kit.box(0.5, H + 1.45, 0.0, 3.6, 0.12, 0.38, R)      # crest cap
kit.box(0.21, H + 0.35, 0.0, 1.2, 0.7, 0.26, T)      # lower step left
kit.box(0.79, H + 0.35, 0.0, 1.2, 0.7, 0.26, T)      # lower step right
kit.box(0.07, H + 0.3, 0.0, 0.9, 0.6, 0.22, T)       # edge blocks
kit.box(0.93, H + 0.3, 0.0, 0.9, 0.6, 0.22, T)
# relief panels on the raised crest blocks (photo: carved panels on the steps)
kit.box(0.21, H + 0.35, 0.14, 0.7, 0.4, 0.03, R)
kit.box(0.79, H + 0.35, 0.14, 0.7, 0.4, 0.03, R)
# decorative metal grille transom over the glazed entry + address plaque
for i in range(14):
    kit.box(0.31 + 0.64 * (i + 0.5) / 14, 3.05, 0.04, 0.04, 0.3, 0.03, 'painted-steel@2a2a2a')
kit.box(0.99, 1.5, 0.05, 0.3, 0.2, 0.03, 'painted-steel@2a2a2a')  # address plaque
# tall narrow slit windows in the piers (photo); the left one has a round head
for f, rh in ((0.1, True), (0.9, False)):
    kit.box(f, 6.0, 0.02, 0.45, 2.4, 0.04, 'glass-reflective@1c2430')
    kit.box(f, 4.9, 0.03, 0.55, 0.12, 0.06, R)   # sill
