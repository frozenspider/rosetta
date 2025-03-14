pub mod pandoc;

use crate::parser::MarkdownSection;
use crate::TranslationError;
use std::path::Path;

pub type AlreadyTranslated = Vec<MarkdownSection>;

pub trait GeneratorBuilder {
    type Built: Generator;

    async fn build(
        &self,
        output_path: &Path,
        continue_translation: bool,
        max_parser_section_len: usize,
    ) -> Result<(Self::Built, AlreadyTranslated), TranslationError>;
}

pub trait Generator {
    async fn write(&mut self, md: MarkdownSection) -> Result<(), TranslationError>;

    async fn finalize(&mut self) -> Result<(), TranslationError>;
}
