"""Export the three runtime LOD tiers for the authored Urban Utility body.

    blender -b --factory-startup -P scripts/assets/export-urban-utility-lods.py -- \
        --blend <body.blend> --output <dir outside the repository>

The near tier is the zstack vehicle-skill build as reviewed. The mid and far tiers
use the same reducers the authored fleet uses (scripts/assets/vehicle_mid_lod.py
and scripts/assets/vehicle_lod.py), each reloading the near snapshot so a
reduction never stacks on another. Writes a build report; promotes nothing.
"""
import argparse
import hashlib
import json
import sys
from pathlib import Path

import bpy

sys.path.insert(0, str(Path(__file__).resolve().parent))
from vehicle_lod import prepare_vehicle_far_lod  # noqa: E402
from vehicle_mid_lod import prepare_vehicle_mid_lod  # noqa: E402
from glb_tangents import repair_tangents  # noqa: E402
from vehicle_two_tone_split import cut_two_tone_split, dissolve_flat_hidden  # noqa: E402

CAPS = [165160, 22000, 6000]
# The lower-body sage, linear base colour. The authored 0.09/0.19/0.21 drew as a saturated
# blue-teal in game (sRGB 28/65/83 on the day side card) where the approved design renders
# show a muted grey-green (sRGB 104/127/124 beside cream 198/195/191). This value was fitted
# so the game's sage sits at the design's sage-to-cream ratio: a per-channel power-law fit
# of the game's tone response through the two measured paints (cream and the old sage) on
# the same card, inverted at the design ratio. Measured, not picked by eye.
ACCENT_LINEAR = (0.34, 0.40, 0.35)
LENGTH_M = 4.7
WHEEL_RADIUS_M = 0.34
BODY = 'urbanUtility'
VEHICLE_ID = 'urban-utility-v1'
TYPE_IDS = ['urbanUtility']


def tag_lamp_channels():
    """Mark the lamp materials with the channel the far reducer groups lenses by.

    The authored fleet tags each lamp material `installedLampChannel` in ('head','tail'); the
    far reducer reads that property to find the four lens groups. The zstack vehicle skill emits
    emissive materials without it, so the far tier cannot resolve its lenses. Tag them from the
    material names, which the skill's palette fixes.
    """
    tagged = []
    for material in bpy.data.materials:
        name = material.name
        channel = None
        if name in ('Lamp_Head', 'VehicleHeadlamp', 'VehicleDRL', 'VehicleFogLamp'):
            channel = 'head'
        elif name in ('Lamp_Tail', 'Lens_Tail', 'VehicleTaillamp', 'VehicleTaillampLens',
                      'VehicleBrakeLamp', 'VehicleReverseLamp'):
            channel = 'tail'
        elif name in ('Lamp_Indicator', 'VehicleIndicator'):
            channel = 'indicator'
        if channel:
            material['installedLampChannel'] = channel
            tagged.append(f'{name}={channel}')
    return tagged


def cull_backfaces():
    """Make every opaque material single-sided; keep only the glass double-sided.

    The zstack skill's palette leaves `use_backface_culling` off, so the export marked all 24
    materials `doubleSided`. A body panel is a single-surface sheet: double-siding it wastes fill
    and lets a lamp's back face glow through the cabin, which
    `verify-generated-vehicle-assembly` rejects. Glass keeps both faces, because you do see the
    inside of a windscreen.
    """
    kept = []
    culled = []
    for material in bpy.data.materials:
        transmissive = 'Transmission Weight' in {i.name for i in
                         (material.node_tree.nodes['Principled BSDF'].inputs
                          if material.use_nodes and 'Principled BSDF' in material.node_tree.nodes else [])}
        value = material.node_tree.nodes['Principled BSDF'].inputs['Transmission Weight'].default_value             if transmissive else 0.0
        if value > 0.05 or material.name in ('Glass', 'GlassTint', 'VehicleGlass', 'VehicleGlassTint',
                                             'VehicleTaillampLens'):
            material.use_backface_culling = False
            kept.append(material.name)
        else:
            material.use_backface_culling = True
            culled.append(material.name)
    return culled, kept


# The zstack skill's non-lamp contract materials -> the runtime's names (the skill's EXPORT_NAMES).
# The overlay tints only `VehiclePaint` and rebuilds glazing only on `VehicleGlass`, and
# verify-hero-greenhouse-integrity counts faces by those names, so `Paint` / `Glass` drew untinted
# raw glTF glass and failed the gate.
CONTRACT_MATERIALS = {
    'Paint': 'VehiclePaint', 'Paint_Accent': 'VehicleTrimGloss',
    'Glass': 'VehicleGlass', 'GlassTint': 'VehicleGlassTint',
    'Trim_Black': 'VehicleTrimBlack', 'Trim_Satin': 'VehicleTrimSatin', 'Trim_Chrome': 'VehicleChrome',
    'Metal_Brushed': 'VehicleSteel', 'Metal_Cast': 'VehicleMetalCast', 'Metal_Dark': 'VehicleMetalDark',
    'Tyre': 'VehicleTyre', 'Rim': 'VehicleAlloy', 'Brake': 'VehicleBrakeDisc', 'Caliper': 'VehicleCaliper',
    'Reflector': 'VehicleLampReflector',
    'Interior': 'VehicleInterior', 'Interior_Accent': 'VehicleSeatInsert',
    'Interior_Trim': 'VehicleInteriorTrim', 'Screen': 'VehicleScreen',
    'Underbody': 'VehicleUnderbody', 'Void': 'VehicleVoid', 'Mirror': 'VehicleMirror',
}


def rename_contract_materials():
    """Rename the skill's paint, glass, trim and cabin materials to the runtime vocabulary."""
    renamed = []
    for src, dst in CONTRACT_MATERIALS.items():
        material = bpy.data.materials.get(src)
        if material is None:
            continue
        if bpy.data.materials.get(dst) is not None:
            raise ValueError(f'cannot rename {src} to {dst}: {dst} already exists')
        material.name = dst
        renamed.append(f'{src}->{dst}')
    return renamed


def flatten_transmission():
    """Export transmissive materials as alpha-blended glass, never KHR_materials_transmission.

    three renders a transmissive material with a ViewportTextureNode that copies the framebuffer
    mid-pass; built during the environment probe bake it ends a render pass the bake ends again,
    and the boot GPU gate rejects the boot ("RenderPassEncoder was already ended"). Zero the
    transmission and carry the see-through as alpha instead (alpha = 1 - 0.75 * transmission).
    Runs after `cull_backfaces`, which reads the transmission to keep glass double-sided.
    """
    flattened = []
    for material in bpy.data.materials:
        if not material.use_nodes or not material.node_tree:
            continue
        for node in material.node_tree.nodes:
            if node.type != 'BSDF_PRINCIPLED':
                continue
            weight = node.inputs.get('Transmission Weight')
            if weight is None or weight.is_linked or weight.default_value <= 0.0:
                continue
            alpha = node.inputs.get('Alpha')
            if alpha is not None and not alpha.is_linked:
                alpha.default_value = min(alpha.default_value, 1.0 - 0.75 * weight.default_value)
                material.surface_render_method = 'BLENDED'
            weight.default_value = 0.0
            flattened.append(material.name)
    return flattened


def assert_runtime_glb(path):
    """Fail the export if a GLB would still break the boot: morph targets, transmission, raw names."""
    raw = Path(path).read_bytes()
    doc = json.loads(raw[20:20 + int.from_bytes(raw[12:16], 'little')])
    morphs = sum(len(p.get('targets', [])) for m in doc.get('meshes', []) for p in m['primitives'])
    names = {m.get('name') for m in doc.get('materials', [])}
    raw_names = sorted(names & set(CONTRACT_MATERIALS))
    if morphs or 'KHR_materials_transmission' in doc.get('extensionsUsed', []) or raw_names:
        raise ValueError(f'{Path(path).name} is not runtime-safe: {morphs} morph targets, '
                         f'extensions {doc.get("extensionsUsed", [])}, raw material names {raw_names}')


def rename_lamp_materials():
    """Rename the skill's lamp materials to the runtime vocabulary.

    The overlay and verify-vehicle-lamp-dynamics bind lamp channels by MATERIAL NAME
    (LAMP_CHANNEL_BY_MATERIAL in src/render/passes/vehicle-hero-overlay.js). The zstack skill
    emits `Lamp_Head` / `Lamp_Tail` / `Lamp_Indicator`, which that table does not know, so the
    runtime drew zero driven lenses and the gate could not find a head, brake or indicator lens.
    Map them onto the names the runtime binds, so front and rear really are switchable.
    """
    mapping = {
        'Lamp_Head': 'VehicleHeadlamp',
        'Lamp_DRL': 'VehicleDRL',
        'Lamp_Tail': 'VehicleTaillamp',
        'Lens_Tail': 'VehicleTaillampLens',
        'Lamp_Indicator': 'VehicleIndicator',
        'Lamp_Brake': 'VehicleBrakeLamp',
        'Lamp_Reverse': 'VehicleReverseLamp',
        'Lamp_Fog': 'VehicleFogLamp',
        'Lamp_Dome': 'VehicleDomeLight',
    }
    renamed = []
    for material in bpy.data.materials:
        target = mapping.get(material.name)
        if not target:
            continue
        if target == 'VehicleTaillampLens' and any(
                m.name == 'VehicleTaillampLens' for m in bpy.data.materials if m is not material):
            target = f'{target}_{material.name.split("_")[-1]}'
        renamed.append(f'{material.name}->{target}')
        material.name = target
    return renamed


def normalise_wheel_names():
    """Rename the wheel meshes to the fleet convention the reducers and gates share.

    `prepare_vehicle_far_lod` keeps a wheel mesh only when its name starts with `Wheel_` and its
    last underscore-delimited role is one of (Tyre, Lip, Hub, Barrel, Cap, Spoke); the authored
    far tier keeps the same `Wheel_<corner>_<role>` names, and `verify-vehicle-wheel-contract`
    measures the drawn wheel from them. The zstack vehicle skill names them `Tyre_FL` / `Rim_FL`,
    so the far tier silently dropped every tyre and the gate then measured no hub at all.
    """
    roles = {'Tyre': 'Tyre', 'Rim': 'Lip', 'Hub': 'Hub', 'Barrel': 'Barrel',
             'Cap': 'Cap', 'Disc': 'Disc', 'Caliper': 'Caliper'}
    renamed = []
    for obj in bpy.context.scene.objects:
        if obj.type != 'MESH':
            continue
        for corner in ('FL', 'FR', 'RL', 'RR'):
            if obj.name == f'Tyre_{corner}':
                new = f'Wheel_{corner}_Tyre'
            elif obj.name == f'Rim_{corner}':
                new = f'Wheel_{corner}_Lip'
            else:
                continue
            obj.name = obj.data.name = new
            renamed.append(f'{new}')
    return renamed


def bind_lamps_to_shell():
    """Join the lamp lenses into the shell so the far reducer can find them.

    `prepare_vehicle_far_lod` deletes every object that is not shell, wheel or an existing
    `SourceLamp_*` node BEFORE it measures the four lens groups, and it expects a Tripo-style
    source whose lamps are faces on the shell itself. This body carries each lamp as its own
    node, so the reducer would delete them all and then find no lens groups. Merge every tagged
    lens into the body, which keeps the emissive faces (and their areas) on the shell the
    reducer measures. The near and mid tiers are exported before this runs and keep their own
    lamp nodes, which is what the overlay binds channels to.
    """
    tagged = [m for m in bpy.data.materials if m.get('installedLampChannel') in ('head', 'tail')]
    if not tagged:
        return []
    names = {m.name for m in tagged}
    body = bpy.data.objects.get('Body')
    if body is None:
        raise ValueError('far tier needs a Body shell to bind lamps into')
    bpy.ops.object.select_all(action='DESELECT')
    body.select_set(True)
    bpy.context.view_layer.objects.active = body
    merged = []
    for ob in [o for o in bpy.context.scene.objects
               if o.type == 'MESH' and o is not body
               and any(m and m.name in names for m in o.data.materials)]:
        ob.select_set(True)
        merged.append(ob.name)
    if merged:
        bpy.ops.object.join()
    return merged


def drop_shape_keys():
    """Remove the crumple-zone morph targets before any tier is exported.

    Four crush keys on every exterior slot shipped 412 glTF morph targets on the near tier. The
    hero bake merges meshes into InstancedMesh batches, so they crashed `MorphNode.update` and
    tripped mergeGeometries' "morphTargetsRelative must be consistent"; the runtime does hero
    crush by instance deformation instead. A decimate or blend modifier on the mid and far tiers
    also cannot be applied to a mesh carrying shape keys. `shape_key_clear` releases the targets
    and leaves the basis geometry -- and therefore the exported shape -- untouched. The reviewed
    source .blend keeps its keys.
    """
    dropped = []
    for obj in bpy.context.scene.objects:
        if obj.type != 'MESH' or not obj.data.shape_keys:
            continue
        count = len(obj.data.shape_keys.key_blocks)
        obj.shape_key_clear()
        dropped.append(f'{obj.name}({count})')
    return dropped


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def triangle_count():
    count = 0
    for obj in bpy.context.scene.objects:
        if obj.type != 'MESH':
            continue
        obj.data.calc_loop_triangles()
        count += len(obj.data.loop_triangles)
    return count


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--blend', required=True)
    parser.add_argument('--output', required=True)
    args = parser.parse_args(sys.argv[sys.argv.index('--') + 1:])

    output = Path(args.output).resolve()
    output.mkdir(parents=True, exist_ok=True)
    source = Path(args.blend).resolve()

    # reload the reviewed body for every tier, so no reduction stacks on another
    bpy.ops.wm.open_mainfile(filepath=str(source))
    # The cream/sage split is authored per face, so it drew as a staircase of quads on the
    # nose, the front wing and the rear corner. Cut it along a smooth curve first, so every
    # tier reduces the same split; then pay for the cut's extra triangles out of coplanar
    # cabin faces, which keeps the near tier under its budget without changing any shape.
    accent = bpy.data.materials.get('Paint_Accent')
    if accent is not None and accent.use_nodes:
        bsdf = next(n for n in accent.node_tree.nodes if n.type == 'BSDF_PRINCIPLED')
        bsdf.inputs['Base Color'].default_value = (*ACCENT_LINEAR, 1.0)
        print(json.dumps({'accentLinear': ACCENT_LINEAR}))
    split = cut_two_tone_split('Paint', 'Paint_Accent')
    print(json.dumps({'twoToneSplit': split}))
    freed = dissolve_flat_hidden(('Interior',), split.get('trianglesAdded', 0))
    print(json.dumps({'coplanarCabinTrianglesFreed': freed}))
    renamed = normalise_wheel_names()
    if renamed:
        print(json.dumps({'wheelNodesRenamed': renamed}))
    mats = rename_lamp_materials()
    if mats:
        print(json.dumps({'lampMaterialsRenamed': mats}))
    mats = rename_contract_materials()
    if mats:
        print(json.dumps({'contractMaterialsRenamed': mats}))
    tagged = tag_lamp_channels()
    if tagged:
        print(json.dumps({'lampChannelsTagged': tagged}))
    culled, kept = cull_backfaces()
    print(json.dumps({'backfaceCulled': len(culled), 'doubleSidedKept': kept}))
    flattened = flatten_transmission()
    if flattened:
        print(json.dumps({'transmissionFlattened': flattened}))
    dropped = drop_shape_keys()
    if dropped:
        print(json.dumps({'shapeKeysDropped': len(dropped)}))
    near = output / 'urbanUtility-near.blend'
    bpy.ops.wm.save_as_mainfile(filepath=str(near))

    lods = []
    for level in range(3):
        if level in (1, 2):
            # Each reduced tier starts from its own copy of the REVIEWED body (already free of
            # shape keys), never from a sibling tier's output. `near` is never re-saved.
            bpy.ops.wm.open_mainfile(filepath=str(near))
            tier = output / f'urbanUtility-tier{level}.blend'
            bpy.ops.wm.save_as_mainfile(filepath=str(tier))
            if level == 1:
                prepare_vehicle_mid_lod(source_blend=tier)
            else:
                bpy.ops.wm.open_mainfile(filepath=str(tier))
                joined = bind_lamps_to_shell()
                if joined:
                    print(json.dumps({'lod': 2, 'lampsBoundToShell': joined}))
                prepare_vehicle_far_lod()
        path = output / f'urbanUtility-lod{level}.glb'
        bpy.ops.export_scene.gltf(filepath=str(path), export_format='GLB', export_apply=True,
                                  export_yup=True, export_animations=False, export_skins=False,
                                  export_tangents=True, export_extras=True, export_morph=False)
        repaired = repair_tangents(path)
        if repaired:
            print(json.dumps({'lod': level, 'repairedTangents': repaired}))
        assert_runtime_glb(path)
        triangles = triangle_count()
        if triangles > CAPS[level]:
            raise ValueError(f'LOD{level} exceeds the tier budget: {triangles} > {CAPS[level]}')
        lods.append(dict(file=path.name, sha256=digest(path), bytes=path.stat().st_size,
                         triangles=triangles))

    report = dict(
        schemaVersion=1, kind='generated-vehicle', vehicleId=VEHICLE_ID, body=BODY,
        lengthM=LENGTH_M, wheelRadiusM=WHEEL_RADIUS_M, typeIds=TYPE_IDS,
        generator='scripts/assets/export-urban-utility-lods.py',
        source=dict(method='zstack vehicle skill (Blender); reference-to-rig build',
                     file=source.name, sha256=digest(source), bytes=source.stat().st_size),
        recipeSha256={name: digest(Path(__file__).with_name(name)) for name in
                      ['vehicle_lod.py', 'vehicle_mid_lod.py', 'vehicle_two_tone_split.py']},
        lods=lods,
        status='candidate; round-trip, interaction and visual review required')
    (output / 'build-report.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({'lods': lods}, indent=2))


if __name__ == '__main__':
    main()
