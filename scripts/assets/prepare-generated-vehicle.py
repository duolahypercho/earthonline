"""Prepare a reviewed generated vehicle outside the repository; never promote it.

The output is relative to public/assets/vehicles. Copying that package into the
game and running artifacts/vehicles/write-manifest.py is a separate operation.
Acceptance JSON: {"verdict":"accepted", "lodSha256":[...], "evidence":[paths]}.
Evidence remains private; only its digests enter the public receipt. Acceptance
records a scoped asset review, and does not assert a project-wide quality gate.
"""
import argparse
import hashlib
import json
from pathlib import Path
import re
import shutil

ROOT = Path(__file__).resolve().parents[2]

STUDY_METHOD = 'Authored study source; Blender rig conversion'
# A shading repair of LODs that were already accepted and shipped: the source is the
# previous runtime LOD0 GLB (bound by digest), geometry is unchanged apart from split
# normals (scripts/assets/repair-vehicle-glb-normals.py).
REPAIR_METHOD = 'Normals repair of the shipped runtime GLB (scripts/assets/repair-vehicle-glb-normals.py)'


def digest(path):
    result = hashlib.sha256()
    with open(path, 'rb') as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b''):
            result.update(chunk)
    return result.hexdigest()


def read(path):
    return json.loads(Path(path).read_text())


def normalize_study_report(report, vehicle_id=None):
    """Accept both the legacy flat study report and the v3 bodies-map report."""
    if 'bodies' not in report:
        if vehicle_id and not report.get('vehicleId'):
            report = dict(report, vehicleId=vehicle_id)
        return report
    body_key = None
    if vehicle_id and vehicle_id in report['bodies']:
        body_key = vehicle_id
    elif vehicle_id:
        body_key = next(
            (key for key, value in report['bodies'].items()
             if value.get('vehicleId') == vehicle_id),
            None,
        )
    if not body_key:
        if len(report['bodies']) != 1:
            raise ValueError('a v3 study report needs an explicit --vehicle-id when it contains multiple bodies')
        body_key = next(iter(report['bodies']))
    body = dict(report['bodies'][body_key])
    body.setdefault('vehicleId', vehicle_id or body_key)
    body.setdefault('body', body_key)
    body.setdefault('generator', report.get('generator', 'vehicle-v3'))
    return body


def relative_file(root, relative):
    path = Path(relative)
    if path.is_absolute() or '..' in path.parts or '\\' in str(relative):
        raise ValueError('asset path must be relative and contained')
    resolved = (Path(root) / path).resolve()
    if not resolved.is_relative_to(Path(root).resolve()):
        raise ValueError('asset path escapes its root')
    return resolved


def verify_file(path, expected):
    if Path(path).stat().st_size != expected['bytes'] or digest(path) != expected['sha256']:
        raise ValueError(f'asset digest/size mismatch: {Path(path).name}')


def validate_receipt(receipt, asset_root):
    """Fail closed before a generated asset can enter the runtime allowlist."""
    if receipt.get('schemaVersion') != 1 or receipt.get('kind') != 'generated-vehicle':
        raise ValueError('unsupported generated vehicle receipt')
    if not re.fullmatch(r'[a-z][a-z0-9-]*', receipt['vehicleId']):
        raise ValueError('invalid vehicle id')
    if receipt.get('acceptance', {}).get('verdict') != 'accepted':
        raise ValueError('candidate lacks a scoped acceptance record')
    if len(receipt.get('lods', [])) != 3:
        raise ValueError('generated vehicle requires three reviewed LODs')
    hashes = [lod['sha256'] for lod in receipt['lods']]
    if receipt['acceptance'].get('lodSha256') != hashes:
        raise ValueError('acceptance does not bind these LODs')
    if not receipt['acceptance'].get('evidence'):
        raise ValueError('acceptance has no evidence digests')
    for lod in receipt['lods']:
        if not lod['file'].startswith(f"generated/{receipt['vehicleId']}/"):
            raise ValueError('generated output is outside its vehicle directory')
        verify_file(relative_file(asset_root, lod['file']), lod)
    source = receipt['source']
    method = source.get('method')
    # Vehicles are hand-built; an AI-generated body (Tripo or any other
    # text/image-to-3D source) has no accepted method and fails here.
    if method == STUDY_METHOD:
        # A study conversion has no generated reference views; the source
        # binding is the authored study file's digest, and the study is its
        # own donor.
        digests = [source['sha256'], receipt['donor']['sha256']]
        if receipt['donor'].get('kind') != 'authored-study-source':
            raise ValueError('study vehicle must record its source as the donor')
    elif method == REPAIR_METHOD:
        # The repaired body is only as sound as what it repaired: the donor is the
        # shipped runtime GLB, and the upstream source it was built from stays on record.
        digests = [source['sha256'], receipt['donor']['sha256']]
        if receipt['donor'].get('kind') != 'shipped-runtime-glb':
            raise ValueError('normals repair must record the shipped GLB it repaired as the donor')
        if receipt.get('upstream', {}).get('method') not in (STUDY_METHOD, REPAIR_METHOD):
            raise ValueError('normals repair must carry the accepted upstream source')
    else:
        raise ValueError('unsupported source provenance')
    for value in digests:
        if not re.fullmatch(r'[a-f0-9]{64}', value):
            raise ValueError('invalid source evidence digest')
    if not 0 < receipt['wheelRadiusM'] < receipt['lengthM'] or not receipt['typeIds']:
        raise ValueError('invalid vehicle dimensions or catalogue assignment')
    return receipt


def review_lods(candidate, report, vehicle_id, acceptance):
    """Bind the candidate LODs to the review, exactly as reported."""
    lods = []
    for lod in report['lods']:
        verify_file(relative_file(candidate, lod['file']), lod)
        lods.append(dict(file=f'generated/{vehicle_id}/{Path(lod["file"]).name}',
                         sha256=lod['sha256'], bytes=lod['bytes']))
    if acceptance.get('verdict') != 'accepted' or acceptance.get('lodSha256') != [lod['sha256'] for lod in lods]:
        raise ValueError('acceptance does not approve the exact candidate LODs')
    evidence = []
    for item in acceptance.get('evidence', []):
        if isinstance(item, dict):
            if not re.fullmatch(r'[a-f0-9]{64}', str(item.get('sha256', ''))):
                raise ValueError('acceptance evidence has an invalid digest')
            if int(item.get('bytes', -1)) < 0:
                raise ValueError('acceptance evidence has an invalid byte count')
            evidence.append({'sha256': str(item['sha256']), 'bytes': int(item['bytes'])})
        else:
            path = Path(item)
            evidence.append(dict(sha256=digest(path), bytes=path.stat().st_size))
    if not evidence:
        raise ValueError('acceptance requires existing review evidence files')
    return lods, evidence


def write_package(receipt, report, candidate, output):
    """Stage the reviewed package outside the repository for manual promotion."""
    vehicle_id = receipt['vehicleId']
    destination = Path(output).resolve()
    if destination.is_relative_to(ROOT) or ROOT.is_relative_to(destination):
        raise ValueError('prepare output must be outside the repository')
    if destination.exists():
        raise ValueError('prepare output already exists; use a new directory')
    registry_path = ROOT / 'public/assets/vehicles/generated-vehicles.json'
    registry = read(registry_path) if registry_path.exists() else dict(schemaVersion=1, bodies={})
    if registry.get('schemaVersion') != 1:
        raise ValueError('unsupported generated vehicle registry')
    relative_receipt = f'generated/{vehicle_id}/receipt.json'
    registry['bodies'][report['body']] = relative_receipt
    for source_lod, output_lod in zip(report['lods'], receipt['lods']):
        output_file = destination / output_lod['file']
        output_file.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(relative_file(candidate, source_lod['file']), output_file)
    validate_receipt(receipt, destination)
    (destination / relative_receipt).write_text(json.dumps(receipt, indent=2) + '\n')
    (destination / 'generated-vehicles.json').write_text(json.dumps(registry, indent=2) + '\n')
    print(f'Prepared {vehicle_id}; production files and source asset lock are unchanged: {destination}')


def prepare_study(args):
    """Package a reviewed study conversion: the authored study is its own donor."""
    report_path = Path(args.candidate) / 'build-report.json'
    report = normalize_study_report(read(report_path), args.vehicle_id)
    acceptance = read(args.acceptance)
    source = Path(args.source)
    source_sha = digest(source)
    if report.get('sourceSha256') and report['sourceSha256'] != source_sha:
        raise ValueError('study source differs from the build report')
    vehicle_id = report['vehicleId']
    if not re.fullmatch(r'[a-z][a-z0-9-]*', vehicle_id):
        raise ValueError('invalid vehicle id')
    lods, evidence = review_lods(Path(args.candidate), report, vehicle_id, acceptance)
    receipt = dict(schemaVersion=1, kind='generated-vehicle', vehicleId=vehicle_id,
        body=report['body'], lengthM=report['lengthM'], wheelRadiusM=report['wheelRadiusM'],
        typeIds=report['typeIds'], lods=lods,
        source=dict(method=STUDY_METHOD, sha256=source_sha, bytes=source.stat().st_size,
            file=source.name),
        donor=dict(path='study-source:' + source.name, sha256=source_sha,
                   bytes=source.stat().st_size, kind='authored-study-source'),
        assembly=dict(generator=report.get('generator', 'scripts/assets/build-robo-cab-v1.py'),
                      reportSha256=digest(report_path)),
        acceptance=dict(verdict='accepted', lodSha256=acceptance['lodSha256'], evidence=evidence,
            scope=acceptance.get('scope'), visualVerdict=acceptance.get('visualVerdict'),
            authorization=acceptance.get('authorization')))
    write_package(receipt, report, args.candidate, args.output)


def prepare_repair(args):
    """Package a normals repair: the previous shipped LOD0 GLB is source and donor."""
    report_path = Path(args.candidate) / 'build-report.json'
    report = read(report_path)
    if report.get('method') != REPAIR_METHOD:
        raise ValueError('build report is not a normals repair')
    acceptance = read(args.acceptance)
    source = Path(args.source)
    source_sha = digest(source)
    if report['sourceSha256'] != source_sha:
        raise ValueError('repair source differs from the build report')
    vehicle_id = report['vehicleId']
    lods, evidence = review_lods(Path(args.candidate), report, vehicle_id, acceptance)
    upstream = report.get('upstream', {}).get('source') or {}
    receipt = dict(schemaVersion=1, kind='generated-vehicle', vehicleId=vehicle_id,
        body=report['body'], lengthM=report['lengthM'], wheelRadiusM=report['wheelRadiusM'],
        typeIds=report['typeIds'], lods=lods,
        source=dict(method=REPAIR_METHOD, sha256=source_sha, bytes=source.stat().st_size,
                    file=source.name),
        donor=dict(path=f'shipped-runtime-glb:generated/{vehicle_id}/{source.name}', sha256=source_sha,
                   bytes=source.stat().st_size, kind='shipped-runtime-glb'),
        upstream=upstream,
        assembly=dict(generator=report['generator'], reportSha256=digest(report_path),
                      upstreamGenerator=report.get('upstream', {}).get('generator')),
        acceptance=dict(verdict='accepted', lodSha256=acceptance['lodSha256'], evidence=evidence))
    write_package(receipt, report, args.candidate, args.output)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--study', action='store_true',
        help='accepted for existing invocations; study conversion is the default mode')
    for option in ('candidate', 'source', 'acceptance', 'output'):
        parser.add_argument('--' + option, required=True)
    parser.add_argument('--vehicle-id', default=None,
                        help='explicit body key for a v3 bodies-map study report')
    parser.add_argument('--repair', action='store_true',
        help='package a normals repair of a shipped GLB set (source = previous LOD0 GLB)')
    args = parser.parse_args()
    prepare_repair(args) if args.repair else prepare_study(args)
