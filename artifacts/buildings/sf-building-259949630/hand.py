# Hand ornament: white frieze + dentil band between the rusticated base and the cream upper wall,
# and low barred basement windows under the base.
G=4.695; U=3.395
lv2=G+U
kit.box(0.5, lv2-0.2, 0.0, kit.L, 0.45, 0.3, 'painted-white@f2ece0')
kit.box(0.5, lv2+0.1, 0.0, kit.L, 0.12, 0.45, 'painted-white@f2ece0')
for _i in range(int(kit.L/0.3)):
    kit.box((_i*0.3+0.2)/kit.L, lv2-0.5, 0.0, 0.13, 0.13, 0.36, 'painted-white@f2ece0')   # dentils
for f in (0.09,0.22,0.37):
    kit.box(f, 0.7, 0.0, 1.0, 0.8, 0.05, 'painted-steel@2a2a2a')   # barred basement window
    for n in range(6):
        kit.box(f, 0.7, 0.05, 0.025, 0.8, 0.03, 'painted-steel@8a8a8a', dr=-0.4+n*0.16)
