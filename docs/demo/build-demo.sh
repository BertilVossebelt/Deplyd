#!/bin/sh
# Builds a throwaway repository and a GitHub-made-of-files for the README demo.
# Nothing here reaches the network: DEPLYD_STUB_DIR answers every route.
set -e
DEMO="$1"
SELF_ROOT=$(cd "$(dirname "$0")/../.." && pwd)
rm -rf "$DEMO"; mkdir -p "$DEMO/repo/.github/workflows" "$DEMO/stub" "$DEMO/home"
cd "$DEMO/repo"
export HOME="$DEMO/home" USERPROFILE="$DEMO/home" APPDATA="$DEMO/home" XDG_CONFIG_HOME="$DEMO/home"
export GIT_CONFIG_GLOBAL="$DEMO/home/gitconfig" GIT_CONFIG_NOSYSTEM=1
export GIT_ALLOW_PROTOCOL=file GIT_TERMINAL_PROMPT=0
git init -q --initial-branch=main
git config user.name "Bertil"; git config user.email "bertil@example.invalid"

cat > .github/workflows/deploy-production.yml <<'YML'
name: Deploy production
on:
  push:
    branches: [main]
jobs:
  deploy-api:
    runs-on: ubuntu-latest
    environment:
      name: production
    defaults:
      run:
        working-directory: services/api
    steps:
      - uses: actions/checkout@v4
      - name: Build
        run: echo build
      - name: Ship
        run: echo ship
YML
cat > .github/workflows/deploy-web.yml <<'YML'
name: Deploy web
on:
  push:
    branches: [main]
jobs:
  deploy-web:
    runs-on: ubuntu-latest
    environment:
      name: production
    defaults:
      run:
        working-directory: services/web
    steps:
      - uses: actions/checkout@v4
      - name: Build web bundle
        run: echo build
      - name: Deploy web bundle
        run: echo ship
YML
mkdir -p services/api services/web
echo seed > services/api/seed.txt; echo seed > services/web/index.html
D="2026-09-08T09:12:00"
GIT_AUTHOR_DATE="$D" GIT_COMMITTER_DATE="$D" \
  git commit -qam "chore: scaffold services and deploy workflows" 2>/dev/null || {
  git add -A
  GIT_AUTHOR_DATE="$D" GIT_COMMITTER_DATE="$D" GIT_AUTHOR_NAME="Alan Turing" GIT_AUTHOR_EMAIL="alan@example.invalid" GIT_COMMITTER_NAME="Alan Turing" GIT_COMMITTER_EMAIL="alan@example.invalid" git commit -qm "chore: scaffold services and deploy workflows"; }

merge() { # number service MM-DD HH:MM author | title
  num=$1; svc=$2; date=$3; hour=$4; who=$5; shift 5; title="$*"
  case "$who" in
    bertil) name="Bertil";           mail="bertil@example.invalid" ;;
    grace)  name="Grace Hopper";     mail="grace@example.invalid" ;;
    alan)   name="Alan Turing";      mail="alan@example.invalid" ;;
    kj)     name="Katherine Johnson";mail="kj@example.invalid" ;;
  esac
  subject="$svc"
  [ "$svc" = both ] && subject="api,web"

  git checkout -q -b "feature-$num"
  if [ "$svc" = both ]; then
    echo "$num" > "services/api/change-$num.txt"
    echo "$num" > "services/web/change-$num.txt"
  else
    echo "$num" > "services/$svc/change-$num.txt"
  fi
  git add -A

  GIT_AUTHOR_NAME="$name" GIT_AUTHOR_EMAIL="$mail"   GIT_COMMITTER_NAME="$name" GIT_COMMITTER_EMAIL="$mail"   GIT_AUTHOR_DATE="2026-${date}T${hour}:00" GIT_COMMITTER_DATE="2026-${date}T${hour}:00"     git commit -qm "feat($subject): $title (#$num)"

  git checkout -q main

  GIT_AUTHOR_NAME="$name" GIT_AUTHOR_EMAIL="$mail"   GIT_COMMITTER_NAME="$name" GIT_COMMITTER_EMAIL="$mail"   GIT_AUTHOR_DATE="2026-${date}T${hour}:40" GIT_COMMITTER_DATE="2026-${date}T${hour}:40"     git merge -q --no-ff "feature-$num" -m "Merge pull request #$num from Deplyd/feature-$num"

  git branch -q -D "feature-$num"
}

# A team's worth of history. Ada is the one running deplyd, so only her five
# changes show in "deplyd changes" - which is what keeps the report short.
merge 398 web 08-24 15:10 grace preload the pricing fonts
merge 399 api 08-27 16:40 alan  drop the unused export queue
merge 400 web 09-01 09:05 kj    fix the skip link on the dashboard
merge 401 api 09-09 10:05 bertil rotate the signing key on a schedule
merge 402 web 09-10 14:20 grace drop the legacy bundle from the critical path
merge 403 api 09-11 09:40 grace  retry webhook delivery with a backoff
merge 404 web 09-14 11:15 kj    ship the new pricing page
merge 405 api 09-15 16:30 alan  cache the entitlement lookup
merge 406 api 09-17 10:50 kj     reject malformed cursors with a 400
merge 407 web 09-18 13:05 grace lazy-load the dashboard charts
merge 408 api 09-21 09:25 alan   widen the audit log to cover exports
merge 409 web 09-22 15:45 kj    fix the footer on narrow screens
merge 410 api 09-24 09:30 bertil collapse duplicate deploy records
# WEB last shipped here, before #412 existed.
WEB_DEPLOYED=$(git rev-parse HEAD)

merge 411 api 09-24 13:10 bertil add invoice export endpoint
merge 412 both 09-25 10:20 bertil paginate the invoice export
merge 413 web 09-25 16:05 grace surface deploy status in the header
merge 414 web 09-25 17:05 bertil show the deployed sha in the footer

# API shipped again after #412 landed, so it has it and WEB does not.
API_DEPLOYED=$(git log --format=%H --grep="Merge pull request #412" -1)

# A copy of the shipped example hook, the way the README tells you to use it:
# copy one into your own repo and point deplyd at it.
mkdir -p hooks
if [ -f "$SELF_ROOT/example-hooks/notify.ps1" ]; then
  cp "$SELF_ROOT/example-hooks/notify.ps1" hooks/notify.ps1
  cp "$SELF_ROOT/example-hooks/notify.sh" hooks/notify.sh 2>/dev/null || true
fi

git remote add origin https://github.invalid/Deplyd/widgets.git
# A default branch to compare against, so "pending" can be worked out.
git update-ref refs/remotes/origin/main HEAD
git symbolic-ref refs/remotes/origin/HEAD refs/remotes/origin/main

PR412=$(git log --format=%H --grep="Merge pull request #412" -1)
S="$DEMO/stub"; R=https://github.invalid/Deplyd/widgets
cat > "$S/runs-deploy-production.yml.json" <<JSON
{"workflow_runs":[{"id":500,"created_at":"2026-09-25T11:05:00Z","html_url":"$R/actions/runs/500","status":"completed","conclusion":"success","head_sha":"$API_DEPLOYED"}]}
JSON
cat > "$S/runs-deploy-web.yml.json" <<JSON
{"workflow_runs":[{"id":510,"created_at":"2026-09-24T10:02:00Z","html_url":"$R/actions/runs/510","status":"completed","conclusion":"success","head_sha":"$WEB_DEPLOYED"}]}
JSON
cat > "$S/jobs-500.json" <<'JSON'
{"jobs":[{"id":9000,"name":"deploy-api","conclusion":"success","started_at":"2026-09-25T11:05:00Z","steps":[{"name":"Checkout","conclusion":"success"},{"name":"Build","conclusion":"success"},{"name":"Ship","conclusion":"success"}]}]}
JSON
cat > "$S/jobs-510.json" <<'JSON'
{"jobs":[{"id":9010,"name":"deploy-web","conclusion":"success","started_at":"2026-09-24T10:02:00Z","steps":[{"name":"Checkout","conclusion":"success"},{"name":"Build web bundle","conclusion":"success"},{"name":"Deploy web bundle","conclusion":"skipped"}]}]}
JSON
cat > "$S/deployments-production.json" <<JSON
[{"id":77,"sha":"$API_DEPLOYED","environment":"production"},{"id":78,"sha":"$WEB_DEPLOYED","environment":"production"}]
JSON
echo "[{\"log_url\":\"$R/actions/runs/500\"}]" > "$S/statuses-77.json"
echo "[{\"log_url\":\"$R/actions/runs/510\"}]" > "$S/statuses-78.json"
cat > "$S/pr-412.json" <<JSON
{"number":412,"title":"paginate the invoice export","state":"closed","merged_at":"2026-09-25T10:20:00Z","merge_commit_sha":"$PR412","head":{"ref":"feature-412"}}
JSON
echo "api=$API_DEPLOYED web=$WEB_DEPLOYED"

# The state the watcher scene swaps in: WEB catches up, shipping #412 and #413.
# Copying these over the stub mid-run is a deploy happening underneath a watcher.
mkdir -p "$DEMO/after"
HEAD_SHA=$(git rev-parse HEAD)
cat > "$DEMO/after/runs-deploy-web.yml.json" <<JSON
{"workflow_runs":[{"id":511,"created_at":"2026-09-25T17:30:00Z","html_url":"$R/actions/runs/511","status":"completed","conclusion":"success","head_sha":"$HEAD_SHA"}]}
JSON
cat > "$DEMO/after/jobs-511.json" <<'JSON'
{"jobs":[{"id":9011,"name":"deploy-web","conclusion":"success","started_at":"2026-09-25T17:30:00Z","steps":[{"name":"Checkout","conclusion":"success"},{"name":"Build web bundle","conclusion":"success"},{"name":"Deploy web bundle","conclusion":"success"}]}]}
JSON
cat > "$DEMO/after/deployments-production.json" <<JSON
[{"id":77,"sha":"$API_DEPLOYED","environment":"production"},{"id":80,"sha":"$HEAD_SHA","environment":"production"}]
JSON
echo "[{\"log_url\":\"$R/actions/runs/511\"}]" > "$DEMO/after/statuses-80.json"
