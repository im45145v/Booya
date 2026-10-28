"""Checks the output of the layouts_gallery example: every layout exists in
every shape, has the right size, and the layouts really look different.
Run: python3 tests/e2e/check_gallery.py <gallery dir>"""
import itertools, sys
from pathlib import Path
from PIL import Image, ImageChops, ImageStat

shots = Path(sys.argv[1]) / "shots"
failed = []
SHAPES = {"landscape": lambda w, h: w > h, "portrait": lambda w, h: h > w, "square": lambda w, h: w == h}


def diff(a, b):
    if a.size != b.size:
        return 255
    return sum(ImageStat.Stat(ImageChops.difference(a.convert("RGB"), b.convert("RGB"))).mean) / 3


def check(ok, msg):
    print(("ok   " if ok else "FAIL ") + msg)
    if not ok:
        failed.append(msg)


for shape, test in SHAPES.items():
    names = [f"layout-{k}-{shape}.png" for k in ("screen", "bubble", "split", "camera")]
    names += [f"export-{shape}-{i}.png" for i in range(4)]
    imgs = {}
    for n in names:
        f = shots / n
        if not f.exists():
            check(False, f"{n} exists")
            continue
        imgs[n] = Image.open(f)
        check(test(*imgs[n].size), f"{n} has the {shape} shape {imgs[n].size}")
    for group in (names[:4], names[4:]):
        for a, b in itertools.combinations([g for g in group if g in imgs], 2):
            d = diff(imgs[a], imgs[b])
            # The mixed video has two segments that share a layout, so only the layout stills must all differ.
            if group is names[:4]:
                check(d > 3, f"{a} and {b} look different (mean difference {d:.1f})")
    ex = [imgs[n] for n in names[4:] if n in imgs]
    far = max((diff(a, b) for a, b in itertools.combinations(ex, 2)), default=0)
    check(far > 5, f"the {shape} export changes layout over time (largest difference {far:.1f})")

if failed:
    print("FAILED:", *failed, sep="\n  ")
    sys.exit(1)
print("gallery ok")
