# Dark-brown woodwork seen in the photo: a belt band at ground sill level, heavy
# hoods over the centre windows of the two upper storeys, and a pedimented
# entablature with pilasters over the arched entry. Floors: ground 5.0, uppers 3.27.
BR = 'wood-siding@4a2a26'
L = kit.L
kit.box(0.5, 1.0, 0.0, L, 0.16, 0.03, 'painted-white@6a2a2e')           # maroon belt band
for y0 in (5.85, 9.13):                                                  # window head 1.5 m over sill
    y1 = y0 + 1.5
    for dr in (-0.62, 0.62):
        kit.box(0.52, (y0 + y1) / 2, 0.0, 0.14, 1.6, 0.07, BR, dr=dr)    # jambs
    kit.box(0.52, y1 + 0.12, 0.0, 1.5, 0.2, 0.14, BR)                    # frieze + hood
    kit.box(0.52, y1 + 0.26, 0.0, 1.7, 0.08, 0.22, BR)                   # cap
# entry portico: pilasters, entablature and a low pediment
for dr in (-1.55, 1.55):
    kit.box(0.54, 1.5, 0.0, 0.22, 3.0, 0.14, BR, dr=dr)
kit.box(0.54, 3.1, 0.0, 3.6, 0.26, 0.3, BR)
kit.gable(0.54, 3.3, 3.4, 0.2, 0.35, BR, BR)
