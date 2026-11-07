# PDF.js runtime

- Version: 6.5.438, Mozilla PDF.js / pdfjs-dist.
- Upstream: https://github.com/mozilla/pdf.js
- Archive: https://registry.npmjs.org/pdfjs-dist/-/pdfjs-dist-6.5.438.tgz
- Registry integrity (verified before extraction): `sha512-vg8Pw1x+AFKy4AMDvhuWl5cOYV8d698UoEANoFFhc79hqXDnZQqnOXoxw3HG907WHOg1JYOvLhP756boddFxpA==`
- Retrieved: 2026-11-07.
- `legacy/build/pdf.min.mjs` and `legacy/build/pdf.worker.min.mjs` are copied to this directory without modification. The `cmaps`, `standard_fonts`, `wasm` and `iccs` directories are copied verbatim. Upstream licenses are retained.
- All runtime assets are embedded by `build.rs` and served from `/static/pdfjs/`; there are no CDN or third-party runtime requests and no Node build step.
- Keep the main library, worker and auxiliary data on the same version. Verify the published archive integrity before replacing the complete set and rerun PDF rendering and protection tests.
