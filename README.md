# Deplyd

_Is my pull request deployed?_

Deplyd answers that for any repo that deploys through GitHub Actions. It names the
commit each environment was last deployed from and tells you which of your changes are
in it.

GitHub's deployments page shows one row per deploy run with one commit title. 
It never shows which commits went out, so answering this by hand means opening runs,
reading logs, and comparing SHAs. This does that for you.

Built for the case where you cannot change anything: no access to the servers, no
authority over the deploy workflows, no pipeline step you are allowed to add. It
installs nothing into your repository.

```bash
$ deplyd status pr 412
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

## Install

**macOS and Linux**

```bash
curl -fsSL https://raw.githubusercontent.com/BertilVossebelt/Deplyd/main/install.sh | sh
```

**Windows**

```powershell
irm https://raw.githubusercontent.com/BertilVossebelt/Deplyd/main/install.ps1 | iex
```

**From source** 

Requires Rust 1.98 or newer.

```bash
git clone https://github.com/BertilVossebelt/Deplyd.git
cd deplyd && cargo install --path crates/deplyd
```

Then open a new terminal and run `dp status` from inside any repo.

`dp update` and `dp uninstall` tell you how to do those. If you built from source, sign
in with `gh auth login` and add completion with `deplyd completions <shell>`.

## Usage

Run it from inside any repo.

Most verbs take a second word. The first says what you want, the second says
what about.

```
deplyd status                     what each target is running, what is pending, and your changes in it
       status pr <number>         is that pull request live?
       status commit <ref>        is that commit live? any ref, HEAD included

       watch                      deploys and changes going live, as they happen
       watch pr <number>          the same, until that pull request is live
       watch commit <ref>         the same, for any ref
       watch stop <id>            ask a background watcher to stop
       watch log <id>             what one last said
       watch startup              watches that come back when the machine does
       watch startup disable <id> stop one coming back

       list environments          environments that -E accepts
       list authors               names that -A accepts
       list watchers              what is watching in the background
       list hooks                 scripts a watcher kicks

       hooks add <script>         kick this when something happens
       hooks remove <script>      stop kicking it
       hooks test                 kick them all with a made-up event

       config                     what detection concluded about this repo
       config init                write that out as a file you can correct

       remember author <name>     what -A means when you leave it off
       remember environment <env> the same for -E
       remember repo <path>       the same for --repo-path
       remember depth <n>         the same for -D
       remember every <duration>  how often a watch looks, when not told

       completions                shell completion scripts
       quota                      what is left of GitHub's hourly allowance
       check                      prove it can only read
       update                     whether a newer release is out
       uninstall                  how to remove it from this machine
```
`dp` is the same binary under a shorter name. Each word shortens while it stays
unambiguous, so `dp li env` and `dp st pr 412` work.

Every verb offers only the options it acts on, so `deplyd <verb> --help` is the
short list that applies to it.

| Option                    | Taken by                                                                     |
|---------------------------|------------------------------------------------------------------------------|
| `-E, --environment <env>` | `status`, `watch`, `config`; a prefix will do                                |
| `-A, --author <name>`     | the same, default `git config user.name`                                     |
| `--anyone`                | the same, every author rather than yours                                     |
| `-D, --depth <n>`         | `status`, `watch`; how far back, default 200                                 |
| `-T, --take <n>`          | `status`, `watch`; how many to list, default 10                              |
| `-S, --skip <n>`          | `status`, `watch`; skip this many, for paging                                |
| `-J, --json`              | `status`, `watch`; machine-readable output                                   |
| `--repo-path <path>`      | anything that opens a repo, default the cwd                                  |
| `-F, --force`             | `config init`, to rewrite a file that exists                                 |
| `-h, --help`              | anything; on `status` and `watch` it also explains the terms the report uses |
| `-V, --version`           | which deplyd this is                                                         |

When `--depth` stops a target early the count reads `355+`, so a ceiling is never shown
as a total.

## Watching

`deplyd watch` looks again every so often and prints what changed: a deploy starting,
succeeding or failing, and a change crossing from pending to live. `-E`, `-A` and
`--anyone` narrow it as they do `status`.

```bash
deplyd watch --anyone -E production
deplyd watch pr 412 --for 2h          # exits 0 the moment it is live
deplyd watch commit HEAD              # the same, for any ref
deplyd watch --every 5m --json        # one JSON object per line, per event
```

| Option                |                                              |
|-----------------------|----------------------------------------------|
| `--every <duration>`  | how often to look, default 60s, floor 10s    |
| `--for <duration>`    | stop after this long                         |
| `-B, --background`    | let go of the terminal and keep watching     |
| `--at-startup`        | and again when the machine starts            |

### In the background

`deplyd watch --background` gives you the terminal back and writes what it sees
to a log instead.

```bash
deplyd watch --background -E production
deplyd list watchers            # what is running
deplyd watch log a1b2c3         # what one last said
deplyd watch stop a1b2c3        # ask it to stop
```

Stopping takes up to one interval: a watcher notices on its next look. Finished
ones stay in the list, so `deplyd watch log` still works afterwards.

### At startup

`--at-startup` also writes the watch into wherever this machine looks at login.
It implies `--background`.

```bash
deplyd watch --at-startup -E production
deplyd watch startup            # what comes back at boot
deplyd watch startup disable a1b2c3   # stop it coming back
```

| Platform |                                                                                     |
|----------|-------------------------------------------------------------------------------------|
| Windows  | a `.cmd` in the Startup folder                                                      |
| macOS    | a LaunchAgent plist in `~/Library/LaunchAgents`                                     |
| Linux    | a systemd user unit in `~/.config/systemd/user`                                     |

`disable` stops it running but leaves the file in place, and names the path if
you want to remove it yourself.

Every look costs API requests, more of them the more deploy workflows a repo has, out
of an hourly allowance shared with `gh`. So be careful not to run it too fast for
too long. Especially if you have a lot of deploy workflows.

`deplyd quota` shows what is left and when it refills. GitHub reports that
differently depending on what you ask, so the figure is the lowest of what it
said - a floor, not a count. When it runs out, deplyd says so and stops.

## Hooks

A watcher can kick your own scripts when something happens. Register them once;
every event is handed to each one as a single JSON object on stdin.

```bash
deplyd hooks add ./notify.sh   # register
deplyd list hooks              # list them
deplyd hooks test              # kick them all with a made-up event
deplyd hooks remove ./notify.sh
```

| Field    |                                                                        |
|----------|------------------------------------------------------------------------|
| `kind`   | `deploy.started`, `deploy.succeeded`, `deploy.failed` or `change.live` |
| `label`  | which target it concerns                                               |
| `id`     | a run id, a pull request number, or a short sha                        |
| `title`  | the run's or the change's title                                        |
| `url`    | where to go and look                                                   |
| `author` | who wrote the change, absent for a deploy                              |

There are working examples in [example-hooks/](example-hooks/): a desktop
notification for Windows, macOS and Linux, each landing in the notification
centre rather than a dialog box.

```bash
deplyd hooks add ./example-hooks/notify.sh
```

A hook gets 30 seconds, then it is stopped. One that fails is reported and the
watch carries on.

## In a script

`status pr` and `status commit` exit on their verdict, so no parsing is needed:

```bash
deplyd status pr 412 -E production || echo "not deployed"
```

| Exit |                                                              |
|------|--------------------------------------------------------------|
| `0`  | deployed, on evidence with nothing shaky about it            |
| `2`  | not deployed, including a change no target covers            |
| `3`  | reverted                                                     |
| `4`  | not merged                                                   |
| `5`  | no such pull request, or no access to it                     |
| `6`  | deployed, but a target is `UNCERTAIN`: read the report first |
| `1`  | could not run                                                |

`6` means the change is in the deployed commit, but a newer deploy did not complete or
the commit could not be read reliably. Accept only `0` to be strict.

`--json` prints one document and nothing else. `deplyd status --json` lists every target
and change, and exits `0` whenever it could answer at all.

## What it reports

Per target:

|             |                                                                               |
|-------------|-------------------------------------------------------------------------------|
| the date    | when that target finished deploying, not when the commit was written          |
| `DEPLYD`    | built and released, with no newer deploy failing or in flight                 |
| `UNCERTAIN` | a newer deploy did not complete, or the commit could not be read reliably     |
| `because`   | what made it uncertain: the run that failed, or the reading that did not hold |
| `skipped`   | steps the run skipped. Changes to those are not live                          |

Per change:

|                                |                                                                             |
|--------------------------------|-----------------------------------------------------------------------------|
| `DEPLYD in <sha>`              | it is in the deployed commit                                                |
| `DEPLYD in <sha> as a copy`    | the commit is absent but the same change is there, cherry-picked or rebased |
| `REVERTED undone before <sha>` | it shipped, then was undone before the deployed commit                      |
| `NOT DEPLYD deployed <sha>`    | neither the commit nor an equivalent change is there                        |
| `NOT MERGED`                   | still open, or closed without merging                                       |
| `NOT COVERED`                  | it changed no path any target covers                                        |

`DEPLYD` means the code was built and the deploy ran to completion. Nothing outside the
server can prove the process actually cycled, which is why it is not `VERIFIED`.

## Adapting it to your repo

GitHub Actions is flexible enough that detection will not always land. `deplyd config`
shows what it concluded; `deplyd config init` writes that out as a file you can
correct. Every
key is optional. Move it to the repo root as `.deplyd.json` and commit it to share the
fixes.

```json
{
  "deployPattern": "deploy|release|shipit",
  "environments": {
    "production": { "workflows": ["shipit.yml"] },
    "staging": {}
  },
  "ignoreJobs": ["merge", "notify", "smoke"],
  "targetJobs": ["ship-it"],
  "scopes": {
    "API": ["services/api"],
    "WEB": ["services/web"]
  }
}
```

| Key                             | Fixes                                                                       |
|---------------------------------|-----------------------------------------------------------------------------|
| `deployPattern`                 | a deploy workflow not being found, or a non-deploy one being treated as one |
| `environments`                  | the wrong environment list. Your names replace the detected ones            |
| `environments.<name>.workflows` | the wrong workflows for an environment. An explicit list always wins        |
| `ignoreJobs`                    | a job showing up as a target that should not                                |
| `targetJobs`                    | a job that should be a target and is not                                    |
| `scopes`                        | which paths a target covers. Keyed by the label reported                    |

### What detection looks for

| It is treated as    | When                                                                                                                                                                                   |
|---------------------|----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| a deploy workflow   | the filename or `name:` contains `deploy`, `release`, `publish`, `ship` or `cd`; or a job declares an `environment:`; or a step uses a known deploy action or runs an applying command |
| an environment      | the name appears in the filename, in a job's `environment:`, or in a `workflow_dispatch` choice input                                                                                  |
| a target            | a job that ships: it declares an `environment:`, runs a known deploy or publish step, or hands off to another workflow                                                                 |
| that, more loosely  | if no job in the workflow looks like it ships: any job that is not plumbing, with at least three steps                                                                                 |
| that target's scope | the job's `defaults.run.working-directory`; or a directory all its steps agree on; or the workflow's own `paths:` trigger filter                                                       |

A job's name becomes its label, so `deploy-api` reports as `API`. Environments are
optional: a repo with one `deploy.yml` and none at all works.

## Read-only

Deplyd never writes to GitHub and never changes your code. The dangerous operations do not exist rather than being refused: every git call names
a verb from a fixed set with no `push` and no `checkout`, and every GitHub request is
one of a few GET routes with no method to set. Because a compiled binary cannot audit
the source it came from, every run exercises its own refusal paths first, and a build
whose guard has been weakened refuses to read a repository at all. This stops accidents, not someone determined to get around them.


```
$ deplyd check

  git verbs           PASS  13 allowed, none of them write
  write refusals      PASS  8 of 8 refused
  reads still work    PASS  5 of 5 allowed
  github routes       PASS  7 routes, all GET, all inside the repo
  hooks               none registered, so nothing else is ever started
```

Deplyd does need access to some write operations for its functionality. 
These are kept to a minimum and never change your remote repository. The exceptions are:

- `git fetch`, which updates your own remote-tracking refs. This is necessary for accurate reporting.
- its own files in your config directory - settings, watcher records and their
  logs, old ones tidied away - or `.deplyd.json` in the repo if you put one there
- one file in this machine's startup folder, and only if you ask for it with
  `--at-startup`. The only thing it writes outside its own directory
## Limitations

- Deploys outside GitHub Actions are invisible. There is no run to read.
- Workflows that deploy a branch input or call another workflow need the run log, and
  GitHub deletes logs after the retention period. Past that the target is `UNCERTAIN`.
- One workflow serving several environments needs its jobs to declare `environment:`.
- Only the newest fifteen runs per workflow are looked at, and the walk stops after
  three in a row reveal no new target. The output says when that happens.
- `DEPLYD` means the commit shipped. A later commit rewriting the same lines is listed
  underneath rather than judged.

## Development

Dot-source the dev installer to put a build of the current tree on PATH for that
terminal only. Nothing is written to your profile or rc file.

```bash
. ./dev-install.sh        # . .\dev-install.ps1 on Windows
```

The dot matters: a script cannot change the PATH of the shell that ran it. Each takes
`--release`, `--persist` and `--revert`, spelled `-Release`, `-Persist` and `-Revert` in
PowerShell. `--persist` installs for real, which is what testing the uninstaller needs.

```bash
cargo test
```

No GitHub account or network needed: fixture repositories are built with
`GIT_ALLOW_PROTOCOL=file`, so git itself refuses ssh and https.

Cutting a release is in [RELEASING.md](RELEASING.md).

## Licence

MIT, copyright Bertil Vossebelt.

Provided as-is, with no warranty and no obligation on me to maintain or support it.

It is built to read only, and proves that about itself on every run. That is a guard,
not a guarantee. Deciding whether it is safe to point at your repositories is your call,
and whatever happens as a result is your responsibility.
