# Small gabled shingle porch on white square posts at the entry (photo h290), right of the
# French doors. Eave 2.6 m, 2.4 m wide, 1.6 m deep.
WHITE = 'painted-white@f6f4ee'
kit.gable(0.8, 2.6, 2.4, 1.6, 0.45, 'asphalt-roof@7a7468', WHITE, soffit=WHITE, eave=0.1)
for dr in (-1.15, 1.15):
    kit.box(0.8, 1.3, 1.45, 0.14, 2.6, 0.14, WHITE, dr=dr)
