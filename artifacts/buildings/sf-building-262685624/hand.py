# Hand ornament for the cream flats, Hyde Union-Filbert (sf-building-262685624).
# The reference reads as stacked window columns: a dark red-brown spandrel
# panel fills the wall between one storey's window head and the next one's
# sill, so each column becomes a vertical stripe. Drawn on the centre column
# and the shallow left bay's front. Positions: (fraction along the Hyde front
# as the viewer reads it, height above the front grade, offset out of the wall).
PANEL = 'painted-white@6a2d26'
RAIL = 'painted-white@7a3b32'
LV = [0.0, 3.1, 6.23, 9.36]
SILL, HEAD = 0.85, 2.4
for f, off, w in ((0.46, 0.0, 1.1), (0.2, 0.3, 1.1)):
    for k in (1, 2):
        y0, y1 = LV[k] + HEAD + 0.06, LV[k + 1] + SILL - 0.06
        kit.box(f, (y0 + y1) / 2, off, w, y1 - y0, 0.05, PANEL)
        for dy in (y0 + 0.05, y1 - 0.05):                       # moulded rails
            kit.box(f, dy, off, w + 0.12, 0.08, 0.09, RAIL)
    kit.box(f, LV[3] + HEAD + 0.18, off, w, 0.24, 0.05, PANEL)   # top head band
