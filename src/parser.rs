pub mod pandoc;

use std::path::Path;
use super::ParseError;

#[derive(Debug, Clone)]
pub struct MarkdownSection(String);

pub trait Parser {
    fn parse(&self, input: &Path) -> Result<Vec<MarkdownSection>, ParseError>;
}
