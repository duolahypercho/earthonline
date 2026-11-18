# Writes src/vehicles/hero-vehicle-manifest.js from the GLBs in public/assets/vehicles.
# Run after every export: the export is not byte-deterministic, so the SHA-256 allowlist must follow it.
import hashlib, os, json, sys, importlib.util
ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
PUB = os.path.join(ROOT, 'public', 'assets', 'vehicles')
def sha(p):
    with open(p, 'rb') as f: return hashlib.sha256(f.read()).hexdigest()
BODIES = {
    # key: (lengthM, wheelRadiusM, typeIds, [lod files])
    'sedan': dict(lengthM=4.72, wheelRadiusM=0.335, typeIds=['sedan', 'wagon', 'coupe'],
                  lods=['project-sedan-v1.glb', 'project-sedan-v1-lod1.glb', 'project-sedan-v1-lod2.glb']),
    'compactHatch': dict(lengthM=4.06, wheelRadiusM=0.315, typeIds=['compactHatch'], lods=[f'fleet-v1/compactHatch-lod{i}.glb' for i in range(3)]),
    'compactSuv': dict(lengthM=4.55, wheelRadiusM=0.36, typeIds=['compactSuv'], lods=[f'fleet-v1/compactSuv-lod{i}.glb' for i in range(3)]),
    'pickup': dict(lengthM=5.89, wheelRadiusM=0.39, typeIds=['pickup'], lods=[f'fleet-v1/pickup-lod{i}.glb' for i in range(3)]),
    'deliveryVan': dict(lengthM=5.53, wheelRadiusM=0.375, typeIds=['deliveryVan'], lods=[f'fleet-v1/deliveryVan-lod{i}.glb' for i in range(3)]),
    'taxi': dict(lengthM=4.92, wheelRadiusM=0.345, typeIds=['taxi'], lods=[f'fleet-v1/taxi-lod{i}.glb' for i in range(3)]),
}
LEGACY_BODIES = BODIES.copy()
fleet_report = os.path.join(PUB, 'fleet-v2', 'build-report.json')
if os.path.exists(fleet_report):
    with open(fleet_report) as f: fleet = json.load(f)
    # The fleet grew past its original ten bodies (long-roof classes and six
    # trims). Require the report to be internally complete rather than pinned to
    # a body count, so adding a body does not silently block every manifest
    # regeneration.
    if not fleet['bodies'] or any(len(b.get('lods', [])) != 3 for b in fleet['bodies'].values()):
        raise ValueError('fleet-v2 promotion requires a complete three-LOD body set')
    BODIES = {
        key: dict(lengthM=b['lengthM'], wheelRadiusM=b['wheelRadiusM'],
                  typeIds=b['typeIds'], lods=['fleet-v2/' + lod['file'] for lod in b['lods']])
        for key, b in fleet['bodies'].items()
    }
    for body in fleet['bodies'].values():
        for key, b in body.get('variants', {}).items():
            BODIES[key] = dict(lengthM=b['lengthM'], wheelRadiusM=b['wheelRadiusM'],
                typeIds=b['typeIds'], lods=['fleet-v2/' + lod['file'] for lod in b['lods']])
# fleet-v3 (scripts/assets/vehicle-v3) replaces a fleet-v2 body key by key: an
# authored feature-line body on the same catalogue slot, footprint and rig.
V3_BODIES = {}
# A fleet-v3 slot may instead carry a vehicle-skill build (a hand-shaped Blender cage exported by
# its own script); its report record names the method in `sourceModel`.
SKILL_BODIES = {}
fleet_v3_report = os.path.join(PUB, 'fleet-v3', 'build-report.json')
if os.path.exists(fleet_v3_report):
    with open(fleet_v3_report) as f: fleet_v3 = json.load(f)
    for key, b in fleet_v3['bodies'].items():
        if b.get('sourceModel'):
            SKILL_BODIES[key] = b['source']
        if len(b.get('lods', [])) != 3:
            raise ValueError('fleet-v3 promotion requires three LODs: ' + key)
        if key in BODIES and b['typeIds'] != BODIES[key]['typeIds']:
            raise ValueError('fleet-v3 override changes the catalogue assignment: ' + key)
        V3_BODIES[key] = BODIES[key] = dict(lengthM=b['lengthM'], wheelRadiusM=b['wheelRadiusM'],
            typeIds=b['typeIds'], lods=['fleet-v3/' + lod['file'] for lod in b['lods']])
# Keep the authored inventory even when a generated body takes its catalogue
# slot. Generated geometry and source pixels receive their own provenance.
PROCEDURAL_BODIES = BODIES.copy()
generated = {}
registry_path = os.path.join(PUB, 'generated-vehicles.json')
if os.path.exists(registry_path):
    spec = importlib.util.spec_from_file_location('generated_vehicle',
        os.path.join(ROOT, 'scripts', 'assets', 'prepare-generated-vehicle.py'))
    receipt_module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(receipt_module)
    with open(registry_path) as f: registry = json.load(f)
    if registry.get('schemaVersion') != 1:
        raise ValueError('unsupported generated vehicle registry')
    for key, receipt_rel in registry['bodies'].items():
        receipt_path = receipt_module.relative_file(PUB, receipt_rel)
        receipt = receipt_module.validate_receipt(receipt_module.read(receipt_path), PUB)
        if receipt['body'] != key:
            raise ValueError('generated body key must match the receipt: ' + key)
        # A generated body that replaces a procedural one keeps its catalogue
        # slot; a body with no procedural counterpart (e.g. a two-door study
        # conversion) enters on the receipt's own typeIds.
        if key in BODIES and receipt['typeIds'] != BODIES[key]['typeIds']:
            raise ValueError('generated override changes the catalogue assignment: ' + key)
        generated[key] = receipt
        BODIES[key] = dict(lengthM=receipt['lengthM'], wheelRadiusM=receipt['wheelRadiusM'],
            typeIds=receipt['typeIds'], lods=[lod['file'] for lod in receipt['lods']])
# fleet-v3 is the authored replacement for every slot it covers, generated ones included:
# the generated GLBs stay on disk and keep their own provenance record.
for key, b in V3_BODIES.items():
    if key in generated and b['typeIds'] != BODIES[key]['typeIds']:
        raise ValueError('fleet-v3 override changes the catalogue assignment: ' + key)
    BODIES[key] = b
out = {}
for key, b in BODIES.items():
    lods = []
    for rel in b['lods']:
        p = os.path.join(PUB, rel)
        if not os.path.exists(p):
            print('missing', rel); lods = None; break
        lods.append(dict(url='/assets/vehicles/' + rel, sha256=sha(p), bytes=os.path.getsize(p)))
    if lods is None: continue
    out[key] = dict(lengthM=b['lengthM'], wheelRadiusM=b['wheelRadiusM'], typeIds=b['typeIds'], lods=lods)
js = ('// GENERATED by artifacts/vehicles/write-manifest.py - do not edit by hand.\n'
      '// SHA-256 allowlist for the hero vehicle bodies (AGENTS.md: a production GLB is\n'
      '// verified before it is parsed). Regenerate after every Blender export.\n'
      'export const HERO_VEHICLE_MANIFEST = Object.freeze(' + json.dumps(out, indent=2) + ');\n')
with open(os.path.join(ROOT, 'src', 'vehicles', 'hero-vehicle-manifest.js'), 'w') as f: f.write(js)
print('manifest bodies:', list(out.keys()))

# The same export, written once more as a provenance manifest, so a GLB swapped
# without a re-export is caught by the digest recorded here and by the runtime
# hash allowlist. Every body here is lofted procedurally by sedan_lib.py /
# fleet_lib.py: no third-party vehicle is referenced and no source image
# enters the chain, so sourcePixels is none.
prov_outputs = {}
for key, b in list(LEGACY_BODIES.items()) + list(PROCEDURAL_BODIES.items()):
    for rel in b['lods']:
        p = os.path.join(PUB, rel)
        if not os.path.exists(p):
            continue
        prov_outputs['assets/vehicles/' + rel] = dict(
            path='public/assets/vehicles/' + rel,
            bytes=os.path.getsize(p),
            sha256=sha(p),
            body=key,
            kind='authored-vehicle-glb',
        )
provenance = dict(
    schemaVersion=1,
    asset='project-vehicles-v1',
    thirdPartyContent=False,
    sourcePixels='none',
    copyrightOwner='Earth Online contributors',
    generationMethod='procedural Blender loft; authored vehicle libraries and scripts/assets/build-vehicle-fleet-v2.py, exported headless to glTF-binary'
        + ('; fleet-v3 feature-line bodies from scripts/assets/vehicle-v3 (proportions measured from photographs of production cars, no source pixels, no marks or badges)' if set(V3_BODIES) - set(SKILL_BODIES) else '')
        + ''.join(f'; fleet-v3 {key} from the zstack /vehicle skill (hand-shaped mirrored subdivision cage in Blender, no AI 3D generator, no source pixels, no marks or badges), exported by {src}'
                  for key, src in sorted(SKILL_BODIES.items())),
    generator='artifacts/vehicles/write-manifest.py',
    consumer='src/vehicles/hero-vehicle-manifest.js runtime SHA-256 allowlist',
    outputs=dict(sorted(prov_outputs.items())),
)
prov_path = os.path.join(PUB, 'project-vehicles-v1.provenance.json')
with open(prov_path, 'w') as f: f.write(json.dumps(provenance, indent=2) + '\n')
print('provenance outputs:', len(prov_outputs))
if generated:
    methods = sorted({receipt['source']['method'] for receipt in generated.values()})
    generated_provenance = dict(schemaVersion=1, asset='generated-vehicles',
        sourcePixels='declared per vehicle in each receipt source block',
        generationMethod='; '.join(methods),
        generator='artifacts/vehicles/write-manifest.py',
        consumer='src/vehicles/hero-vehicle-manifest.js runtime SHA-256 allowlist',
        vehicles=generated,
        outputs={
            'assets/vehicles/' + lod['file']: dict(path='public/assets/vehicles/' + lod['file'],
                sha256=lod['sha256'], bytes=lod['bytes'], body=key, kind='generated-vehicle-glb')
            for key, receipt in generated.items() for lod in receipt['lods']
        })
    with open(os.path.join(PUB, 'generated-vehicles.provenance.json'), 'w') as f:
        f.write(json.dumps(generated_provenance, indent=2) + '\n')
    print('generated provenance bodies:', list(generated))
