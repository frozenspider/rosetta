pub mod pandoc;

use crate::parser::MarkdownSection;
use crate::TranslationError;
use std::path::Path;

pub trait GeneratorBuilder {
    type Built: Generator;

    async fn build(&self, output_path: &Path) -> Result<Self::Built, TranslationError>;
}

pub trait Generator {
    async fn write(&mut self, md: MarkdownSection) -> Result<(), TranslationError>;

    async fn finalize(&mut self) -> Result<(), TranslationError>;
}
