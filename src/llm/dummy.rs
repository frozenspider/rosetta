use super::{LLMBuilder, LLM};
use crate::parser::{MarkdownSection, MarkdownSubsection};
use crate::{LLMError, TranslationConfig};

pub struct DummyLLMBuilder;

impl LLMBuilder for DummyLLMBuilder {
    type Built = DummyLLM;

    async fn build(&self, _cfg: TranslationConfig) -> Result<Self::Built, LLMError> {
        Ok(DummyLLM)
    }
}

pub struct DummyLLM;

impl LLM for DummyLLM {
    async fn translate(&self, _section: MarkdownSection) -> Result<MarkdownSection, LLMError> {
        Ok(MarkdownSection(vec![MarkdownSubsection("Dummy output".to_owned())]))
    }
}
