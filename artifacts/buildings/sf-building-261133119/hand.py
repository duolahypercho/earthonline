# House numerals above the entry gate (photo shows four small dark figures on the wall).
for i in range(4):
    kit.box(0.69 + i * 0.04, 3.05, 0.03, 0.1, 0.2, 0.02, 'painted-steel@2a2220')

# Circular iron medallion at the middle of the gate (photo): ring of two arches (top half, bottom half).
kit.arch_ring(0.77, 1.55, 0.7, 0.35, 0.05, 0.05, 'painted-steel@141516', off=0.08)
kit.arch_ring(0.77, 1.55, 0.7, -0.35, 0.05, 0.05, 'painted-steel@141516', off=0.08)
