"""Pack the pinned coastal scan into the three texture layers used by the game.

python3 native/tools/cook-intertidal-rock.py external/cc0

The source is the CC0 store: `<repo>/external/cc0` (filled by
`scripts/setup-assets.py`) or a copy outside the repository (the folder
EO_CC0_ROOT names). Every other folder inside the repository is refused.
"""
import argparse
import hashlib
import io
import json
from pathlib import Path

from PIL import Image

REPO = Path(__file__).resolve().parents[2]
SIZE = 512
ROLES = ("albedo", "normal", "arm")


def check_root(root):
    """`root` (resolved) is `<repo>/external/cc0` or outside the repository."""
    if (root == REPO or REPO in root.parents) and root != (REPO / "external" / "cc0").resolve():
        raise ValueError("use the external CC0 store (<repo>/external/cc0, or a copy outside the repository)")


def cook(root):
    check_root(root)
    manifest = json.loads((REPO / "native/assets/cc0-manifest.json").read_text())
    source = next(a for a in manifest["assets"] if a["id"] == "seaside_rock")
    inputs = []
    image = Image.new("RGB", (SIZE, SIZE * len(ROLES)))
    for i, role in enumerate(ROLES):
        file = next(f for f in source["files"] if f["role"] == role)
        data = (root / file["path"]).read_bytes()
        if len(data) != file["bytes"] or hashlib.sha256(data).hexdigest() != file["sha256"]:
            raise ValueError(f"source verification failed for {role}")
        with Image.open(io.BytesIO(data)) as original:
            tile = original.convert("RGB").resize((SIZE, SIZE), Image.Resampling.BOX)
        image.paste(tile, (0, i * SIZE))
        inputs.append({k: file[k] for k in ("role", "url", "sha256", "bytes")})

    out = REPO / "public/assets/shore/seaside-rock.png"
    out.parent.mkdir(parents=True, exist_ok=True)
    image.save(out, format="PNG", compress_level=9)
    record = {k: source[k] for k in ("name", "page", "authors", "licence", "size_m")}
    record.update(cooker="native/tools/cook-intertidal-rock.py", inputs=inputs,
                  panels=list(ROLES), width=SIZE, image=out.name,
                  sha256=hashlib.sha256(out.read_bytes()).hexdigest())
    out.with_suffix(".provenance.json").write_text(json.dumps(record, indent=2) + "\n")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source_root", type=Path)
    cook(parser.parse_args().source_root.resolve())
