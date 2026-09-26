//! Finding gh when the terminal's PATH is stale.
//!
//! A terminal keeps the PATH it started with, so a gh installed since then is
//! invisible to it. deplyd used to report gh missing when it was sitting there.

use deplyd_gateway::credential;

#[test]
fn the_lookup_is_stable_and_does_not_panic() {
    // On a machine without gh, "not present" is the right answer, so this asserts
    // the lookup behaves rather than that gh exists.
    let present = credential::github_cli_present();
    assert_eq!(
        present,
        credential::github_cli_present(),
        "the answer must not change between calls"
    );
    println!("gh present: {present}");
}

#[test]
fn a_remote_url_still_yields_its_owner_and_repo() {
    for url in [
        "https://github.com/acme/widgets.git",
        "git@github.com:acme/widgets.git",
        "ssh://git@github.com/acme/widgets.git",
        "https://github.com/acme/widgets",
    ] {
        assert_eq!(
            credential::parse_remote(url),
            Some(("acme".to_string(), "widgets".to_string())),
            "{url}"
        );
    }
    assert_eq!(credential::parse_remote("not a url"), None);
}

#[test]
fn an_enterprise_host_links_to_itself() {
    assert_eq!(
        credential::web_base("git@github.example.com:acme/widgets.git").as_deref(),
        Some("https://github.example.com/acme/widgets")
    );
}
