# Hand ornament, Hyde 1911 narrow flats: coloured tile spandrels read off the photo.
# A brown/blue patterned tile panel at the floor line between the two bay storeys,
# and two green cross-tile panels near the top. Heights: ground 4.37, uppers 3.02.
L = kit.L
# brown panel with a row of blue tiles (bay face is 0.5 m proud)
kit.box(0.5, 7.4, 0.56, 3.4, 0.8, 0.04, 'painted-white@7a5238')
for k in range(7):
    kit.box(0.5, 7.4, 0.6, 0.34, 0.34, 0.03, 'painted-white@3a5a8a', dr=(k - 3) * 0.46)
# two green tile panels with a pale cross
for f in (0.3, 0.7):
    kit.box(f, 10.9, 0.03, 1.5, 0.7, 0.04, 'painted-white@3c8a58')
    kit.box(f, 10.9, 0.07, 0.62, 0.1, 0.03, 'painted-white@d8e4c8')
    kit.box(f, 10.9, 0.07, 0.1, 0.5, 0.03, 'painted-white@d8e4c8')
