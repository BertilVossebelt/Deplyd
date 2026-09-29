#!/bin/sh
# A deplyd hook: one desktop notification per event.
#
#   deplyd hooks add ./example-hooks/notify.sh
#   deplyd hooks test
#
# macOS goes through osascript into Notification Center; Linux through
# notify-send into whatever notification daemon the desktop runs. Both leave
# something you can look at later rather than a dialog you have to dismiss.
#
# Deplyd hands a hook one JSON event on stdin and nothing else. Copy this and
# change it; it is meant to be a starting point, not a dependency.

set -eu

payload=$(cat)
[ -n "$payload" ] || exit 0

# jq if it is here, a plain substitution if it is not. The fallback reads one
# flat object and would not survive a quote inside a value - which deplyd does
# not send, but is the first thing to fix if you build on this.
field() {
  if command -v jq >/dev/null 2>&1; then
    printf '%s' "$payload" | jq -r ".$1 // empty"
  else
    printf '%s' "$payload" | sed -n "s/.*\"$1\":\"\([^\"]*\)\".*/\1/p"
  fi
}

kind=$(field kind)
label=$(field label)
title=$(field title)

case "$kind" in
  deploy.started)   headline="Deploying $label" ;;
  deploy.succeeded) headline="$label is live" ;;
  deploy.failed)    headline="$label failed to deploy" ;;
  change.live)      headline="Your change is live on $label" ;;
  *)                headline="$label: $kind" ;;
esac

if command -v osascript >/dev/null 2>&1; then
  # Quotes doubled, so a commit message with one in it cannot end the string
  # and become AppleScript of its own.
  safe_headline=$(printf '%s' "$headline" | sed 's/"/\\"/g')
  safe_title=$(printf '%s' "$title" | sed 's/"/\\"/g')
  osascript -e "display notification \"$safe_title\" with title \"$safe_headline\""
  exit 0
fi

if command -v notify-send >/dev/null 2>&1; then
  # A failure is worth interrupting for; everything else can wait its turn.
  urgency=normal
  [ "$kind" = "deploy.failed" ] && urgency=critical
  notify-send --app-name=deplyd --urgency="$urgency" -- "$headline" "$title"
  exit 0
fi

echo "deplyd: no notifier here (wanted osascript or notify-send)" >&2
echo "  $headline - $title" >&2
exit 1
