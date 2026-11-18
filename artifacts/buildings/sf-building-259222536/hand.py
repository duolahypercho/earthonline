# Photo: red clay-tile hood on the bay, red-trimmed storey bands, dark-red
# surround round the two white garage doors. Floors: ground 2.9, uppers 2.85.
RED = 'painted-white@a8402f'
kit.box(0.5, 10.95, 0.28, 2.2, 0.22, 0.9, 'painted-white@b4532f')      # tile cap on the bay
for y in (2.95, 5.8, 8.65):                                              # storey bands
    kit.box(0.5, y, 0.0, kit.L, 0.16, 0.06, RED)
# garage surround: head beam, centre post, two jambs
kit.box(0.5, 2.85, 0.0, kit.L * 0.92, 0.14, 0.07, RED)
kit.box(0.5, 1.4, 0.0, 0.12, 2.8, 0.07, RED)
for sgn in (-1, 1):
    kit.box(0.5, 1.4, 0.0, 0.12, 2.8, 0.07, RED, dr=sgn * kit.L * 0.46)
