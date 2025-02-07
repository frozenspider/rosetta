pub mod pandoc;

use std::path::Path;
use super::ParseError;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MarkdownSection(Vec<MarkdownSubsection>);

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MarkdownSubsection(String);

pub trait Parser {
    fn parse(&self, input: &Path) -> Result<Vec<MarkdownSection>, ParseError>;
}
