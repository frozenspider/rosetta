use super::{LLMBuilder, LLM};
use crate::parser::MarkdownSection;
use crate::{TranslationConfig, TranslationError};

pub struct DummyLLMBuilder;

impl LLMBuilder for DummyLLMBuilder {
    type Built = DummyLLM;

    fn build(&self, _cfg: TranslationConfig) -> Result<Self::Built, anyhow::Error> {
        Ok(DummyLLM)
    }
}

pub struct DummyLLM;

impl LLM for DummyLLM {
    fn translate(&self, _section: MarkdownSection) -> Result<String, TranslationError> {
        Ok("Dummy output".to_owned())
    }
}
