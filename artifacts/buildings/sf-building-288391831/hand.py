# Cream carved brackets + hood boards over the window/door heads, a plank step, cream corner boards and a deep dark eave (photo h111).
CREAM = 'painted-white@e2d6bc'
for f, hw in ((0.158, 1.5), (0.242, 1.5), (0.474, 1.5), (0.674, 1.5), (0.874, 1.5)):
    for dr in (-0.62, 0.62):
        kit.bracket(f, 2.85, 0.0, 0.12, 0.4, 0.1, CREAM, dr=dr)
    kit.box(f, 2.9, 0.0, hw, 0.12, 0.08, CREAM)            # hood board over the head
kit.box(0.2, 2.5, 0.0, 1.0, 0.1, 0.08, CREAM)               # door hood
kit.box(0.2, 0.1, 0.0, 1.4, 0.2, 0.8, 'wood-siding@6a4a38') # door step
for f in (0.004, 0.996):
    kit.box(f, 2.0, 0.0, 0.25, 4.0, 0.08, CREAM)            # corner boards
kit.box(0.5, 4.0, 0.0, kit.L, 0.2, 0.7, 'asphalt-roof@4a4440')  # deep shingle eave band
