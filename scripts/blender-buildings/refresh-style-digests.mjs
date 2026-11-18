// Refresh the `styleSha256` build records after a notes-only edit of style.json.
//
// A building's public/assets/buildings/<id>/provenance.json records the
// SHA-256 of the style.json it was built from. When only the free-text
// "notes" of a style.json are reworded (no geometry input changes), this
// script moves each record that matched the style.json at the base revision
// to the new file's digest and marks it with `styleNotesRevised`. A record
// that did not match the base revision (stale before the edit) is left as
// it is, and so is any style.json whose other fields changed.
//
//   node scripts/blender-buildings/refresh-style-digests.mjs [--base <git rev>] [--dry-run]
//
// The base revision defaults to HEAD.
import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { readFileSync, readdirSync, writeFileSync, existsSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const REPO = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const argv = process.argv.slice(2);
const base = argv.includes('--base') ? argv[argv.indexOf('--base') + 1] : 'HEAD';
const dryRun = argv.includes('--dry-run');
const NOTE = 'notes text reworded for publication after the build; geometry inputs unchanged';

const sha256 = (buf) => createHash('sha256').update(buf).digest('hex');
const atBase = (rel) => {
  try {
    return execFileSync('git', ['show', `${base}:${rel}`], { cwd: REPO, maxBuffer: 64 << 20 });
  } catch {
    return null;
  }
};
const withoutNotes = (text) => {
  const j = JSON.parse(text);
  delete j.notes;
  return JSON.stringify(j);
};

const dir = path.join(REPO, 'public/assets/buildings');
const counts = { refreshed: 0, staleBefore: 0, unchanged: 0, notNotesOnly: 0, noRecord: 0 };
const stale = [];
for (const id of readdirSync(dir).sort()) {
  const provPath = path.join(dir, id, 'provenance.json');
  if (!existsSync(provPath)) continue;
  const raw = readFileSync(provPath, 'utf8');
  const prov = JSON.parse(raw);
  const rec = prov.inputs && typeof prov.inputs.styleSha256 === 'string' ? prov.inputs : null;
  if (!rec || typeof rec.style !== 'string') {
    counts.noRecord += 1;
    continue;
  }
  const stylePath = path.join(REPO, rec.style);
  if (!existsSync(stylePath)) {
    counts.noRecord += 1;
    continue;
  }
  // Digests are of the bytes as committed: a checkout with core.autocrlf
  // holds CRLF, the repository LF.
  const now = Buffer.from(readFileSync(stylePath, 'utf8').replace(/\r\n/g, '\n'), 'utf8');
  const nowSha = sha256(now);
  if (rec.styleSha256 === nowSha) {
    counts.unchanged += 1;
    continue;
  }
  const before = atBase(rec.style);
  if (!before || sha256(before) !== rec.styleSha256) {
    counts.staleBefore += 1;
    stale.push(id);
    continue;
  }
  if (withoutNotes(before.toString('utf8')) !== withoutNotes(now.toString('utf8'))) {
    counts.notNotesOnly += 1;
    console.warn(`${id}: style.json changed beyond its notes; record left as it is`);
    continue;
  }
  // Text edit of the one record: the digest, then the marker on the line after it.
  const line = raw.match(new RegExp(`^([ \\t]*)"styleSha256": "${rec.styleSha256}"(,?)(\\r?\\n)`, 'm'));
  if (!line) throw new Error(`${id}: styleSha256 line not found`);
  const [whole, indent, comma, eol] = line;
  const next = `${indent}"styleSha256": "${nowSha}",${eol}${indent}"styleNotesRevised": ${JSON.stringify(NOTE)}${comma}${eol}`;
  const updated = raw.replace(whole, next);
  const check = JSON.parse(updated);
  if (check.inputs.styleSha256 !== nowSha || check.inputs.styleNotesRevised !== NOTE) throw new Error(`${id}: edit did not land`);
  if (!dryRun) writeFileSync(provPath, updated);
  counts.refreshed += 1;
}
// Each artifacts/buildings/<id>/manifest-fragment.json records the digest of
// that provenance.json: a record that matched the base revision follows it.
let fragments = 0;
for (const id of readdirSync(dir).sort()) {
  const provRel = `public/assets/buildings/${id}/provenance.json`;
  const fragPath = path.join(REPO, 'artifacts/buildings', id, 'manifest-fragment.json');
  if (!existsSync(fragPath) || !existsSync(path.join(REPO, provRel))) continue;
  const raw = readFileSync(fragPath, 'utf8');
  const rec = JSON.parse(raw).provenance;
  if (!rec || rec.path !== provRel || typeof rec.sha256 !== 'string') continue;
  const before = atBase(provRel);
  if (!before || sha256(before) !== rec.sha256) continue;
  const nowSha = sha256(Buffer.from(readFileSync(path.join(REPO, provRel), 'utf8').replace(/\r\n/g, '\n'), 'utf8'));
  if (nowSha === rec.sha256) continue;
  const updated = raw.replace(`"sha256": "${rec.sha256}"`, `"sha256": "${nowSha}"`);
  if (JSON.parse(updated).provenance.sha256 !== nowSha) throw new Error(`${id}: fragment edit did not land`);
  if (!dryRun) writeFileSync(fragPath, updated);
  fragments += 1;
}
console.log(`${dryRun ? '(dry run) ' : ''}manifest fragments following their provenance: ${fragments}`);
console.log(`${dryRun ? '(dry run) ' : ''}refreshed ${counts.refreshed}, already current ${counts.unchanged}, stale before the edit (left) ${counts.staleBefore}, changed beyond notes (left) ${counts.notNotesOnly}, no style record ${counts.noRecord}`);
if (stale.length) console.log(`stale records left: ${stale.length}`);
