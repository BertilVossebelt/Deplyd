# Deplyd
_Pull request deployment checker for GitHub Actions written for Powershell._

Deplyd tells you whether your pull request is live. Run it in any repo that deploys
through GitHub Actions: it names the commit each environment was last deployed from, and
tells you which of your changes are in it.

GitHub's deployments page shows one row per deploy run with one commit title. It never
shows which commits went out, so "is my PR live?" means opening runs, reading logs and
comparing SHAs by hand. This does that for you.

It was built for the case where you cannot change anything: no access to the servers, no
authority over the deploy workflows, no pipeline step you are allowed to add. Everything
it reports is recovered from outside, from what GitHub already exposes. It installs
nothing into your repository, changes nothing in it, and only ever reads it. Its own
files, remembered defaults and whatever `deplyd init` writes, stay in deplyd's own
directory.

```powershell
$ deplyd pr 412
Inspecting production deploys...
  reading log for run 10234567890...
  reading log for run 10221004455...

API (production)
  a1b2c3d4e  2025-05-14 11:02:37 +0200  Merge pull request #418 from acme/release
  run 10234567890 - https://github.com/acme/widgets/actions/runs/10234567890
  scope services/api
  PUBLISHED: built and released, no newer deploy failing or in flight

WEB (production)
  4d5e6f7a8  2025-05-12 16:41:09 +0200  Merge pull request #401 from acme/release
  run 10221004455 - https://github.com/acme/widgets/actions/runs/10221004455
  scope services/web
  SKIPPED (1): Deploy web bundle
  UNCERTAIN: a newer deploy (run 10234599887) failed

PR #412  feat(billing): add invoice export endpoint
merge commit 9f8e7d6c5
  API       LIVE in a1b2c3d4e
  WEB       NOT LIVE - deployed commit is 4d5e6f7a8
```

## Install

The installer adds a `deplyd` function to your PowerShell profile so you can run it from
anywhere, with tab completion for the commands and `dp` as a short form, and
`uninstall.ps1` takes it out again. It also looks for the
[GitHub CLI](https://cli.github.com/), which deplyd uses to read from GitHub, offers to
install it if it is missing, and offers to sign you in if you are not. Skip that and
the first `deplyd status` stops with the same instruction, `gh auth login`, rather
than reporting on nothing.

**Windows**

```powershell
git clone https://github.com/BertilVossebelt/deplyd.git
cd deplyd
. .\install.ps1
```

**macOS**

PowerShell 7 first, if you do not have it:

```bash
brew install --cask powershell
```

```bash
git clone https://github.com/BertilVossebelt/deplyd.git
cd deplyd
pwsh
```

Then, inside PowerShell:

```powershell
. ./install.ps1
```

**Linux**

PowerShell 7 first. Installing it is per-distro rather than one command: see
[Microsoft's install
instructions](https://learn.microsoft.com/powershell/scripting/install/installing-powershell-on-linux).

```bash
git clone https://github.com/BertilVossebelt/deplyd.git
cd deplyd
pwsh
```

Then, inside PowerShell:

```powershell
. ./install.ps1
```

### Verification

```powershell
./tests/run-tests.ps1
```

No GitHub account or network needed for that.

## Usage

Run it from inside any repo.

| Command               |                                                             |
|-----------------------|-------------------------------------------------------------|
| `deplyd status`       | the last deployed commit, and your changes in it            |
| `deplyd pr 412`       | is that pull request live?                                  |
| `deplyd authors`      | names that `-A` accepts                                     |
| `deplyd environments` | environments that `-E` accepts                              |
| `deplyd config`       | what detection concluded about this repo                    |
| `deplyd init`         | write that conclusion to a settings file, to correct by hand |
| `deplyd check`        | prove it can only read                                      |
| `deplyd help`         | all of the above, and what you get from `deplyd` on its own |

Commands can be shortened while they stay unambiguous, so `deplyd env` and `deplyd auth`
work. Tab completes the commands, and the values of `-E` and `-A` from what the repo
actually has: `-E pr<tab>` gives `production`, `-A Ada<tab>` gives `Ada Lovelace`.

| Option              |                                                       |
|---------------------|-------------------------------------------------------|
| `-E <env>`          | environment, or a prefix of one: `-E prod`, `-E stag` |
| `-A <name>`         | author to filter on, default `git config user.name`   |
| `-T <n>` / `-S <n>` | how many changes to list, and how many to skip        |
| `-RepoPath <path>`  | repo to inspect, default the current directory        |
| `-Json`             | machine-readable output, for `status` and `pr`        |
| `-Force`            | let `init` rewrite a settings file that already exists |

Defaults can be kept:

```powershell
deplyd remember author "Ada"
deplyd remember environment staging
deplyd remember repo /path/to/repo
```

## Using it in a script

`-Json` prints one document and nothing else. `deplyd pr` also exits on its verdict, so
a release check is one line:

```bash
deplyd pr 412 -E production -Json > verdict.json || echo "do not ship"
```

| Exit |                                                                       |
|------|-----------------------------------------------------------------------|
| `0`  | live, on evidence with nothing shaky about it                         |
| `2`  | not live, including a change no deployed target covers                |
| `3`  | reverted                                                              |
| `4`  | not merged                                                            |
| `5`  | no such pull request, or no access to it                              |
| `6`  | live, but a target is `UNCERTAIN`: read the report before trusting it |
| `1`  | deplyd could not run: a bad argument, or the audit refused            |

`0` is the only code that means "in what shipped, and the reading is sound". `6` exists
so a gate is never told *shipped* on evidence deplyd has itself called into question:
the change is in the deployed commit, but a newer deploy did not complete, the commit
could not be read from the checkout step, or the two records of it disagree.

That last one is worth knowing if you gate on this. A repo whose deployments are
created for a moving branch ref can produce disagreements, and those runs return `6`
where an earlier version returned `0` - the verdict has not changed, the evidence for
it got weaker and now says so. Treat `6` as "look before shipping" rather than "do not
ship", or accept only `0` if you would rather be strict.

`deplyd status -Json` lists every target and every change, and exits `0` whenever it
could answer at all.

## What it reports

Per target:

|             |                                                                                                                     |
|-------------|---------------------------------------------------------------------------------------------------------------------|
| `PUBLISHED` | built and released, with no newer deploy failing or in flight. Says so when GitHub's deployment record names the same commit |
| `UNCERTAIN` | a newer deploy failed or is still running, or the deployed commit could not be read reliably                        |
| `SKIPPED`   | steps the run skipped, by name. Common with quick-deploy workflows, and changes to a skipped component are not live |

Per pull request:

|                            |                                                                             |
|----------------------------|-----------------------------------------------------------------------------|
| `LIVE in <sha>`            | its merge commit is contained in the deployed commit                        |
| `LIVE in <sha> as <other>` | the commit is absent but the same change is there, cherry-picked or rebased |
| `REVERTED`                 | it shipped, then was undone before the deployed commit, which is named      |
| `NOT LIVE`                 | neither the commit nor an equivalent change is there                        |
| `NOT MERGED`               | still open, or closed without merging                                       |
| `NOT COVERED`              | it changed no path any target covers. Lists every target, what it covers and the files, since a wrong `scopes` entry looks identical to a genuinely unrelated change |

In the default list, a pull request that was reverted before the deployed commit is
marked `(REVERTED)`. The other per-PR findings need `deplyd pr <number>`.

`PUBLISHED` means the code was built and the deploy ran to completion. Nothing outside
the server can prove the process actually cycled, unless your app reports its build SHA.
The label says what the evidence supports, which is why it is not `VERIFIED`.

There are two records of what a run deployed, made differently: the commit `checkout`
resolved in the log, and the commit GitHub's deployment record was created for. Where
both are in hand they are compared, and agreement is reported. A disagreement usually
means the branch moved mid-deploy, so deplyd says so rather than picking a winner. The
deployment record also outlives the log, which GitHub deletes after the retention
period, so it is the last source left for an older deploy.

## Adapting it to your repo

GitHub Actions is flexible enough that detection will not always land. Run
`deplyd config` to see what it concluded, and `deplyd init` to write that conclusion to
a settings file kept beside deplyd, where the wrong lines can be corrected by hand
rather than authored from nothing. Every key is optional, and between them they cover the
cases detection misses, so you adapt the tool to the repo rather than the repo to the
tool.

The file stays out of your working tree. To share the fixes with a team, move it to the
repo root as `.deplyd.json` and commit it: deplyd reads that in preference when it is
there. From then on `deplyd init -Force` rewrites that file rather than a copy that
would never take effect, which is the one case where deplyd edits something git is
tracking. It says so when it does.

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

| Key                             | Fixes                                                                                                    |
|---------------------------------|----------------------------------------------------------------------------------------------------------|
| `deployPattern`                 | a deploy workflow not being found, or a non-deploy one being treated as one                              |
| `environments`                  | the wrong environment list. Your names replace the detected ones                                         |
| `environments.<name>.workflows` | the wrong workflows for an environment. An explicit list always wins                                     |
| `ignoreJobs`                    | a job showing up as a target that should not. Matched as a substring                                     |
| `scopes`                        | which paths a target covers, when its job sets no `working-directory`. Keyed by the label deplyd reports |

### What detection looks for

Useful for reading `deplyd config` output, and for telling which key to set when something
is wrong. None of it is a requirement.

| It is treated as             | When                                                                                                                                                                                                                                      | Otherwise set                   |
|------------------------------|-------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|---------------------------------|
| a deploy workflow            | the filename or `name:` contains `deploy`, `release`, `publish`, `ship` or `cd`, or one of its jobs declares an `environment:`                                                                                                            | `deployPattern`                 |
| an environment               | the name appears in the filename, in a job's `environment:`, or in a `workflow_dispatch` choice input                                                                                                                                     | `environments`                  |
| that environment's workflows | the same name matches the workflow                                                                                                                                                                                                        | `environments.<name>.workflows` |
| a target                     | a job that is not plumbing, with at least three steps. Names containing `merge`, `environment`, `notify`, `trigger`, `lint`, `test`, `summary`, `setup`, `prepare` or `complete` are skipped, which catches `deploy-test-api` by accident | `ignoreJobs`                    |
| that target's scope          | the job sets `defaults.run.working-directory`                                                                                                                                                                                             | `scopes`                        |

A job's name becomes its label, so `deploy-api` reports as `API`.

Environments are optional: a repo with one `deploy.yml` and none at all works, and
targets are reported without an environment name. Where one workflow deploys to staging
and then production in the same run, the jobs are told apart by their `environment:`
declarations; without those, a run cannot be attributed to one environment and no
config key can recover it.

If nothing is recognised at all, the error lists every job it skipped and why.

## Read-only

Every `git` and `gh` call goes through one allowlist: reads only, `gh api` GET only, no
`git config` assignment, and no `Invoke-Expression`, `Start-Process` or shell spawning
that could hide a command. Before loading any of its own files it scans them for
anything calling `git` or `gh` outside that gateway, and refuses to run if it finds any.
The scan comes first so that nothing it would object to has already executed.
`deplyd check` shows the rules, the audit result, and how many of the shipped files it
covers: `install.ps1`, `uninstall.ps1` and `shell-init.ps1` are outside it, because
deplyd never loads them.

The one write against your repository is `git fetch`, which updates your own
remote-tracking refs. It runs when a commit is missing locally, or before listing pending
work. Nothing is sent to GitHub, and a fetch cannot change anything there in any case.

Everything else deplyd writes goes inside its own directory: the defaults kept by
`deplyd remember`, and the per-repo settings `deplyd init` scaffolds. Nothing lands in
your working tree, so there is no untracked file to explain and no `.gitignore` to
change. `deplyd check` prints the directory it writes to. Moving that file to the repo
root as `.deplyd.json` and committing it is a deliberate act, and deplyd reads it from
there in preference when you do.

## Limitations

- Deploys outside GitHub Actions are invisible. There is no run to read.
- A pull request's age does not matter, but the newest deploy's does. Workflows that
  deploy a branch input or call another workflow read that run's log, and GitHub
  deletes logs after the retention period: 90 days by default, set under Settings,
  Actions, General to at most 90 on public repos or 400 on private ones. Past that,
  deplyd falls back to GitHub's deployment record, which is kept. The target is still
  reported, marked `UNCERTAIN`, and says the commit came from the deployment record
  rather than from the checkout step. Only a deploy with neither is unreadable.
- One workflow serving several environments needs its jobs to declare `environment:`,
  because the API does not expose `workflow_dispatch` inputs.
- Each leg of a matrix job becomes its own target, labelled by its matrix values.
- A checkout hidden inside a composite action is assumed to take the ref the run was
  triggered on. If it takes something else, the workflow file cannot show that.
- `LIVE` means the commit shipped. Reverts are detected and named. A later commit
  rewriting the same lines is listed underneath rather than judged, and only the first
  such commit is shown, since after that the lines are someone else's.
- The read-only checks stop accidents, not someone determined to get around them.
- PowerShell only for now. The full suite passes on Windows PowerShell 5.1 and on
  PowerShell 7 under Linux. macOS is the same code path but has not been tested there.

## Licence

MIT, copyright Bertil Vossebelt.

Provided as-is, with no warranty of any kind, and with no obligation on me to maintain
or support it.

It is built to read only, and refuses to run if its own source calls `git` or `gh`
outside that gateway. That is a guard, not a guarantee. Deciding whether it is safe to
point at your repositories and environments is your call, and whatever happens as a
result is your responsibility, including anything it was never meant to do. Read the
source, run the tests, fork it and change it, or do not use it.
