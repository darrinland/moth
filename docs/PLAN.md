# Moth: implementation plan to feature complete

Moth is a modern, cross-platform successor to Apophysis: explore random fractal flames,
refine one, render it at any size, and come back to it later. Everything is built on a
`Generator` abstraction, so other fractal types can plug in beside the flame.

This plan is ordered. Each step says what it delivers and how we know it is done. Sizes are
relative effort (S = hours, M = a day or two, L = several days, XL = a week or more), not
promises.

## What "feature complete" means

Feature complete is reached at the end of **Milestone 5**. It is the sum of everything asked
for so far, 2D and PNG/JPEG only:

- Explore a gallery of random flames, steered by randomness settings (allowed variations,
  transform counts, wildness, symmetry, palette style), and evolve any pick.
- Refine one flame with a live preview: shapes and variation parameters, gradient, tone, framing,
  with undo and redo.
- Render it at any size and format in the background, including huge images and a queue.
- **History:** every flame the user selects is stored and can be pulled back up.
- Save and open `.flame` files; import Apophysis and flam3 flames faithfully; every exported
  image carries its recipe so it can be reopened and re-rendered.
- Fast enough to feel instant: GPU previews and renders, with the CPU renderer as fallback.
- Other fractal generators plug in through the same abstraction, proven by at least one that
  is not a flame, plus user-defined variations.
- Polished and shippable on macOS (Apple Silicon) and Windows (x64).

Out of scope on purpose: animation, 3D, `.apo` import, the xaos (chaos) matrix editor,
palette extraction from images, a web build, Linux, and Intel Macs. They can be revisited after
1.0; Linux and Intel Mac support are on the post-1.0 list at the end of this document.

## Where we are

Done (commits `a8e6ae1` and `fd54727`, plus this plan and the README):

- [x] Rust workspace: `moth-core`, `moth-cli`, `moth-ui`.
- [x] `Generator` trait; flame genome, 26 variations (cross-checked against the Draves and
      Reckase paper, with a conformance test), chaos-game CPU renderer.
- [x] Oversampling, adaptive density filter, strip rendering for huge images with a streaming
      PNG writer, transparent background, 8/16-bit PNG and JPEG.
- [x] Randomizer and mutator with render-based camera framing and rejection of empty or blob
      flames.
- [x] `.flame` import and export (flam3 coefficient order; unsupported variations are reported);
      recipe embedded in PNG and JPEG.
- [x] CLI: `random`, `mutate`, `render`.
- [x] `README.md` with usage for the app and the command line; this plan.
- [x] App with three stages (Explore, Refine, Render): gallery, randomness drawer, evolve, live
      preview with pan and zoom, shape editing, basic gradient editor, tone and frame
      controls, undo and redo, export queue, `File` menu, self-test mode.

Known issues carried into the plan:

- CPU throughput is about 36M samples/s on a 4 performance + 6 efficiency core Mac. One
  benchmark ran about twice as fast as the same code in the CLI and was never explained.
- Faint blurry specks appear in dark regions of some flames.
- Brightness and gamma constants are our own, so a flame will not look identical to Apophysis.
- Only non-parametric variations exist; imported parametric variations are skipped (and
  reported).
- 17 style-level clippy warnings.
- Emoji in the UI are a fragile choice: some glyphs are missing from egui's built-in font.

## Principles

1. **Core stays UI-free.** Anything the app does must be reachable from `moth-core` and testable
   without a window.
2. **Faithful to the format.** A flame saved by Moth must open elsewhere, and flames from
   Apophysis and flam3 must open here and look the same. Where we deviate from the paper, we
   write it down.
3. **Clean room.** Work from the paper and the documented formats. Never copy Apophysis or
   flam3 source (both believed to be GPL; verify before relying on that).
4. **No silent changes to a recipe.** Widgets only change values the user changed. The
   self-test checks this.
5. **Everything long-running is a job.** Previews, exports and (later) batches all go through
   cancellable background jobs, so the UI never blocks and a queue is a thin layer.
6. **Measure, then optimize.** Performance work starts from a benchmark and ends with one.
7. **Two platforms from day one.** Every step must work on macOS (Apple Silicon) and Windows
   (x64), and CI enforces it. Platform-specific code (data folders, file managers, GPU backends,
   shortcut labels) is isolated and tested.

## Milestone 0: foundation (do first)

| # | Step | Size | Done when |
|---|------|------|-----------|
| 0.1 | ~~Choose the license.~~ **Decided: MIT** (2026-10-22); `LICENSE` added and every crate declares it. Still to do: verify the licenses of flam3 and Apophysis 7X so the clean-room rule rests on fact. | S | `LICENSE` committed (done); a note in `docs/` records what was checked about flam3 and Apophysis. |
| 0.2 | CI on GitHub Actions: `fmt --check`, `clippy -D warnings`, `test` on **macOS and Windows**. Fix the existing clippy warnings first. Where a runner has no GPU, GPU tests use a software adapter (WARP on Windows); checks that need real hardware are run locally and listed in the release checklist. | M | Green CI on both platforms for `main`. |
| 0.3 | Move the self-test into CI as a headless job where a display is available (or keep it local and document it). | S | `MOTH_SMOKE` result is part of the release checklist. |
| 0.4 | ~~`README.md`: what Moth is, how to build and run, the CLI, the file format.~~ **Done**: the README covers building, the app, the CLI, formats and rendering. Keep it current as each feature lands. | S | A new contributor can build and render a flame from the README alone (checked). |
| 0.5 | Replace emoji in the UI with an icon font or drawn icons, so no glyph can go missing. | S | No empty boxes anywhere; checked by screenshot in the self-test. |
| 0.6 | **GPU feasibility spike.** The GPU renderer is required for 1.0, so retire its biggest risks first. A throwaway wgpu compute program on Metal (Apple Silicon) and DX12 (Windows x64) that runs the chaos game for two or three variations into an atomic `u32` histogram. Measure samples per second against the CPU, atomic contention on bright pixels, and precision (f32 on the GPU against f64 on the CPU). | L | A short report in `docs/` with measured numbers per platform, the chosen histogram layout, and a go or adjust decision for steps 3.4 to 3.6. |
| 0.7 | **One definition per variation.** Today each variation is hand-written Rust. Before adding more (step 2.2) and parameters (step 2.1), choose a form that produces both the Rust and the WGSL from a single definition, such as a small DSL or a build-time generator, so the CPU and GPU cannot drift. Port the existing 26 to it. | L | The 26 variations come from one source; the paper-conformance test still passes; a GPU render of each variation matches the CPU within tolerance. |
| 0.8 | **Cross-platform audit.** Data folder (see step 1.1 for where each platform keeps it), file dialogs, reveal in the file manager (already done for both), shortcut labels (`⌘Z` on macOS, `Ctrl+Z` on Windows), fonts and HiDPI, and the self-test on Windows. | M | The self-test passes on both platforms; no macOS-only wording appears on Windows. |

## Milestone 1: persistence, history and favorites

Everything later (favorites, batch export, settings) builds on one local store, so it comes
before more features.

| # | Step | Size | Done when |
|---|------|------|-----------|
| 1.1 | **Library store** in `moth-core`: SQLite (via `rusqlite`, bundled) in the user data folder. On macOS that is the platform default (`~/Library/Application Support/Moth`). On Windows the build is portable, so data lives in a folder beside the exe (see decisions). Tables for flames (id, content hash, flame XML, name, created, last selected, parent id, favorite, note) and rendered files. Thumbnails are PNG files next to the database, keyed by id. | L | Unit tests cover insert, dedupe by content hash, lookup, delete and migration from version 1; the store survives being killed mid-write. |
| 1.2 | **Record selections.** Whenever a flame is selected in Explore (and whenever an edit is committed in Refine) it is stored. Identical flames are deduplicated by hash and just bump `last selected`; edits are saved as new entries with a `parent id`, so lineage is kept. | M | Selecting five flames and editing one creates the expected rows; selecting the same flame twice creates none. |
| 1.3 | **History tab.** A second top-level tab beside the stage stepper (`Create` and `History`). A grid of past flames, newest first, with thumbnails; actions: open in Refine, send to Explore as a new batch, favorite, delete, show lineage. Search by name or note; filter by favorites. | L | A flame selected yesterday can be found today and reopened exactly as it was; verified by closing and restarting the app in the self-test. |
| 1.4 | **Retention.** History is unlimited (decided 2026-10-22). A storage indicator shows how much space it uses. Whenever there are entries older than 30 days, the History tab shows a dismissible banner offering to clear them, with the count and size, and **Clear them**, **Keep** and **Remind me in 7 days**. Favorites are never removed. A manual "clear all (keep favorites)" is also available. | S | The offer appears only when old entries exist; clearing removes exactly the old, non-favorite entries and their thumbnails; favorites and recent entries are untouched (tested). |
| 1.5 | **Favorites** as a filter in History and a star on Explore cards. | S | Starred flames survive clearing history. |
| 1.6 | **Settings persistence**: randomness settings, last export options, window size, theme, last used folders. | M | Everything the user sets is restored on the next launch. |
| 1.7 | **Autosave and crash recovery** of the flame being refined. | M | Killing the app mid-edit and reopening offers the unsaved flame back. |
| 1.8 | **Rendered files** are recorded against the flame that produced them, with the settings used. | S | History shows what has been rendered from a flame and offers "Show". |

Decisions already taken: record on every selection (as requested), deduplicate to avoid
clutter. Open: retention defaults (see the end of this document).

## Milestone 2: a complete Refine and Explore

| # | Step | Size | Done when |
|---|------|------|-----------|
| 2.1 | **Parametric variations**, part one: extend the variation model so a variation can carry named parameters with ranges and defaults (the same schema later drives the UI sliders). Import and export them with flam3 attribute names. | L | A flame using `blob` or `pdj` round-trips through `.flame` with its parameters. |
| 2.2 | **All 49 variations from the paper**, then the most-used Apophysis additions (`julian`, `juliascope`, `ngon`, `curl`, `rectangles`, `radial_blur` and similar). Each is added with a conformance test against the paper's formula. | XL | The paper's catalog is complete; imports of common Apophysis flames report few or no skipped variations. |
| 2.3 | **Parameter editing in Refine** for parametric variations (sliders generated from the schema), plus reset-to-default. | M | Every parametric variation is editable and undoable. |
| 2.4 | **Symmetry** as described in the paper: rotational and dihedral, using the extra balanced transforms with color left unchanged. In the randomizer settings and as a Refine action ("make symmetric"). | M | A generated flame with order-3 symmetry has equal density in its three branches (tested). |
| 2.5 | **Randomness settings, completed**: palette style (random, analogous, complementary, mono, from a preset), per-variation likelihood, presets ("styles") the user can save and name. | M | A saved style regenerates a recognizably similar batch from a given seed. |
| 2.6 | **Seeds you can share**: show the seed, copy it, "generate from seed". | S | The same seed and settings reproduce the same batch on another machine. |
| 2.7 | **Evolve options**: drift only the shapes, only the colors, or both; evolve from a favorite in History. | S | Each option changes only what it says (tested). |
| 2.8 | **Gradient editor, finished**: preset gradients, import `.gradient`/`.ugr` files, drag to reposition stops. | M | A palette from an Apophysis `.ugr` imports unchanged. |
| 2.9 | **Fix the faint-speck artifact** with a density floor in the tone mapper. | S | Dark regions of the reference flames are clean without dimming real detail (before/after screenshots). |

## Milestone 3: fidelity and speed

| # | Step | Size | Done when |
|---|------|------|-----------|
| 3.1 | **Tone-mapping fidelity.** Study the paper's log-density, gamma and vibrancy description and flam3's documented parameter meanings; bring brightness, gamma, vibrancy and background in line. Keep our density filter ordering only if it measurably wins, and document any deviation. | L | Ten reference flames from Apophysis, rendered here and there, match closely by eye and by a numeric difference threshold. |
| 3.2 | **Golden-image tests** for the renderer using our own output as the baseline, to catch regressions. | M | A deliberate renderer change fails the suite and a refresh command updates it. |
| 3.3 | **CPU performance.** Build a benchmark suite first; resolve the unexplained two-times gap; look at SIMD and cheaper math for hot variations. | L | Documented samples per second on three reference flames, with a target set from the first measurement. |
| 3.4 | **GPU renderer (wgpu), stage one**: the chaos game in a compute shader, with variations generated from the same table as the CPU code so the two cannot drift; fixed-point atomic histogram. | XL | A flame renders on the GPU and matches the CPU render within a set tolerance. |
| 3.5 | **GPU stage two**: density filter and tone mapping on the GPU; progressive refinement in Refine (the image sharpens while idle); thumbnails rendered on the GPU. | XL | Explore thumbnails appear in well under a second; Refine edits feel real-time. |
| 3.6 | **GPU huge renders** by bands, bounded by the adapter's limits; automatic CPU fallback, with a setting to force either. | L | A poster-size render completes on the GPU without exceeding memory, and falls back cleanly on a machine without one. |
| 3.7 | **Cancel and progress** for every render job, with real percentages. | M | Cancelling an export stops it within a second and removes the partial file. |
| 3.8 | **Post-process slot** in the pipeline for optional denoise or upscale, empty by default. | S | A test post-process (for example a simple sharpen) can be added without touching the renderer. |

The GPU renderer is a **requirement for 1.0** (decided 2026-10-22). The spike in step 0.6 and
the single-source variations in step 0.7 come first so this milestone starts from measured
facts. The CPU renderer stays: it is the reference the GPU is tested against and the fallback for
machines without a usable adapter. Remaining risk: GPU atomics on the histogram are the known
bottleneck, and shader precision can change how a flame looks.

## Milestone 4: more generators and user-defined math

| # | Step | Size | Done when |
|---|------|------|-----------|
| 4.1 | **Make `Generator` dynamic.** An object-safe trait, a registry, a parameter schema (names, types, ranges, defaults) from which the UI builds its sliders, and a `generator` field in every save file and in the library. | L | The gallery, Refine, Render, History and the CLI work against the registry; the flame is just the first entry. |
| 4.2 | **Strange attractors** (Clifford, de Jong, Ikeda), reusing the density pipeline. | M | Random, evolve, save, history and export all work for attractors. |
| 4.3 | **Escape-time fractals** (Mandelbrot, Julia, Burning Ship, Newton), with their own simple pipeline and a coloring option. | L | Same, plus deep zoom with enough precision for the default views. |
| 4.4 | **Buddhabrot and Nebulabrot** on the density pipeline. | M | Same. |
| 4.5 | **Classic IFS** (Barnsley and friends) as presets of the flame generator, or as a thin generator of their own. | S | The classics render and evolve. |
| 4.6 | **User-defined variations** from a small expression language, compiled for both the CPU and the GPU, with hot reload and sliders from declared parameters. | XL | A variation written in the app appears in the gallery and in Refine without a rebuild and renders the same on CPU and GPU. |
| 4.7 | **Decide the plugin story** (WASM modules, shader snippets, or in-app expressions only) and document it. | S | A decision record in `docs/`. |

## Milestone 5: batch and workflow features (feature complete)

| # | Step | Size | Done when |
|---|------|------|-----------|
| 5.1 | **Render queue, completed**: reorder, cancel, retry, persist across restarts, pause. | M | Quitting with a queue and reopening resumes it. |
| 5.2 | **Batch export**: select several favorites or history entries and render them all at chosen settings. | M | Twenty favorites render unattended to a folder with sensible file names. |
| 5.3 | **Output presets** (named size, format and quality combinations) and file-name templates. | S | A preset applies in one click and survives restarts. |
| 5.4 | **Open from anywhere**: drag a `.flame`, PNG or JPEG onto the window; recent files; open from History. | S | Dropping a Moth PNG opens its flame in Refine. |
| 5.5 | **Export extras**: toggle embedding the recipe; copy the image to the clipboard; save only the `.flame`. | S | Each does what it says. |
| 5.6 | **Light and dark themes**, HiDPI correctness, a keyboard shortcut map and a help overlay. | M | Both themes pass the self-test screenshots with no clipped or unreadable text. |

**Feature complete is declared here.** Before moving on, run the full checklist at the end of
this document.

## Milestone 6: ship

| # | Step | Size | Done when |
|---|------|------|-----------|
| 6.1 | App icon (a moth), About dialog with third-party licenses. | S | Shown on all platforms. |
| 6.2 | Packaging for **macOS and Windows**: a macOS `.app` in a `.dmg`, and a Windows portable zip (the exe, with its data folder created beside it; no installer); a release workflow in CI that builds both from a tag. **Builds are unsigned for now** (decided 2026-10-22): the docs explain how to open an unsigned app on macOS (right-click Open, or clear the quarantine flag) and the SmartScreen prompt on Windows. Signing and notarization are deferred, and the workflow is kept ready to add them. | L | A tagged release produces downloadable, working artifacts for both platforms. |
| 6.3 | Documentation: user guide, file-format notes, architecture overview, contributing notes. | M | Each exists and links from the README. |
| 6.4 | Performance and memory pass on the slowest machine we can test; update the stated budgets. | M | Budgets met or documented. |
| 6.5 | Accessibility pass: keyboard-only use of every stage, contrast, readable text sizes. | M | Every stage can be driven from the keyboard. |
| 6.6 | Beta with real flames from a real Apophysis library; triage. | M | No known data-loss or corruption bugs open. |

## Testing strategy

- **Unit tests** in the core for every format, variation, store operation and renderer property
  (determinism, light conservation, strips equal full renders).
- **Conformance tests** for variations against the paper, added with every variation.
- **Golden images** for renderer regressions (step 3.2).
- **Self-test** (`MOTH_SMOKE=<dir>`) that drives every stage and checks behaviors such as undo
  restoring exactly. It captures only the app's own window. Extend it with each new feature,
  and with a restart check for history (step 1.3).
- **Cross-platform CI** from Milestone 0.

## Feature-complete checklist

Run through this before calling it done.

- [ ] Generate, evolve and pick; randomness settings steer results; seeds reproduce batches.
- [ ] Refine: edit shapes, variation parameters, gradient, tone and frame; undo and redo.
- [ ] Render any size to PNG (8/16-bit) or JPEG, transparent where supported, in bands for huge
      sizes, with progress and cancel; queue and batch work.
- [ ] History keeps every selected flame; favorites survive clearing; a flame can be reopened
      after a restart.
- [ ] `.flame` files and Moth images open; Apophysis and flam3 flames import and look right;
      every skipped item is reported.
- [ ] GPU previews and renders, CPU fallback, results agree.
- [ ] At least one non-flame generator, and user-defined variations.
- [ ] Both themes, HiDPI, keyboard use.
- [ ] CI green on macOS and Windows; self-test passes.

## Decisions

Taken (2026-10-22):

1. **License: MIT**, copyright Darrin Landroche.
2. **GPU renderer: required for 1.0.** The CPU renderer stays as the reference and fallback. A
   machine with no usable GPU still runs Moth on the CPU renderer, and says so.
3. **History retention: unlimited**, with an offer to clear entries older than 30 days (favorites
   are never cleared).
4. **Signing: none for now.** macOS and Windows builds are unsigned; revisit later.
5. **Platforms for 1.0: macOS on Apple Silicon and Windows on x64.** Intel Macs and Linux are
   post-1.0.
6. **User data.** macOS uses the platform default folder. Windows is portable: data lives in a
   folder beside the exe.
7. **Windows packaging: portable zip**, no installer.

Open: none for now.

## After 1.0 (todo)

- Linux builds (AppImage or similar), including the CI runner and the `Ctrl`/file-manager
  behavior on Linux.
- Intel Mac builds.
