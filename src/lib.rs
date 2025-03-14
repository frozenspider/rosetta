#![allow(async_fn_in_trait)]

pub mod generator;
pub mod llm;
pub mod parser;
pub mod utils;

use crate::generator::{Generator, GeneratorBuilder};
use crate::llm::{LLMBuilder, LLM};
use crate::parser::Parser;
use config::Config;
use std::fmt::Display;
use std::fs;
use std::path::Path;

pub async fn translate(
    settings: Config,
    input: &Path,
    output: &Path,
    cfg: TranslationConfig,
    send_progress: impl SendProgress,
) -> Result<(), TranslationError> {
    let parser = parser::pandoc::PandocParser {
        max_section_len: cfg.max_section_len,
        skip_if_present: true
    };

    let api_key = settings
        .get_string("openai.api_key")
        .map_err(|e| TranslationError::OtherError(anyhow::Error::new(e)))?;

    let model =
        settings
        .get_string("openai.model")
        .map_err(|e| TranslationError::OtherError(anyhow::Error::new(e)))?;

    let llm_builder = llm::openai::OpenAiGPTBuilder::new(model, api_key);

    let generator_builder = generator::pandoc::PandocGeneratorBuilder;

    let translator = LlmTranslationService {
        parser,
        llm_builder,
        generator_builder,
        send_progress,
    };

    translator.translate(input, output, cfg).await
}

#[derive(Debug, Clone)]
pub struct TranslationConfig {
    pub src_lang: String,
    pub dst_lang: String,
    pub subject: String,
    pub tone: String,
    pub additional_instructions: String,
    pub continue_translation: bool,
    pub max_section_len: usize,
}

impl Default for TranslationConfig {
    fn default() -> Self {
        TranslationConfig {
            src_lang: "English".to_owned(),
            dst_lang: "Russian".to_owned(),
            subject: "Unknown".to_owned(),
            tone: "formal".to_owned(),
            additional_instructions: "".to_owned(),
            continue_translation: false,
            max_section_len: 5000
        }
    }
}

pub trait TranslationService {
    async fn translate(
        &self,
        input: &Path,
        output: &Path,
        cfg: TranslationConfig,
    ) -> Result<(), TranslationError>;
}

#[derive(Debug)]
pub enum ParseError {
    UnsupportedFormatError { supported_formats: Vec<String> },
    OtherError(anyhow::Error),
}

impl Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParseError::UnsupportedFormatError { supported_formats } => {
                write!(
                    f,
                    "Unsupported format. Supported formats: {:?}",
                    supported_formats
                )
            }
            ParseError::OtherError(e) => {
                write!(f, "{}", e)
            }
        }
    }
}

#[derive(Debug)]
pub enum LLMError {
    ConnectionError(anyhow::Error),
    ApiError(anyhow::Error),
    InteractionError(anyhow::Error),
    OtherError(anyhow::Error),
}

#[derive(Debug)]
pub enum TranslationError {
    ParseError(ParseError),
    IoError(std::io::Error),
    LLMError(LLMError),
    OtherError(anyhow::Error),
}

impl Display for TranslationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TranslationError::ParseError(e) => {
                write!(f, "Parsing failed: {}", e)
            }
            TranslationError::IoError(e) => {
                write!(f, "IO error: {}", e)
            }
            TranslationError::LLMError(LLMError::ConnectionError(e)) => {
                write!(f, "LLM connection error: {}", e)
            }
            TranslationError::LLMError(LLMError::ApiError(e)) => {
                write!(f, "LLM API error: {}", e)
            }
            TranslationError::LLMError(LLMError::InteractionError(e)) => {
                write!(f, "LLM interaction error: {}", e)
            }
            TranslationError::LLMError(LLMError::OtherError(e)) => {
                write!(f, "Unexpected LLM error: {}", e)
            }
            TranslationError::OtherError(e) => {
                write!(f, "Error: {}", e)
            }
        }
    }
}

#[derive(Debug)]
pub enum TranslationStatus {
    Started,
    Progress(Progress),
    Success,
    Error(TranslationError),
}

#[derive(Debug, Clone)]
pub struct Progress {
    pub processed_sections: usize,
    pub total_sections: usize,
}

pub trait SendProgress: Send + Sync {
    fn send_progress(&self, progress: Progress);
}

pub struct DummySendProgress;

impl SendProgress for DummySendProgress {
    fn send_progress(&self, _progress: Progress) {}
}

pub struct LlmTranslationService<P, LB, GB, SP> {
    parser: P,
    llm_builder: LB,
    generator_builder: GB,
    send_progress: SP,
}

impl<P, LB, GB, SP> TranslationService for LlmTranslationService<P, LB, GB, SP>
where
    P: Parser,
    LB: LLMBuilder,
    GB: GeneratorBuilder,
    SP: SendProgress,
{
    async fn translate(
        &self,
        input: &Path,
        output: &Path,
        cfg: TranslationConfig,
    ) -> Result<(), TranslationError> {
        if !input.exists() {
            return Err(TranslationError::IoError(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("File not found: {:?}", input),
            )));
        }
        fs::create_dir_all(output.parent().expect("output parent"))
            .map_err(TranslationError::IoError)?;

        let input_sections = self
            .parser
            .parse(input)
            .await
            .map_err(TranslationError::ParseError)?;
        let total_sections = input_sections.len();

        let (mut gen, already_translated_sections) =
            self.generator_builder.build(output, cfg.continue_translation, cfg.max_section_len).await?;

        {
            let mut already_translated_sections = already_translated_sections.into_iter();
            let llm = self
                .llm_builder
                .build(cfg)
                .await
                .map_err(TranslationError::LLMError)?;

            for (current, section) in input_sections.into_iter().enumerate() {
                let mut prev_translated_section = already_translated_sections.next();

                if prev_translated_section.as_ref().is_some_and(|prev| prev.0.len() != section.0.len()) {
                    log::info!("Section {} has incomplete translation", current);
                    prev_translated_section = None;
                    // Drain the iterator
                    while let Some(_) = already_translated_sections.next() {}
                }

                let translated_section =
                    if let Some(prev_translated_section) = prev_translated_section {
                        log::info!("Section {} already translated", current);
                        prev_translated_section
                    } else {
                        llm
                            .translate(section)
                            .await
                            .map_err(TranslationError::LLMError)?
                    };

                gen.write(translated_section).await?;

                self.send_progress.send_progress(Progress {
                    processed_sections: current + 1,
                    total_sections,
                });
            }
        }

        gen.finalize().await?;

        Ok(())
    }
}
