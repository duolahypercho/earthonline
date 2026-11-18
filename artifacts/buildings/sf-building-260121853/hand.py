# Hand ornament, Hyde 1909 flats (photo-checked).
# Basement windows with grilles (small low windows under the ground windows)
for f in (0.16, 0.615):
    kit.box(f, 0.95, 0.02, 0.8, 0.7, 0.05, 'glass-reflective@2a2e34')
    kit.box(f, 0.95, 0.06, 0.9, 0.06, 0.06, 'painted-steel@202226')
    for k in range(5):
        kit.box(f, 0.95, 0.07, 0.03, 0.7, 0.03, 'painted-steel@202226', dr=(k - 2) * 0.17)
# Ground windows are tripartite: two thin white mullions, plus a green diamond-tile frieze above each
for f in (0.2, 0.615):
    for dr in (-0.29, 0.29):
        kit.box(f, 2.4, 0.05, 0.05, 1.9, 0.05, 'painted-white@f0ead8', dr=dr)
    kit.box(f, 3.75, 0.03, 1.8, 0.3, 0.03, 'painted-white@3f6b4a')
    for k in range(7):
        kit.box(f, 3.75, 0.06, 0.18, 0.18, 0.02, 'painted-white@8fb07a', dr=(k - 3) * 0.25)
# Raised arch surround with a keystone over the recessed entry
kit.arch_ring(0.405, 2.6, 1.9, 0.95, 0.17, 0.06, 'painted-white@f4f2e8', off=0.02, keystone='painted-white@e8e4d4')
