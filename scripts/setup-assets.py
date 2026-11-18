#!/usr/bin/env python3
"""Fill <repo>/external/ with the optional open assets the game reads.

The street people, their motions and walking styles, the scanned CC0
surfaces, the sounds, the official sign art and sign font, and the far-shore
elevation are too large for Git (about 2 GB). Each set has a tracked manifest
that pins every file by source, size and SHA-256. This script downloads
exactly those files into one git-ignored folder per set:

    external/people          native/assets/people-manifest.json
    external/people-motions  native/assets/rocketbox-motions-manifest.json
    external/people-styles   native/assets/100style-manifest.json
    external/cc0             native/assets/cc0-manifest.json
    external/audio           native/assets/audio-manifest.json
    external/sign-art        native/eo-props/data/sign-art-manifest.json
    external/dem             native/assets/dem-manifest.json

The game reads each set from its folder there unless the set's environment
variable (EO_PEOPLE_ROOT and the others, see native/README.md) names another
folder. Never commit anything under external/.

    python3 scripts/setup-assets.py                    # every set
    python3 scripts/setup-assets.py --only people,dem  # some sets
    python3 scripts/setup-assets.py --dry-run          # what is missing; no request is made
    python3 scripts/setup-assets.py --root <folder>    # another folder than <repo>/external

How each set is obtained, as its manifest records it:

  * people, people-motions, people-styles, dem: one URL per file.
  * cc0: Poly Haven maps one URL per file; each ambientCG set is one ZIP
    (pinned itself) whose pinned members are kept under <set>/x/. The
    `_index/` listings and preview thumbnails used to choose the sets record
    no URL and the game never reads them: they are listed as skipped.
  * audio: the manifest names each source dataset at a pinned revision
    (`sources`); a clip's URL is that revision's file at the clip's path
    without its first folder. The bank's marker file MANIFEST.tsv (one line
    per clip: source, Freesound ID, uploader, licence, URLs, size, SHA-256,
    path, roles, screen verdict) is written from the manifest once the clips
    verify.
  * sign-art: the FHWA SVGs are single entries of very large release ZIPs:
    the ZIP's directory and each pinned entry are read with HTTP Range
    requests (as native/tools/fetch-sign-art.mjs does) instead of
    downloading the archive; the Caltrans PDFs are plain URLs; the Overpass
    fonts and licence are members of the pinned release ZIP.

Rules this script keeps: Python 3 standard library only; requests go only to
the hosts the manifests name, and to the download hosts those hosts redirect
to (REDIRECT_HOSTS below; any other redirect is refused); a generic
User-Agent and nothing about the user or the project in any request; one
request at a time with a short pause between requests; every file is written
to a `.part` file, checked for size and SHA-256, then renamed into place, so
an interrupted run resumes where it stopped and a file already present with
the right digest is never fetched again; each archive is extracted into its
own new empty temporary folder and only its pinned members are kept; nothing
downloaded is ever executed. Files in the folders that no manifest names are
left alone.
"""

from __future__ import annotations

import argparse
import hashlib
import http.client
import json
import os
import posixpath
import re
import shutil
import socket
import ssl
import stat
import struct
import sys
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request
import zipfile
import zlib
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
USER_AGENT = 'asset-fetch/1.0'
CHUNK = 1 << 20
TIMEOUT_S = 60
ATTEMPTS = 5

# (folder under external/, the game's variable for it, manifest under the repository)
SETS = [
    ('people', 'EO_PEOPLE_ROOT', 'native/assets/people-manifest.json'),
    ('people-motions', 'EO_PEOPLE_MOTIONS_ROOT', 'native/assets/rocketbox-motions-manifest.json'),
    ('people-styles', 'EO_PEOPLE_STYLES_ROOT', 'native/assets/100style-manifest.json'),
    ('cc0', 'EO_CC0_ROOT', 'native/assets/cc0-manifest.json'),
    ('audio', 'EO_AUDIO_ROOT', 'native/assets/audio-manifest.json'),
    ('sign-art', 'EO_SIGN_ART_ROOT', 'native/eo-props/data/sign-art-manifest.json'),
    ('dem', 'EO_DEM_ROOT', 'native/assets/dem-manifest.json'),
]

# The download hosts a host named in a manifest sends its files from (its own
# CDN), by that host. A redirect anywhere else is refused.
REDIRECT_HOSTS = {
    'huggingface.co': ('*.hf.co',),
    'github.com': ('objects.githubusercontent.com', 'release-assets.githubusercontent.com', 'codeload.github.com'),
    'ambientcg.com': ('acg-download.struffelproductions.com',),
}


class Refused(Exception):
    """A request or a file this script will not accept; never retried."""


class Transient(Exception):
    """A failure worth retrying (a cut transfer, a busy server)."""


class Item:
    """One pinned file: where it goes under the set folder and how it is obtained."""

    __slots__ = ('kind', 'rel', 'bytes', 'sha256', 'url', 'entry', 'archive', 'note')

    def __init__(self, kind, rel, size, sha256, url=None, entry=None, archive=None, note=''):
        self.kind = kind  # 'url', 'remote-zip', 'archive' or 'skip'
        self.rel = rel
        self.bytes = int(size)
        self.sha256 = sha256
        self.url = url
        self.entry = entry
        self.archive = archive
        self.note = note


# ---------------------------------------------------------------- the plans


def plan_files(man):
    """people, people-motions, people-styles: `files` with one URL each."""
    return [Item('url', f['path'], f['bytes'], f['sha256'], url=f['url']) for f in man['files']]


def plan_dem(man):
    return [Item('url', t['file'], t['bytes'], t['sha256'], url=t['url']) for t in man['tiles']]


def plan_cc0(man):
    items = []
    for a in man['assets']:
        for f in a['files']:
            if f.get('url'):
                items.append(Item('url', f['path'], f['bytes'], f['sha256'], url=f['url']))
            elif f.get('archive'):
                # fetch-cc0.mjs extracted each ZIP into x/ beside it.
                base = posixpath.join(posixpath.dirname(f['archive']), 'x')
                entry = posixpath.relpath(f['path'], base)
                items.append(Item('archive', f['path'], f['bytes'], f['sha256'], entry=entry, archive=f['archive']))
            else:
                items.append(Item('skip', f['path'], f['bytes'], f['sha256'], note='no source URL or archive recorded'))
    for f in man.get('index', []):
        items.append(Item('skip', f['path'], f['bytes'], f['sha256'], note='set-choosing listing or preview: no URL recorded, never read by the game'))
    return items


def audio_sources(man):
    """The audio manifest's `sources`: folder -> (dataset, revision)."""
    out = {}
    for key, text in man['sources'].items():
        m = re.search(r'https://huggingface\.co/datasets/([\w.-]+/[\w.-]+)@([0-9a-f]{40})', text)
        if not m:
            raise Refused(f'audio manifest: source {key} names no dataset revision')
        out[key.replace('_', '-')] = (m.group(1), m.group(2))
    return out


def plan_audio(man):
    sources = audio_sources(man)
    items, seen = [], {}
    for c in man['clips']:
        if c['path'] in seen:
            if seen[c['path']] != c['sha256']:
                raise Refused(f"audio manifest: {c['path']} listed with two digests")
            continue
        seen[c['path']] = c['sha256']
        folder, _, rest = c['path'].partition('/')
        if folder not in sources or not rest:
            raise Refused(f"audio manifest: no source for {c['path']}")
        dataset, revision = sources[folder]
        url = f'https://huggingface.co/datasets/{dataset}/resolve/{revision}/' + urllib.parse.quote(rest)
        items.append(Item('url', c['path'], c['bytes'], c['sha256'], url=url))
    return items


def plan_sign_art(man):
    files = man['files']
    plain = {f['url']: f for f in files if 'zipEntry' not in f and 'archiveEntry' not in f}
    items = []
    for f in files:
        if 'zipEntry' in f:
            items.append(Item('remote-zip', f['path'], f['bytes'], f['sha256'], url=f['url'], entry=f['zipEntry']))
        elif 'archiveEntry' in f:
            arch = plain.get(f['url'])
            if arch is None:
                raise Refused(f"sign-art manifest: the archive of {f['path']} is not pinned")
            items.append(Item('archive', f['path'], f['bytes'], f['sha256'], entry=f['archiveEntry'], archive=arch['path']))
        else:
            items.append(Item('url', f['path'], f['bytes'], f['sha256'], url=f['url']))
    return items


PLANS = {
    'people': plan_files,
    'people-motions': plan_files,
    'people-styles': plan_files,
    'cc0': plan_cc0,
    'audio': plan_audio,
    'sign-art': plan_sign_art,
    'dem': plan_dem,
}


# ---------------------------------------------------------------- files


def safe_rel(rel):
    """A manifest path: relative, forward slashes, no empty, '.' or '..' part."""
    parts = rel.split('/')
    if not rel or rel.startswith('/') or '\\' in rel or ':' in parts[0] or any(p in ('', '.', '..') for p in parts):
        raise Refused(f'refused path {rel!r}')
    return parts


def under(base, rel):
    return base.joinpath(*safe_rel(rel))


def sha256_file(path):
    h = hashlib.sha256()
    with open(path, 'rb') as f:
        for chunk in iter(lambda: f.read(CHUNK), b''):
            h.update(chunk)
    return h.hexdigest()


def verified(path, size, sha):
    try:
        st = path.stat()
    except OSError:
        return False
    return stat.S_ISREG(st.st_mode) and st.st_size == size and sha256_file(path) == sha


def write_atomic(dest, data):
    dest.parent.mkdir(parents=True, exist_ok=True)
    part = dest.with_name(dest.name + '.part')
    with open(part, 'wb') as f:
        f.write(data)
        f.flush()
        os.fsync(f.fileno())
    os.replace(part, dest)


def mb(n):
    return f'{n / 1e6:.1f} MB'


# ---------------------------------------------------------------- network


def host_allowed(host, allowed):
    host = (host or '').lower()
    for a in allowed:
        if a.startswith('*.') and host.endswith(a[1:]):
            return True
        if host == a:
            return True
    return False


class _Redirects(urllib.request.HTTPRedirectHandler):
    max_redirections = 5

    def __init__(self, net):
        super().__init__()
        self.net = net

    def redirect_request(self, req, fp, code, msg, headers, newurl):
        u = urllib.parse.urlsplit(newurl)
        if u.scheme != 'https' or not host_allowed(u.hostname, self.net.allowed):
            raise Refused(f'refusing a redirect to {u.scheme}://{u.hostname}')
        return super().redirect_request(req, fp, code, msg, headers, newurl)


class Net:
    """One request at a time, a pause between requests, only the allowed hosts."""

    def __init__(self, pause):
        self.pause = pause
        self.allowed = set()
        self.last = 0.0
        self.requests = 0
        ctx = ssl.create_default_context()
        self.opener = urllib.request.build_opener(urllib.request.HTTPSHandler(context=ctx), _Redirects(self))

    def allow_from(self, urls):
        self.allowed = set()
        for url in urls:
            host = urllib.parse.urlsplit(url).hostname
            self.allowed.add(host)
            self.allowed.update(REDIRECT_HOSTS.get(host, ()))

    def open(self, url, headers=None):
        u = urllib.parse.urlsplit(url)
        if u.scheme != 'https' or not host_allowed(u.hostname, self.allowed):
            raise Refused(f'refusing a request to {u.scheme}://{u.hostname}')
        wait = self.last + self.pause - time.monotonic()
        if wait > 0:
            time.sleep(wait)
        h = {'User-Agent': USER_AGENT, 'Accept-Encoding': 'identity'}
        h.update(headers or {})
        self.requests += 1
        try:
            r = self.opener.open(urllib.request.Request(url, headers=h), timeout=TIMEOUT_S)
        except urllib.error.HTTPError as e:
            self.last = time.monotonic()
            e.close()
            if e.code in (408, 425, 429) or e.code >= 500:
                raise Transient(f'HTTP {e.code} from {u.hostname}')
            raise Refused(f'HTTP {e.code} from {u.hostname}')
        except (urllib.error.URLError, OSError, http.client.HTTPException) as e:
            self.last = time.monotonic()
            raise Transient(f'{u.hostname}: {e}')
        final = urllib.parse.urlsplit(r.geturl()).hostname
        if not host_allowed(final, self.allowed):
            r.close()
            raise Refused(f'refusing a reply from {final}')
        return _Closing(r, self)


class _Closing:
    """A reply that stamps the end of the request when it is closed."""

    def __init__(self, r, net):
        self.r, self.net = r, net

    def __enter__(self):
        return self.r

    def __exit__(self, *exc):
        self.r.close()
        self.net.last = time.monotonic()
        return False


def retrying(what, fn):
    """`fn` up to ATTEMPTS times while it fails transiently."""
    for attempt in range(1, ATTEMPTS + 1):
        try:
            return fn()
        except (Transient, OSError, http.client.HTTPException) as e:
            if attempt == ATTEMPTS:
                raise Refused(f'{what}: gave up after {ATTEMPTS} attempts ({e})')
            print(f'    {what}: attempt {attempt} failed ({e}); retrying', flush=True)
            time.sleep(2.0 * 2 ** (attempt - 1))


def download(net, url, dest, size, sha):
    """`url` into `dest` through `dest.part` (resumed when present), checked for size and SHA-256."""
    part = dest.with_name(dest.name + '.part')
    dest.parent.mkdir(parents=True, exist_ok=True)
    fresh = [False]

    def once():
        have = part.stat().st_size if part.exists() else 0
        if fresh[0] or have > size:
            have = 0
        h = hashlib.sha256()
        if have:
            with open(part, 'rb') as f:
                for chunk in iter(lambda: f.read(CHUNK), b''):
                    h.update(chunk)
        if have < size or size == 0:
            headers = {'Range': f'bytes={have}-'} if have else {}
            with net.open(url, headers) as r:
                if have and r.status == 206:
                    m = re.match(r'bytes (\d+)-', r.headers.get('Content-Range') or '')
                    if not m or int(m.group(1)) != have:
                        raise Transient(f'unexpected Content-Range {r.headers.get("Content-Range")!r}')
                    mode = 'ab'
                elif r.status == 200:
                    have, h, mode = 0, hashlib.sha256(), 'wb'
                else:
                    raise Refused(f'HTTP {r.status}')
                with open(part, mode) as out:
                    while True:
                        chunk = r.read(CHUNK)
                        if not chunk:
                            break
                        have += len(chunk)
                        if have > size:
                            raise Refused(f'longer than the {size} B the manifest records')
                        h.update(chunk)
                        out.write(chunk)
                    out.flush()
                    os.fsync(out.fileno())
        if have != size:
            raise Transient(f'{have} of {size} B received')
        got = h.hexdigest()
        if got != sha:
            part.unlink()
            if not fresh[0]:
                # A resumed part may have joined two versions: once more from the start.
                fresh[0] = True
                raise Transient(f'SHA-256 {got} differs from the manifest')
            raise Refused(f'SHA-256 {got} differs from the manifest')
        os.replace(part, dest)

    retrying(url, once)


class RemoteZip:
    """A ZIP on a server that answers Range requests: its directory and single entries."""

    def __init__(self, net, url):
        self.net, self.url, self.entries = net, url, None

    def range(self, a, b):
        def once():
            with self.net.open(self.url, {'Range': f'bytes={a}-{b}'}) as r:
                if r.status != 206:
                    raise Refused(f'no range support (HTTP {r.status})')
                m = re.match(r'bytes (\d+)-(\d+)/(\d+)', r.headers.get('Content-Range') or '')
                if not m or int(m.group(1)) != a:
                    raise Transient(f'unexpected Content-Range {r.headers.get("Content-Range")!r}')
                want = int(m.group(2)) - a + 1
                body = r.read(want + 1)
                if len(body) != want:
                    raise Transient(f'{len(body)} of {want} B received')
                return body, int(m.group(3))

        return retrying(f'{self.url} bytes {a}-{b}', once)

    def directory(self):
        if self.entries is not None:
            return self.entries
        _, size = self.range(0, 0)
        tail_len = min(size, (1 << 16) + 22)
        tail, _ = self.range(size - tail_len, size - 1)
        eocd = tail.rfind(b'PK\x05\x06')
        if eocd < 0:
            raise Refused('no end of central directory')
        count, cd_size, cd_off = struct.unpack_from('<HII', tail, eocd + 10)
        if 0xFFFFFFFF in (cd_size, cd_off) or count == 0xFFFF:
            loc = eocd - 20
            if loc < 0 or tail[loc:loc + 4] != b'PK\x06\x07':
                raise Refused('no ZIP64 locator')
            (z64,) = struct.unpack_from('<Q', tail, loc + 8)
            z, _ = self.range(z64, z64 + 55)
            if z[:4] != b'PK\x06\x06':
                raise Refused('no ZIP64 end of central directory')
            cd_size, cd_off = struct.unpack_from('<QQ', z, 40)
        cd, _ = self.range(cd_off, cd_off + cd_size - 1)
        entries, p = {}, 0
        while p + 46 <= len(cd) and cd[p:p + 4] == b'PK\x01\x02':
            flags, method = struct.unpack_from('<HH', cd, p + 8)
            crc, csize, usize = struct.unpack_from('<III', cd, p + 16)
            nl, el, kl = struct.unpack_from('<HHH', cd, p + 28)
            (off,) = struct.unpack_from('<I', cd, p + 42)
            name = cd[p + 46:p + 46 + nl].decode('utf-8' if flags & 0x800 else 'cp437')
            e = p + 46 + nl
            while e + 4 <= p + 46 + nl + el:
                xid, xsz = struct.unpack_from('<HH', cd, e)
                if xid == 1:
                    q = e + 4
                    if usize == 0xFFFFFFFF:
                        (usize,) = struct.unpack_from('<Q', cd, q)
                        q += 8
                    if csize == 0xFFFFFFFF:
                        (csize,) = struct.unpack_from('<Q', cd, q)
                        q += 8
                    if off == 0xFFFFFFFF:
                        (off,) = struct.unpack_from('<Q', cd, q)
                e += 4 + xsz
            entries[name] = (flags, method, crc, csize, usize, off)
            p += 46 + nl + el + kl
        self.entries = entries
        return entries

    def read(self, name, size):
        e = self.directory().get(name)
        if e is None:
            raise Refused(f'no entry {name!r} in the archive')
        flags, method, crc, csize, usize, off = e
        if usize != size:
            raise Refused(f'the archive lists {usize} B, the manifest {size}')
        if flags & 1:
            raise Refused('encrypted entry')
        if method not in (0, 8):
            raise Refused(f'compression method {method}')
        # The local header, its name and extra field (guessed at most 1 KB) and the data in one request.
        buf, _ = self.range(off, off + 30 + 1024 + csize - 1)
        if buf[:4] != b'PK\x03\x04':
            raise Refused('bad local header')
        nl, el = struct.unpack_from('<HH', buf, 26)
        start = 30 + nl + el
        if start + csize > len(buf):
            buf, _ = self.range(off + start, off + start + csize - 1)
            start = 0
        data = buf[start:start + csize]
        if method == 8:
            d = zlib.decompressobj(-15)
            raw = d.decompress(data, usize + 1)
            raw += d.flush()
        else:
            raw = data
        if len(raw) != usize or zlib.crc32(raw) & 0xFFFFFFFF != crc:
            raise Refused('the entry does not decompress to its recorded size and CRC')
        return raw


# ---------------------------------------------------------------- sets


# (set folder, path) of every file verified this run.
verified_cache = {}


class Tally:
    def __init__(self, name, files):
        self.name, self.files = name, files
        self.ok = self.fetched = self.skipped = self.size = 0
        self.failed = []  # (path, why)
        self.missing_bytes = 0


def extract_members(archive, members, set_dir, tmp_root, tally):
    """The pinned members of a local ZIP, through a new empty temporary folder."""
    tmp_root.mkdir(parents=True, exist_ok=True)
    tmp = Path(tempfile.mkdtemp(prefix='extract-', dir=str(tmp_root)))
    try:
        with zipfile.ZipFile(archive) as z:
            infos = {i.filename: i for i in z.infolist()}
            for it in members:
                try:
                    info = infos.get(it.entry)
                    if info is None:
                        raise Refused(f'no entry {it.entry!r} in {archive.name}')
                    kind = (info.external_attr >> 16) & 0o170000
                    if info.is_dir() or kind == stat.S_IFLNK or info.flag_bits & 1:
                        raise Refused(f'{it.entry} is not a plain file')
                    if info.file_size != it.bytes:
                        raise Refused(f'{it.entry}: the archive lists {info.file_size} B, the manifest {it.bytes}')
                    out = under(tmp, it.entry)
                    out.parent.mkdir(parents=True, exist_ok=True)
                    h, n = hashlib.sha256(), 0
                    with z.open(info) as src, open(out, 'wb') as dst:
                        for chunk in iter(lambda: src.read(CHUNK), b''):
                            n += len(chunk)
                            if n > it.bytes:
                                raise Refused(f'{it.entry} is longer than recorded')
                            h.update(chunk)
                            dst.write(chunk)
                    if h.hexdigest() != it.sha256:
                        raise Refused(f'{it.entry}: SHA-256 {h.hexdigest()} differs from the manifest')
                    dest = under(set_dir, it.rel)
                    dest.parent.mkdir(parents=True, exist_ok=True)
                    os.replace(out, dest)
                    tally.ok += 1
                    tally.fetched += 1
                    tally.size += it.bytes
                    print(f'    extracted {it.rel} ({mb(it.bytes)})', flush=True)
                except (Refused, zipfile.BadZipFile, OSError) as e:
                    tally.failed.append((it.rel, str(e)))
    except (zipfile.BadZipFile, OSError) as e:
        for it in members:
            if not any(p == it.rel for p, _ in tally.failed):
                tally.failed.append((it.rel, f'{archive.name}: {e}'))
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


def write_audio_index(set_dir, man, items):
    """MANIFEST.tsv, the bank's marker: one line per verified clip of the manifest."""
    ok = {it.rel: it for it in items if verified_cache.get((set_dir, it.rel))}
    if not ok:
        return
    roles = {}
    for c in man['clips']:
        roles.setdefault(c['path'], []).append(c['role'])
    clean = lambda s: re.sub(r'[\t\r\n]+', ' ', str(s if s is not None else ''))
    rows = ['source\tfreesound_id\tuploader\tlicence\tsource_url\tfetched_from\tbytes\tsha256\tpath\troles\tscreen\tscreen_note']
    seen = set()
    for c in man['clips']:
        it = ok.get(c['path'])
        if it is None or c['path'] in seen:
            continue
        seen.add(c['path'])
        if c.get('freesound_id'):
            src = [c['source'], c['freesound_id'], c.get('uploader'), c['licence'], f"https://freesound.org/s/{c['freesound_id']}/"]
        else:
            src = [c['source'], '', c.get('uploader'), c['licence'], 'https://github.com/wyzowl/sfx-kit']
        # Every clip in the game's bank passed the automatic screen (write-audio-manifest.mjs).
        rows.append('\t'.join(clean(v) for v in src + [it.url, it.bytes, it.sha256, it.rel, ','.join(roles[c['path']]), 'pass', '']))
    write_atomic(set_dir / 'MANIFEST.tsv', ('\n'.join(rows) + '\n').encode('utf-8'))
    print(f'    wrote MANIFEST.tsv ({len(rows) - 1} clips)', flush=True)


def run_set(name, manifest_rel, root, net, dry_run):
    man = json.loads((REPO / manifest_rel).read_text(encoding='utf-8'))
    items = PLANS[name](man)
    set_dir = root / name
    tally = Tally(name, len(items))
    print(f'{name}: {len(items)} files pinned by {manifest_rel} -> {set_dir}', flush=True)
    todo = []
    for it in items:
        if it.kind == 'skip':
            tally.skipped += 1
            continue
        try:
            path = under(set_dir, it.rel)
        except Refused as e:
            tally.failed.append((it.rel, str(e)))
            continue
        if verified(path, it.bytes, it.sha256):
            verified_cache[(set_dir, it.rel)] = True
            tally.ok += 1
            tally.size += it.bytes
        else:
            todo.append(it)
            tally.missing_bytes += it.bytes
    if tally.skipped:
        notes = sorted({it.note for it in items if it.kind == 'skip'})
        print(f'    {tally.skipped} skipped: {"; ".join(notes)}', flush=True)
    if dry_run:
        print(f'    {tally.ok} present, {len(todo)} to fetch ({mb(tally.missing_bytes)})', flush=True)
        for it in todo[:10]:
            print(f'      {it.rel}', flush=True)
        if len(todo) > 10:
            print(f'      ... and {len(todo) - 10} more', flush=True)
        return tally
    if todo:
        net.allow_from([it.url for it in items if it.url])
    # 1. plain downloads (archives first: their members wait on them).
    plain = [it for it in todo if it.kind == 'url']
    for i, it in enumerate(plain, 1):
        try:
            download(net, it.url, under(set_dir, it.rel), it.bytes, it.sha256)
            verified_cache[(set_dir, it.rel)] = True
            tally.ok += 1
            tally.fetched += 1
            tally.size += it.bytes
            print(f'    [{i}/{len(plain)}] {it.rel} ({mb(it.bytes)})', flush=True)
        except Refused as e:
            tally.failed.append((it.rel, str(e)))
            print(f'    [{i}/{len(plain)}] FAILED {it.rel}: {e}', flush=True)
    # 2. members of the pinned local archives.
    by_archive = {}
    for it in todo:
        if it.kind == 'archive':
            by_archive.setdefault(it.archive, []).append(it)
    for arch, members in by_archive.items():
        arch_path = under(set_dir, arch)
        if not verified_cache.get((set_dir, arch)):
            for it in members:
                tally.failed.append((it.rel, f'its archive {arch} is not there'))
            continue
        extract_members(arch_path, members, set_dir, root / '.tmp', tally)
        for it in members:
            if not any(p == it.rel for p, _ in tally.failed):
                verified_cache[(set_dir, it.rel)] = True
    # 3. single entries of remote archives, read by range.
    by_zip = {}
    for it in todo:
        if it.kind == 'remote-zip':
            by_zip.setdefault(it.url, []).append(it)
    for url, members in by_zip.items():
        rz = RemoteZip(net, url)
        for it in members:
            try:
                raw = rz.read(it.entry, it.bytes)
                got = hashlib.sha256(raw).hexdigest()
                if got != it.sha256:
                    raise Refused(f'SHA-256 {got} differs from the manifest')
                write_atomic(under(set_dir, it.rel), raw)
                verified_cache[(set_dir, it.rel)] = True
                tally.ok += 1
                tally.fetched += 1
                tally.size += it.bytes
                print(f'    {it.rel} ({mb(it.bytes)}, read from {url.rsplit("/", 1)[-1]})', flush=True)
            except Refused as e:
                tally.failed.append((it.rel, str(e)))
                print(f'    FAILED {it.rel}: {e}', flush=True)
    if name == 'audio':
        write_audio_index(set_dir, man, [it for it in items if it.kind == 'url'])
    return tally


def inside(path, base):
    p, b = os.path.normcase(str(path)), os.path.normcase(str(base))
    return p == b or p.startswith(b.rstrip(os.sep) + os.sep)


def main(argv=None):
    names = [s[0] for s in SETS]
    ap = argparse.ArgumentParser(
        description='Download the optional open assets into <repo>/external/<set>/, each file checked against its manifest.',
        epilog='Sets: ' + ', '.join(f'{n} ({e})' for n, e, _ in SETS),
    )
    ap.add_argument('--only', help='comma-separated sets (default: all)')
    ap.add_argument('--dry-run', action='store_true', help='say what is present and what would be fetched; make no request')
    ap.add_argument('--root', help='the folder that holds the sets (default: <repo>/external)')
    ap.add_argument('--pause', type=float, default=0.2, help='seconds between requests (default 0.2)')
    a = ap.parse_args(argv)
    only = names
    if a.only:
        only = [s.strip() for s in a.only.split(',') if s.strip()]
        bad = [s for s in only if s not in names]
        if bad:
            ap.error(f'unknown set {", ".join(bad)}; the sets are {", ".join(names)}')
    default = REPO / 'external'
    root = Path(a.root).expanduser() if a.root else default
    root = root if root.is_absolute() else Path.cwd() / root
    # Inside this repository only external/ itself: no other tracked folder is a target.
    if inside(root.resolve(), REPO.resolve()) and os.path.normcase(str(root.resolve())) != os.path.normcase(str(default.resolve())):
        ap.error(f'refusing {root}: inside the repository the sets go only into {default}')
    if not a.dry_run:
        root.mkdir(parents=True, exist_ok=True)
    net = Net(a.pause)
    tallies = []
    started = time.monotonic()
    for name, _env, manifest in SETS:
        if name not in only:
            continue
        try:
            tallies.append(run_set(name, manifest, root, net, a.dry_run))
        except (Refused, OSError, KeyError, ValueError) as e:
            t = Tally(name, 0)
            t.failed.append(('(manifest)', str(e)))
            tallies.append(t)
            print(f'{name}: FAILED: {e}', flush=True)
    tmp = root / '.tmp'
    if tmp.is_dir():
        shutil.rmtree(tmp, ignore_errors=True)
    print()
    print(f'{"set":<16}{"files":>7}{"ok":>7}{"fetched":>9}{"failed":>8}{"skipped":>9}{"verified":>12}')
    for t in tallies:
        print(f'{t.name:<16}{t.files:>7}{t.ok:>7}{t.fetched:>9}{len(t.failed):>8}{t.skipped:>9}{mb(t.size):>12}')
    failed = [(t.name, p, w) for t in tallies for p, w in t.failed]
    if a.dry_run:
        print(f'dry run: {mb(sum(t.missing_bytes for t in tallies))} would be fetched into {root}; no request was made')
    else:
        print(f'{net.requests} requests in {time.monotonic() - started:.0f} s into {root}')
    for name, p, w in failed:
        print(f'FAILED {name}: {p}: {w}')
    return 1 if failed else 0


if __name__ == '__main__':
    sys.exit(main())
