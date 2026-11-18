# Deep vertical concrete fins between window bays and scored joints on the plinth (street photo).
C = 'concrete-precast@d2c8b4'
J = 'concrete-precast@8d8576'
H = 24.4
for k in range(1, 6):
    f = k / 6
    kit.box(f, (5.0 + H - 0.6) / 2, 0.0, 2.0, H - 0.6 - 5.0, 0.6, C)
for y in (1.2, 2.4, 3.6, 4.8):
    kit.box(0.5, y, 0.03, kit.L, 0.06, 0.03, J)          # horizontal board joints
for i in range(1, 8):
    kit.box(i / 8, 2.5, 0.03, 0.06, 5.0, 0.03, J)        # vertical scored joints
kit.box(0.5, 5.05, 0.1, kit.L, 0.2, 0.2, C)              # plinth cap ledge
