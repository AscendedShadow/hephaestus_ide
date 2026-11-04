//! Editor preferences (IDE-014).
//!
//! Effective preferences for a buffer are resolved in this order, later entries winning:
//!
//! 1. built-in defaults,
//! 2. `editor` in user settings,
//! 3. `editor` in workspace settings (merged with user settings, see `settings::layers`),
//! 4. `editor.languages.<language id>` overrides (from the merged settings),
//! 5. `.editorconfig` properties for the file (unless `use_editorconfig` is `false`).
//!
//! Changing a preference never rewrites existing text: indentation settings affect new
//! indentation, and whitespace/final-newline/line-ending policies apply when saving.

use std::collections::BTreeMap;

use ide_core::{
    document::LineEnding,
    editorconfig::{self, EndOfLine, IndentStyle},
};
use serde::{Deserialize, Serialize};

pub const DEFAULT_FONT_SIZE: f32 = 14.;
pub const DEFAULT_TAB_WIDTH: usize = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LineEndingSetting {
    Lf,
    Crlf,
}

/// One layer of editor preferences; `None` inherits from the layer below.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct EditorSettings {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_family: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_size: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tab_width: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub insert_spaces: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub word_wrap: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line_numbers: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_whitespace: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trim_trailing_whitespace: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub insert_final_newline: Option<bool>,
    /// Line ending for new files.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line_ending: Option<LineEndingSetting>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format_on_save: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub use_editorconfig: Option<bool>,
    /// Per-language overrides keyed by language id (`rust`, `python`, …).
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub languages: BTreeMap<String, EditorSettings>,
}

impl EditorSettings {
    fn overlay(&mut self, other: &EditorSettings) {
        macro_rules! take {
            ($($field:ident),*) => {
                $(if other.$field.is_some() {
                    self.$field = other.$field.clone();
                })*
            };
        }
        take!(
            font_family,
            font_size,
            tab_width,
            insert_spaces,
            word_wrap,
            line_numbers,
            show_whitespace,
            trim_trailing_whitespace,
            insert_final_newline,
            line_ending,
            format_on_save,
            use_editorconfig
        );
    }

    /// Validation problems, with the setting path for each.
    pub fn problems(&self) -> Vec<String> {
        let mut problems = Vec::new();
        let check = |prefix: String, layer: &EditorSettings, problems: &mut Vec<String>| {
            if let Some(size) = layer.font_size
                && !(6.0..=72.0).contains(&size)
            {
                problems.push(format!(
                    "{prefix}font_size must be between 6 and 72 (got {size})"
                ));
            }
            if let Some(width) = layer.tab_width
                && !(1..=16).contains(&width)
            {
                problems.push(format!(
                    "{prefix}tab_width must be between 1 and 16 (got {width})"
                ));
            }
            if layer
                .font_family
                .as_deref()
                .is_some_and(|family| family.trim().is_empty())
            {
                problems.push(format!("{prefix}font_family must not be empty"));
            }
        };
        check("editor.".into(), self, &mut problems);
        for (language, layer) in &self.languages {
            if ide_core::language::by_id(language).is_none() {
                problems.push(format!(
                    "editor.languages: unknown language id {language:?}"
                ));
            }
            check(
                format!("editor.languages.{language}."),
                layer,
                &mut problems,
            );
        }
        problems
    }
}

/// Fully resolved preferences for one buffer.
#[derive(Clone, Debug, PartialEq)]
pub struct EditorPreferences {
    pub font_family: Option<String>,
    pub font_size: f32,
    pub tab_width: usize,
    pub insert_spaces: bool,
    pub word_wrap: bool,
    pub line_numbers: bool,
    pub show_whitespace: bool,
    pub trim_trailing_whitespace: bool,
    pub insert_final_newline: bool,
    pub line_ending: Option<LineEnding>,
    pub format_on_save: bool,
    /// Human-readable description of the layers that contributed.
    pub sources: Vec<String>,
}

impl Default for EditorPreferences {
    fn default() -> Self {
        Self {
            font_family: None,
            font_size: DEFAULT_FONT_SIZE,
            tab_width: DEFAULT_TAB_WIDTH,
            insert_spaces: true,
            word_wrap: false,
            line_numbers: true,
            show_whitespace: false,
            trim_trailing_whitespace: false,
            insert_final_newline: false,
            line_ending: None,
            format_on_save: false,
            sources: vec!["defaults".into()],
        }
    }
}

/// Resolve preferences for a buffer of `language`, with EditorConfig properties when the
/// file has a path.
pub fn resolve(
    settings: &EditorSettings,
    language: &str,
    editorconfig: Option<&editorconfig::Properties>,
) -> EditorPreferences {
    let mut layer = EditorSettings::default();
    let mut sources = vec!["defaults".to_string()];
    layer.overlay(settings);
    if *settings != EditorSettings::default() {
        sources.push("settings".into());
    }
    if let Some(language_layer) = settings.languages.get(language) {
        layer.overlay(language_layer);
        sources.push(format!("editor.languages.{language}"));
    }
    let defaults = EditorPreferences::default();
    let valid_size = layer.font_size.filter(|size| (6.0..=72.0).contains(size));
    let valid_width = layer.tab_width.filter(|width| (1..=16).contains(width));
    let mut preferences = EditorPreferences {
        font_family: layer.font_family.filter(|family| !family.trim().is_empty()),
        font_size: valid_size.unwrap_or(defaults.font_size),
        tab_width: valid_width.unwrap_or(defaults.tab_width),
        insert_spaces: layer.insert_spaces.unwrap_or(defaults.insert_spaces),
        word_wrap: layer.word_wrap.unwrap_or(defaults.word_wrap),
        line_numbers: layer.line_numbers.unwrap_or(defaults.line_numbers),
        show_whitespace: layer.show_whitespace.unwrap_or(defaults.show_whitespace),
        trim_trailing_whitespace: layer
            .trim_trailing_whitespace
            .unwrap_or(defaults.trim_trailing_whitespace),
        insert_final_newline: layer
            .insert_final_newline
            .unwrap_or(defaults.insert_final_newline),
        line_ending: layer.line_ending.map(|ending| match ending {
            LineEndingSetting::Lf => LineEnding::Lf,
            LineEndingSetting::Crlf => LineEnding::CrLf,
        }),
        format_on_save: layer.format_on_save.unwrap_or(defaults.format_on_save),
        sources: Vec::new(),
    };
    if layer.use_editorconfig != Some(false)
        && let Some(properties) = editorconfig.filter(|properties| !properties.is_empty())
    {
        if let Some(style) = properties.indent_style {
            preferences.insert_spaces = style == IndentStyle::Space;
        }
        let width = match preferences.insert_spaces {
            true => properties.indent_size.or(properties.tab_width),
            false => properties.tab_width.or(properties.indent_size),
        };
        if let Some(width) = width.filter(|width| (1..=16).contains(width)) {
            preferences.tab_width = width;
        }
        if let Some(trim) = properties.trim_trailing_whitespace {
            preferences.trim_trailing_whitespace = trim;
        }
        if let Some(final_newline) = properties.insert_final_newline {
            preferences.insert_final_newline = final_newline;
        }
        match properties.end_of_line {
            Some(EndOfLine::Lf) => preferences.line_ending = Some(LineEnding::Lf),
            Some(EndOfLine::CrLf) => preferences.line_ending = Some(LineEnding::CrLf),
            // Classic Mac CR endings are not supported for editing; keep the setting.
            Some(EndOfLine::Cr) | None => {}
        }
        sources.extend(
            properties
                .sources
                .iter()
                .map(|path| path.display().to_string()),
        );
    }
    preferences.sources = sources;
    preferences
}

/// Edits applying the save-time whitespace policies to `text` (UTF-8 byte ranges).
pub fn save_edits(
    text: &str,
    preferences: &EditorPreferences,
) -> Vec<(std::ops::Range<usize>, String)> {
    let mut edits = Vec::new();
    if preferences.trim_trailing_whitespace {
        let mut offset = 0;
        for line in text.split_inclusive('\n') {
            let content = line.strip_suffix('\n').unwrap_or(line);
            let content = content.strip_suffix('\r').unwrap_or(content);
            let trimmed = content.trim_end_matches([' ', '\t']);
            if trimmed.len() < content.len() {
                edits.push((
                    offset + trimmed.len()..offset + content.len(),
                    String::new(),
                ));
            }
            offset += line.len();
        }
    }
    if preferences.insert_final_newline && !text.is_empty() && !text.ends_with('\n') {
        edits.push((text.len()..text.len(), "\n".into()));
    }
    edits
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn settings(json: &str) -> EditorSettings {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn language_overrides_and_editorconfig_take_precedence() {
        let editor = settings(
            r#"{"tab_width": 4, "font_size": 15, "word_wrap": true,
                "languages": {"go": {"insert_spaces": false, "tab_width": 8}}}"#,
        );
        let rust = resolve(&editor, "rust", None);
        assert_eq!(
            (rust.tab_width, rust.insert_spaces, rust.font_size),
            (4, true, 15.)
        );
        assert!(rust.word_wrap);
        let go = resolve(&editor, "go", None);
        assert_eq!((go.tab_width, go.insert_spaces), (8, false));
        assert!(go.sources.contains(&"editor.languages.go".to_string()));

        let properties = editorconfig::Properties {
            indent_style: Some(IndentStyle::Space),
            indent_size: Some(2),
            trim_trailing_whitespace: Some(true),
            end_of_line: Some(EndOfLine::CrLf),
            sources: vec![PathBuf::from("/p/.editorconfig")],
            ..Default::default()
        };
        let go = resolve(&editor, "go", Some(&properties));
        assert_eq!((go.tab_width, go.insert_spaces), (2, true));
        assert!(go.trim_trailing_whitespace);
        assert_eq!(go.line_ending, Some(LineEnding::CrLf));
        assert_eq!(go.sources.last().unwrap(), "/p/.editorconfig");

        let disabled = settings(r#"{"use_editorconfig": false}"#);
        assert_eq!(
            resolve(&disabled, "go", Some(&properties)).tab_width,
            DEFAULT_TAB_WIDTH
        );
    }

    #[test]
    fn invalid_values_are_reported_and_ignored() {
        let editor = settings(
            r#"{"font_size": 200, "tab_width": 0, "languages": {"klingon": {}, "rust": {"font_family": " "}}}"#,
        );
        assert_eq!(editor.problems().len(), 4, "{:?}", editor.problems());
        let resolved = resolve(&editor, "rust", None);
        assert_eq!(resolved.font_size, DEFAULT_FONT_SIZE);
        assert_eq!(resolved.tab_width, DEFAULT_TAB_WIDTH);
        assert_eq!(resolved.font_family, None);
        assert!(serde_json::from_str::<EditorSettings>(r#"{"tabwidth": 2}"#).is_err());
    }

    #[test]
    fn save_policies_trim_whitespace_and_add_a_final_newline() {
        let preferences = EditorPreferences {
            trim_trailing_whitespace: true,
            insert_final_newline: true,
            ..Default::default()
        };
        let text = "a  \nb\t\r\n  \nc";
        let edits = save_edits(text, &preferences);
        assert_eq!(
            ide_core::text_edit::apply(text, &edits).unwrap(),
            "a\nb\r\n\nc\n"
        );
        assert!(save_edits("", &preferences).is_empty());
        assert!(save_edits("x  ", &EditorPreferences::default()).is_empty());
    }
}
