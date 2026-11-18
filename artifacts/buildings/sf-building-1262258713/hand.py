# Two round portholes with stucco surrounds between the floors at the left (photo sv-h231).
for y in (6.6, 9.9):
    for r in (1, -1):
        kit.arch_ring(0.12, y, 0.6, 0.3 * r, 0.09, 0.05, 'painted-stucco@f0d8a8')
        kit.arch_fill(0.12, y, 0.6, 0.3 * r, 'glass-clear', off=0.02)
