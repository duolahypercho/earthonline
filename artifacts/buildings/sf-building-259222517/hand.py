# Hand ornament, 1911 flats (photo-read). Ground 4.6 m, uppers ~3.7 m.
GOLD = 'painted-white@b8902f'
GREEN = 'painted-white@12362c'
CREAM = 'painted-white@e3d9bd'
L = kit.L


def window(f0, f1, y0, y1, lights=3, bar=0.07):
    """Dark-green framed window: four rails, mullions, glass behind."""
    fc, w, h, yc = (f0 + f1) / 2, (f1 - f0) * L, y1 - y0, (y0 + y1) / 2
    kit.box(fc, yc, 0.0, w, h, 0.03, 'glass-clear')
    kit.box(fc, y0 + bar / 2, 0.0, w, bar, 0.09, GREEN)
    kit.box(fc, y1 - bar / 2, 0.0, w, bar, 0.09, GREEN)
    for sgn in (-1, 1):
        kit.box(fc, yc, 0.0, bar, h, 0.09, GREEN, dr=sgn * (w - bar) / 2)
    for i in range(1, lights):
        kit.box(fc, yc, 0.0, bar * 0.8, h, 0.08, GREEN, dr=-w / 2 + w * i / lights)


# left ground window: three lights over a short brick wall; window over the garage
window(0.06, 0.28, 2.9, 4.1)
window(0.7, 0.95, 3.2, 4.2)
# gilt hood moulds over the ground windows and over every upper window / bay
for f0, f1, y in ((0.06, 0.28, 4.1), (0.7, 0.95, 4.2)):
    kit.box((f0 + f1) / 2, y + 0.1, 0.0, (f1 - f0) * L + 0.2, 0.18, 0.12, GOLD)
for head in (7.0, 10.4):
    kit.box(0.5, head + 0.12, 0.0, 2.1, 0.2, 0.1, GOLD)                 # centre pair
for head in (8.05, 10.45):
    for f in (0.19, 0.81):
        kit.box(f, head + 0.12, 0.7, 2.7, 0.2, 0.1, GOLD)              # bay head
# cream quoins at both corners of the brick ground storey
for f, sgn in ((0.012, 1), (0.988, -1)):
    for i in range(9):
        wq = 0.5 if i % 2 == 0 else 0.3
        kit.box(f, 0.25 + i * 0.5, 0.0, wq, 0.4, 0.05, CREAM, dr=sgn * wq / 2)
