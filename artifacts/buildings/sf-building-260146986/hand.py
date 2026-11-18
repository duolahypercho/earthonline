# Clerestory muntins and a corbelled panel line on the brick parapet (street photo).
for i in range(1, 22):
    kit.box(0.05 + 0.75 * i / 22, 3.9, 0.07, 0.04, 0.7, 0.03, 'painted-steel@161c1c')
kit.box(0.5, 4.38, 0.04, kit.L * 0.9, 0.05, 0.03, 'painted-stucco@156a60')   # corbel line
kit.box(0.5, 4.7, 0.04, kit.L * 0.9, 0.05, 0.03, 'painted-stucco@156a60')
