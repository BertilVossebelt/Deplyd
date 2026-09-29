//! The one deletion deplyd performs, and everything it refuses to.

use std::path::PathBuf;

use deplyd_gateway::tidy::{TidyError, remove_watcher_file};

/// A directory shaped like deplyd's own, under the system temp directory.
fn own_directory(name: &str) -> PathBuf {
    let directory = std::env::temp_dir()
        .join(format!("deplyd-tidy-{}-{name}", std::process::id()))
        .join("deplyd")
        .join("watchers");
    std::fs::create_dir_all(&directory).expect("directory");
    directory
}

fn touch(path: &PathBuf) {
    std::fs::write(path, "{}").expect("write");
}

#[test]
fn a_record_in_deplyds_own_directory_goes() {
    let directory = own_directory("goes");
    let record = directory.join("6abb78c89ca8.json");
    let log = directory.join("6abb78c89ca8.log");
    touch(&record);
    touch(&log);

    remove_watcher_file(&directory, &record).expect("the record should go");
    remove_watcher_file(&directory, &log).expect("and its log");
    assert!(!record.exists());
    assert!(!log.exists());
}

#[test]
fn a_file_that_is_already_gone_is_said_rather_than_ignored() {
    let directory = own_directory("gone");
    let error = remove_watcher_file(&directory, &directory.join("6abb78c89ca8.log"))
        .expect_err("there is nothing there");
    assert!(
        matches!(error, TidyError::CouldNotRemove { .. }),
        "got: {error}"
    );
}

#[test]
fn a_directory_that_is_not_deplyds_is_refused_untouched() {
    // The right shape of file in the wrong place: a settings.json belonging to
    // something else entirely would look just like this.
    let directory = std::env::temp_dir()
        .join(format!("deplyd-tidy-{}-notours", std::process::id()))
        .join("watchers");
    std::fs::create_dir_all(&directory).expect("directory");
    let file = directory.join("6abb78c89ca8.json");
    touch(&file);

    let error = remove_watcher_file(&directory, &file).expect_err("not ours");
    assert!(matches!(error, TidyError::NotOurs(_)), "got: {error}");
    assert!(file.exists(), "refused means untouched");
}

#[test]
fn a_file_outside_the_directory_is_refused_untouched() {
    let directory = own_directory("outside");
    let beside = directory
        .parent()
        .expect("deplyd/")
        .join("6abb78c89ca8.json");
    touch(&beside);

    let error = remove_watcher_file(&directory, &beside).expect_err("outside");
    assert!(matches!(error, TidyError::Outside { .. }), "got: {error}");
    assert!(beside.exists());
}

#[test]
fn a_path_that_climbs_out_is_refused_untouched() {
    let directory = own_directory("climbs");
    let beside = directory
        .parent()
        .expect("deplyd/")
        .join("6abb78c89ca8.json");
    touch(&beside);
    // Names the same file, spelled from inside the directory.
    let climbing = directory.join("..").join("6abb78c89ca8.json");

    let error = remove_watcher_file(&directory, &climbing).expect_err("climbs out");
    assert!(matches!(error, TidyError::Outside { .. }), "got: {error}");
    assert!(beside.exists());
}

#[test]
fn something_that_is_not_a_record_is_refused_untouched() {
    let directory = own_directory("notarecord");
    for name in ["settings.json", "6abb78c89ca8.txt", "notes.log", ".json"] {
        let file = directory.join(name);
        touch(&file);
        let error = remove_watcher_file(&directory, &file).expect_err(name);
        assert!(
            matches!(error, TidyError::NotARecord(_)),
            "{name}: got {error}"
        );
        assert!(file.exists(), "{name} should be untouched");
    }
}
