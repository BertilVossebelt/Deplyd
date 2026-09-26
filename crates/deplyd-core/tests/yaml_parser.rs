//! The YAML subset parser, against the shapes workflows are actually written in.

use deplyd_core::yaml::{Node, parse};

fn fixture(name: &str, file: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("crate sits two levels under the workspace root")
        .join("tests/fixtures")
        .join(name)
        .join(".github/workflows")
        .join(file);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

#[test]
fn reads_a_conventional_workflow() {
    let doc = parse(&fixture("conventional", "deploy-production-api.yml")).expect("should parse");

    assert_eq!(doc.get_str(&["name"]), Some("Deploy production API"));
    let job = doc.at(&["jobs", "deploy-api"]).expect("job present");
    assert_eq!(
        job.at(&["environment"]).and_then(Node::scalar_or_named),
        Some("production-api")
    );
    assert_eq!(
        job.get_str(&["defaults", "run", "working-directory"]),
        Some("services/api")
    );
    assert_eq!(job.at(&["steps"]).map(|s| s.items().len()), Some(3));
}

#[test]
fn four_space_indentation_reads_the_same_as_two() {
    // The PowerShell version needed a fixture for this because its regexes assumed
    // a depth. A parser reads indentation from the file.
    let doc = parse(&fixture("four-space", "deploy.yml")).expect("should parse");
    let jobs = doc.at(&["jobs"]).expect("jobs present");

    let names: Vec<&str> = jobs.entries().iter().map(|(k, _)| k.as_str()).collect();
    assert_eq!(names, vec!["deploy-staging", "deploy-production"]);

    assert_eq!(
        jobs.at(&["deploy-production", "environment"])
            .and_then(Node::scalar_or_named),
        Some("production")
    );
}

#[test]
fn job_order_is_kept() {
    // Targets are reported in job order, so a hash map would make the report vary.
    let doc = parse(&fixture("staged-pipeline", "deploy.yml")).expect("should parse");
    let names: Vec<&str> = doc
        .at(&["jobs"])
        .expect("jobs")
        .entries()
        .iter()
        .map(|(k, _)| k.as_str())
        .collect();
    assert_eq!(names, vec!["deploy-staging", "deploy-production"]);
}

#[test]
fn reads_a_workflow_dispatch_choice_input() {
    let doc = parse(&fixture("choice-input", "deploy-anywhere.yml")).expect("should parse");
    let input = doc
        .at(&["on", "workflow_dispatch", "inputs", "environment"])
        .expect("input present");

    assert_eq!(input.get_str(&["type"]), Some("choice"));
    let options: Vec<&str> = input
        .at(&["options"])
        .expect("options")
        .items()
        .iter()
        .filter_map(Node::as_str)
        .collect();
    assert_eq!(options, vec!["demo", "sandbox"]);
}

#[test]
fn on_is_a_key_not_a_boolean() {
    // YAML 1.1 reads `on` as true, which would lose the trigger block entirely.
    let doc = parse(&fixture("staged-pipeline", "deploy.yml")).expect("should parse");
    assert!(
        doc.get("on").is_some(),
        "the `on:` block should be readable"
    );
    let branches: Vec<&str> = doc
        .at(&["on", "push", "branches"])
        .expect("branches")
        .items()
        .iter()
        .filter_map(Node::as_str)
        .collect();
    assert_eq!(branches, vec!["main"], "flow sequences should parse");
}

#[test]
fn reads_a_reusable_workflow_call() {
    let doc = parse(&fixture("external-reusable", "deploy-production.yml")).expect("should parse");
    assert_eq!(
        doc.get_str(&["jobs", "deploy", "uses"]),
        Some("acme/shared-workflows/.github/workflows/deploy.yml@main")
    );
    assert_eq!(
        doc.get_str(&["jobs", "deploy", "with", "environment"]),
        Some("production")
    );
}

#[test]
fn reads_a_step_with_nested_with_block() {
    let doc = parse(&fixture("conventional", "deploy-production-api.yml")).expect("should parse");
    let steps = doc.at(&["jobs", "deploy-api", "steps"]).expect("steps");
    let first = &steps.items()[0];

    assert_eq!(first.get_str(&["uses"]), Some("actions/checkout@v4"));
    assert_eq!(
        first.get_str(&["with", "ref"]),
        Some("${{ inputs.branch }}")
    );

    let second = &steps.items()[1];
    assert_eq!(second.get_str(&["name"]), Some("Publish api"));
    assert_eq!(second.get_str(&["run"]), Some("echo publishing"));
}

#[test]
fn reads_a_matrix() {
    let doc = parse(&fixture("matrix", "deploy-production.yml")).expect("should parse");
    let services: Vec<&str> = doc
        .at(&["jobs", "deploy", "strategy", "matrix", "service"])
        .expect("matrix")
        .items()
        .iter()
        .filter_map(Node::as_str)
        .collect();
    assert_eq!(services, vec!["api", "web"]);
}

#[test]
fn consumes_block_scalars_without_reading_them_as_keys() {
    // A shell script's own colons must not become mapping keys.
    let text = "\
jobs:
  deploy:
    steps:
      - name: Ship
        run: |
          echo one: two
          curl https://example.com
      - name: After
        run: echo done
";
    let doc = parse(text).expect("should parse");
    let steps = doc.at(&["jobs", "deploy", "steps"]).expect("steps");
    assert_eq!(
        steps.items().len(),
        2,
        "the block must not swallow the next step"
    );
    assert_eq!(steps.items()[1].get_str(&["name"]), Some("After"));
    assert!(
        steps.items()[0]
            .get_str(&["run"])
            .unwrap()
            .contains("one: two")
    );
}

#[test]
fn strips_comments_but_not_hashes_inside_values() {
    let doc = parse("a: main # pinned\nb: echo a#b\nc: \"has # inside\"\n").expect("should parse");
    assert_eq!(doc.get_str(&["a"]), Some("main"));
    assert_eq!(doc.get_str(&["b"]), Some("echo a#b"));
    assert_eq!(doc.get_str(&["c"]), Some("has # inside"));
}

#[test]
fn environment_reads_the_same_written_either_way() {
    let inline = parse("environment: production\n").expect("parses");
    let block = parse("environment:\n  name: production\n  url: https://x\n").expect("parses");
    let flow = parse("environment: {name: production}\n").expect("parses");

    for doc in [&inline, &block, &flow] {
        assert_eq!(
            doc.at(&["environment"]).and_then(Node::scalar_or_named),
            Some("production")
        );
    }
}

#[test]
fn refuses_anchors_rather_than_guessing() {
    let error = parse("defaults: &base\n  run:\n    working-directory: services/api\n")
        .expect_err("an anchor should be refused");
    assert!(error.reason.contains("anchor"), "got: {error}");
    assert_eq!(error.line, 1);
}

#[test]
fn refuses_aliases_rather_than_guessing() {
    let error = parse("a: &x 1\nb: *x\n").expect_err("an alias should be refused");
    assert!(
        error.reason.contains("anchor") || error.reason.contains("alias"),
        "got: {error}"
    );
}

#[test]
fn refuses_a_second_document() {
    let error = parse("name: one\n---\nname: two\n").expect_err("two documents should be refused");
    assert!(error.reason.contains("document"), "got: {error}");
}

#[test]
fn an_error_names_the_line() {
    let error = parse("jobs:\n  deploy:\n    environment: &shared\n").expect_err("refused");
    assert_eq!(error.line, 3, "should point at the offending line");
}

#[test]
fn every_fixture_workflow_parses() {
    // The hand-written cases above cover shapes I thought of. This covers the ones
    // the PowerShell suite collected, which is a different and better list.
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("workspace root")
        .join("tests/fixtures");

    let mut parsed = 0;
    let mut failures = Vec::new();

    let fixtures = std::fs::read_dir(&root).expect("fixtures should be readable");
    for fixture in fixtures.flatten() {
        let workflows = fixture.path().join(".github/workflows");
        let Ok(files) = std::fs::read_dir(&workflows) else {
            continue;
        };
        for file in files.flatten() {
            let path = file.path();
            if !path.extension().is_some_and(|e| e == "yml" || e == "yaml") {
                continue;
            }
            let text = std::fs::read_to_string(&path).expect("workflow readable");
            match parse(&text) {
                Ok(doc) => {
                    parsed += 1;
                    // Parsing to an empty document would be a silent failure.
                    assert!(
                        doc.get("jobs").is_some(),
                        "{}: parsed but found no jobs",
                        path.display()
                    );
                }
                Err(error) => failures.push(format!("{}: {error}", path.display())),
            }
        }
    }

    assert!(
        failures.is_empty(),
        "{} of {} fixture workflows failed to parse:\n{}",
        failures.len(),
        parsed + failures.len(),
        failures.join("\n")
    );
    assert!(
        parsed >= 15,
        "expected to parse the whole fixture set, got {parsed}"
    );
}
