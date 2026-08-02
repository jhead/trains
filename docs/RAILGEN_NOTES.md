# Notes from `railgen` research

Source: local experiment gallery at `~/dev/railgen` (“Permanent Way”), authored for this project.  
**OK to read and port into Rail Town** (JS → Rust). Prefer idiomatic Bevy/Rust rewrites over pasting canvas harness code. Gallery HTML/UI stays out of the game binary.

## What it is

Feasibility demos for **procedural pixel-art railroad track** (angle budgets, sprite banks, SDF banding, junctions, pixel crawl). Not a terrain/town engine. Rail Town keeps owning world gen in `rail_map`.

## Policies we adopt

| Policy | Why | Where it lands |
| --- | --- | --- |
| **Continuous-enough sim, finite render vocab** | Live arbitrary-angle Bresenham looks noisy; ~32–64 clean slope sprites is the real ceiling | Track: tile / N-dir graph for MVP; sprite bank later if curves need it |
| **Integer world-pixel camera** | Sub-pixel pan/zoom causes shimmer (“crawl”) | `rail_town` map camera: snap translation; discrete zoom steps |
| **Bake track art on edit** | Never full-frame procedural track every tick | Dirty chunks → regenerate sprites when track changes |
| **Min curve radius + min turnout divergence (~10°)** | Below that, flangeways vanish at pixel scale | Placement rules in `rail_sim` / track tools |
| **World-anchored decoration noise** | Screen-space dither/noise crawls under camera move | Hash on `floor(world_x, world_y)` for grass/ballast |

## Later / skip

- **Later:** Port `demos/spritebank.js` / `junction.js` math into Rust when track art graduates past tiles; SDF banding as WGSL experiment; offline `seqDepth` QA.
- **Skip:** Runtime supersample→downsample→palette as the main look; shipping the gallery shell as game UI; depending on railgen JS at runtime.

## MVP track art path

1. Placeholders / 8–16 direction tile sprites.  
2. Chunk bake-on-edit.  
3. Optional hand-authored or Rust-baked rotation bank (N≈32) when autofill/curves outgrow tiles.
