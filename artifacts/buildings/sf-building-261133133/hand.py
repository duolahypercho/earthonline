# Hand ornament for the 1908 hall/church front (sf-building-261133133).
# Small gabled hoods over the centre door and over the arched double-door entry,
# plus the big front gable of the church end (photo sv-h261/h291) with a blue
# arched stained-glass window in it. Walls are capped at 7.2 m.
ROOF = 'asphalt-roof@4a5568'
kit.gable(0.473, 2.9, 1.7, 0.7, 0.8, ROOF, 'wood-siding@d9d0c2', soffit='painted-white@3d6aa8', eave=0.1)
kit.gable(0.908, 3.1, 1.5, 0.6, 0.7, ROOF, 'wood-siding@e8e0cf', soffit='painted-white@3d6aa8', eave=0.1)
# front gable over the church end (right-hand section of the front)
kit.gable(0.80, 7.2, 6.0, 0.25, 1.7, ROOF, 'wood-siding@e8e0cf', soffit='painted-white@3d6aa8', eave=0.15)
kit.arch_fill(0.80, 5.2, 2.0, 1.2, 'glass-reflective@3a4f8a', off=0.27, bars=3, bar_mat='painted-white@3d6aa8')
kit.arch_ring(0.80, 5.2, 2.0, 1.2, 0.18, 0.06, 'painted-white@3d6aa8', off=0.27)
