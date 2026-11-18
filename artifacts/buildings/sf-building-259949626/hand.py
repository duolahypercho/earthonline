# Hand ornament: heavy cream belt cornice over storey 1 and white urn cartouches
# on the dark spandrels of the two cant bays (photo: urn plaques between bay storeys).
C='painted-white@ece6d6'
G=4.468; U=3.23
kit.box(0.5, 4.418, 0.0, kit.L, 0.32, 0.45, 'painted-white@ece6d6')   # heavy belt course
kit.box(0.5, 4.208, 0.0, kit.L-0.2, 0.1, 0.35, 'painted-white@ece6d6')   # soffit step
for k in (3,4,5):
    for f in (0.19,0.795):
        kit.box(f, G+(k-1)*U-0.35, 0.74, 0.7, 0.38, 0.04, C)   # cartouche panel
        kit.box(f, G+(k-1)*U-0.35, 0.78, 0.22, 0.22, 0.03, C)   # urn boss
