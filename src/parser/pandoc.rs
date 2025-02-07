use super::{MarkdownSection, MarkdownSubsection, Parser};
use crate::ParseError;

use anyhow::anyhow;
use pandoc::OutputKind;
use regex::Regex;
use std::fs;
use std::path::Path;

pub struct PandocParser {
    pub max_section_len: usize,
}

impl Parser for PandocParser {
    fn parse(&self, input: &Path) -> Result<Vec<MarkdownSection>, ParseError> {
        // if input.extension().expect("extension") != "docx" {
        //     return Err(ParseError::UnsupportedFormatError {
        //         supported_formats: vec!["docx".to_owned()],
        //     });
        // }

        let markdown = {
            let tmp_dir = tempfile::tempdir().map_err(|e| ParseError::OtherError(e.into()))?;
            let tmp_dir_path = tmp_dir.path();
            let mut pandoc = pandoc::new();
            let output_path = tmp_dir_path.join("rosetta.md");
            pandoc.add_input(input);
            pandoc.set_output(OutputKind::File(output_path.clone()));
            pandoc
                .execute()
                .map_err(|e| ParseError::OtherError(e.into()))?;

            fs::read_to_string(output_path).map_err(|e| ParseError::OtherError(e.into()))?
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
        };
        let input_path = create_temp_file_with_content(
            &dir,
            "This is a test document.\n\nIt has two sections.",
        );

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
        };
        let input_path = create_temp_file_with_content(&dir, "");

        let sections = parser.parse(&input_path).unwrap();

        assert_eq!(sections.len(), 0);
    }
}
