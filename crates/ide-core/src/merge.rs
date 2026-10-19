use std::ops::Range;

const MARKER_LENGTH: usize = 7;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Current,
    Incoming,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    Unresolved,
    Current,
    Incoming,
    CurrentThenIncoming,
    IncomingThenCurrent,
    Base,
    Custom,
}

impl Choice {
    pub fn accepts(self, side: Side) -> bool {
        matches!(
            (self, side),
            (Self::Current, Side::Current)
                | (Self::Incoming, Side::Incoming)
                | (Self::CurrentThenIncoming | Self::IncomingThenCurrent, _)
        )
    }

    pub fn is_resolved(self) -> bool {
        self != Self::Unresolved
    }

    pub fn toggle(self, side: Side, accept: bool) -> Self {
        let other = match side {
            Side::Current => Side::Incoming,
            Side::Incoming => Side::Current,
        };
        let only = |side| match side {
            Side::Current => Self::Current,
            Side::Incoming => Self::Incoming,
        };
        match (accept, self.accepts(other)) {
            (true, false) => only(side),
            (true, true) if self.accepts(side) => self,
            (true, true) => Self::both(other),
            (false, true) => only(other),
            (false, false) => Self::Unresolved,
        }
    }

    pub fn both(first: Side) -> Self {
        match first {
            Side::Current => Self::CurrentThenIncoming,
            Side::Incoming => Self::IncomingThenCurrent,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conflict {
    pub current: String,
    pub base: Option<String>,
    pub incoming: String,
    pub current_label: String,
    pub incoming_label: String,
    pub original: String,
}

impl Conflict {
    pub fn side(&self, side: Side) -> &str {
        match side {
            Side::Current => &self.current,
            Side::Incoming => &self.incoming,
        }
    }

    pub fn text(&self, choice: Choice) -> Option<String> {
        Some(match choice {
            Choice::Unresolved => self.original.clone(),
            Choice::Current => self.current.clone(),
            Choice::Incoming => self.incoming.clone(),
            Choice::CurrentThenIncoming => join(&self.current, &self.incoming),
            Choice::IncomingThenCurrent => join(&self.incoming, &self.current),
            Choice::Base => self.base.clone()?,
            Choice::Custom => return None,
        })
    }

    pub fn classify(&self, text: &str) -> Choice {
        [
            Choice::Unresolved,
            Choice::Current,
            Choice::Incoming,
            Choice::CurrentThenIncoming,
            Choice::IncomingThenCurrent,
            Choice::Base,
        ]
        .into_iter()
        .find(|&choice| self.text(choice).as_deref() == Some(text))
        .unwrap_or(Choice::Custom)
    }
}

fn join(first: &str, second: &str) -> String {
    let mut text = first.to_string();
    if !text.is_empty() && !text.ends_with('\n') && !second.is_empty() {
        text.push('\n');
    }
    text.push_str(second);
    text
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Segment {
    Common(String),
    Conflict(usize),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Merge {
    segments: Vec<Segment>,
    conflicts: Vec<Conflict>,
    ranges: Vec<Range<usize>>,
    text: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Marker {
    Start,
    Base,
    Separator,
    End,
}

fn marker(line: &str) -> Option<(Marker, &str)> {
    let kind = match line.as_bytes().first()? {
        b'<' => Marker::Start,
        b'|' => Marker::Base,
        b'=' => Marker::Separator,
        b'>' => Marker::End,
        _ => return None,
    };
    let lead = line.as_bytes()[0];
    if line.len() < MARKER_LENGTH || !line.as_bytes()[..MARKER_LENGTH].iter().all(|&b| b == lead) {
        return None;
    }
    let rest = line[MARKER_LENGTH..].trim_end_matches(['\n', '\r']);
    match kind {
        Marker::Separator if rest.is_empty() => Some((kind, "")),
        Marker::Separator => None,
        _ if rest.is_empty() => Some((kind, "")),
        _ => rest.strip_prefix(' ').map(|label| (kind, label)),
    }
}

impl Merge {
    pub fn parse(text: &str) -> Self {
        let mut merge = Self {
            text: text.to_string(),
            ..Self::default()
        };
        let mut common = String::new();
        let mut offset = 0;
        let mut lines = text.split_inclusive('\n').peekable();
        while let Some(line) = lines.next() {
            let start = offset;
            offset += line.len();
            let Some((Marker::Start, current_label)) = marker(line) else {
                common.push_str(line);
                continue;
            };
            let mut original = line.to_string();
            let mut sections = [String::new(), String::new(), String::new()];
            let mut section = 0;
            let mut has_base = false;
            let mut incoming_label = None;
            while let Some(&line) = lines.peek() {
                match marker(line) {
                    Some((Marker::Start, _)) => break,
                    Some((Marker::Base, _)) if section == 0 => {
                        section = 1;
                        has_base = true;
                    }
                    Some((Marker::Separator, _)) if section < 2 => section = 2,
                    Some((Marker::End, label)) if section == 2 => {
                        incoming_label = Some(label.to_string());
                    }
                    _ => sections[section].push_str(line),
                }
                original.push_str(line);
                offset += line.len();
                lines.next();
                if incoming_label.is_some() {
                    break;
                }
            }
            let Some(incoming_label) = incoming_label else {
                common.push_str(&original);
                continue;
            };
            if !common.is_empty() {
                merge
                    .segments
                    .push(Segment::Common(std::mem::take(&mut common)));
            }
            let [current, base, incoming] = sections;
            merge
                .segments
                .push(Segment::Conflict(merge.conflicts.len()));
            merge.ranges.push(start..offset);
            merge.conflicts.push(Conflict {
                current,
                base: has_base.then_some(base),
                incoming,
                current_label: current_label.to_string(),
                incoming_label,
                original,
            });
        }
        if !common.is_empty() {
            merge.segments.push(Segment::Common(common));
        }
        merge
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn segments(&self) -> &[Segment] {
        &self.segments
    }

    pub fn conflicts(&self) -> &[Conflict] {
        &self.conflicts
    }

    pub fn range(&self, ix: usize) -> Range<usize> {
        self.ranges[ix].clone()
    }

    pub fn choice(&self, ix: usize) -> Choice {
        let range = &self.ranges[ix];
        match self.text.get(range.clone()) {
            Some(text) => self.conflicts[ix].classify(text),
            None => Choice::Custom,
        }
    }

    pub fn unresolved(&self) -> usize {
        (0..self.conflicts.len())
            .filter(|&ix| !self.choice(ix).is_resolved())
            .count()
    }

    pub fn side_text(&self, side: Side) -> String {
        self.segments
            .iter()
            .map(|segment| match segment {
                Segment::Common(text) => text.as_str(),
                Segment::Conflict(ix) => self.conflicts[*ix].side(side),
            })
            .collect()
    }

    pub fn resolve(&mut self, ix: usize, choice: Choice) -> Option<(Range<usize>, String)> {
        let replacement = self.conflicts.get(ix)?.text(choice)?;
        let range = self.ranges[ix].clone();
        if self.text.get(range.clone()) == Some(replacement.as_str()) {
            return None;
        }
        self.text.replace_range(range.clone(), &replacement);
        self.shift(range.clone(), replacement.len(), Some(ix));
        Some((range, replacement))
    }

    pub fn sync(&mut self, text: &str) {
        if text == self.text {
            return;
        }
        let old = self.text.as_bytes();
        let new = text.as_bytes();
        let mut prefix = old.iter().zip(new).take_while(|(a, b)| a == b).count();
        while !self.text.is_char_boundary(prefix) {
            prefix -= 1;
        }
        let limit = old.len().min(new.len()) - prefix;
        let mut suffix = old
            .iter()
            .rev()
            .zip(new.iter().rev())
            .take(limit)
            .take_while(|(a, b)| a == b)
            .count();
        while !self.text.is_char_boundary(old.len() - suffix) {
            suffix -= 1;
        }
        let edited = prefix..old.len() - suffix;
        let inserted = new.len() - suffix - prefix;
        self.shift(edited, inserted, None);
        self.text = text.to_string();
    }

    fn shift(&mut self, edited: Range<usize>, inserted: usize, owner: Option<usize>) {
        let Range { start: a, end: b } = edited;
        let moved = |offset: usize| offset + inserted - (b - a);
        let mut claimed = false;
        for (ix, range) in self.ranges.iter_mut().enumerate() {
            let (affected, after) = match owner {
                Some(owner) => (ix == owner, ix > owner),
                None if a == b => {
                    let inside = (range.start <= a && a < range.end)
                        || (range.start == range.end && range.start == a);
                    let take = inside && !claimed;
                    claimed |= take;
                    (take, range.start >= b)
                }
                None => (a < range.end && b > range.start, range.start >= b),
            };
            if affected {
                let start = range.start.min(a);
                *range = start..moved(range.end.max(b));
            } else if after {
                *range = moved(range.start)..moved(range.end);
            }
        }
    }
}

#[cfg(test)]
#[path = "merge_tests.rs"]
mod tests;
