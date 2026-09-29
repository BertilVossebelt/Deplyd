# Example hooks

Starting points, not dependencies. Copy one and change it.

| Script       | Where the notification lands                              |
|--------------|-----------------------------------------------------------|
| `notify.ps1` | Windows Action Center, via WinRT toast                    |
| `notify.sh`  | macOS Notification Center, or a Linux notification daemon |

```bash
deplyd hooks add ./example-hooks/notify.sh    # or notify.ps1 on Windows
deplyd hooks test                             # kick it with a made-up event
deplyd watch --background
```

Both read one JSON event on stdin and nothing else. `notify.sh` uses `jq` when
it is there and a plain substitution when it is not; the fallback reads a flat
object and would not survive a quote inside a value, which is the first thing to
fix if you build on it.

They exit non-zero when there is no notifier to reach, so `deplyd hooks test`
tells you before a watcher does.
