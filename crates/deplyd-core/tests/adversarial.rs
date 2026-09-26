//! Input the parser was not written for.
//!
//! Every case here is a shape someone could put in a workflow. The rule is the same
//! throughout: read it correctly, or refuse it by name. Returning something plausible
//! and wrong is the one outcome that is not allowed.

use deplyd_core::yaml::{Node, parse};

fn scalar(node: &Node, path: &[&str]) -> String {
    node.get_str(path).unwrap_or_default().to_string()
}

#[test]
fn nothing_at_all_is_not_an_error() {
    assert_eq!(parse("").expect("empty"), Node::Null);
    assert_eq!(parse("# only a comment\n").expect("comments"), Node::Null);
}

#[test]
fn windows_line_endings_read_the_same() {
    let unix = parse("name: A\njobs:\n  d:\n    runs-on: x\n").expect("unix");
    let windows = parse("name: A\r\njobs:\r\n  d:\r\n    runs-on: x\r\n").expect("windows");
    assert_eq!(unix, windows);
}

#[test]
fn a_colon_inside_a_value_is_not_a_separator() {
    assert_eq!(
        scalar(&parse("name: Deploy: the thing\n").unwrap(), &["name"]),
        "Deploy: the thing"
    );
    assert_eq!(
        scalar(&parse("url: https://example.com/a:b\n").unwrap(), &["url"]),
        "https://example.com/a:b"
    );
}

#[test]
fn a_hash_without_a_space_before_it_is_not_a_comment() {
    assert_eq!(
        scalar(&parse("run: echo a#b\n").unwrap(), &["run"]),
        "echo a#b"
    );
    assert_eq!(
        scalar(&parse("ref: main # pinned\n").unwrap(), &["ref"]),
        "main"
    );
}

#[test]
fn a_windows_path_keeps_its_backslashes() {
    let doc = parse("dir: C:\\Users\\ada\n").expect("parses");
    assert_eq!(scalar(&doc, &["dir"]), "C:\\Users\\ada");
}

#[test]
fn nested_flow_sequences_are_read_as_sequences() {
    // Kept as strings they would read as the literal "[a, b]", which is the sort of
    // confident wrong answer this parser exists to avoid.
    let doc = parse("matrix: [[a, b], [c]]\n").expect("parses");
    let outer = doc.get("matrix").expect("matrix");
    assert_eq!(outer.items().len(), 2);
    let inner: Vec<&str> = outer.items()[0]
        .items()
        .iter()
        .filter_map(Node::as_str)
        .collect();
    assert_eq!(inner, vec!["a", "b"]);
}

#[test]
fn a_sequence_of_sequences_is_read_as_one() {
    let doc = parse("a:\n  - - x\n    - y\n").expect("parses");
    let outer = doc.get("a").expect("a");
    assert_eq!(outer.items().len(), 1);
    let inner: Vec<&str> = outer.items()[0]
        .items()
        .iter()
        .filter_map(Node::as_str)
        .collect();
    assert_eq!(inner, vec!["x", "y"]);
}

#[test]
fn a_flow_mapping_reads_its_values() {
    let doc = parse("environment: {name: production, url: https://x}\n").expect("parses");
    assert_eq!(
        doc.at(&["environment"]).and_then(Node::scalar_or_named),
        Some("production")
    );
}

#[test]
fn empty_collections_stay_empty() {
    assert_eq!(
        parse("branches: []\n").unwrap().get("branches"),
        Some(&Node::Seq(vec![]))
    );
    assert_eq!(
        parse("environment: {}\n").unwrap().get("environment"),
        Some(&Node::Map(vec![]))
    );
}

#[test]
fn a_key_with_no_value_is_null_not_a_swallowed_sibling() {
    let doc = parse("a:\nb: 2\n").expect("parses");
    assert_eq!(doc.get("a"), Some(&Node::Null));
    assert_eq!(scalar(&doc, &["b"]), "2");
}

#[test]
fn a_sequence_item_whose_body_is_below_it() {
    let doc = parse("steps:\n  -\n    name: x\n    run: y\n").expect("parses");
    let steps = doc.get("steps").expect("steps");
    assert_eq!(steps.items().len(), 1);
    assert_eq!(steps.items()[0].get_str(&["name"]), Some("x"));
}

#[test]
fn unicode_survives_intact() {
    let doc = parse("name: Déployé → prod 🚀\n").expect("parses");
    assert_eq!(scalar(&doc, &["name"]), "Déployé → prod 🚀");
}

#[test]
fn a_document_that_is_only_a_scalar_is_refused() {
    // A workflow is a mapping. Anything else is not one, and saying so beats
    // returning an empty document that reads as "this repo deploys nothing".
    let error = parse("just a string\n").expect_err("refused");
    assert!(error.reason.contains("key"), "got: {error}");
}

#[test]
fn runaway_nesting_is_refused_rather_than_crashing() {
    // A malformed file once turned into a stack overflow here. Refusing is a worse
    // answer than reading it, and a much better one than crashing.
    let mut text = String::from("a:\n");
    for depth in 1..200 {
        text.push_str(&" ".repeat(depth * 2));
        text.push_str("b:\n");
    }
    let error = parse(&text).expect_err("should refuse");
    assert!(error.reason.contains("nested"), "got: {error}");
}

#[test]
fn a_deeply_but_reasonably_nested_document_still_parses() {
    let text = "a:\n  b:\n    c:\n      d:\n        e: 1\n";
    let doc = parse(text).expect("parses");
    assert_eq!(doc.get_str(&["a", "b", "c", "d", "e"]), Some("1"));
}

#[test]
fn duplicate_keys_keep_the_first() {
    let doc = parse("a: 1\na: 2\n").expect("parses");
    assert_eq!(scalar(&doc, &["a"]), "1");
}

#[test]
fn trailing_whitespace_is_not_part_of_a_value() {
    let doc = parse("name: A   \njobs:  \n  d: x  \n").expect("parses");
    assert_eq!(scalar(&doc, &["name"]), "A");
    assert_eq!(scalar(&doc, &["jobs", "d"]), "x");
}

#[test]
fn a_block_scalar_does_not_swallow_what_follows() {
    let doc = parse("run: |\n  echo one: two\n  # a shell comment\nnext: 2\n").expect("parses");
    assert_eq!(scalar(&doc, &["next"]), "2");
    assert!(scalar(&doc, &["run"]).contains("one: two"));
}
