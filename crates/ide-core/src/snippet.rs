//! LSP snippet expansion (https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/specification/#snippet_syntax).
//!
//! Supports tab stops (`$1`, `${1}`), placeholders with nesting (`${1:name}`), choices
//! (`${1|a,b|}` — the first choice is inserted), the final stop `$0`, variables (`$TM_FILENAME`,
//! `${VAR:default}` — unknown variables expand to their default or nothing) and escapes
//! (`\$`, `\}`, `\\`). Stops are returned in navigation order with `$0` last.

use std::ops::Range;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snippet {
    pub text: String,
    /// Byte ranges of tab stops in `text`, in navigation order (`$0` last). Repeated
    /// indices keep only their first occurrence.
    pub stops: Vec<Range<usize>>,
}

struct Parser<'a> {
    chars: std::iter::Peekable<std::str::CharIndices<'a>>,
    source: &'a str,
    text: String,
    stops: Vec<(u32, Range<usize>)>,
    variable: &'a dyn Fn(&str) -> Option<String>,
}

impl Parser<'_> {
    fn parse_until(&mut self, terminator: Option<char>) {
        while let Some(&(_, character)) = self.chars.peek() {
            if Some(character) == terminator {
                return;
            }
            self.chars.next();
            match character {
                '\\' => match self.chars.peek() {
                    Some(&(_, next)) if matches!(next, '$' | '}' | '\\' | ',' | '|') => {
                        self.text.push(next);
                        self.chars.next();
                    }
                    _ => self.text.push('\\'),
                },
                '$' => self.dollar(),
                other => self.text.push(other),
            }
        }
    }

    fn number(&mut self) -> Option<u32> {
        let mut digits = String::new();
        while let Some(&(_, character)) = self.chars.peek() {
            if character.is_ascii_digit() {
                digits.push(character);
                self.chars.next();
            } else {
                break;
            }
        }
        digits.parse().ok()
    }

    fn name(&mut self) -> String {
        let mut name = String::new();
        while let Some(&(_, character)) = self.chars.peek() {
            if character.is_ascii_alphanumeric() || character == '_' {
                name.push(character);
                self.chars.next();
            } else {
                break;
            }
        }
        name
    }

    fn dollar(&mut self) {
        match self.chars.peek().map(|&(_, character)| character) {
            Some(character) if character.is_ascii_digit() => {
                let index = self.number().unwrap_or(0);
                let at = self.text.len();
                self.stops.push((index, at..at));
            }
            Some('{') => {
                let (open, _) = self.chars.next().unwrap();
                if self.chars.peek().is_some_and(|&(_, c)| c.is_ascii_digit()) {
                    let index = self.number().unwrap_or(0);
                    let start = self.text.len();
                    match self.chars.peek().map(|&(_, c)| c) {
                        Some(':') => {
                            self.chars.next();
                            self.parse_until(Some('}'));
                        }
                        Some('|') => {
                            self.chars.next();
                            let mut choice = String::new();
                            let mut first = true;
                            while let Some((_, character)) = self.chars.next() {
                                match character {
                                    '\\' => {
                                        if let Some((_, next)) = self.chars.next()
                                            && first
                                        {
                                            choice.push(next);
                                        }
                                    }
                                    ',' => first = false,
                                    '|' => break,
                                    other if first => choice.push(other),
                                    _ => {}
                                }
                            }
                            self.text.push_str(&choice);
                        }
                        _ => {}
                    }
                    self.chars.next_if(|&(_, c)| c == '}');
                    let end = self.text.len();
                    self.stops.push((index, start..end));
                } else {
                    let name = self.name();
                    if name.is_empty() {
                        // Not a valid construct: keep it literally.
                        self.text.push_str(&self.source[open - 1..=open]);
                        return;
                    }
                    let value = (self.variable)(&name);
                    match self.chars.peek().map(|&(_, c)| c) {
                        Some(':') => {
                            self.chars.next();
                            let start = self.text.len();
                            self.parse_until(Some('}'));
                            if let Some(value) = value {
                                self.text.truncate(start);
                                self.text.push_str(&value);
                            }
                        }
                        _ => {
                            // Transforms (`/regex/format/`) are not supported: skip them.
                            while let Some(&(_, c)) = self.chars.peek() {
                                if c == '}' {
                                    break;
                                }
                                self.chars.next();
                            }
                            self.text.push_str(&value.unwrap_or_default());
                        }
                    }
                    self.chars.next_if(|&(_, c)| c == '}');
                }
            }
            Some(character) if character.is_ascii_alphabetic() || character == '_' => {
                let name = self.name();
                self.text
                    .push_str(&(self.variable)(&name).unwrap_or_default());
            }
            _ => self.text.push('$'),
        }
    }
}

/// Expand an LSP snippet. `variable` resolves snippet variables such as `TM_FILENAME`.
pub fn expand(snippet: &str, variable: &dyn Fn(&str) -> Option<String>) -> Snippet {
    let mut parser = Parser {
        chars: snippet.char_indices().peekable(),
        source: snippet,
        text: String::new(),
        stops: Vec::new(),
        variable,
    };
    parser.parse_until(None);
    let mut stops = parser.stops;
    // Navigation order: 1, 2, …, then 0; placeholders nested inside others are recorded
    // before their parent, so sort by index and keep the first occurrence of each.
    stops.sort_by_key(|(index, range)| (if *index == 0 { u32::MAX } else { *index }, range.start));
    stops.dedup_by_key(|(index, _)| *index);
    let has_final = stops.iter().any(|(index, _)| *index == 0);
    let mut ordered: Vec<Range<usize>> = stops.into_iter().map(|(_, range)| range).collect();
    if !has_final {
        ordered.push(parser.text.len()..parser.text.len());
    }
    Snippet {
        text: parser.text,
        stops: ordered,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(snippet: &str) -> Snippet {
        expand(snippet, &|name| {
            (name == "TM_FILENAME").then(|| "main.rs".to_string())
        })
    }

    #[test]
    fn placeholders_choices_and_final_stop() {
        let snippet = plain("fn ${1:name}(${2:arg}: ${3|i32,u8|}) {\n\t$0\n}");
        assert_eq!(snippet.text, "fn name(arg: i32) {\n\t\n}");
        let pieces: Vec<&str> = snippet
            .stops
            .iter()
            .map(|r| &snippet.text[r.clone()])
            .collect();
        assert_eq!(pieces, ["name", "arg", "i32", ""]);
        assert_eq!(snippet.stops[3], 21..21);
    }

    #[test]
    fn nesting_variables_escapes_and_implicit_final_stop() {
        let snippet =
            plain("${1:outer ${2:inner}} \\$5 \\} ${TM_FILENAME} ${UNKNOWN:fallback} $UNKNOWN_TOO");
        assert_eq!(snippet.text, "outer inner $5 } main.rs fallback ");
        assert_eq!(&snippet.text[snippet.stops[0].clone()], "outer inner");
        assert_eq!(&snippet.text[snippet.stops[1].clone()], "inner");
        assert_eq!(
            snippet.stops.last(),
            Some(&(snippet.text.len()..snippet.text.len()))
        );
        // Repeated stops keep the first; a lone `$` stays literal.
        let repeated = plain("$1 + $1 = $ 2");
        assert_eq!(repeated.text, " +  = $ 2");
        assert_eq!(repeated.stops, vec![0..0, 9..9]);
        assert_eq!(plain("price: 5$").text, "price: 5$");
    }
}
