pub mod dummy;
pub mod openai;

use super::parser::MarkdownSection;
use super::{LLMError, TranslationConfig};

pub trait LLMBuilder {
    type Built: LLM;

    async fn build(&self, cfg: TranslationConfig) -> Result<Self::Built, LLMError>;
}

pub trait LLM {
    async fn translate(&self, section: MarkdownSection) -> Result<MarkdownSection, LLMError>;
}
