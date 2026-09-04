# Blitz patch queue

Flectar Mail builds two locally patched crates from Blitz. The checked-in
sources under `crates/blitz-dom` and `crates/blitz-paint` are generated from the
official crates.io archives identified by `upstream.toml`, followed by the
patches listed in `series`.

Normal Cargo builds do not download or apply patches. Both the root workspace
and the standalone Android workspace use the checked-in sources through
`[patch.crates-io]`. Patch reconstruction happens only during verification or
an intentional upgrade.

## Current patches

| Patch | Purpose | Upstream status | Removal condition |
| --- | --- | --- | --- |
| `0001-bound-document-event-queue.patch` | Bounds resource-completion buffers while a document is not being polled. | Not submitted | Blitz exposes equivalent bounded/configurable delivery. |
| `0002-email-table-layout-and-borders.patch` | Preserves anonymous or blockified cells and prevents latent collapsed borders from creating gaps or black grids. | Not submitted | Released Blitz passes the corresponding renderer tests without the patch. |
| `0003-inline-ancestor-backgrounds.patch` | Finds painted backgrounds on inline ancestors so CTA labels and similar content remain visible. | Not submitted | Released Blitz paints the same fragments correctly. |
| `0004-repository-integration.patch` | Records the intentional Apple dependency for cargo-machete and removes one unused direct paint dependency. | Local only | The upstream manifests no longer need these adjustments. |

The application-level regression coverage is in `src/renderer.rs`, including
the collapsed-table, anonymous-cell, CTA-background, selection, and resource
loading cases.

## Verify the checked-in sources

Run:

```bash
scripts/blitz-patches.sh verify
```

The command downloads the pinned official archives, verifies their crates.io
checksums and source revision, applies every patch in a temporary directory,
and compares the result byte-for-byte with both checked-in crates. CI runs the
same command, so an undocumented edit to vendored Blitz code fails clearly.

## Prepare an upstream update

To inspect a newer published version without modifying the repository:

```bash
scripts/blitz-patches.sh stage NEW_VERSION
```

The command obtains release checksums from the official sparse crates.io index
and leaves two trees in a temporary directory:

- `upstream/crates`: untouched release sources.
- `patched/crates`: the result of applying the current queue.

If a patch conflicts, clean hunks are applied and `.rej` files are left in the
patched tree. Resolve and test there before changing the repository.

Promote an update in its own pull request:

1. Rebase each patch conceptually against the pristine staged tree; do not
   combine unrelated patches or include formatting-only changes.
2. Replace the two checked-in crate trees with the fully patched result.
3. Update `upstream.toml`, all four exact Blitz versions in the root manifest,
   and both Cargo lockfiles.
4. Regenerate only the patch files whose upstream context changed.
5. Run `scripts/blitz-patches.sh verify`.
6. Run the default, capability-minimal, GPU, Android, and renderer regression
   checks documented in `.github/workflows`.

When an upstream release contains a fix, first prove that the related Flectar
regression passes without its patch, then remove the patch from `series` and
delete the file. A temporary GitHub fork may be used to submit an upstream pull
request, but Flectar builds do not depend on a long-lived fork or moving Git
branch.
