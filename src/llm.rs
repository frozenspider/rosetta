pub mod dummy;

use super::{TranslationConfig, TranslationError};
use super::parser::MarkdownSection;

pub trait LLMBuilder {
    type Built: LLM;

    async fn build(&self, cfg: TranslationConfig) -> Result<Self::Built, anyhow::Error>;
}

pub trait LLM {
    async fn translate(&self, section: MarkdownSection) -> Result<MarkdownSection, TranslationError>;
}
