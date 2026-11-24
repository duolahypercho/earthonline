"""Bake the regional relief used offshore and in gaps in the city soundings."""
from collections import deque
import csv
import gzip
import hashlib
import io
import math
from pathlib import Path
import struct
import sys
from urllib.parse import quote
from urllib.request import urlopen

root = Path(__file__).resolve().parents[2]
out = root / "public/data/sf"
query = "z[(36.80):1:(38.70)][(-123.62):1:(-121.32)]"
url = "https://coastwatch.pfeg.noaa.gov/erddap/griddap/ETOPO_2022_v1_15s.csv?" + quote(query)
raw = Path(sys.argv[1]).read_bytes() if len(sys.argv)>1 else urlopen(url, timeout=120).read()
rows = csv.reader(io.StringIO(raw.decode()))
assert next(rows)==["latitude","longitude","z"]
next(rows)
lats, lons, values = [], [], []
for slat,slon,sz in rows:
    lat,lon,z=map(float,(slat,slon,sz))
    if not lats or lat!=lats[-1]:lats.append(lat)
    if len(lats)==1:lons.append(lon)
    assert math.isfinite(z) and -10000<z<10000
    values.append(z)
w,h=len(lons),len(lats)
assert len(values)==w*h and all(abs((b-a)-1/240)<1e-9 for axis in [lats,lons] for a,b in zip(axis,axis[1:]))
# Disconnected depressions in the inland DEM are not ocean.
ocean=bytearray(w*h)
queue=deque(i*w for i in range(h) if values[i*w]<0)
while queue:
    i=queue.popleft()
    if ocean[i] or values[i]>=0:continue
    ocean[i]=1;x=i%w;y=i//w
    if x:queue.append(i-1)
    if x<w-1:queue.append(i+1)
    if y:queue.append(i-w)
    if y<h-1:queue.append(i+w)
elev=[max(-32767,min(32767,round(z*4))) if sea else 32767 for z,sea in zip(values,ocean)]
data=struct.pack("<4sIIddd",b"EOS1",w,h,lons[0],lats[0],1/240)+struct.pack(f"<{w*h}h",*elev)
bio=io.BytesIO()
with gzip.GzipFile(fileobj=bio,mode="wb",mtime=0) as stream:stream.write(data)
target=out/"sf-shelf.bin.gz";target.write_bytes(bio.getvalue())
(out/"sf-shelf.NOTICE.txt").write_text(
    "Regional sea floor outside the city's high-resolution ground grid.\n"
    "NOAA National Centers for Environmental Information, 2022. ETOPO 2022 15 Arc-Second Global Relief Model. "
    "https://doi.org/10.25921/fd45-gt74\nU.S. Government public-domain data. Source heights relative to the EGM2008 geoid, "
    "used as approximate mean sea level on the outer shelf. Not for navigation.\n"
    f"Source: {url}\nRetrieved source CSV SHA-256: {hashlib.sha256(raw).hexdigest()}\n"
    "Produced by scripts/world-data/build-sf-shelf.py. Quarter-metre signed heights, south-to-north, disconnected inland terrain excluded.\n"
    f"Output SHA-256: {hashlib.sha256(target.read_bytes()).hexdigest()}\n")
print(w,h,sum(ocean),target.stat().st_size)
