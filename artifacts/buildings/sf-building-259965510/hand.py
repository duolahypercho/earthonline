# One round porthole above the door, stucco surround (photo sv-h261). Rings from two half-arches.
for r in (1, -1):
    kit.arch_ring(0.22, 6.0, 0.6, 0.3 * r, 0.09, 0.05, 'painted-stucco@f0d8a8')
    kit.arch_fill(0.22, 6.0, 0.6, 0.3 * r, 'glass-clear', off=0.02)
