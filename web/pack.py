"""Puts the wasm-bindgen glue and the gzipped program into the page template."""

import base64
import gzip
import os
import sys

glue_path, wasm_path, template_path, out_path = sys.argv[1:5]
glue = open(glue_path, encoding="utf-8").read()
wasm = open(wasm_path, "rb").read()
packed = base64.b64encode(gzip.compress(wasm, compresslevel=9, mtime=0)).decode("ascii")
page = open(template_path, encoding="utf-8").read()
# Inside an inline script, "</script" would end it early.
assert "</script" not in glue.lower()
page = page.replace("@@GLUE@@", glue, 1).replace("@@WASM@@", packed, 1)
os.makedirs(os.path.dirname(out_path) or ".", exist_ok=True)
with open(out_path, "w", encoding="utf-8") as f:
    f.write(page)
print(f"{out_path}: {len(page) / 1e6:.1f} MB (program {len(wasm) / 1e6:.1f} MB, gzipped {len(packed) * 3 / 4 / 1e6:.1f} MB)")
