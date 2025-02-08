use super::{LLMBuilder, LLM};
use crate::parser::{MarkdownSection, MarkdownSubsection};
use crate::{TranslationConfig, TranslationError};

pub struct PlaygroundLLMBuilder;

impl LLMBuilder for PlaygroundLLMBuilder {
    type Built = PlaygroundLLM;

    fn build(&self, _cfg: TranslationConfig) -> Result<Self::Built, anyhow::Error> {
        // TODO: Establish connection to the LLM API
        Ok(PlaygroundLLM);
        todo!()
    }
}

pub struct PlaygroundLLM;

impl LLM for PlaygroundLLM {
    fn translate(&self, _section: MarkdownSection) -> Result<MarkdownSection, TranslationError> {
        // TODO: Implement translation using the LLM API
        todo!()
    }
}
