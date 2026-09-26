# Releasing

`.github/workflows/release.yml` runs three jobs in order. Each needs the one before it.

| Job       | Does                                                                   |
|-----------|------------------------------------------------------------------------|
| `guard`   | `cargo fmt --check`, `clippy -D warnings`, `cargo test`, then `deplyd check` on the built binary |
| `build`   | Five targets, each archived and uploaded as an artifact                |
| `publish` | Checksums, Sigstore attestation, creates the release                   |

`publish` is gated on `if: github.event_name == 'push'`. A `workflow_dispatch` is a
rehearsal: it stops after `build` and publishes nothing.

The guard runs again in CI on purpose. The attestation says a binary came from a
commit; the guard job is what makes that worth anything, by recording what was true of
that commit before it was signed.

## Steps

Run the same checks the guard will, so a push does not fail on something local would
have caught:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets
```

Bump `version` in the workspace `Cargo.toml`, commit, and push `main`. Then rehearse:

```bash
gh workflow run release.yml -f version=v0.2.0
gh run watch <id>
```

Green, so tag it. The tag is what publishes:

```bash
git tag -a v0.2.0 -m "deplyd 0.2.0"
git push origin v0.2.0
gh run watch <id>
```

Then check what came out, and install it the way anyone else would:

```bash
gh release view v0.2.0
gh attestation verify <file> --repo BertilVossebelt/Deplyd
```

## When it fails

A tag that published nothing has to go before you can reuse it:

```bash
gh release delete v0.2.0 --yes    # only if a release object was created
git push origin :v0.2.0
git tag -d v0.2.0
```

Fix, tag, push again.

## Notes

`aarch64-unknown-linux-gnu` is `optional: true`. If arm64 runners are unavailable to
the account, the release goes out with four binaries rather than not at all.

The action versions are pinned and behind. Bump them on their own, never alongside a
release: a rehearsal exercises `upload-artifact`, but `download-artifact`, the
attestation and `gh release create` only ever run on a real tag, so that is the one
part no rehearsal can prove.
