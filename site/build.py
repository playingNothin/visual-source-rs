#!/usr/bin/env python3
"""Assembles the self-contained offline site (site/index.html + site/sw.js).

Everything is inlined into ONE html file so it works from file:// with zero
network: the stylesheet, dagre.min.js, the app logic, the favicon and the
Rust engine compiled to WebAssembly (as base64).

Run from the repository root AFTER building the wasm engine:

    rustup target add wasm32-unknown-unknown
    cd wasm-src && cargo build --target wasm32-unknown-unknown --release && cd ..
    python3 site/build.py

The finished file is written to site/index.html; docs/index.html gets a copy
for GitHub Pages (Settings -> Pages -> deploy from branch main, folder /docs).
"""

import base64
import pathlib

HERE = pathlib.Path(__file__).resolve().parent
WASM = HERE.parent / 'wasm-src/target/wasm32-unknown-unknown/release/visual_source_rs_wasm.wasm'


def b64(path: pathlib.Path) -> str:
    return base64.b64encode(path.read_bytes()).decode('ascii')


def main() -> None:
    style = (HERE / 'assets/style.css').read_text()
    dagre = (HERE / 'assets/dagre.min.js').read_text()
    favicon = b64(HERE / 'parts/favicon64.png')
    wasm = b64(WASM)
    body = (HERE / 'parts/index_body.html').read_text()
    app = (HERE / 'parts/app.js').read_text()
    extra = (HERE / 'parts/extra.css').read_text()

    app = app.replace('__WASM_B64__', wasm)
    body = body.replace('__FAVICON_B64__', favicon)

    html = f"""<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta content="width=device-width, initial-scale=1.0" name="viewport">
<title>basicallyLua — offline</title>
<link href="data:image/png;base64,{favicon}" rel="icon" type="image/png">
<style>
{style}
{extra}
</style>
</head>
<body>
{body}
<script>
{dagre}
</script>
<script>
{app}
</script>
</body>
</html>
"""
    out = HERE / 'index.html'
    out.write_text(html)
    print(f'index.html: {out} ({out.stat().st_size / 1024:.0f} KB, wasm {len(wasm) / 1024:.0f} KB b64)')

    sw = """// VisualSource offline cache: keeps this page available with no network.
// The page itself is fully self-contained (engine included), so caching one
// document is all that is needed.
// NETWORK-FIRST: when online, the latest deployed page is always fetched and
// the cache is refreshed, so every site redeploy shows up on the next visit.
// The cache is only used as the offline fallback.
const CACHE = 'vs4-offline-v12';
self.addEventListener('install', (e) => {
  e.waitUntil(caches.open(CACHE).then((c) => c.addAll(['./'])));
  self.skipWaiting();
});
self.addEventListener('activate', (e) => {
  e.waitUntil(caches.keys().then((keys) =>
    Promise.all(keys.filter((k) => k !== CACHE).map((k) => caches.delete(k)))));
  self.clients.claim();
});
self.addEventListener('fetch', (e) => {
  e.respondWith(
    fetch(e.request).then((res) => {
      const copy = res.clone();
      caches.open(CACHE).then((c) => c.put(e.request, copy));
      return res;
    }).catch(() => caches.match(e.request).then((hit) => hit || caches.match('./')))
  );
});
"""
    (HERE / 'sw.js').write_text(sw)

    docs = HERE.parent / 'docs/index.html'
    docs.write_text(html)
    (HERE.parent / 'docs/sw.js').write_text(sw)
    print(f'copied to {docs} for GitHub Pages')


if __name__ == '__main__':
    main()
