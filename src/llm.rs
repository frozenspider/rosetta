use crate::{TranslationConfig, TranslationError};

pub trait LLMBuilder {
    type Built: LLM;

    fn build(&self, cfg: TranslationConfig) -> Result<Self::Built, anyhow::Error>;
}

pub trait LLM {
    fn translate(&self, section: String) -> Result<String, TranslationError>;
}
