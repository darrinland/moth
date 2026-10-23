# Clean-room record: flam3 and Apophysis licenses

Checked 2026-10-22 for plan step 0.1. Principle 3 in [PLAN.md](PLAN.md) says Moth is written from
the paper and the documented formats, and never copies Apophysis or flam3 source. This note
records what the licenses of those two projects say, so that rule rests on fact.

## What was checked

| Project | License found | Where it was checked |
|---------|---------------|----------------------|
| flam3 | GNU GPL, version 3 (the file opens "Version 3, 29 June 2007") | `COPYING` in [scottdraves/flam3](https://github.com/scottdraves/flam3) on the `master` branch |
| Apophysis 7X | GNU GPL, version 2 or any later version | License header in `Variations/varPreSpherical.pas` of the [apophysis7x mirror](https://git.speice.io/mirror-github-bspeice/apophysis7x) |
| Apophysis (original) | GPL, per Wikipedia | [Apophysis (software)](https://en.wikipedia.org/wiki/Apophysis_(software)); not checked against source |

Notes on the checks:

- The flam3 `COPYING` file was read directly. Older distribution packages list GPL v2 or later
  instead, so the license of any specific release should be checked when it is used.
- The Apophysis 7X header was read from a third-party mirror, not the original SourceForge
  repository. The SourceForge project page lists GPL v2.0. The original SourceForge repository was
  not checked for activity.

## Consequence for Moth

- Both projects are GPL. Moth is MIT (see [LICENSE](../LICENSE)). MIT and GPL code cannot be mixed
  in one distributed work without the GPL applying to the whole, so no GPL code may enter this
  repository.
- Variations, file formats and the flame model are described by Draves and Reckase's paper and by
  the documented `.flame` format. Those are the sources to implement from. A formula or a file
  format is not covered by copyright; the source code that expresses it is.
- Before any port or reuse of code from flam3 or Apophysis, stop and ask. It would change the
  project's license.
- Principle 3 stands unchanged.
