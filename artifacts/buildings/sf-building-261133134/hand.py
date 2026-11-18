# Small red alarm bell and address plaque on the plain wall above the entry (photo).
kit.box(0.47, 3.1, 0.05, 0.14, 0.14, 0.08, 'painted-steel@a82020')
kit.box(0.31, 3.15, 0.03, 0.34, 0.16, 0.03, 'painted-white@d8d4c8')
# One wide blinds window per upper floor on the flat right pier (the frame grid leaves the wall blank).
for lvl in (3.6, 6.75, 9.9):
    c, w, h = lvl + 1.55, 2.0, 1.9
    kit.box(0.76, c, 0.0, w, h, 0.04, 'painted-steel@1c2428')
    kit.box(0.76, c, 0.04, w - 0.2, h - 0.2, 0.02, 'glass-reflective')
    for dy in (-h / 2, h / 2):
        kit.box(0.76, c + dy, 0.0, w + 0.16, 0.08, 0.1, 'painted-white@3f8f88')
    for dr in (-w / 2, w / 2):
        kit.box(0.76, c, 0.0, 0.08, h, 0.1, 'painted-white@3f8f88', dr=dr)
    for k in range(1, 4):
        kit.box(0.76, c - h / 2 + 0.08 + k * (h - 0.16) / 4, 0.06, w - 0.2, 0.02, 0.02, 'painted-white@5a6468')

# Teal ledge across the bottom of the right pier (photo: teal outline round the right bay).
kit.box(0.74, 3.62, 0.06, 0.52 * kit.L, 0.14, 0.12, 'painted-white@6fb0a8')
# Fire-escape stair rising diagonally through the left recessed balconies, one flight per storey.
for lvl in (3.6, 6.75, 9.9):
    if lvl > 9.0:
        continue
    for i in range(9):
        kit.box(0.34 + i * 0.0125, lvl + 0.35 + i * 0.32, -0.45, 0.2, 0.05, 0.7, 'painted-steel@4a5054')
    kit.box(0.34 + 4 * 0.0125, lvl + 1.9, -0.8, 1.5, 0.05, 0.05, 'painted-steel@4a5054')
# Address numerals above the gate.
for j in range(4):
    kit.box(0.17 + j * 0.02, 3.2, 0.03, 0.09, 0.16, 0.02, 'painted-white@3a3a3a')
