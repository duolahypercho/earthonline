# Hand ornament: heavy moulded belt cornice with dentils between the first floor and the bays.
kit.box(0.5, 4.498, 0.0, kit.L, 0.34, 0.45, 'painted-white@e0dcd0')   # heavy belt course
kit.box(0.5, 4.2780000000000005, 0.0, kit.L-0.2, 0.1, 0.35, 'painted-white@e0dcd0')   # soffit step
for _i in range(int(kit.L/0.3)):
    kit.box((_i*0.3+0.2)/kit.L, 4.178, 0.0, 0.12, 0.12, 0.37, 'painted-white@e0dcd0')   # dentils
