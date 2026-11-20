"""Dimensions in metres and placement anchors for the furnished terrace (glTF Y up).

An assembly's anchor follows the floor plan. Distances within it are physical
metres shared by its meshes, contact shadows, portable assets and colliders.
"""
import math

STOREY = 10 / 3
ASSEMBLIES = [
    ((1.04, 0, 2.7), ("upholstered-sofa", "woven-lap-throw", "coffee-table",
                        "clothbound-book", "sitting-room-coffee-cup", "sitting-room-rug")),
    ((2.375, 0, 7.575), ("oak-dining-table", "dining-chair", "dining-chair-rear",
                           "dining-chair-west-front", "dining-chair-west-rear",
                           "dining-chair-east-front", "dining-chair-east-rear",
                           "painted-fruit-bowl", "red-apple", "red-apple-small", "orchard-pear",
                           "striped-table-runner", "open-recipe-book", "stoneware-mug", "stoneware-mug-rear")),
    ((.15, 0, 12.72), ("base-cabinets", "oven-range", "sink-basin", "brass-faucet",
                         "stacked-plates", "mixing-bowl", "range-hood", "enamel-stockpot",
                         "chopping-board", "pantry-canister-1", "pantry-canister-2",
                         "pantry-canister-3", "kitchen-shelves", "kitchen-plant", "kitchen-tiles",
                         "enamel-refrigerator")),
    ((1.825, STOREY, 12.46), ("double-bed", "bedside-table", "bedside-lamp")),
    ((2.29, 2 * STOREY, .58), ("workroom-desk", "desk-chair", "desk-books")),
    ((.29, STOREY, 2.19), ("bathtub",)),
    ((2.01, STOREY, .20), ("washstand",)),
    ((.20, 2 * STOREY, 12.24), ("stocked-bookcase",)),
    ((3.095, 2 * STOREY, 6.835), ("workroom-chair",)),
    ((.57, STOREY, 9.28), ("potted-plant",)),
    ((4.14, 2 * STOREY, .72), ("workroom-plant",)),
    ((3.50, 0, 12.65), ("window-seat",)),
    ((.15, 0, 5.17), ("living-wall-shelves",)),
    ((4.03, 0, .58), ("living-plant",)),
    ((.16, 0, 7.575), ("oak-sideboard",)),
    ((2.37, STOREY-.15, 7.51), ("dining-pendant",)),
]
GROUP = {name: i for i, (_, names) in enumerate(ASSEMBLIES) for name in names}


# Seat centre and yaw. The mesh faces -Z in its portable orientation.
DINING_CHAIRS = {
    "dining-chair": (2.374, 6.342, math.pi),
    "dining-chair-rear": (2.374, 8.902, 0),
    "dining-chair-west-front": (1.51, 7.15, -math.pi / 2),
    "dining-chair-west-rear": (1.54, 8.02, -math.pi / 2),
    "dining-chair-east-front": (3.22, 7.14, math.pi / 2),
    "dining-chair-east-rear": (3.28, 8.015, math.pi / 2),
}


def scale(point, origin, factors):
    return tuple(a + (v - a) * f for v, a, f in zip(point, origin, factors))


def furniture_point(name, point):
    if name == "dining-pendant":
        # A 46 cm shade centred over the table at the default surveyed site.
        # The pendant is ceiling-mounted. Its horizontal size is resolved in metres at placement.
        return scale(point, (2.37, 0, 7.51), (.605, 1, .605))
    if name in DINING_CHAIRS:
        cx, cz, angle = DINING_CHAIRS[name]
        x, h, z = point
        c, s = math.cos(angle), math.sin(angle)
        return cx + (x-cx)*c + (z-cz)*s, h, cz - (x-cx)*s + (z-cz)*c
    group = GROUP.get(name)
    if group is None:
        return point
    anchor = ASSEMBLIES[group][0]
    if name in ("upholstered-sofa", "woven-lap-throw"):
        return scale(point, anchor, (1, .89, 1))
    if group == 1:
        # Keep vessels round as the tabletop narrows and lower their bases to its surface.
        if name.startswith("stoneware-mug"):
            x, z = (2.18, 7.03) if name == "stoneware-mug" else (2.89, 8.13)
            p = scale(point, (x, .79, z), (.65, .65, .65))
            return (p[0] + (x - anchor[0]) * (-.23), p[1] - .04, p[2])
        if name in ("red-apple", "red-apple-small", "orchard-pear", "painted-fruit-bowl"):
            p = scale(point, (2.39, .79, 7.49), (.77, .77, .77))
            return (p[0], p[1] - .04, p[2])
        if name == "open-recipe-book":
            return (point[0] - .05, point[1] - .04, point[2])
        return scale(point, anchor, (.77, .75 / .79, 1))
    if group == 2:
        if name == "enamel-refrigerator":
            return scale(point, (1.24, 0, 12.72), (1, .92, .78))
        # Keep the range and cabinets joined. An extractor meets the ceiling.
        x, h, z = scale(point, anchor, (.78, .92, .9))
        if name == "range-hood" and point[1] > 2.6:
            h += .08 * point[1] * min(1, (point[1] - 2.6) / .5)
        return x, h, z
    if group == 3:
        return scale(point, anchor, (.83, .88, .88))
    if group == 4 and name != "desk-chair":
        return scale(point, anchor, (.89, .75 / .80, .83))
    if name == "bathtub":
        return scale(point, anchor, (.82, 1, .86))
    return point
