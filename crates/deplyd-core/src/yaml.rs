//! Enough YAML to read a GitHub Actions workflow, and no more. Anything else is
//! refused by name and line number: something plausible and wrong is the one answer
//! not allowed. Mappings keep their order, because job order is target order.

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Node {
    Scalar(String),
    Seq(Vec<Node>),
    /// Insertion-ordered. Duplicate keys keep the first, as GitHub does.
    Map(Vec<(String, Node)>),
    Null,
}

impl Node {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Node::Scalar(text) => Some(text),
            _ => None,
        }
    }

    /// The value for a key, if this is a mapping that has one.
    pub fn get(&self, key: &str) -> Option<&Node> {
        match self {
            Node::Map(entries) => entries
                .iter()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value),
            _ => None,
        }
    }

    /// A string at a path, e.g. `defaults.run.working-directory`.
    pub fn get_str(&self, path: &[&str]) -> Option<&str> {
        self.at(path).and_then(Node::as_str)
    }

    pub fn at(&self, path: &[&str]) -> Option<&Node> {
        let mut here = self;
        for key in path {
            here = here.get(key)?;
        }
        Some(here)
    }

    pub fn entries(&self) -> &[(String, Node)] {
        match self {
            Node::Map(entries) => entries,
            _ => &[],
        }
    }

    pub fn items(&self) -> &[Node] {
        match self {
            Node::Seq(items) => items,
            _ => &[],
        }
    }

    /// A value that may be written either as a scalar or as a mapping with a `name`.
    /// `environment: production` and `environment:\n  name: production` mean the same
    /// thing to GitHub, and both appear in the wild.
    pub fn scalar_or_named(&self) -> Option<&str> {
        match self {
            Node::Scalar(text) => Some(text),
            Node::Map(_) => self.get_str(&["name"]),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct YamlError {
    pub line: usize,
    pub reason: String,
    pub text: String,
}

impl fmt::Display for YamlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "line {}: {} - {}",
            self.line,
            self.reason,
            self.text.trim()
        )
    }
}

impl std::error::Error for YamlError {}

/// Refused by name, so the message says what was found.
const UNSUPPORTED: &[(&str, &str)] = &[
    ("&", "a YAML anchor"),
    ("*", "a YAML alias"),
    ("!!", "a YAML tag"),
    ("? ", "a complex mapping key"),
];

/// How deep nesting may go. A workflow never approaches it; a parser that recurses
/// on input should have a floor, since a bug here once overflowed the stack.
const MAX_DEPTH: usize = 64;

struct Line {
    number: usize,
    indent: usize,
    dash: bool,
    content: String,
    content_indent: usize,
}

pub fn parse(text: &str) -> Result<Node, YamlError> {
    let lines = scan(text)?;
    if lines.is_empty() {
        return Ok(Node::Null);
    }
    let mut parser = Parser { lines, at: 0 };
    let indent = parser.lines[0].indent;
    parser.node(indent, 0)
}

/// The lines that carry content, with the sequence dash already split off.
fn scan(text: &str) -> Result<Vec<Line>, YamlError> {
    let mut lines = Vec::new();

    for (index, raw) in text.replace("\r\n", "\n").lines().enumerate() {
        let number = index + 1;
        let trimmed = raw.trim_start();

        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if trimmed == "---" || trimmed == "..." {
            // A second document would change which jobs deplyd believes exist.
            if lines.is_empty() {
                continue;
            }
            return Err(YamlError {
                line: number,
                reason: "more than one YAML document".into(),
                text: raw.into(),
            });
        }

        let indent = raw.len() - trimmed.len();
        let (dash, content, content_indent) = if trimmed == "-" {
            (true, String::new(), indent + 1)
        } else if let Some(rest) = trimmed.strip_prefix("- ") {
            let extra = rest.len() - rest.trim_start().len();
            (true, rest.trim_start().to_string(), indent + 2 + extra)
        } else {
            (false, trimmed.to_string(), indent)
        };

        lines.push(Line {
            number,
            indent,
            dash,
            content,
            content_indent,
        });
    }

    Ok(lines)
}

struct Parser {
    lines: Vec<Line>,
    at: usize,
}

impl Parser {
    fn current(&self) -> Option<&Line> {
        self.lines.get(self.at)
    }

    fn refuse(&self, line: &Line, reason: &str) -> YamlError {
        YamlError {
            line: line.number,
            reason: reason.into(),
            text: line.content.clone(),
        }
    }

    fn node(&mut self, indent: usize, depth: usize) -> Result<Node, YamlError> {
        if depth > MAX_DEPTH {
            let line = self.current().map(|line| line.number).unwrap_or(0);
            return Err(YamlError {
                line,
                reason: format!("nested more than {MAX_DEPTH} deep"),
                text: String::new(),
            });
        }
        match self.current() {
            None => Ok(Node::Null),
            Some(line) if line.dash && line.indent == indent => self.sequence(indent, depth),
            Some(_) => self.mapping(indent, depth),
        }
    }

    fn sequence(&mut self, indent: usize, depth: usize) -> Result<Node, YamlError> {
        let mut items = Vec::new();

        while let Some(line) = self.current() {
            if !line.dash || line.indent != indent {
                break;
            }

            let content_indent = line.content_indent;
            let empty = line.content.is_empty();

            if empty {
                // The item's body is on the following lines.
                self.at += 1;
                match self.current() {
                    Some(next) if next.indent > indent => {
                        let deeper = next.indent;
                        items.push(self.node(deeper, depth + 1)?);
                    }
                    _ => items.push(Node::Null),
                }
                continue;
            }

            let content = line.content.clone();
            let number = line.number;
            check_supported(&content, number)?;

            // An item that is itself a sequence: "- - x". The inner dash is peeled
            // off here, not just re-entered - leaving it made this branch match
            // again, forever.
            if content.starts_with("- ") || content == "-" {
                let rest = content.strip_prefix("- ").unwrap_or("");
                let extra = rest.len() - rest.trim_start().len();
                self.lines[self.at].indent = content_indent;
                self.lines[self.at].content = rest.trim_start().to_string();
                self.lines[self.at].content_indent = content_indent + 2 + extra;
                items.push(self.sequence(content_indent, depth + 1)?);
                continue;
            }

            // A plain item, as an options list is written: "- demo". There is no key,
            // so the mapping rules below would reject it on its own value.
            if split_key(&content).is_none() {
                self.at += 1;
                items.push(inline_value(&content, number)?);
                continue;
            }

            // Otherwise rewrite the dash away and let the ordinary rules apply, so
            // that "- name: Build" is a mapping whose later keys line up under
            // "name" rather than under the dash.
            self.lines[self.at].dash = false;
            self.lines[self.at].indent = content_indent;
            items.push(self.node(content_indent, depth + 1)?);
        }

        Ok(Node::Seq(items))
    }

    fn mapping(&mut self, indent: usize, depth: usize) -> Result<Node, YamlError> {
        let mut entries: Vec<(String, Node)> = Vec::new();

        while let Some(line) = self.current() {
            if line.indent != indent || line.dash {
                break;
            }

            let number = line.number;
            let content = line.content.clone();
            check_supported(&content, number)?;

            let Some((key, rest)) = split_key(&content) else {
                // A bare scalar where a key was expected. In a workflow that means
                // the shape is not what this parser understands.
                let line = self.current().expect("checked above");
                return Err(self.refuse(line, "expected 'key:' here"));
            };

            self.at += 1;

            let value = if let Some(style) = block_scalar_style(&rest) {
                self.block_scalar(indent, style)?
            } else if rest.is_empty() {
                match self.current() {
                    Some(next) if next.indent > indent => {
                        let deeper = next.indent;
                        self.node(deeper, depth + 1)?
                    }
                    // A dash at the same indent as its key is how sequences under a
                    // key are usually written.
                    Some(next) if next.dash && next.indent == indent => {
                        self.sequence(indent, depth + 1)?
                    }
                    _ => Node::Null,
                }
            } else {
                inline_value(&rest, number)?
            };

            if !entries.iter().any(|(existing, _)| *existing == key) {
                entries.push((key, value));
            }
        }

        Ok(Node::Map(entries))
    }

    /// A `|` or `>` block: every following line indented past the key belongs to it.
    /// deplyd never reads the contents of a `run:`, but it has to consume them, or
    /// a shell script's own colons would be parsed as mapping keys.
    fn block_scalar(&mut self, indent: usize, fold: bool) -> Result<Node, YamlError> {
        let mut collected: Vec<String> = Vec::new();
        while let Some(line) = self.current() {
            if line.indent <= indent {
                break;
            }
            let mut text = line.content.clone();
            if line.dash {
                text = format!("- {text}");
            }
            collected.push(text);
            self.at += 1;
        }
        let joined = if fold {
            collected.join(" ")
        } else {
            collected.join("\n")
        };
        Ok(Node::Scalar(joined))
    }
}

fn check_supported(content: &str, number: usize) -> Result<(), YamlError> {
    for (marker, description) in UNSUPPORTED {
        let found = match *marker {
            // An anchor or alias is a token of its own; "&&" in a run line is not.
            "&" => starts_token(content, '&'),
            "*" => starts_token(content, '*'),
            _ => content.contains(marker),
        };
        if found {
            return Err(YamlError {
                line: number,
                reason: format!("{description} is not supported"),
                text: content.into(),
            });
        }
    }
    Ok(())
}

/// Whether a value position begins with the given marker character.
fn starts_token(content: &str, marker: char) -> bool {
    let Some((_, rest)) = split_key(content) else {
        return false;
    };
    rest.starts_with(marker)
}

/// Splits `key: value`, returning the key and whatever follows. Quotes are respected
/// so that a colon inside a string is not mistaken for the separator.
fn split_key(content: &str) -> Option<(String, String)> {
    let bytes: Vec<char> = content.chars().collect();
    let mut quote: Option<char> = None;

    for (index, character) in bytes.iter().enumerate() {
        match quote {
            Some(open) => {
                if *character == open {
                    quote = None;
                }
            }
            None => {
                if *character == '"' || *character == '\'' {
                    quote = Some(*character);
                } else if *character == ':' {
                    let after = bytes.get(index + 1);
                    if after.is_none() || after == Some(&' ') || after == Some(&'\t') {
                        let key: String = bytes[..index].iter().collect();
                        let rest: String = bytes[index + 1..].iter().collect();
                        return Some((unquote(key.trim()), strip_comment(rest.trim())));
                    }
                }
            }
        }
    }
    None
}

fn block_scalar_style(rest: &str) -> Option<bool> {
    let head = rest.trim();
    // Chomping and indentation indicators ride along: |- >- |+ >2 are all the same
    // to deplyd, which only needs the block consumed rather than interpreted.
    if head.starts_with('|') && head[1..].chars().all(|c| "+-0123456789".contains(c)) {
        return Some(false);
    }
    if head.starts_with('>') && head[1..].chars().all(|c| "+-0123456789".contains(c)) {
        return Some(true);
    }
    None
}

fn inline_value(rest: &str, number: usize) -> Result<Node, YamlError> {
    let text = rest.trim();

    if text.starts_with('[') {
        if !text.ends_with(']') {
            return Err(YamlError {
                line: number,
                reason: "a flow sequence spanning lines is not supported".into(),
                text: rest.into(),
            });
        }
        let inner = &text[1..text.len() - 1];
        let mut items = Vec::new();
        for piece in split_flow(inner) {
            items.push(inline_value(piece.trim(), number)?);
        }
        return Ok(Node::Seq(items));
    }

    if text.starts_with('{') {
        if !text.ends_with('}') {
            return Err(YamlError {
                line: number,
                reason: "a flow mapping spanning lines is not supported".into(),
                text: rest.into(),
            });
        }
        let inner = &text[1..text.len() - 1];
        let mut entries = Vec::new();
        for piece in split_flow(inner) {
            let Some((key, value)) = split_key(piece.trim()) else {
                return Err(YamlError {
                    line: number,
                    reason: "expected 'key: value' inside a flow mapping".into(),
                    text: piece.to_string(),
                });
            };
            entries.push((key, inline_value(value.trim(), number)?));
        }
        return Ok(Node::Map(entries));
    }

    if text.is_empty() || text == "~" || text.eq_ignore_ascii_case("null") {
        return Ok(Node::Null);
    }

    Ok(Node::Scalar(unquote(text)))
}

/// Splits on commas that are not inside quotes or nested brackets.
fn split_flow(inner: &str) -> Vec<&str> {
    let mut pieces = Vec::new();
    let mut depth = 0usize;
    let mut quote: Option<char> = None;
    let mut start = 0usize;

    for (index, character) in inner.char_indices() {
        match quote {
            Some(open) => {
                if character == open {
                    quote = None;
                }
            }
            None => match character {
                '"' | '\'' => quote = Some(character),
                '[' | '{' => depth += 1,
                ']' | '}' => depth = depth.saturating_sub(1),
                ',' if depth == 0 => {
                    pieces.push(&inner[start..index]);
                    start = index + 1;
                }
                _ => {}
            },
        }
    }

    let tail = &inner[start..];
    if !tail.trim().is_empty() || !pieces.is_empty() {
        pieces.push(tail);
    }
    pieces
}

/// Removes a trailing comment. A `#` only starts one when it follows whitespace, so
/// `run: echo a#b` keeps its hash and `ref: main # pinned` loses the note.
fn strip_comment(text: &str) -> String {
    let characters: Vec<char> = text.chars().collect();
    let mut quote: Option<char> = None;

    for index in 0..characters.len() {
        let character = characters[index];
        match quote {
            Some(open) => {
                if character == open {
                    quote = None;
                }
            }
            None => {
                if character == '"' || character == '\'' {
                    quote = Some(character);
                } else if character == '#' && (index == 0 || characters[index - 1].is_whitespace())
                {
                    let kept: String = characters[..index].iter().collect();
                    return kept.trim_end().to_string();
                }
            }
        }
    }
    text.to_string()
}

fn unquote(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.len() >= 2 {
        let first = trimmed.chars().next().unwrap_or(' ');
        let last = trimmed.chars().last().unwrap_or(' ');
        if (first == '"' && last == '"') || (first == '\'' && last == '\'') {
            return trimmed[1..trimmed.len() - 1].to_string();
        }
    }
    trimmed.to_string()
}
