# Releasing

`version` in the workspace `Cargo.toml` is the switch. Change it in a pull request,
merge, and the release goes out. Merge anything else and nothing happens.

```diff
-version = "0.2.1"
+version = "0.2.2"
```

There is nothing else to do. No tag to push, no workflow to start, no release to
write. Deciding the number is the only judgement involved.

## What runs

`.github/workflows/release.yml`, on every push to `main`. Each job needs the one
before it.

| Job       | Does                                                                   |
|-----------|------------------------------------------------------------------------|
| `decide`  | Works out the version, and whether it has been released already        |
| `guard`   | `cargo fmt --check`, `clippy -D warnings`, `cargo test`, then `deplyd check` on the built binary |
| `build`   | Five targets, each archived and uploaded as an artifact                |
| `publish` | Checksums, Sigstore attestation, checks it verifies, creates the tag and the release |

`decide` compares the version against the releases that exist. A merge that left it
alone stops there, because CI has already run on that commit and there is nothing to
publish.

The guard runs again even though CI ran on the pull request. The attestation says a
binary came from a commit; the guard job is what makes that worth anything, by
recording what was true of that commit before it was signed.

## Rehearsing

Builds every target and publishes nothing:

```bash
gh workflow run release.yml -f version=v0.2.2
gh run watch <id>
```

Worth doing after changing the workflow itself, or the pinned action versions. It
cannot prove `download-artifact`, the attestation or `gh release create`, which only
run when something is actually published.

## Prereleases

A version with a suffix is published as a prerelease, so it does not become what
"latest release" means and the installers keep ignoring it:

```
version = "0.3.0-rc.1"
```

Cargo accepts that, and so does this.

## By hand

Pushing a tag still works, and skips the version check:

```bash
git tag -a v0.2.2 -m "deplyd 0.2.2"
git push origin v0.2.2
```

## Checking what came out

```bash
gh release view v0.2.2
gh attestation verify <file> --repo BertilVossebelt/Deplyd --bundle attestation.json
```

`--bundle` is the check the installer runs: it reads `attestation.json`, published
with the release, rather than asking GitHub for it, so it needs no account and no
token. Leave the flag off to check against the API instead, which wants a sign-in.
`publish` runs the same check over every archive before the release is created.

## When it fails

Nothing is published unless `guard` and all five builds pass, so a failure normally
leaves nothing behind: fix it and merge again. If it failed after the release was
created, remove it and the tag before reusing the version:

```bash
gh release delete v0.2.2 --cleanup-tag --yes
```

## Notes

`aarch64-unknown-linux-gnu` is `optional: true`. If arm64 runners are unavailable to
the account, the release goes out with four binaries rather than not at all.

Bump the pinned action versions on their own, never in the same change as a release.
A rehearsal exercises `upload-artifact`, but `download-artifact`, the attestation and
`gh release create` only ever run on a real publish, so that is the part no rehearsal
can prove.
