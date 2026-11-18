# Hand ornament for the 1907 dark-brown flats, Hyde at Allen (sf-building-262707054).
# Edwardian window dress the generator draws as flat trim: heavy dark
# architraves round every flat window (one frame round each storey-1 pair),
# a frieze and a projecting cornice cap over each, a cased double door, and
# deep entablature hoods over the ground windows. Positions: (fraction along
# the Hyde front as the viewer reads it, height above the front grade, offset).
TRIM = 'painted-white@2f221c'
L = kit.L
LV = [0.0, 3.3, 6.707, 10.113]          # hbFloors: ground 3.3, uppers 3.4067
W = 0.85                                # windowW


def surround(f0, f1, y0, y1, jamb=0.14, mullions=(), cap=True, frieze=0.22):
    """Architrave round the span f0..f1 (opening edges) from sill y0 to head y1."""
    fc = (f0 + f1) / 2
    w = (f1 - f0) * L
    for dr in (-(w / 2 + jamb / 2), w / 2 + jamb / 2):
        kit.box(fc, (y0 + y1) / 2, 0.0, jamb, y1 - y0 + 0.1, 0.07, TRIM, dr=dr)
    for f in mullions:
        kit.box(f, (y0 + y1) / 2, 0.0, 0.2, y1 - y0, 0.07, TRIM)
    kit.box(fc, y0 - 0.05, 0.0, w + 2 * jamb + 0.16, 0.09, 0.15, TRIM)       # sill shelf
    kit.box(fc, y0 - 0.2, 0.0, w + 2 * jamb - 0.1, 0.18, 0.05, TRIM)         # apron
    if cap:
        kit.box(fc, y1 + 0.05 + frieze / 2, 0.0, w + 2 * jamb, frieze, 0.06, TRIM)   # frieze
        kit.box(fc, y1 + 0.05 + frieze + 0.03, 0.0, w + 2 * jamb + 0.12, 0.06, 0.13, TRIM)
        kit.box(fc, y1 + 0.05 + frieze + 0.1, 0.0, w + 2 * jamb + 0.26, 0.09, 0.22, TRIM)  # cornice


hw = W / 2 / L
# storey 1: the two flat pairs under the bays, one frame each, and the centre window
for a, b in ((0.13, 0.26), (0.72, 0.85)):
    surround(a - hw, b + hw, LV[1] + 0.85, LV[1] + 2.35, mullions=((a + b) / 2,))
for k in (1, 2, 3):
    surround(0.49 - hw, 0.49 + hw, LV[k] + 0.85, LV[k] + 2.35)
# ground: the big left window, the cased double door, the barred pair
surround(0.07, 0.30, 0.5, 2.4, jamb=0.18, frieze=0.3)
surround(0.41, 0.58, 0.0, 2.6, jamb=0.16, cap=False)
kit.box(0.495, 2.0, 0.0, (0.58 - 0.41) * L, 0.12, 0.08, TRIM)           # transom bar
surround(0.68, 0.87, 0.9, 1.8, jamb=0.16, mullions=(0.775,), frieze=0.26)
