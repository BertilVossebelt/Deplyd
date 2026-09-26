# Deplyd

_Is my pull request deployed?_

Deplyd answers that for any repo that deploys through GitHub Actions. It names the
commit each environment was last deployed from, and tells you which of your changes are
in it.

GitHub's deployments page shows one row per deploy run with one commit title. It never
shows which commits went out, so answering this by hand means opening runs, reading
logs and comparing SHAs. This does that for you.

It was built for the case where you cannot change anything: no access to the servers,
no authority over the deploy workflows, no pipeline step you are allowed to add.
Everything it reports is recovered from what GitHub already exposes. It installs nothing
into your repository and only ever reads it.

```powershell
$ deplyd pr 412
Inspecting production deploys...

API (production)
  a1b2c3d4e  2026-05-14 11:02:37 +0200  Merge pull request #418 from acme/release
  run 10234567890 - https://github.com/acme/widgets/actions/runs/10234567890
  scope services/api
  DEPLYD: built and released, no newer deploy failing or in flight

WEB (production)
  4d5e6f7a8  2026-05-12 16:41:09 +0200  Merge pull request #401 from acme/release
  run 10221004455 - https://github.com/acme/widgets/actions/runs/10221004455
  scope services/web
  SKIPPED (1) - changes to these are not deployd:
    Deploy web bundle
  UNCERTAIN: a newer deploy did not complete
    run 10234599887  failed

PR #412  feat(billing): add invoice export endpoint
merge commit 9f8e7d6c5
  API       DEPLYD in a1b2c3d4e
  WEB       NOT DEPLYD - deployed commit is 4d5e6f7a8
```

## Install

**macOS and Linux**

```bash
curl -fsSL https://raw.githubusercontent.com/BertilVossebelt/deplyd/main/install.sh | sh
```

**Windows**

```powershell
irm https://raw.githubusercontent.com/BertilVossebelt/deplyd/main/install.ps1 | iex
```

The installer checks the download against the published checksums, verifies its
provenance, and puts it on your PATH. Nothing needs root or administrator rights.

Or take a binary from [releases](https://github.com/BertilVossebelt/deplyd/releases)
and put it on your PATH yourself. There is no runtime to install either way.

You also need the [GitHub CLI](https://cli.github.com/), signed in:

```bash
gh auth login
```

That is a device flow against access you already have. deplyd deliberately never asks
you to create a personal access token: on an organisation repository that can need an
owner's approval, and needing permission from someone is the thing this tool exists to
avoid. In CI, `GITHUB_TOKEN` is used instead, so no login step is needed there.

Optional, once:

```bash
deplyd completions bash >> ~/.bashrc     # or zsh, fish, powershell, elvish
```

Every release is signed and recorded in a public transparency log, so a download can
be checked against the repository it claims to come from:

```bash
gh attestation verify deplyd --repo BertilVossebelt/deplyd
```

Building from source needs Rust 1.98 or newer:

```bash
git clone https://github.com/BertilVossebelt/deplyd.git
cd deplyd && cargo install --path crates/deplyd
```

## Usage

Run it from inside any repo.

| Command                 |                                                       |
|-------------------------|-------------------------------------------------------|
| `deplyd status`         | the last deployed commit, and your changes in it      |
| `deplyd pr 412`         | is that pull request live?                            |
| `deplyd commit a1b2c3d` | is that commit live? takes any ref, including `HEAD`  |
| `deplyd environments`   | environments that `-E` accepts                        |
| `deplyd authors`        | names that `-A` accepts                               |
| `deplyd config`         | what detection concluded about this repo              |
| `deplyd init`           | write that conclusion to a file you can correct       |
| `deplyd check`          | prove it can only read                                |

Commands shorten while they stay unambiguous, so `deplyd env` and `deplyd auth` work,
and tab completion fills in environments and authors from what the repo actually has.

| Option              |                                                  |
|---------------------|--------------------------------------------------|
| `-E <env>`          | environment, or a prefix: `-E prod`, `-E stag`   |
| `-A <name>`         | author, default `git config user.name`           |
| `-T <n>` / `-S <n>` | how many changes to list, and how many to skip   |
| `--repo-path <p>`   | repo to inspect, default the current directory   |
| `--json`            | machine-readable output, for the verdicts        |
| `--force`           | let `init` rewrite a file that already exists    |

Defaults can be kept:

```bash
deplyd remember author "Ada"
deplyd remember environment staging
deplyd remember repo /path/to/repo
```

## In a script

`--json` prints one document and nothing else. `pr` and `commit` exit on their verdict,
so a release gate is one line:

```bash
deplyd pr 412 -E production --json > verdict.json || echo "do not ship"
```

| Exit |                                                              |
|------|--------------------------------------------------------------|
| `0`  | deployd, on evidence with nothing shaky about it             |
| `2`  | not deployd, including a change no target covers             |
| `3`  | reverted                                                     |
| `4`  | not merged                                                   |
| `5`  | no such pull request, or no access to it                     |
| `6`  | deployd, but a target is `UNCERTAIN`: read the report first  |
| `1`  | deplyd could not run                                         |

`0` is the only code meaning "in what shipped, and the reading is sound". `6` exists so
a gate is never told *shipped* on evidence deplyd has itself questioned: the change is
in the deployed commit, but a newer deploy did not complete, or the commit could not be
read reliably. Treat `6` as "look before shipping", or accept only `0` to be strict.

`deplyd status --json` lists every target and change, and exits `0` whenever it could
answer at all.

## What it reports

Per target:

|             |                                                                          |
|-------------|--------------------------------------------------------------------------|
| `DEPLYD`    | built and released, with no newer deploy failing or in flight            |
| `UNCERTAIN` | a newer deploy did not complete, or the commit could not be read reliably |
| `SKIPPED`   | steps the run skipped. Changes to those are not live                    |

Per change:

|                              |                                                       |
|------------------------------|-------------------------------------------------------|
| `DEPLYD in <sha>`            | it is in the deployed commit                          |
| `DEPLYD in <sha> as <other>` | the commit is absent but the same change is there, cherry-picked or rebased |
| `REVERTED`                   | it shipped, then was undone before the deployed commit |
| `NOT DEPLYD`                 | neither the commit nor an equivalent change is there   |
| `NOT MERGED`                 | still open, or closed without merging                  |
| `NOT COVERED`                | it changed no path any target covers. Lists every target and what it covers, since a wrong scope looks identical to an unrelated change |

`DEPLYD` means the code was built and the deploy ran to completion. Nothing outside the
server can prove the process actually cycled, which is why it is not `VERIFIED`.

There are two records of what a run deployed: the commit `checkout` resolved in the log,
and the commit GitHub's deployment record was created for. Where both are in hand they
are compared, and a disagreement is reported rather than resolved. That usually means the
branch moved mid-deploy. The deployment record also outlives the log, which GitHub
deletes after the retention period.

## Adapting it to your repo

GitHub Actions is flexible enough that detection will not always land. Run
`deplyd config` to see what it concluded, and `deplyd init` to write that conclusion to
a file you can correct by hand rather than author from nothing. Every key is optional.

The file stays out of your working tree. To share the fixes, move it to the repo root as
`.deplyd.json` and commit it: deplyd reads that in preference when it is there.

```json
{
  "deployPattern": "deploy|release|shipit",
  "environments": {
    "production": { "workflows": ["shipit.yml"] },
    "staging": {}
  },
  "ignoreJobs": ["merge", "notify", "smoke"],
  "scopes": {
    "API": ["services/api"],
    "WEB": ["services/web"]
  }
}
```

| Key                             | Fixes                                               |
|---------------------------------|-----------------------------------------------------|
| `deployPattern`                 | a deploy workflow not being found, or a non-deploy one being treated as one |
| `environments`                  | the wrong environment list. Your names replace the detected ones |
| `environments.<name>.workflows` | the wrong workflows for an environment. An explicit list always wins |
| `ignoreJobs`                    | a job showing up as a target that should not        |
| `scopes`                        | which paths a target covers. Keyed by the label deplyd reports |

### What detection looks for

Useful for reading `deplyd config`, and for telling which key to set. None of it is a
requirement.

| It is treated as   | When                                                           |
|--------------------|----------------------------------------------------------------|
| a deploy workflow  | the filename or `name:` contains `deploy`, `release`, `publish`, `ship` or `cd`; or a job declares an `environment:`; or a step uses a known deploy action or runs an applying command |
| an environment     | the name appears in the filename, in a job's `environment:`, or in a `workflow_dispatch` choice input |
| a target           | a job that is not plumbing, with at least three steps. Names containing `merge`, `notify`, `lint`, `test`, `setup` and similar are skipped |
| that target's scope | the job's `defaults.run.working-directory`; or a directory all its steps agree on; or the workflow's own `paths:` trigger filter |

A job's name becomes its label, so `deploy-api` reports as `API`.

Environments are optional: a repo with one `deploy.yml` and none at all works. Where one
workflow deploys to staging and then production, the jobs are told apart by their
`environment:` declarations; without those, a run cannot be attributed to one.

## Read-only

deplyd cannot write to your repository and does not ask you to take that on trust.

The dangerous operations do not exist rather than being refused: every git call names a
verb from a fixed set with no `push`, no `reset` and no `checkout`, and every GitHub
request is one of six routes with no method to set. Arguments that would write are
refused before anything runs. And because a compiled binary cannot audit the source it
came from, every invocation exercises its own refusal paths first. A build whose guard
has been weakened refuses to read a repository at all.

```powershell
$ deplyd check
  git verbs           PASS  13 allowed, none of them write
  write refusals      PASS  8 of 8 refused
  reads still work    PASS  5 of 5 allowed
  github routes       PASS  6 routes, all GET, all inside the repo
```

The one write against your repository is `git fetch`, which updates your own
remote-tracking refs. Nothing is sent to GitHub. Everything else deplyd writes goes in
its own directory, which `deplyd check` prints.

This stops accidents, not someone determined to get around them. It also says nothing
about dependencies, which run with the same permissions deplyd does.

## Limitations

- Deploys outside GitHub Actions are invisible. There is no run to read.
- Workflows that deploy a branch input or call another workflow need the run log, and
  GitHub deletes logs after the retention period. Past that, deplyd falls back to the
  deployment record and marks the target `UNCERTAIN`.
- One workflow serving several environments needs its jobs to declare `environment:`,
  because the API does not expose `workflow_dispatch` inputs.
- Only the newest fifteen runs per workflow are looked at, and the walk stops after
  three in a row reveal no new target. The output says when that happens.
- Each leg of a matrix job becomes its own target, labelled by its matrix values.
- A checkout inside a composite action is assumed to take the run's own ref.
- Workflow YAML is read by a parser written for the subset workflows use. Anything else
  is refused by name and line number rather than guessed at, and `deplyd config` names
  any workflow it could not read.
- `DEPLYD` means the commit shipped. A later commit rewriting the same lines is listed
  underneath rather than judged.

## Development

```bash
cargo test
```

No GitHub account or network needed: fixture repositories are built with
`GIT_ALLOW_PROTOCOL=file`, so git itself refuses ssh and https.

## Licence

MIT, copyright Bertil Vossebelt.

Provided as-is, with no warranty and no obligation on me to maintain or support it.

It is built to read only, and proves that about itself on every run. That is a guard,
not a guarantee. Deciding whether it is safe to point at your repositories is your call,
and whatever happens as a result is your responsibility. Read the source, run the tests,
fork it and change it, or do not use it.
