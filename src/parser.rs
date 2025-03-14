pub mod pandoc;

use std::path::Path;
use super::ParseError;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MarkdownSection(pub Vec<MarkdownSubsection>);

#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct MarkdownSubsection(pub String);

pub trait Parser {
    fn max_section_len(&self) -> usize;

    async fn parse(&self, input: &Path) -> Result<Vec<MarkdownSection>, ParseError>;
}
