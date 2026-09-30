//! What a watcher's record says about it, and what it will not do.

use deplyd_core::watchers::{State, Watcher};

fn record(every: u64, quiet_for: i64) -> Watcher {
    let now = deplyd_core::watchers::now();
    Watcher {
        id: "abc123".into(),
        pid: 42,
        repo: "/tmp/repo".into(),
        environment: "production".into(),
        author: "Ada".into(),
        every_secs: every,
        started_at: now - 600,
        last_seen: now - quiet_for,
        stop_requested: false,
        stopped_at: None,
        log: "/tmp/repo.log".into(),
    }
}

#[test]
fn a_fresh_heartbeat_is_running() {
    assert_eq!(record(60, 0).state(), State::Running);
}

#[test]
fn a_slow_look_is_not_mistaken_for_a_dead_watcher() {
    // One interval late is ordinary: the look itself takes time, and GitHub is
    // not always quick. Calling that dead would make the list lie constantly.
    assert_eq!(record(60, 70).state(), State::Running);
}

#[test]
fn a_heartbeat_that_stopped_is_lost_rather_than_running() {
    assert_eq!(record(60, 600).state(), State::Lost);
}

#[test]
fn a_fast_watcher_still_gets_a_floor_before_it_is_presumed_gone() {
    // Three times ten seconds does not survive one hook taking its full 30s
    // timeout, and request_stop refuses anything not live - so a healthy
    // watcher would become unstoppable.
    assert_eq!(record(10, 25).state(), State::Running);
    assert_eq!(
        record(10, 80).state(),
        State::Running,
        "a slow hook is not a death"
    );
    assert_eq!(record(10, 200).state(), State::Lost);
}

#[test]
fn asking_it_to_stop_shows_before_it_has_gone() {
    let mut watcher = record(60, 0);
    watcher.stop_requested = true;
    assert_eq!(watcher.state(), State::Stopping);
    assert!(watcher.is_live(), "still worth listing until it goes");
}

#[test]
fn a_stopped_watcher_stays_stopped_even_if_its_heartbeat_was_recent() {
    // It stopped cleanly. A heartbeat from a second before that must not read
    // as "running" and offer a stop that would do nothing.
    let mut watcher = record(60, 0);
    watcher.stopped_at = Some(deplyd_core::watchers::now());
    assert_eq!(watcher.state(), State::Stopped);
    assert!(!watcher.is_live());
}

#[test]
fn two_watchers_started_in_the_same_second_get_different_names() {
    let first = deplyd_core::watchers::new_id(1000);
    let second = deplyd_core::watchers::new_id(1001);
    assert_ne!(first, second, "the pid is what tells them apart");
}

#[test]
fn a_heartbeat_does_not_wipe_a_stop_someone_else_asked_for() {
    // The two live in different processes, and `watchers stop` writes to the
    // file underneath the watcher. Stamping from memory puts stop_requested
    // back to false, and nothing ever stops.
    let mut mine = record(60, 0);

    let mut asked = mine.clone();
    asked.stop_requested = true;

    let told = {
        mine.absorb(Some(&asked), deplyd_core::watchers::now());
        mine.stop_requested
    };

    assert!(told, "the watcher has to see what the other process wrote");
    assert_eq!(mine.state(), State::Stopping);
}

#[test]
fn a_heartbeat_still_stamps_the_time_when_nothing_changed() {
    let mut mine = record(60, 500);
    mine.absorb(None, 1_800_000_000);
    assert_eq!(mine.last_seen, 1_800_000_000);
    assert!(!mine.stop_requested, "nothing asked, nothing invented");
}

#[test]
fn a_record_that_vanished_does_not_stop_the_watcher() {
    // Nothing on disk to read. Carrying on beats exiting on a missing file:
    // the watch is the point, the record is bookkeeping.
    let mut mine = record(60, 0);
    mine.absorb(None, deplyd_core::watchers::now());
    assert!(!mine.stop_requested);
    assert_eq!(mine.state(), State::Running);
}

#[test]
fn what_is_running_is_never_buried_by_what_has_finished() {
    // Records are kept for ever, and the listing relies on this order to put
    // live ones first.
    let now = deplyd_core::watchers::now();
    let mut held: Vec<Watcher> = (0..40)
        .map(|i| {
            let mut watcher = record(60, 0);
            watcher.id = format!("old{i:03}");
            watcher.started_at = now - 86_400 * i;
            watcher.stopped_at = Some(now - 86_400 * i);
            watcher
        })
        .collect();

    let mut running = record(60, 0);
    running.id = "live0001".into();
    running.started_at = now - 999_999;
    held.push(running);

    let (live, finished): (Vec<_>, Vec<_>) = held.iter().partition(|w| w.is_live());
    assert_eq!(live.len(), 1, "one of them is still going");
    assert_eq!(live[0].id, "live0001");
    assert_eq!(finished.len(), 40);
}

#[test]
fn what_finished_long_ago_is_named_for_tidying_and_the_rest_is_not() {
    // Forty finished, one a day. The newest ten stay by count whatever their
    // age, the next twenty stay because a month has not passed, and the last
    // ten have served their purpose.
    let now = deplyd_core::watchers::now();
    let day = 86_400;
    let mut held: Vec<Watcher> = (0..40i64)
        .map(|i| {
            let mut watcher = record(60, 0);
            watcher.id = format!("old{i:03}");
            watcher.started_at = now - day * (i + 2);
            watcher.stopped_at = Some(now - day * (i + 1));
            watcher
        })
        .collect();

    let mut running = record(60, 0);
    running.id = "live0001".into();
    running.started_at = now - day * 400;
    held.push(running);

    let stale: Vec<&str> = deplyd_core::watchers::stale(&held, now)
        .iter()
        .map(|w| w.id.as_str())
        .collect();

    assert!(!stale.contains(&"live0001"), "a live one is never tidied");
    for i in 0..30 {
        let id = format!("old{i:03}");
        assert!(!stale.contains(&id.as_str()), "{id} should be kept");
    }
    for i in 30..40 {
        let id = format!("old{i:03}");
        assert!(stale.contains(&id.as_str()), "{id} should go");
    }
    assert_eq!(stale.len(), 10);
}

#[test]
fn the_newest_finished_are_kept_however_old_they_are() {
    let now = deplyd_core::watchers::now();
    let held: Vec<Watcher> = (0..deplyd_core::watchers::KEEP_FINISHED as i64)
        .map(|i| {
            let mut watcher = record(60, 0);
            watcher.id = format!("ancient{i:02}");
            watcher.stopped_at = Some(now - 86_400 * (365 + i));
            watcher
        })
        .collect();

    assert!(
        deplyd_core::watchers::stale(&held, now).is_empty(),
        "ten a year old are still the newest ten"
    );
}

#[test]
fn one_that_went_quiet_is_dated_from_its_last_heartbeat() {
    let now = deplyd_core::watchers::now();
    let mut held: Vec<Watcher> = (0..deplyd_core::watchers::KEEP_FINISHED as i64)
        .map(|i| {
            let mut watcher = record(60, 0);
            watcher.id = format!("recent{i:02}");
            watcher.stopped_at = Some(now - 60 * i);
            watcher
        })
        .collect();

    // Never marked stopped: its heartbeat just ended, forty days ago.
    let mut quiet = record(60, 40 * 86_400);
    quiet.id = "quiet001".into();
    held.push(quiet);

    let stale = deplyd_core::watchers::stale(&held, now);
    assert_eq!(stale.len(), 1);
    assert_eq!(stale[0].id, "quiet001");
    assert_eq!(stale[0].finished_at(), now - 40 * 86_400);
}
