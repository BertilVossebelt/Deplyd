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

```
$ deplyd pr 412
Inspecting production deploys...

production  ·  2 targets

  API  services/api
  ✓ DEPLYD     a1b2c3d  2026-05-14 11:02  Merge pull request #418 from BertilVossebelt/release
    run        10234567890

  WEB  services/web
  ! UNCERTAIN  4d5e6f7  2026-05-12 16:41  Merge pull request #401 from BertilVossebelt/release
    because    run 10234599887 failed
    run        10221004455
    skipped    Deploy web bundle

PR #412  feat(billing): add invoice export endpoint
  merge      9f8e7d6

  API       ✓ DEPLYD       in a1b2c3d
  WEB       ✗ NOT DEPLYD   deployed 4d5e6f7
```

Commit SHAs and run numbers are links where the terminal supports them.

## Install

One command. It installs the GitHub CLI if you have not got it, checks the download
against the published checksum and signature, installs deplyd and `dp`, puts them on
your PATH, signs you in, and turns on tab completion. A download that does not check
out is not installed, and checking it needs no account and no token.

**macOS and Linux**

```bash
curl -fsSL https://raw.githubusercontent.com/BertilVossebelt/Deplyd/main/install.sh | sh
```

**Windows**

```powershell
irm https://raw.githubusercontent.com/BertilVossebelt/Deplyd/main/install.ps1 | iex
```

**From source**, needing Rust 1.98 or newer

```bash
git clone https://github.com/BertilVossebelt/Deplyd.git
cd deplyd && cargo install --path crates/deplyd
```

Then open a new terminal, and from inside any repo:

```bash
dp status
```

## Uninstall

The installer takes back what it put there, and nothing else.

**macOS and Linux**

```bash
curl -fsSL https://raw.githubusercontent.com/BertilVossebelt/Deplyd/main/install.sh | sh -s -- --uninstall
```

**Windows**, where `iex` cannot pass a switch, so the script becomes a block first

```powershell
& ([scriptblock]::Create((irm https://raw.githubusercontent.com/BertilVossebelt/Deplyd/main/install.ps1))) -Uninstall
```

That removes the two binaries, the PATH entry and the completion block. It asks
before removing the GitHub CLI, which was probably here first, and leaves your
sign-in and any `.deplyd.json` where they are. Remembered defaults and the cache stay
too: add `--purge`, or `-Purge`, to take those as well.

`dp uninstall` prints the right line for the machine you are on. Printing it is all
it does - deplyd does not delete, which is the point of `dp check`.

## Setup

The installer does all of this. You only need it if you built from source, or said no
to something.

| | |
|---------------|--------------------------------------------------------------------|
| Sign in       | `gh auth login` |
| Completion    | `deplyd completions powershell >> $PROFILE.CurrentUserAllHosts`, or bash, zsh, fish, elvish |
| Short name    | `dp` is a link to `deplyd`, made next to it |

Signing in is a device flow against access you already have. deplyd never asks you to
create a personal access token: on an organisation repository that can need an owner's
approval, and needing permission from someone is the thing this tool exists to avoid.
In CI, `GITHUB_TOKEN` is used instead, so there is no login step there.

Every release is signed and recorded in a public transparency log. Each one also
publishes the signature as `attestation.json`, which checks without signing in:

```bash
gh attestation verify deplyd --repo BertilVossebelt/Deplyd --bundle attestation.json
```

## Usage

Run it from inside any repo.

| Command                 |                                                      |
|-------------------------|------------------------------------------------------|
| `deplyd status`         | the last deployed commit, and your changes in it     |
| `deplyd pr 412`         | is that pull request live?                           |
| `deplyd commit a1b2c3d` | is that commit live? takes any ref, including `HEAD` |
| `deplyd environments`   | environments that `-E` accepts                       |
| `deplyd authors`        | names that `-A` accepts                              |
| `deplyd config`         | what detection concluded about this repo             |
| `deplyd init`           | write that conclusion to a file you can correct      |
| `deplyd remember`       | keep a default author, environment or repo           |
| `deplyd completions`    | shell completion scripts                             |
| `deplyd check`          | prove it can only read                               |

`dp` is the same binary under a shorter name. Commands shorten too, while they stay
unambiguous, so `dp env` and `dp auth` work.

| Option                    |                                                |
|---------------------------|------------------------------------------------|
| `-E, --environment <env>` | environment, or a prefix: `-E prod`, `-E stag` |
| `-A, --author <name>`     | author, default `git config user.name`         |
| `-T, --take <n>`          | how many changes to list, default 10           |
| `-S, --skip <n>`          | skip this many, for paging                     |
| `--repo-path <path>`      | repo to inspect, default the current directory |
| `-J, --json`              | machine-readable output, for `status` and `pr` |
| `-F, --force`             | let `init` rewrite a file that already exists  |

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

| Exit |                                                            |
|------|------------------------------------------------------------|
| `0`  | deplyd, on evidence with nothing shaky about it            |
| `2`  | not deplyd, including a change no target covers            |
| `3`  | reverted                                                   |
| `4`  | not merged                                                 |
| `5`  | no such pull request, or no access to it                   |
| `6`  | deplyd, but a target is `UNCERTAIN`: read the report first |
| `1`  | deplyd could not run                                       |

`0` is the only code meaning "in what shipped, and the reading is sound". `6` exists so
a gate is never told *shipped* on evidence deplyd has itself questioned: the change is
in the deployed commit, but a newer deploy did not complete, or the commit could not be
read reliably. Treat `6` as "look before shipping", or accept only `0` to be strict.

`deplyd status --json` lists every target and change, and exits `0` whenever it could
answer at all.

## What it reports

Per target:

|             |                                                                           |
|-------------|---------------------------------------------------------------------------|
| `DEPLYD`    | built and released, with no newer deploy failing or in flight             |
| `UNCERTAIN` | a newer deploy did not complete, or the commit could not be read reliably |
| `because`   | what made it uncertain: the run that failed, or the reading that did not hold |
| `skipped`   | steps the run skipped. Changes to those are not live                     |

Per change:

|                                |                                                       |
|--------------------------------|-------------------------------------------------------|
| `DEPLYD in <sha>`              | it is in the deployed commit                          |
| `DEPLYD in <sha> as a copy`    | the commit is absent but the same change is there, cherry-picked or rebased |
| `REVERTED undone before <sha>` | it shipped, then was undone before the deployed commit |
| `NOT DEPLYD deployed <sha>`    | neither the commit nor an equivalent change is there  |
| `NOT MERGED`                   | still open, or closed without merging                 |
| `NOT COVERED`                  | it changed no path any target covers. Lists every target and what it covers, since a wrong scope looks identical to an unrelated change |

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

A file that will not parse stops the run and gets named. Carrying on without it would
silently drop the corrections it exists to hold.

### What detection looks for

Useful for reading `deplyd config`, and for telling which key to set. None of it is a
requirement.

| It is treated as   | When                                                           |
|--------------------|----------------------------------------------------------------|
| a deploy workflow  | the filename or `name:` contains `deploy`, `release`, `publish`, `ship` or `cd`; or a job declares an `environment:`; or a step uses a known deploy action or runs an applying command |
| an environment     | the name appears in the filename, in a job's `environment:`, or in a `workflow_dispatch` choice input |
| a target           | a job that is not plumbing, with at least three steps. Names containing `merge`, `notify`, `lint`, `test`, `setup` and similar are skipped |
| that target's scope | the job's `defaults.run.working-directory`; or a directory all its steps agree on; or the workflow's own `paths:` trigger filter |

Those words match whole words only, so a job called `deploy-latest` is not read as a
test job.

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

```
$ deplyd check

deplyd read-only self-check

  git verbs           PASS  13 allowed, none of them write: rev-parse, rev-list, log, show, merge-base, shortlog, cat-file, diff, status, cherry, ls-files, config, fetch
  write refusals      PASS  8 of 8 refused
  reads still work    PASS  5 of 5 allowed
  github routes       PASS  6 routes, all GET, all inside the repo

  writes on disk      only inside ~/.config/deplyd
                      remembered defaults, and what deplyd init scaffolds
  the one git write   fetch, which updates your own remote-tracking refs
                      nothing is sent, and a fetch cannot change a remote

  These ran just now, against the code compiled into this binary,
  not against source sitting beside it.
```

The one write against your repository is `git fetch`, which updates your own
remote-tracking refs. Nothing is sent to GitHub. Everything else deplyd writes goes in
its own directory, which `deplyd check` prints: remembered defaults, what `init`
scaffolds, and a cache of runs that have already finished.

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
- On Windows PowerShell 5.1, completion after a single `-` does not fire: that shell
  never calls a native completer for one. `--` completes normally.

## Development

### Running:

To try a change the way someone else would meet it, dot-source the dev installer. It
builds the tree and puts that build on PATH for the current terminal, with completion,
writing nothing to your profile or rc file. Close the terminal and nothing of it is
left.

**macOS and Linux**

```bash
. ./dev-install.sh
```

**Windows**

```powershell
. .\dev-install.ps1
```

The dot matters: a script cannot change the PATH of the shell that ran it.

Each takes its own spelling of the same three options, one dash in PowerShell and two
in sh:

|                                               | sh          | PowerShell |
|-----------------------------------------------|-------------|------------|
| Build with the release profile                | `--release` | `-Release` |
| Install it for real, to work on the installer | `--persist` | `-Persist` |
| Undo a `--persist`                            | `--revert`  | `-Revert`  |

A persistent one is what `install.sh --uninstall` and `install.ps1 -Uninstall` need
something to remove. Neither dev installer is the installer: that one only ever takes
a published release, and refuses anything it cannot verify.

### Testing:

Run the tests with:
```bash
cargo test
```

No GitHub account or network needed: fixture repositories are built with
`GIT_ALLOW_PROTOCOL=file`, so git itself refuses ssh and https.

### Releasing:

Cutting a release is in [RELEASING.md](RELEASING.md).

## Licence

MIT, copyright Bertil Vossebelt.

Provided as-is, with no warranty and no obligation on me to maintain or support it.

It is built to read only, and proves that about itself on every run. That is a guard,
not a guarantee. Deciding whether it is safe to point at your repositories is your call,
and whatever happens as a result is your responsibility. Read the source, run the tests,
fork it and change it, or do not use it.
