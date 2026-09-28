//! What changed between two looks at the same repository.
//!
//! Nothing here polls, waits or prints. A caller takes a [`Snapshot`] whenever it
//! likes and asks what is new since the last one, which is what makes the deciding
//! part of watching testable without a network or a clock.

use std::collections::BTreeMap;

use serde::Serialize;

/// A deploy run, as far as the last look could tell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunState {
    pub label: String,
    /// GitHub's own words: queued, in_progress, completed.
    pub status: String,
    /// Only once completed: success, failure, cancelled, and the rest.
    pub conclusion: Option<String>,
    pub url: String,
}

impl RunState {
    fn finished(&self) -> bool {
        self.status == "completed"
    }

    fn went_well(&self) -> bool {
        self.conclusion.as_deref() == Some("success")
    }
}

/// A change that is live, kept by commit so two looks can be compared.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveChange {
    /// "PR #412", or a short sha.
    pub id: String,
    pub title: String,
    pub author: String,
    pub label: String,
}

/// One look at the repository. Ordered maps, so two snapshots compare and print
/// in the same order every time rather than however a hash landed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub runs: BTreeMap<u64, RunState>,
    pub live: BTreeMap<String, LiveChange>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    DeployStarted,
    DeploySucceeded,
    DeployFailed,
    ChangeLive,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::DeployStarted => "deploy.started",
            Kind::DeploySucceeded => "deploy.succeeded",
            Kind::DeployFailed => "deploy.failed",
            Kind::ChangeLive => "change.live",
        }
    }
}

/// Something that happened between two looks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    pub kind: Kind,
    /// Which target it concerns.
    pub label: String,
    /// A run id, a pull request number, or a short sha.
    pub id: String,
    pub title: String,
    pub url: String,
    /// Empty for a deploy, which is nobody's in particular.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub author: String,
}

/// What is new in `after` that was not in `before`.
///
/// A first look has nothing to compare against, so the caller keeps that one as a
/// baseline rather than announcing the whole world as news.
pub fn changes(before: &Snapshot, after: &Snapshot) -> Vec<Event> {
    let mut events = Vec::new();

    for (id, now) in &after.runs {
        match before.runs.get(id) {
            // A run nobody had seen. Announced by where it has got to, since a
            // poll can easily miss the start of a short one.
            None => events.push(run_event(*id, now, true)),
            Some(was) if !was.finished() && now.finished() => {
                events.push(run_event(*id, now, false))
            }
            Some(_) => {}
        }
    }

    for (sha, change) in &after.live {
        if !before.live.contains_key(sha) {
            events.push(Event {
                kind: Kind::ChangeLive,
                label: change.label.clone(),
                id: change.id.clone(),
                title: change.title.clone(),
                url: String::new(),
                author: change.author.clone(),
            });
        }
    }

    events
}

fn run_event(id: u64, state: &RunState, first_seen: bool) -> Event {
    let kind = match (state.finished(), state.went_well()) {
        (false, _) => Kind::DeployStarted,
        (true, true) => Kind::DeploySucceeded,
        (true, false) => Kind::DeployFailed,
    };
    let title = match (first_seen, kind) {
        (_, Kind::DeployStarted) => "deploy started".to_string(),
        (_, Kind::DeploySucceeded) => "deploy succeeded".to_string(),
        (_, Kind::DeployFailed) => match state.conclusion.as_deref() {
            Some(word) if word != "failure" => format!("deploy {word}"),
            _ => "deploy failed".to_string(),
        },
        (_, Kind::ChangeLive) => unreachable!("a run is not a change"),
    };

    Event {
        kind,
        label: state.label.clone(),
        id: id.to_string(),
        title,
        url: state.url.clone(),
        author: String::new(),
    }
}

/// "30s", "5m", "2h", or a bare number of seconds. None for anything else,
/// including zero, which would be a loop with no pause in it.
pub fn parse_duration(text: &str) -> Option<std::time::Duration> {
    let text = text.trim().to_lowercase();
    let (digits, each) = match text.chars().last()? {
        's' => (&text[..text.len() - 1], 1),
        'm' => (&text[..text.len() - 1], 60),
        'h' => (&text[..text.len() - 1], 3600),
        '0'..='9' => (&text[..], 1),
        _ => return None,
    };
    let value: u64 = digits.trim().parse().ok()?;
    (value > 0).then(|| std::time::Duration::from_secs(value * each))
}

/// Which commits of a snapshot are live. Used by a watcher told to wait for one.
pub fn is_live(snapshot: &Snapshot, sha: &str) -> bool {
    snapshot.live.contains_key(sha)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(status: &str, conclusion: Option<&str>) -> RunState {
        RunState {
            label: "API".into(),
            status: status.into(),
            conclusion: conclusion.map(str::to_string),
            url: "https://example.test/run".into(),
        }
    }

    fn snapshot(runs: &[(u64, RunState)], live: &[(&str, &str)]) -> Snapshot {
        Snapshot {
            runs: runs.iter().cloned().collect(),
            live: live
                .iter()
                .map(|(sha, id)| {
                    (
                        (*sha).to_string(),
                        LiveChange {
                            id: (*id).to_string(),
                            title: "a change".into(),
                            author: "Ada".into(),
                            label: "API".into(),
                        },
                    )
                })
                .collect(),
        }
    }

    #[test]
    fn durations_are_read_the_way_people_write_them() {
        use std::time::Duration;
        assert_eq!(parse_duration("30s"), Some(Duration::from_secs(30)));
        assert_eq!(parse_duration("5m"), Some(Duration::from_secs(300)));
        assert_eq!(parse_duration("2h"), Some(Duration::from_secs(7200)));
        assert_eq!(parse_duration(" 90 "), Some(Duration::from_secs(90)));
        assert_eq!(parse_duration("1H"), Some(Duration::from_secs(3600)));
    }

    #[test]
    fn a_duration_that_is_not_one_is_refused() {
        assert_eq!(parse_duration("soon"), None);
        assert_eq!(
            parse_duration("0s"),
            None,
            "a pause of nothing is not a pause"
        );
        assert_eq!(parse_duration(""), None);
        assert_eq!(parse_duration("5x"), None);
        assert_eq!(parse_duration("-5m"), None);
    }

    #[test]
    fn nothing_happening_is_no_events() {
        let one = snapshot(
            &[(1, run("completed", Some("success")))],
            &[("aaa", "PR #1")],
        );
        assert!(changes(&one, &one).is_empty());
    }

    #[test]
    fn a_run_that_appears_unfinished_has_started() {
        let before = Snapshot::default();
        let after = snapshot(&[(1, run("in_progress", None))], &[]);

        let events = changes(&before, &after);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, Kind::DeployStarted);
    }

    #[test]
    fn a_run_finishing_is_reported_once() {
        let before = snapshot(&[(1, run("in_progress", None))], &[]);
        let after = snapshot(&[(1, run("completed", Some("success")))], &[]);

        let events = changes(&before, &after);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, Kind::DeploySucceeded);
        // Asked again with nothing new, it stays quiet.
        assert!(changes(&after, &after).is_empty());
    }

    #[test]
    fn a_short_run_seen_only_once_still_reports_how_it_ended() {
        // Polling can miss the start entirely. Saying nothing would be worse than
        // saying it finished.
        let after = snapshot(&[(7, run("completed", Some("failure")))], &[]);
        let events = changes(&Snapshot::default(), &after);

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, Kind::DeployFailed);
        assert_eq!(events[0].id, "7");
    }

    #[test]
    fn a_conclusion_that_is_not_failure_says_which_it_was() {
        let after = snapshot(&[(7, run("completed", Some("cancelled")))], &[]);
        let events = changes(&Snapshot::default(), &after);
        assert_eq!(events[0].title, "deploy cancelled");
    }

    #[test]
    fn a_change_appearing_live_is_an_event() {
        let before = snapshot(&[], &[("aaa", "PR #1")]);
        let after = snapshot(&[], &[("aaa", "PR #1"), ("bbb", "PR #2")]);

        let events = changes(&before, &after);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, Kind::ChangeLive);
        assert_eq!(events[0].id, "PR #2");
        assert_eq!(events[0].author, "Ada");
    }

    #[test]
    fn a_change_dropping_out_is_not_an_event() {
        // A narrower -E or a rolled back deploy can shrink the list. That is not
        // news of the kind a watcher exists to deliver, and announcing it as one
        // would read as "your change was undone".
        let before = snapshot(&[], &[("aaa", "PR #1"), ("bbb", "PR #2")]);
        let after = snapshot(&[], &[("aaa", "PR #1")]);
        assert!(changes(&before, &after).is_empty());
    }
}
