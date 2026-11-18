# Cream lozenge panels under the upper windows (visible in the photo), storeys 2-4.
# Level heights from the spec: ground 4.815, uppers 3.426.
CREAM = 'cast-stone-trim@ece4d0'
for k in (2, 3, 4):
    y = 4.815 + (k - 1) * 3.426 + 0.4
    for f in (0.25, 0.39, 0.54, 0.83):
        kit.box(f, y, 0.02, 0.9, 0.2, 0.05, CREAM)        # panel
        kit.box(f, y, 0.04, 0.32, 0.12, 0.04, CREAM)       # lozenge centre
