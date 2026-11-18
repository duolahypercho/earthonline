# Timber posts under the full-length shingled lean-to seen in the photo (posts every ~4.4 m).
for f in (0.02, 0.2, 0.4, 0.6, 0.8, 0.98):
    kit.box(f, 1.35, 2.25, 0.16, 2.7, 0.16, 'wood-siding@2a2622')
# Deep dark eave/fascia band at the roof line (photo: low-pitch roof with an overhanging eave over the board wall).
kit.box(0.5, 5.2, 0.0, kit.L, 0.2, 0.55, 'asphalt-roof@3a3834')
