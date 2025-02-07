use crate::ParseError;
use std::path::Path;

pub type Section = String;

pub trait Parser {
    fn parse(&self, input: &Path) -> Result<Vec<Section>, ParseError>;
}
