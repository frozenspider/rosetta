use super::{MarkdownSection, MarkdownSubsection, Parser};
use crate::ParseError;

use anyhow::anyhow;
use pandoc::OutputKind;
use regex::Regex;
use std::path::Path;
use tokio::fs;

pub struct PandocParser {
    pub max_section_len: usize,
    pub skip_if_present: bool,
}

impl Parser for PandocParser {
    fn max_section_len(&self) -> usize {
        self.max_section_len
    }

    async fn parse(&self, input: &Path) -> Result<Vec<MarkdownSection>, ParseError> {
        let markdown = {
            let input = input.to_path_buf();
            let output_path = input.with_extension("md");
            if !output_path.exists() || !self.skip_if_present {
                let output_path_clone = output_path.clone();
                tokio::task::spawn_blocking(move || {
                    let mut pandoc = pandoc::new();
                    pandoc.add_input(&input);
                    pandoc.set_output(OutputKind::File(output_path_clone));
                    pandoc
                        .execute()
                        .map_err(|e| ParseError::OtherError(e.into()))
                })
                .await
                .map_err(|e| ParseError::OtherError(e.into()))??;
            }

            fs::read_to_string(output_path)
                .await
                .map_err(|e| ParseError::OtherError(e.into()))?
        };

        let sentence_break_regex =
            Regex::new(r"[.!?]\p{White_Space}+\p{Uppercase}").expect("valid regex");

        let mut sections = Vec::<MarkdownSection>::new();

        for s in markdown.split("\n\n") {
            let mut s = s.trim();
            let mut section = MarkdownSection::default();
            while s.len() > self.max_section_len {
                let min_break_point = self.max_section_len / 2;

                let Some(m) = sentence_break_regex.find_at(s, min_break_point) else {
                    return Err(ParseError::OtherError(anyhow!(
                        "Could not find a suitable break point to split a section!"
                    )));
                };

                let match_start = m.start() + 1; // Skip past the punctuation
                section
                    .0
                    .push(MarkdownSubsection(s[..match_start].trim().to_owned()));
                s = s[match_start..].trim();
            }
            if !s.is_empty() {
                section.0.push(MarkdownSubsection(s.to_owned()));
            }
            if !section.0.is_empty() {
                sections.push(section);
            }
        }

        Ok(sections)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tempfile::{tempdir, TempDir};

    fn create_temp_file_with_content(dir: &TempDir, content: &str) -> PathBuf {
        let file_path = dir.path().join("test.md");
        fs::write(&file_path, content).unwrap();
        file_path
    }

    #[test]
    fn parse_valid_docx_file() {
        let dir = tempdir().unwrap();

        let parser = PandocParser {
            max_section_len: 100,
            skip_if_present: false,
        };
        let input_path = create_temp_file_with_content(
            &dir,
            "This is a test document.\nIt has multiple sentences.",
        );

        let sections = parser.parse(&input_path).unwrap();

        assert_eq!(sections.len(), 1);
        assert_eq!(
            sections[0],
            MarkdownSection(vec![MarkdownSubsection(
                "This is a test document. It has multiple sentences.".to_owned()
            )])
        );
    }

    #[test]
    fn parse_docx_file_with_long_section() {
        let dir = tempdir().unwrap();

        let parser = PandocParser {
            max_section_len: 60,
            skip_if_present: false,
        };
        let input_path = create_temp_file_with_content(
            &dir,
            "This is a test document, just like that. It has multiple sentences.",
        );

        let sections = parser.parse(&input_path).unwrap();

        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].0.len(), 2);
        assert_eq!(
            sections[0].0[0].0,
            "This is a test document, just like that."
        );
        assert_eq!(sections[0].0[1].0, "It has multiple sentences.");
    }

    #[test]
    fn parse_docx_file_with_multiple_sections() {
        let dir = tempdir().unwrap();

        let parser = PandocParser {
            max_section_len: 60,
            skip_if_present: false,
        };
        let input_path =
            create_temp_file_with_content(&dir, "This is a test document.\n\nIt has two sections.");

        let sections = parser.parse(&input_path).unwrap();

        assert_eq!(sections.len(), 2);
        assert_eq!(sections[0].0.len(), 1);
        assert_eq!(sections[0].0[0].0, "This is a test document.");
        assert_eq!(sections[1].0.len(), 1);
        assert_eq!(sections[1].0[0].0, "It has two sections.");
    }

    #[test]
    fn parse_docx_file_with_no_break_point() {
        let dir = tempdir().unwrap();

        let parser = PandocParser {
            max_section_len: 10,
            skip_if_present: false,
        };
        let input_path =
            create_temp_file_with_content(&dir, "Thisisaverylongwordwithoutbreakpoints.");

        let result = parser.parse(&input_path);

        assert!(result.is_err());
    }

    #[test]
    fn parse_empty_docx_file() {
        let dir = tempdir().unwrap();

        let parser = PandocParser {
            max_section_len: 100,
            skip_if_present: false,
        };
        let input_path = create_temp_file_with_content(&dir, "");

        let sections = parser.parse(&input_path).unwrap();

        assert_eq!(sections.len(), 0);
    }
}
