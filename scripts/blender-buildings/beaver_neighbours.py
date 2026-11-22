"""Architectural context for the two adjoining measured building envelopes."""

import math


def finishes(root, material, scan):
    wood = material("Beaver frontage · smoke-grey lap siding", (0.21, 0.18, 0.15), 0.82)
    scan(root, wood, "WoodSiding011", 2.7, 0.37)
    concrete = []
    for name, rgb, asset in (
        ("Board-marked cement", (0.36, 0.35, 0.31), "wood_textured_concrete"),
        ("Cast concrete", (0.29, 0.27, 0.23), "concrete_wall_008"),
    ):
        mat = material("Courtyard wall · " + name, rgb, 0.89)
        scan(root, mat, asset, 2.4, 0.26)
        concrete.append(mat)
    glaze = material("Neighbour sash · clear glass", (0.82, 0.88, 0.86), 0.04)
    bs = glaze.node_tree.nodes.get("Principled BSDF")
    bs.inputs["Transmission Weight"].default_value = 1
    bs.inputs["IOR"].default_value = 1.47
    blinds = [
        material("Window shade · " + name, rgb, 0.91)
        for name, rgb in (
            ("unbleached", (0.32, 0.25, 0.18)),
            ("grey", (0.18, 0.19, 0.17)),
            ("linen", (0.45, 0.39, 0.30)),
        )
    ]
    inner = material(
        "Neighbouring rooms · shadowed plaster", (0.064, 0.051, 0.039), 0.88
    )
    metal = material("Roof · oxidised vent pipe", (0.17, 0.16, 0.14), 0.68)
    metal.node_tree.nodes.get("Principled BSDF").inputs["Metallic"].default_value = 0.59
    steel = material("Garage door · weathered grey enamel", (0.23, 0.24, 0.23), 0.63)
    tree = steel.node_tree
    nodes = tree.nodes
    shader = nodes.get("Principled BSDF")
    shader.inputs["Metallic"].default_value = 0.24
    coords = nodes.new("ShaderNodeTexCoord")
    split = nodes.new("ShaderNodeSeparateXYZ")
    tree.links.new(coords.outputs["Generated"], split.inputs[0])
    grain = nodes.new("ShaderNodeTexNoise")
    grain.inputs["Scale"].default_value = 14
    grain.inputs["Detail"].default_value = 4
    tree.links.new(coords.outputs["Object"], grain.inputs["Vector"])
    tide = nodes.new("ShaderNodeMath")
    tide.operation = "MULTIPLY_ADD"
    tide.inputs[1].default_value = 0.88
    tide.inputs[2].default_value = -0.66
    tree.links.new(grain.outputs["Fac"], tide.inputs[0])
    damage = nodes.new("ShaderNodeMath")
    damage.operation = "SUBTRACT"
    damage.use_clamp = True
    tree.links.new(tide.outputs[0], damage.inputs[0])
    tree.links.new(split.outputs["Z"], damage.inputs[1])
    chipped = nodes.new("ShaderNodeValToRGB")
    for stop, level, col in zip(
        chipped.color_ramp.elements,
        (0.0, 0.12),
        ((0.24, 0.26, 0.26, 1), (0.25, 0.076, 0.025, 1)),
    ):
        stop.position = level
        stop.color = col
    tree.links.new(damage.outputs[0], chipped.inputs[0])
    tree.links.new(chipped.outputs[0], shader.inputs["Base Color"])
    return wood, concrete, glaze, blinds, inner, metal, steel


def frontage(block, pt, length, base, top, bay, finishes, trim):
    wood, concrete, glass, blinds, interior, metal, steel = finishes

    def panel(mat, t0, t1, h0, h1, d0, d1=None):
        d1 = d0 if d1 is None else d1
        block.face(
            mat, [pt(t0, h0, d0), pt(t1, h0, d1), pt(t1, h1, d1), pt(t0, h1, d0)]
        )

    def ledge(mat, t0, t1, h, back, front):
        block.face(
            mat, [pt(t0, h, back), pt(t1, h, back), pt(t1, h, front), pt(t0, h, front)]
        )

    # The bay's central face aligns with the facade footprint. Its angled
    # returns and the narrow sash alongside it are recessed within the survey.
    sections = (
        (
            (0, 0.25, -0.39, -0.39, True),
            (0.25, 0.32, -0.39, -0.39, False),
            (0.32, 0.414, -0.39, 0, True),
            (0.414, 0.837, 0, 0, True),
            (0.837, 0.932, 0, -0.39, True),
            (0.932, 1, -0.39, -0.39, False),
        )
        if bay
        else (
            (0, 0.085, 0, 0, False),
            (0.085, 0.85, 0, 0, True),
            (0.85, 1, 0, 0, False),
        )
    )
    floors = max(2, round((top - base) / 3.45))
    for level in range(floors):
        floor = base + level * (top - base) / floors
        ceiling = base + (level + 1) * (top - base) / floors
        sill = floor + 0.80
        head = ceiling - 0.48
        if level == 0:
            # The wood superstructure sits on a cement plinth. A sectional
            # garage door and a service door open within its surveyed frontage.
            extent = -0.39 if bay else 0
            openings = []
            if bay:
                openings = [
                    (0.13 - 0.43 / length, 0.13 + 0.43 / length, 2.03, False),
                    (0.63 - 1.41 / length, 0.63 + 1.41 / length, 2.17, True),
                ]
            end = 0
            for left, right, height, garage in openings:
                panel(concrete[0], end, left, base - 2, ceiling, extent)
                panel(concrete[0], left, right, base + height, ceiling, extent)
                end = right
                for t in (left, right):
                    block.face(
                        concrete[0],
                        [
                            pt(t, base, extent),
                            pt(t, base, extent - 0.17),
                            pt(t, base + height, extent - 0.17),
                            pt(t, base + height, extent),
                        ],
                    )
                ledge(concrete[0], left, right, base + height, extent - 0.17, extent)
                door_mat = steel if garage else interior
                panel(door_mat, left, right, base - 0.03, base + height, extent - 0.13)
                if garage:
                    for rail in range(1, 5):
                        h = base + height * rail / 5
                        panel(metal, left, right, h - 0.008, h + 0.008, extent - 0.125)
                    for stile in range(1, 4):
                        t = left + (right - left) * stile / 4
                        panel(
                            steel,
                            t - 0.012 / length,
                            t + 0.012 / length,
                            base,
                            base + height,
                            extent - 0.12,
                        )
                h = base + 0.96
                mid = (left + right) * 0.5 if garage else right - 0.12 / length
                panel(
                    metal,
                    mid - 0.065 / length,
                    mid + 0.065 / length,
                    h,
                    h + 0.022,
                    extent - 0.101,
                )
                ledge(concrete[0], left, right, base, extent - 0.17, extent + 0.04)
                panel(concrete[0], left, right, base - 2, base, extent)
            panel(concrete[0], end, 1, base - 2, ceiling, extent)
            for a, b, da, db, _ in sections:
                panel(
                    trim, a, b, ceiling - 0.075, ceiling + 0.012, da + 0.065, db + 0.065
                )
                block.face(
                    trim,
                    [
                        pt(a, ceiling - 0.075, extent),
                        pt(b, ceiling - 0.075, extent),
                        pt(b, ceiling - 0.075, db + 0.065),
                        pt(a, ceiling - 0.075, da + 0.065),
                    ],
                )
            continue
        for k, (a, b, da, db, window) in enumerate(sections):
            panel(wood, a, b, base - 2 if level == 0 else floor, sill, da, db)
            panel(wood, a, b, head, ceiling, da, db)
            if not window:
                panel(wood, a, b, sill, head, da, db)
                continue
            m = min((b - a) * 0.13, 0.16 / length)
            l, r = a + m, b - m
            slope = (db - da) / (b - a)

            def depth(t):
                return da + (t - a) * slope

            panel(wood, a, l, sill, head, da, depth(l))
            panel(wood, r, b, sill, head, depth(r), db)
            # A 22 cm deep jamb, clear pane and a separately set back room.
            for t, sgn in ((l, -1), (r, 1)):
                block.face(
                    trim,
                    [
                        pt(t, sill, depth(t) + 0.03),
                        pt(t, sill, depth(t) - 0.22),
                        pt(t, head, depth(t) - 0.22),
                        pt(t, head, depth(t) + 0.03),
                    ],
                )
                u = t + sgn * 0.052 / length
                panel(
                    trim,
                    min(t, u),
                    max(t, u),
                    sill - 0.035,
                    head + 0.065,
                    depth(min(t, u)) + 0.023,
                    depth(max(t, u)) + 0.023,
                )
            for h in (sill, head):
                block.face(
                    trim,
                    [
                        pt(l, h, depth(l) - 0.22),
                        pt(r, h, depth(r) - 0.22),
                        pt(r, h, depth(r) + 0.065),
                        pt(l, h, depth(l) + 0.065),
                    ],
                )
                panel(
                    trim, l, r, h - 0.026, h + 0.026, depth(l) + 0.063, depth(r) + 0.063
                )
            panel(interior, l, r, sill, head, depth(l) - 0.39, depth(r) - 0.39)
            shade = blinds[(level + k) % len(blinds)]
            drop = 0.16 + 0.53 * (0.5 + 0.5 * math.sin(8 * level + 3.1 * k))
            panel(
                shade,
                l,
                r,
                head - (head - sill) * drop,
                head,
                depth(l) - 0.275,
                depth(r) - 0.275,
            )
            panel(glass, l, r, sill, head, depth(l) - 0.099, depth(r) - 0.099)
            mullions = max(1, round((r - l) * length / 0.98))
            for i in range(1, mullions):
                t = l + (r - l) * i / mullions
                panel(
                    trim,
                    t - 0.017 / length,
                    t + 0.017 / length,
                    sill,
                    head,
                    depth(t) - 0.064,
                )
            h = sill + (head - sill) * 0.53
            panel(trim, l, r, h - 0.018, h + 0.018, depth(l) - 0.055, depth(r) - 0.055)
    for a, b, da, db, _window in sections:
        for h, projection, thick in (
            (top - 0.19, 0.08, 0.063),
            (top - 0.095, 0.14, 0.078),
            (top + 0.014, 0.18, 0.04),
        ):
            panel(trim, a, b, h, h + thick, da + projection, db + projection)
            block.face(
                trim,
                [
                    pt(a, h, da),
                    pt(b, h, db),
                    pt(b, h, db + projection),
                    pt(a, h, da + projection),
                ],
            )
        count = max(1, round((b - a) * length / 0.73))
        for i in range(count):
            t = a + (i + 0.5) * (b - a) / count
            d = da + (db - da) * (i + 0.5) / count
            panel(
                trim,
                t - 0.042 / length,
                t + 0.042 / length,
                top - 0.30,
                top - 0.18,
                d + 0.076,
            )
            ledge(
                trim, t - 0.042 / length, t + 0.042 / length, top - 0.30, d, d + 0.076
            )


def courtyard_wall(block, pt, length, base, top, mat, finishes, trim):
    _, _, glass, blinds, interior, _, _ = finishes

    def panel(material, left, right, bottom, above, depth=0):
        block.face(
            material,
            [
                pt(left, bottom, depth),
                pt(right, bottom, depth),
                pt(right, above, depth),
                pt(left, above, depth),
            ],
        )

    levels = max(2, round((top - base) / 3.4))
    count = max(1, round(length / 6.5))
    for floor in range(levels):
        low = base + floor * (top - base) / levels
        upper = base + (floor + 1) * (top - base) / levels
        sill = low + 0.91
        head = min(upper - 0.42, sill + 1.39)
        panel(mat, 0, 1, base - 2 if floor == 0 else low, sill)
        panel(mat, 0, 1, head, upper)
        end = 0
        for k in range(count):
            mid = (k + 0.48) / count
            half = min(0.26 / count, 1.02 / length)
            left, right = mid - half, mid + half
            panel(mat, end, left, sill, head)
            end = right
            for edge in (left, right):
                block.face(
                    mat,
                    [
                        pt(edge, sill),
                        pt(edge, sill, -0.15),
                        pt(edge, head, -0.15),
                        pt(edge, head),
                    ],
                )
            for h in (sill, head):
                block.face(
                    mat,
                    [
                        pt(left, h, -0.15),
                        pt(right, h, -0.15),
                        pt(right, h, 0.04),
                        pt(left, h, 0.04),
                    ],
                )
            panel(
                trim,
                left - 0.035 / length,
                right + 0.035 / length,
                sill - 0.035,
                sill,
                0.045,
            )
            panel(interior, left, right, sill, head, -0.35)
            if (k + floor) % 3:
                panel(blinds[(k + floor) % 3], left, right, sill + 0.35, head, -0.28)
            panel(glass, left, right, sill, head, -0.105)
            for x in (left, mid, right - 0.027 / length):
                panel(trim, x, x + 0.027 / length, sill, head, -0.075)
        panel(mat, end, 1, sill, head)


def roof_vents(block, centre, top, metal, dark, scale):
    # Each stack has a visible bore and a flared flashing at the roof penetration.
    x, y = centre
    for offset, height in ((-0.85, 0.49), (0.72, 0.32)):
        cx, cy = x + offset * scale, y + offset * 0.51
        section = (
            (0.15, 0),
            (0.15, 0.015),
            (0.066, 0.075),
            (0.055, 0.12),
            (0.055, height),
            (0.064, height + 0.013),
            (0.061, height + 0.03),
            (0.045, height + 0.03),
            (0.045, height - 0.09),
        )
        for row, ((r, h), (rr, hh)) in enumerate(zip(section, section[1:])):
            for k in range(16):
                a, b = math.tau * k / 16, math.tau * (k + 1) / 16
                block.face(
                    metal if row < 7 else dark,
                    [
                        (cx + r * math.cos(a), cy + r * math.sin(a), top + h),
                        (cx + r * math.cos(b), cy + r * math.sin(b), top + h),
                        (cx + rr * math.cos(b), cy + rr * math.sin(b), top + hh),
                        (cx + rr * math.cos(a), cy + rr * math.sin(a), top + hh),
                    ],
                )
